use discord_vr_core::config;
use std::{io, path::Path};

pub const CONFIG_DIRECTORY: &str = "discord-to-vr";

/// Prefer the previous per-user settings over an older portable copy.
/// Migration errors must not fall back to an unrelated account configuration.
pub fn migrate_config(
    destination: &Path,
    previous_user_config: &Path,
    portable_config: &Path,
) -> io::Result<bool> {
    if destination.try_exists()? {
        return Ok(false);
    }
    let source = if previous_user_config.try_exists()? {
        previous_user_config
    } else {
        portable_config
    };
    if !source.try_exists()? {
        return Ok(false);
    }
    config::migrate_config_at(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn user_settings_take_priority_and_new_settings_are_never_overwritten() {
        let dir = std::env::temp_dir().join(format!(
            "discord-vr-paths-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let previous = dir.join("previous.json");
        let portable = dir.join("portable.json");
        let destination = dir.join("destination.json");
        let document = |id: &str| {
            serde_json::json!({
                "CLIENT_ID":id, "CLIENT_SECRET":"synthetic-secret", "ACCESS_TOKEN":"synthetic-access",
                "PRIVACY_MODE":true, "CUSTOM_SETTING":"keep"
            })
        };
        let original = serde_json::to_vec(&document("123456789")).unwrap();
        fs::write(&previous, &original).unwrap();
        fs::write(
            &portable,
            serde_json::to_vec(&document("987654321")).unwrap(),
        )
        .unwrap();
        assert!(migrate_config(&destination, &previous, &portable).unwrap());
        let migrated = config::load_from_path(&destination, false).unwrap();
        assert_eq!(migrated.client_id, "123456789");
        assert_eq!(migrated.access_token, "synthetic-access");
        assert!(migrated.privacy_mode);
        assert_eq!(fs::read(&previous).unwrap(), original);
        let saved = fs::read(&destination).unwrap();
        let raw: serde_json::Value = serde_json::from_slice(&saved).unwrap();
        assert_eq!(raw["CUSTOM_SETTING"], "keep");
        assert!(raw["CLIENT_SECRET"]
            .as_str()
            .unwrap()
            .starts_with("dpapi:v1:"));
        fs::write(&previous, b"{invalid").unwrap();
        assert!(!migrate_config(&destination, &previous, &portable).unwrap());
        assert_eq!(fs::read(&destination).unwrap(), saved);
        fs::remove_file(&destination).unwrap();
        assert!(migrate_config(&destination, &previous, &portable).is_err());
        assert!(!destination.exists());
        fs::remove_file(&previous).unwrap();
        assert!(migrate_config(&destination, &previous, &portable).unwrap());
        assert_eq!(
            config::load_from_path(&destination, false)
                .unwrap()
                .client_id,
            "987654321"
        );
        for path in [&destination, &portable] {
            fs::remove_file(path).unwrap();
        }
        fs::remove_dir(dir).unwrap();
    }
}
