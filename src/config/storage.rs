use base64::Engine;
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

const PREFIX: &str = "dpapi:v1:";
const SECRET_FIELDS: [&str; 4] = [
    "CLIENT_SECRET",
    "ACCESS_TOKEN",
    "REFRESH_TOKEN",
    "BOT_TOKEN",
];

// DPAPI binds secrets to the current Windows user. Never fall back to plaintext.
fn crypt(data: &[u8], encrypt: bool) -> io::Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len())
            .map_err(|_| io::Error::other("認証情報が大きすぎます"))?,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // Both successful calls allocate output using LocalAlloc.
    let bytes = unsafe {
        let bytes = if output.cbData == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec()
        };
        if !output.pbData.is_null() {
            std::ptr::write_bytes(output.pbData, 0, output.cbData as usize);
            LocalFree(output.pbData as *mut _);
        }
        bytes
    };
    Ok(bytes)
}

pub(super) fn decode_secret(value: &str) -> io::Result<String> {
    let Some(encoded) = value.strip_prefix(PREFIX) else {
        return Ok(value.to_string());
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| io::Error::other("暗号化された認証情報の形式が不正です"))?;
    String::from_utf8(crypt(&bytes, false)?)
        .map_err(|_| io::Error::other("認証情報を復号できませんでした"))
}

pub(super) fn protect_secrets(config: &mut Value) -> io::Result<()> {
    for field in SECRET_FIELDS {
        if let Some(value) = config.get(field).and_then(Value::as_str) {
            if !value.is_empty() && !value.starts_with(PREFIX) {
                let protected = crypt(value.as_bytes(), true)?;
                config[field] = Value::String(format!(
                    "{PREFIX}{}",
                    base64::engine::general_purpose::STANDARD.encode(protected)
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn needs_protection(config: &Value) -> bool {
    SECRET_FIELDS.iter().any(|field| {
        config
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty() && !s.starts_with(PREFIX))
    })
}

/// Write alongside the original, flush, then atomically replace it on Windows.
pub(super) fn atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    let temp = path.with_file_name(format!(".config-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(content)?;
        file.sync_all()?;
        drop(file);
        let source: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_round_trip_and_are_not_plaintext() {
        let mut value = serde_json::json!({
            "CLIENT_SECRET": "test-secret", "ACCESS_TOKEN": "test-access",
            "REFRESH_TOKEN": "test-refresh", "BOT_TOKEN": "test-bot", "EXTRA_SETTING": true
        });
        protect_secrets(&mut value).unwrap();
        assert!(!needs_protection(&value));
        for (field, plain) in [
            ("CLIENT_SECRET", "test-secret"),
            ("ACCESS_TOKEN", "test-access"),
            ("REFRESH_TOKEN", "test-refresh"),
            ("BOT_TOKEN", "test-bot"),
        ] {
            let stored = value[field].as_str().unwrap();
            assert!(stored.starts_with(PREFIX));
            assert_ne!(stored, plain);
            assert_eq!(decode_secret(stored).unwrap(), plain);
        }
        assert_eq!(value["EXTRA_SETTING"], true);
        let once = value.clone();
        protect_secrets(&mut value).unwrap();
        assert_eq!(once, value);
    }

    #[test]
    fn corrupt_ciphertext_is_rejected() {
        assert!(decode_secret("dpapi:v1:not-base64").is_err());
        assert!(decode_secret("dpapi:v1:YWJj").is_err());
    }
}
