use crate::app::AppState;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

const CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);
const REPOSITORY_PATH: &str = "/Yumiiiiiiiiii/discord_To_VR/releases/download/";

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Waiting,
    Installing,
    Error,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    current_version: String,
    pub auto_update: bool,
    supported: bool,
    can_check: bool,
    disabled_reason: Option<String>,
    phase: Phase,
    version: Option<String>,
    notes: Option<String>,
    downloaded: u64,
    total: Option<u64>,
    checked_at_ms: Option<u64>,
    message: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    auto_update: bool,
}
#[derive(Clone)]
pub struct UpdateManager {
    status: Arc<Mutex<UpdateStatus>>,
    operation: Arc<tokio::sync::Mutex<()>>,
    preference_write: Arc<Mutex<()>>,
    path: Arc<PathBuf>,
    network_enabled: bool,
}
impl UpdateManager {
    pub fn new(path: PathBuf, preview: bool, debug: bool) -> Self {
        let network_enabled = !preview && !debug && !cfg!(debug_assertions);
        let (auto_update, problem) = if preview {
            (true, None)
        } else {
            match std::fs::read(&path) {
                Ok(bytes) => match serde_json::from_slice::<Preferences>(&bytes) {
                    Ok(prefs) => (prefs.auto_update, None),
                    Err(_) => (false, Some("更新設定を読めませんでした。自動更新を切り替えて設定を保存し直してください。".into())),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (true, None),
                Err(_) => (false, Some("更新設定を読めませんでした。保存先を確認してください。".into())),
            }
        };
        let supported = network_enabled && installed_executable();
        let disabled_reason = if preview {
            Some("画面プレビューでは更新の通信・適用を行いません。".into())
        } else if debug {
            Some("-debug では Discord の受信テストだけを行います。更新は通常起動で確認してください。".into())
        } else if cfg!(debug_assertions) {
            Some("開発ビルドでは更新を適用しません。配布版を使用してください。".into())
        } else if !supported {
            Some(
                "自動適用にはインストーラー版が必要です。ZIP 版では更新の確認だけを行います。"
                    .into(),
            )
        } else {
            None
        };
        Self {
            status: Arc::new(Mutex::new(UpdateStatus {
                current_version: env!("CARGO_PKG_VERSION").into(),
                auto_update,
                supported,
                can_check: network_enabled,
                disabled_reason,
                phase: Phase::Idle,
                version: None,
                notes: None,
                downloaded: 0,
                total: None,
                checked_at_ms: None,
                message: problem,
            })),
            operation: Arc::new(tokio::sync::Mutex::new(())),
            preference_write: Arc::new(Mutex::new(())),
            path: Arc::new(path),
            network_enabled,
        }
    }
    fn mutate(&self, action: impl FnOnce(&mut UpdateStatus)) {
        action(&mut self.status.lock().unwrap_or_else(|e| e.into_inner()));
    }
    pub fn snapshot(&self) -> UpdateStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn report_migration_failure(&self) {
        self.mutate(|s| {
            s.auto_update = false;
            s.phase = Phase::Error;
            s.message = Some("以前の更新設定を移行できなかったため、自動更新を停止しました。保存先を確認し、自動アップデートを切り替えて設定を保存してください。旧ファイルは変更していません。".into());
        });
    }
    fn fail(&self, message: &str) {
        self.mutate(|s| {
            s.phase = Phase::Error;
            s.message = Some(message.into());
        });
    }
    fn save_preference(&self, enabled: bool) -> Result<(), String> {
        let _write = self
            .preference_write
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.snapshot().phase == Phase::Installing {
            return Err("更新の適用中は切り替えできません。".into());
        }
        if self.network_enabled {
            let bytes = serde_json::to_vec_pretty(&Preferences {
                auto_update: enabled,
            })
            .unwrap();
            let temporary = self.path.with_extension("updates.tmp");
            // This file contains one non-sensitive preference; never touches config.json.
            std::fs::write(&temporary, bytes)
                .and_then(|_| replace_preference(&temporary, &self.path))
                .map_err(|_| {
                    "更新設定を保存できませんでした。保存先を確認してください。".to_string()
                })?;
        }
        self.mutate(|s| s.auto_update = enabled);
        Ok(())
    }
}

pub fn migrate_preferences(
    previous: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<bool> {
    if destination.try_exists()? {
        return Ok(false);
    }
    let bytes = match std::fs::read(previous) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    serde_json::from_slice::<Preferences>(&bytes)
        .map_err(|_| std::io::Error::other("更新設定の形式が不正です"))?;
    let temporary = destination.with_extension("updates.tmp");
    std::fs::write(&temporary, &bytes)?;
    replace_preference(&temporary, destination)?;
    Ok(true)
}

fn replace_preference(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// NSIS remembers the actual installation directory. Never install from a copied ZIP executable.
fn installed_executable() -> bool {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Discord to VR\0"
        .encode_utf16()
        .collect();
    let value: Vec<u16> = "InstallLocation\0".encode_utf16().collect();
    let mut buffer = [0u16; 32768];
    let mut length = std::mem::size_of_val(&buffer) as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut length,
        )
    };
    if result != 0 {
        return false;
    }
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    let installed = PathBuf::from(String::from_utf16_lossy(&buffer[..end]).trim_matches('"'))
        .join("discord_To_VR.exe");
    match (
        installed.canonicalize(),
        std::env::current_exe().and_then(|p| p.canonicalize()),
    ) {
        (Ok(installed), Ok(current)) => installed == current,
        _ => false,
    }
}

fn allowed_download(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path().starts_with(REPOSITORY_PATH)
        && url.path().ends_with("-setup.exe")
}

async fn check(app: &AppHandle, manager: &UpdateManager) -> Result<Option<Update>, String> {
    manager.mutate(|s| {
        s.phase = Phase::Checking;
        s.message = None;
        s.version = None;
        s.notes = None;
        s.downloaded = 0;
        s.total = None;
    });
    let updater = app
        .updater_builder()
        .timeout(Duration::from_secs(30))
        .configure_client(|client| {
            client
                .https_only(true)
                .connect_timeout(Duration::from_secs(10))
        })
        .build()
        .map_err(|_| "更新の準備ができませんでした。")?;
    let result = updater.check().await;
    manager.mutate(|s| {
        s.checked_at_ms = Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        )
    });
    match result {
        Ok(Some(update)) if allowed_download(&update.download_url) => {
            manager.mutate(|s| { s.phase=Phase::Available; s.version=Some(update.version.clone());
                s.notes=update.body.as_ref().map(|body| body.chars().take(4000).collect()); });
            Ok(Some(update))
        }
        Ok(Some(_)) => Err("更新ファイルの配布先が正しくないため適用しませんでした。".into()),
        Ok(None) => { manager.mutate(|s|s.phase=Phase::Current); Ok(None) },
        Err(tauri_plugin_updater::Error::ReleaseNotFound) => Err("更新情報がまだ公開されていないか、配布先を利用できません。現在のアプリはそのまま使えます。".into()),
        Err(_) => Err("更新を確認できませんでした。通信環境を確認して、あとで再試行してください。".into()),
    }
}

async fn run_check(app: AppHandle, manager: UpdateManager) -> Result<(), String> {
    if !manager.network_enabled {
        return Err(manager.snapshot().disabled_reason.unwrap_or_default());
    }
    let _operation = manager
        .operation
        .try_lock()
        .map_err(|_| "更新処理が進行中です。".to_string())?;
    let result: Result<(), String> = async {
        let Some(mut update) = check(&app, &manager).await? else { return Ok(()); };
        let status=manager.snapshot();
        if !status.auto_update || !status.supported { return Ok(()); }
        update.timeout=Some(Duration::from_secs(300));
        manager.mutate(|s| s.phase=Phase::Downloading);
        let bytes = update.download(|chunk,total| manager.mutate(|s| {
            s.downloaded=s.downloaded.saturating_add(chunk as u64); s.total=total;
        }), || {}).await.map_err(|_| "更新ファイルのダウンロードまたは署名・バージョン検証に失敗しました。現在のアプリは変更していません。".to_string())?;
        manager.mutate(|s| s.phase=Phase::Waiting);
        loop {
            if !manager.snapshot().auto_update { manager.mutate(|s|s.phase=Phase::Available); return Ok(()); }
            if app.state::<AppState>().prepare_update().await { break; }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        let apply = {
            let _write = manager.preference_write.lock().unwrap_or_else(|e| e.into_inner());
            let enabled = manager.snapshot().auto_update;
            manager.mutate(|s| s.phase=if enabled { Phase::Installing } else { Phase::Available });
            enabled
        };
        if !apply {
            app.state::<AppState>().cancel_update(&app).await;
            return Ok(());
        }
        // Windows updater starts NSIS, exits this process, and restarts the installed app.
        let result = tauri::async_runtime::spawn_blocking(move || update.install(bytes)).await;
        if !matches!(result, Ok(Ok(()))) {
            app.state::<AppState>().cancel_update(&app).await;
            return Err("更新インストーラーを起動できませんでした。通知の受信を再開しました。".into());
        }
        Ok(())
    }.await;
    if let Err(message) = &result {
        manager.fail(message);
    }
    result
}

pub fn start(app: AppHandle, manager: UpdateManager) {
    if !manager.network_enabled {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
        let mut next_check = tokio::time::Instant::now();
        let mut previously_enabled = false;
        loop {
            let enabled = manager.snapshot().auto_update;
            if enabled && (!previously_enabled || tokio::time::Instant::now() >= next_check) {
                let _ = run_check(app.clone(), manager.clone()).await;
                next_check = tokio::time::Instant::now() + CHECK_INTERVAL;
            }
            previously_enabled = enabled;
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

#[tauri::command]
pub fn get_update_status(manager: State<'_, UpdateManager>) -> UpdateStatus {
    manager.snapshot()
}
#[tauri::command]
pub async fn check_updates(
    app: AppHandle,
    manager: State<'_, UpdateManager>,
) -> Result<(), String> {
    run_check(app, manager.inner().clone()).await
}
#[tauri::command]
pub fn set_auto_update(enabled: bool, manager: State<'_, UpdateManager>) -> Result<(), String> {
    manager.save_preference(enabled)
}

#[cfg(test)]
#[path = "updater_tests.rs"]
mod signed_download_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preference_migration_preserves_opt_out_and_does_not_enable_updates_on_failure() {
        let dir = std::env::temp_dir().join(format!(
            "discord-vr-update-migrate-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let previous = dir.join("previous.json");
        let destination = dir.join("updates.json");
        let original = br#"{"auto_update":false}"#;
        std::fs::write(&previous, original).unwrap();
        assert!(migrate_preferences(&previous, &destination).unwrap());
        assert_eq!(std::fs::read(&previous).unwrap(), original);
        assert!(
            !UpdateManager::new(destination.clone(), false, false)
                .snapshot()
                .auto_update
        );
        std::fs::write(&destination, br#"{"auto_update":true}"#).unwrap();
        assert!(!migrate_preferences(&previous, &destination).unwrap());
        assert!(
            UpdateManager::new(destination.clone(), false, false)
                .snapshot()
                .auto_update
        );
        std::fs::remove_file(&destination).unwrap();
        std::fs::write(&previous, b"{invalid").unwrap();
        assert!(migrate_preferences(&previous, &destination).is_err());
        assert!(!destination.exists());
        let manager = UpdateManager::new(destination, false, false);
        manager.report_migration_failure();
        assert!(!manager.snapshot().auto_update);
        assert!(manager.snapshot().phase == Phase::Error);
        std::fs::remove_file(previous).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn update_urls_only_allow_this_repositories_https_installers() {
        assert!(allowed_download(&reqwest::Url::parse("https://github.com/Yumiiiiiiiiii/discord_To_VR/releases/download/v0.2.1/Discord%20to%20VR_0.2.1_x64-setup.exe").unwrap()));
        for url in ["http://github.com/Yumiiiiiiiiii/discord_To_VR/releases/download/v0.2.1/a-setup.exe",
            "https://github.com/other/repo/releases/download/v0.2.1/a-setup.exe",
            "https://github.com.evil.example/Yumiiiiiiiiii/discord_To_VR/releases/download/v0.2.1/a-setup.exe",
            "https://user:secret@github.com/Yumiiiiiiiiii/discord_To_VR/releases/download/v0.2.1/a-setup.exe",
            "https://github.com/Yumiiiiiiiiii/discord_To_VR/releases/download/v0.2.1/a.zip"] {
            assert!(!allowed_download(&reqwest::Url::parse(url).unwrap()));
        }
    }
    #[test]
    fn preview_and_receive_debug_cannot_start_update_network_or_installation() {
        for (preview, debug) in [(true, false), (false, true)] {
            let path =
                std::env::temp_dir().join(format!("update-preference-{}.json", std::process::id()));
            let manager = UpdateManager::new(path.clone(), preview, debug);
            assert!(!manager.network_enabled);
            assert!(!manager.snapshot().supported);
            manager.save_preference(false).unwrap();
            assert!(!manager.snapshot().auto_update);
            assert!(!path.exists());
        }
    }
}
