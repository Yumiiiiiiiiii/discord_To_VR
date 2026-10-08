mod storage;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::{Mutex, MutexGuard, OnceLock};

// UI mode changes and OAuth refresh run on different threads. Keep every
// read/modify/write transaction together so neither discards the other's data.
static WRITE_LOCK: Mutex<()> = Mutex::new(());
static CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Desktop hosts can use their per-user application directory, including OAuth writes.
pub fn set_config_path(path: PathBuf) -> io::Result<()> {
    CONFIG_PATH
        .set(path)
        .map_err(|_| io::Error::other("設定の保存先は既に指定されています。"))
}

pub fn migrate_config_at(previous: &Path, destination: &Path) -> io::Result<bool> {
    let _guard = write_lock()?;
    if destination.exists() || !previous.exists() {
        return Ok(false);
    }
    let mut raw = read_document(previous)?;
    decode_config(&raw)?;
    storage::protect_secrets(&mut raw)?;
    storage::atomic_write(destination, &serde_json::to_vec_pretty(&raw)?)?;
    Ok(true)
}
fn write_lock() -> io::Result<MutexGuard<'static, ()>> {
    WRITE_LOCK
        .lock()
        .map_err(|_| io::Error::other("設定保存の状態を確認できません。再起動してください。"))
}

const DEFAULT_CLIENT_ID: &str = "YOUR_CLIENT_ID_HERE";
const DEFAULT_CLIENT_SECRET: &str = "YOUR_CLIENT_SECRET_HERE";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

/// Debug is deliberately omitted: this struct contains decrypted credentials.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(rename = "THEME")]
    pub theme: ThemePreference,
    #[serde(rename = "CLIENT_ID")]
    pub client_id: String,
    #[serde(rename = "CLIENT_SECRET")]
    pub client_secret: String,
    #[serde(rename = "REDIRECT_URI")]
    pub redirect_uri: String,
    #[serde(rename = "NOTIFICATION_SOUND")]
    pub notification_sound: String,
    #[serde(rename = "NOTIFICATION_VOLUME")]
    pub notification_volume: f64,
    #[serde(rename = "NOTIFICATION_TIMEOUT")]
    pub notification_timeout: f64,
    #[serde(rename = "MAX_TITLE_LENGTH")]
    pub max_title_length: usize,
    #[serde(rename = "MAX_CONTENT_LENGTH")]
    pub max_content_length: usize,
    #[serde(rename = "PRIVACY_MODE")]
    pub privacy_mode: bool,
    #[serde(rename = "LOG_MESSAGE_CONTENT")]
    pub log_message_content: bool,
    #[serde(rename = "AUTO_EXIT_WITH_VR")]
    pub auto_exit_with_vr: bool,
    #[serde(rename = "WHITELIST_CHANNEL_IDS")]
    pub whitelist_channel_ids: Vec<String>,
    #[serde(rename = "RECEIVE_MODE")]
    pub receive_mode: crate::discord::receive::ReceiveMode,
    #[serde(rename = "BOT_TOKEN")]
    pub bot_token: String,
    #[serde(rename = "BOT_GUILD_ID")]
    pub bot_guild_id: String,
    #[serde(rename = "BOT_USER_ID")]
    pub bot_user_id: String,
    #[serde(rename = "ACCESS_TOKEN")]
    pub access_token: String,
    #[serde(rename = "REFRESH_TOKEN")]
    pub refresh_token: String,
    #[serde(
        rename = "ONBOARDING_COMPLETED",
        skip_serializing_if = "Option::is_none"
    )]
    pub onboarding_completed: Option<bool>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            client_id: DEFAULT_CLIENT_ID.to_string(),
            client_secret: DEFAULT_CLIENT_SECRET.to_string(),
            redirect_uri: "http://localhost/".to_string(),
            notification_sound: String::new(),
            notification_volume: 0.0,
            notification_timeout: 5.0,
            max_title_length: 35,
            max_content_length: 70,
            privacy_mode: false,
            log_message_content: false,
            auto_exit_with_vr: true,
            whitelist_channel_ids: Vec::new(),
            receive_mode: Default::default(),
            bot_token: String::new(),
            bot_guild_id: String::new(),
            bot_user_id: String::new(),
            access_token: String::new(),
            refresh_token: String::new(),
            onboarding_completed: None,
        }
    }
}

impl Config {
    pub fn needs_onboarding(&self) -> bool {
        !self
            .onboarding_completed
            .unwrap_or_else(|| self.validate().is_ok())
    }
    pub fn bot_configured(&self) -> bool {
        !self.bot_token.is_empty() && valid_id(&self.bot_guild_id) && valid_id(&self.bot_user_id)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.client_id == DEFAULT_CLIENT_ID
            || self.client_id.is_empty()
            || !self.client_id.bytes().all(|c| c.is_ascii_digit())
        {
            return Err("CLIENT_ID に Developer Portal の数字の ID を入力してください。");
        }
        if self.client_secret.trim().is_empty() || self.client_secret == DEFAULT_CLIENT_SECRET {
            return Err("CLIENT_SECRET が未設定です。");
        }
        let url = reqwest::Url::parse(&self.redirect_uri)
            .map_err(|_| "REDIRECT_URI が正しい URL ではありません。")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "REDIRECT_URI はユーザー情報・フラグメントを含まない HTTP(S) URL にしてください。",
            );
        }
        if !self.notification_volume.is_finite() || !(0.0..=1.0).contains(&self.notification_volume)
        {
            return Err("NOTIFICATION_VOLUME は 0.0 ～ 1.0 にしてください。");
        }
        if !self.notification_timeout.is_finite()
            || !(1.0..=30.0).contains(&self.notification_timeout)
        {
            return Err("NOTIFICATION_TIMEOUT は 1.0 ～ 30.0 秒にしてください。");
        }
        if !(10..=200).contains(&self.max_title_length) {
            return Err("MAX_TITLE_LENGTH は 10 ～ 200 にしてください。");
        }
        if !(10..=2000).contains(&self.max_content_length) {
            return Err("MAX_CONTENT_LENGTH は 10 ～ 2000 にしてください。");
        }
        let mut unique = std::collections::HashSet::new();
        if self.whitelist_channel_ids.len() > 32
            || self
                .whitelist_channel_ids
                .iter()
                .any(|id| !valid_id(id) || !unique.insert(id))
        {
            return Err("WHITELIST_CHANNEL_IDS は重複しない数字のチャンネル ID を32件以内で指定してください。");
        }
        if !self.bot_token.is_empty()
            && (self.bot_token.len() > 512 || !self.bot_token.bytes().all(|c| c.is_ascii_graphic()))
        {
            return Err("BOT_TOKEN に空白を含まない Bot Token を入力してください。");
        }
        if (!self.bot_guild_id.is_empty()
            || self.receive_mode == crate::discord::receive::ReceiveMode::Auto)
            && !valid_id(&self.bot_guild_id)
        {
            return Err(
                "BOT_GUILD_ID に Bot と自分が参加しているサーバーの数字の ID を入力してください。",
            );
        }
        if (!self.bot_user_id.is_empty()
            || self.receive_mode == crate::discord::receive::ReceiveMode::Auto)
            && !valid_id(&self.bot_user_id)
        {
            return Err("BOT_USER_ID に自分の数字のユーザー ID を入力してください。");
        }
        if self.receive_mode == crate::discord::receive::ReceiveMode::Auto
            && self.bot_token.is_empty()
        {
            return Err("BOT_TOKEN に自動判別用 Bot の Token を入力してください。");
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('0')
        && id.len() <= 20
        && id.bytes().all(|c| c.is_ascii_digit())
        && id.parse::<u64>().is_ok()
}

pub fn safe_exit(code: i32) -> ! {
    if io::stdin().is_terminal() {
        println!("\nEnterキーを押すと終了します...");
        let _ = io::stdin().read_line(&mut String::new());
    }
    process::exit(code);
}

pub fn get_config_path() -> PathBuf {
    if let Some(path) = CONFIG_PATH.get() {
        return path.clone();
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("config.json")))
        .unwrap_or_else(|| PathBuf::from("config.json"))
}

fn read_document(path: &Path) -> io::Result<Value> {
    let content = fs::read_to_string(path)?;
    let raw: Value = serde_json::from_str(content.trim_start_matches('\u{feff}')).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "JSON の形式が不正です（行 {}、列 {}）。",
                e.line(),
                e.column()
            ),
        )
    })?;
    if !raw.is_object() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "設定は JSON オブジェクトで指定してください。",
        ));
    }
    Ok(raw)
}

fn decode_config(raw: &Value) -> io::Result<Config> {
    let mut config: Config = serde_json::from_value(raw.clone()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "設定値の型が不正です。設定例を確認してください。",
        )
    })?;
    config.client_secret = storage::decode_secret(&config.client_secret)?;
    config.access_token = storage::decode_secret(&config.access_token)?;
    config.refresh_token = storage::decode_secret(&config.refresh_token)?;
    config.bot_token = storage::decode_secret(&config.bot_token)?;
    config
        .validate()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(config)
}

/// A check is read-only. Normal startup migrates plaintext only after validation.
pub fn load_from_path(path: &Path, protect: bool) -> io::Result<Config> {
    let _guard = if protect { Some(write_lock()?) } else { None };
    let mut raw = read_document(path)?;
    let config = decode_config(&raw)?;
    if protect && storage::needs_protection(&raw) {
        storage::protect_secrets(&mut raw)?;
        storage::atomic_write(path, &serde_json::to_vec_pretty(&raw)?)?;
        println!("🔐 認証情報を Windows ユーザーに紐づく暗号化形式で保存しました。");
    }
    Ok(config)
}

pub fn load_or_init() -> Config {
    let path = get_config_path();
    println!("📁 設定ファイル: {}", path.display());
    if !path.exists() {
        match crate::system::setup::run(&path) {
            Ok(true) => {}
            Ok(false) => process::exit(0),
            Err(e) => {
                crate::system::report_error(&format!("初回設定を完了できませんでした: {e}"));
                safe_exit(1);
            }
        }
    }
    match load_from_path(&path, true) {
        Ok(config) => config,
        Err(e) => {
            crate::system::report_error(&format!("設定を読み込めませんでした: {e}\n暗号化済みの設定は、保存した Windows ユーザーで開いてください。"));
            safe_exit(1);
        }
    }
}

/// Setup reads existing values without migrating or overwriting the document.
pub fn setup_defaults(path: &Path) -> io::Result<Config> {
    match read_document(path) {
        Ok(raw) => {
            let mut config: Config = serde_json::from_value(raw).map_err(|_| {
                io::Error::other("設定値の型が不正です。config.json を確認してください。")
            })?;
            config.client_secret = storage::decode_secret(&config.client_secret)?;
            config.access_token = storage::decode_secret(&config.access_token)?;
            config.refresh_token = storage::decode_secret(&config.refresh_token)?;
            config.bot_token = storage::decode_secret(&config.bot_token)?;
            Ok(config)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e),
    }
}

pub fn save_setup_at(path: &Path, config: &Config) -> io::Result<()> {
    let _guard = write_lock()?;
    config.validate().map_err(io::Error::other)?;
    let previous = setup_defaults(path)?;
    let mut raw = match read_document(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => serde_json::to_value(Config::default())?,
        Err(e) => return Err(e),
    };
    raw["CLIENT_ID"] = Value::String(config.client_id.clone());
    raw["CLIENT_SECRET"] = Value::String(config.client_secret.clone());
    raw["REDIRECT_URI"] = Value::String(config.redirect_uri.clone());
    raw["PRIVACY_MODE"] = Value::Bool(config.privacy_mode);
    raw["THEME"] = serde_json::to_value(config.theme)?;
    raw["NOTIFICATION_TIMEOUT"] = Value::from(config.notification_timeout);
    raw["NOTIFICATION_VOLUME"] = Value::from(config.notification_volume);
    raw["NOTIFICATION_SOUND"] = Value::String(config.notification_sound.clone());
    raw["MAX_TITLE_LENGTH"] = Value::from(config.max_title_length);
    raw["MAX_CONTENT_LENGTH"] = Value::from(config.max_content_length);
    raw["AUTO_EXIT_WITH_VR"] = Value::Bool(config.auto_exit_with_vr);
    raw["WHITELIST_CHANNEL_IDS"] = serde_json::to_value(&config.whitelist_channel_ids)?;
    raw["RECEIVE_MODE"] = serde_json::to_value(config.receive_mode)?;
    raw["BOT_TOKEN"] = Value::String(config.bot_token.clone());
    raw["BOT_GUILD_ID"] = Value::String(config.bot_guild_id.clone());
    raw["BOT_USER_ID"] = Value::String(config.bot_user_id.clone());
    if let Some(completed) = config.onboarding_completed {
        raw["ONBOARDING_COMPLETED"] = Value::Bool(completed);
    }
    if previous.client_id != config.client_id
        || previous.client_secret != config.client_secret
        || previous.redirect_uri != config.redirect_uri
    {
        raw["ACCESS_TOKEN"] = Value::String(String::new());
        raw["REFRESH_TOKEN"] = Value::String(String::new());
    }
    storage::protect_secrets(&mut raw)?;
    storage::atomic_write(path, &serde_json::to_vec_pretty(&raw)?)
}

pub fn save_tokens(access_token: &str, refresh_token: &str) -> io::Result<()> {
    save_tokens_at(&get_config_path(), access_token, refresh_token)
}

fn save_tokens_at(path: &Path, access_token: &str, refresh_token: &str) -> io::Result<()> {
    let _guard = write_lock()?;
    if access_token.is_empty() {
        return Err(io::Error::other("空のアクセストークンは保存しません。"));
    }
    // Never reset a missing/invalid document to defaults or discard unknown settings.
    let mut raw = read_document(path)?;
    decode_config(&raw)?;
    raw["ACCESS_TOKEN"] = Value::String(access_token.to_string());
    if !refresh_token.is_empty() {
        raw["REFRESH_TOKEN"] = Value::String(refresh_token.to_string());
    }
    storage::protect_secrets(&mut raw)?;
    storage::atomic_write(path, &serde_json::to_vec_pretty(&raw)?)?;
    println!("💾 認証情報を暗号化して保存しました。");
    Ok(())
}

pub fn save_receive_mode_at(
    path: &Path,
    mode: crate::discord::receive::ReceiveMode,
) -> io::Result<()> {
    let _guard = write_lock()?;
    let mut raw = read_document(path)?;
    decode_config(&raw)?;
    raw["RECEIVE_MODE"] = serde_json::to_value(mode)?;
    decode_config(&raw)?;
    storage::protect_secrets(&mut raw)?;
    storage::atomic_write(path, &serde_json::to_vec_pretty(&raw)?)
}

pub fn complete_onboarding_at(path: &Path) -> io::Result<()> {
    let _guard = write_lock()?;
    let mut raw = read_document(path)?;
    decode_config(&raw)?;
    raw["ONBOARDING_COMPLETED"] = Value::Bool(true);
    storage::protect_secrets(&mut raw)?;
    storage::atomic_write(path, &serde_json::to_vec_pretty(&raw)?)
}

pub fn save_privacy_at(path: &Path, enabled: bool) -> io::Result<()> {
    let _guard = write_lock()?;
    let mut raw = read_document(path)?;
    decode_config(&raw)?;
    raw["PRIVACY_MODE"] = Value::Bool(enabled);
    storage::protect_secrets(&mut raw)?;
    storage::atomic_write(path, &serde_json::to_vec_pretty(&raw)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboarding_resumes_until_completed_and_preserves_existing_configuration() {
        assert!(Config::default().needs_onboarding());
        let legacy = valid_config();
        assert!(!legacy.needs_onboarding());
        let path = std::env::temp_dir().join(format!(
            "discord-vr-onboarding-{}.json",
            uuid::Uuid::new_v4()
        ));
        let cfg = Config {
            onboarding_completed: Some(false),
            ..legacy
        };
        save_setup_at(&path, &cfg).unwrap();
        assert!(load_from_path(&path, false).unwrap().needs_onboarding());
        save_tokens_at(&path, "dummy-access", "dummy-refresh").unwrap();
        complete_onboarding_at(&path).unwrap();
        let saved = load_from_path(&path, false).unwrap();
        assert!(!saved.needs_onboarding());
        assert_eq!(saved.access_token, "dummy-access");
        assert_eq!(saved.refresh_token, "dummy-refresh");
        assert_eq!(saved.client_secret, cfg.client_secret);
        let original = b"{broken";
        fs::write(&path, original).unwrap();
        assert!(complete_onboarding_at(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_file(path).unwrap();
    }

    fn valid_config() -> Config {
        Config {
            client_id: "123456789".into(),
            client_secret: "test-secret".into(),
            ..Config::default()
        }
    }

    #[test]
    fn migration_preserves_tokens_and_extensions_without_overwriting() {
        let directory =
            std::env::temp_dir().join(format!("discord-vr-migrate-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let previous = directory.join("old.json");
        let destination = directory.join("new.json");
        let cfg = Config {
            access_token: "migration-access".into(),
            refresh_token: "migration-refresh".into(),
            privacy_mode: true,
            ..valid_config()
        };
        let mut raw = serde_json::to_value(&cfg).unwrap();
        raw["CUSTOM_EXTENSION"] = serde_json::json!({"enabled": true});
        let original = serde_json::to_vec(&raw).unwrap();
        fs::write(&previous, &original).unwrap();
        assert!(migrate_config_at(&previous, &destination).unwrap());
        assert_eq!(fs::read(&previous).unwrap(), original);
        let migrated = load_from_path(&destination, false).unwrap();
        assert_eq!(migrated.client_secret, cfg.client_secret);
        assert_eq!(migrated.access_token, cfg.access_token);
        assert_eq!(migrated.refresh_token, cfg.refresh_token);
        assert!(migrated.privacy_mode);
        let document = read_document(&destination).unwrap();
        assert_eq!(document["CUSTOM_EXTENSION"], raw["CUSTOM_EXTENSION"]);
        assert!(!storage::needs_protection(&document));
        let saved = fs::read(&destination).unwrap();
        fs::write(&previous, b"{broken").unwrap();
        assert!(!migrate_config_at(&previous, &destination).unwrap());
        assert_eq!(fs::read(&destination).unwrap(), saved);
        fs::remove_file(&destination).unwrap();
        assert!(migrate_config_at(&previous, &destination).is_err());
        assert!(!destination.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn mode_changes_survive_token_refresh_and_reject_broken_settings() {
        let path =
            std::env::temp_dir().join(format!("discord-vr-mode-{}.json", uuid::Uuid::new_v4()));
        save_setup_at(&path, &valid_config()).unwrap();
        let mut raw = read_document(&path).unwrap();
        raw["EXTRA_SETTING"] = Value::from("keep");
        fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        let token_path = path.clone();
        let tokens = std::thread::spawn(move || {
            for _ in 0..20 {
                save_tokens_at(&token_path, "new-access", "new-refresh").unwrap();
            }
        });
        for _ in 0..20 {
            save_privacy_at(&path, false).unwrap();
            save_privacy_at(&path, true).unwrap();
        }
        tokens.join().unwrap();
        let saved = load_from_path(&path, false).unwrap();
        assert!(saved.privacy_mode);
        assert_eq!(saved.access_token, "new-access");
        assert_eq!(saved.refresh_token, "new-refresh");
        assert_eq!(read_document(&path).unwrap()["EXTRA_SETTING"], "keep");
        assert!(!storage::needs_protection(&read_document(&path).unwrap()));
        fs::write(&path, b"{broken").unwrap();
        assert!(save_privacy_at(&path, false).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{broken");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn bot_token_and_receive_mode_round_trip_without_losing_oauth_credentials() {
        use crate::discord::receive::ReceiveMode;
        let mut cfg = valid_config();
        cfg.bot_token = "dummy-bot-token".into();
        cfg.bot_guild_id = "300".into();
        cfg.bot_user_id = "200".into();
        let path =
            std::env::temp_dir().join(format!("discord-vr-bot-{}.json", uuid::Uuid::new_v4()));
        save_setup_at(&path, &cfg).unwrap();
        assert!(!fs::read_to_string(&path)
            .unwrap()
            .contains("dummy-bot-token"));
        save_tokens_at(&path, "dummy-access", "dummy-refresh").unwrap();
        save_receive_mode_at(&path, ReceiveMode::Auto).unwrap();
        let saved = load_from_path(&path, false).unwrap();
        assert_eq!(saved.receive_mode, ReceiveMode::Auto);
        assert_eq!(saved.bot_token, cfg.bot_token);
        assert_eq!(saved.access_token, "dummy-access");
        assert_eq!(saved.refresh_token, "dummy-refresh");
        cfg.bot_token.clear();
        cfg.receive_mode = ReceiveMode::Auto;
        assert!(cfg.validate().is_err());
        cfg.receive_mode = ReceiveMode::Normal;
        save_setup_at(&path, &cfg).unwrap();
        assert!(!load_from_path(&path, false).unwrap().bot_configured());
        assert!(save_receive_mode_at(&path, ReceiveMode::Auto).is_err());
        assert_eq!(
            load_from_path(&path, false).unwrap().receive_mode,
            ReceiveMode::Normal
        );
        let mut raw = read_document(&path).unwrap();
        raw["RECEIVE_MODE"] = Value::String("broken".into());
        fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        assert!(save_receive_mode_at(&path, ReceiveMode::Normal).is_err());
        assert_eq!(read_document(&path).unwrap()["RECEIVE_MODE"], "broken");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn whitelist_validates_ids_and_survives_secret_and_mode_updates() {
        let mut cfg = valid_config();
        cfg.whitelist_channel_ids = vec!["123456789012345678".into(), "234567890123456789".into()];
        let path = std::env::temp_dir().join(format!(
            "discord-vr-whitelist-{}.json",
            uuid::Uuid::new_v4()
        ));
        save_setup_at(&path, &cfg).unwrap();
        save_tokens_at(&path, "dummy-access", "dummy-refresh").unwrap();
        save_privacy_at(&path, true).unwrap();
        assert_eq!(
            load_from_path(&path, false).unwrap().whitelist_channel_ids,
            cfg.whitelist_channel_ids
        );
        for invalid in [
            vec![""],
            vec!["0"],
            vec!["abc"],
            vec!["18446744073709551616"],
            vec!["123", "123"],
        ] {
            cfg.whitelist_channel_ids = invalid.into_iter().map(str::to_string).collect();
            assert!(cfg.validate().is_err());
        }
        cfg.whitelist_channel_ids = (1..=33).map(|id| id.to_string()).collect();
        assert!(cfg.validate().is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn invalid_values_are_rejected() {
        let mut config = valid_config();
        assert!(config.validate().is_ok());
        config.notification_volume = 1.1;
        assert!(config.validate().is_err());
        config.notification_volume = 0.0;
        config.notification_timeout = f64::NAN;
        assert!(config.validate().is_err());
        config.notification_timeout = 5.0;
        config.max_content_length = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn legacy_config_gets_safe_defaults() {
        let config = decode_config(&serde_json::json!({
            "CLIENT_ID": "123456789", "CLIENT_SECRET": "test-secret",
            "REDIRECT_URI": "http://localhost/"
        }))
        .unwrap();
        assert!(!config.privacy_mode);
        assert!(!config.log_message_content);
        assert_eq!(config.notification_volume, 0.0);
        assert_eq!(config.theme, ThemePreference::System);
    }

    #[test]
    fn theme_survives_save_and_other_settings_updates() {
        let path =
            std::env::temp_dir().join(format!("discord-vr-theme-{}.json", uuid::Uuid::new_v4()));
        let mut config = valid_config();
        for theme in [
            ThemePreference::Dark,
            ThemePreference::Light,
            ThemePreference::System,
        ] {
            config.theme = theme;
            save_setup_at(&path, &config).unwrap();
            save_tokens_at(&path, "dummy-access", "dummy-refresh").unwrap();
            save_privacy_at(&path, true).unwrap();
            assert_eq!(load_from_path(&path, false).unwrap().theme, theme);
        }
        fs::remove_file(path).unwrap();
        assert!(serde_json::from_value::<Config>(serde_json::json!({"THEME":"unknown"})).is_err());
    }

    #[test]
    fn token_save_preserves_settings_and_invalid_file() {
        let dir = std::env::temp_dir().join(format!("discord-vr-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("config.json");
        let mut raw = serde_json::to_value(valid_config()).unwrap();
        raw["EXTRA_SETTING"] = serde_json::json!({"keep": true});
        raw["REFRESH_TOKEN"] = Value::String("old-refresh".into());
        fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        save_tokens_at(&path, "new-access", "").unwrap();
        let saved = read_document(&path).unwrap();
        assert_eq!(saved["EXTRA_SETTING"], raw["EXTRA_SETTING"]);
        assert!(!storage::needs_protection(&saved));
        let config = load_from_path(&path, false).unwrap();
        assert_eq!(config.access_token, "new-access");
        assert_eq!(config.refresh_token, "old-refresh");
        fs::write(&path, b"{broken").unwrap();
        assert!(save_tokens_at(&path, "new-access", "").is_err());
        assert_eq!(fs::read(&path).unwrap().as_slice(), b"{broken");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn readonly_check_does_not_migrate_plaintext() {
        let path = std::env::temp_dir().join(format!("discord-vr-{}.json", uuid::Uuid::new_v4()));
        let bytes = serde_json::to_vec(&valid_config()).unwrap();
        fs::write(&path, &bytes).unwrap();
        load_from_path(&path, false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        load_from_path(&path, true).unwrap();
        assert!(!storage::needs_protection(&read_document(&path).unwrap()));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn setup_encrypts_immediately_preserves_settings_and_resets_foreign_tokens() {
        let path =
            std::env::temp_dir().join(format!("discord-vr-setup-{}.json", uuid::Uuid::new_v4()));
        let config = valid_config();
        save_setup_at(&path, &config).unwrap();
        assert!(!storage::needs_protection(&read_document(&path).unwrap()));
        save_tokens_at(&path, "old-access", "old-refresh").unwrap();
        let mut raw = read_document(&path).unwrap();
        raw["EXTRA_SETTING"] = serde_json::json!({ "keep": true });
        raw["NOTIFICATION_TIMEOUT"] = Value::from(12.0);
        fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        let mut edited = setup_defaults(&path).unwrap();
        edited.privacy_mode = true;
        save_setup_at(&path, &edited).unwrap();
        assert_eq!(
            load_from_path(&path, false).unwrap().access_token,
            "old-access"
        );
        edited.client_id = "987654321".into();
        save_setup_at(&path, &edited).unwrap();
        let loaded = load_from_path(&path, false).unwrap();
        assert!(loaded.access_token.is_empty());
        assert!(loaded.refresh_token.is_empty());
        assert!(loaded.privacy_mode);
        assert_eq!(loaded.notification_timeout, 12.0);
        assert_eq!(
            read_document(&path).unwrap()["EXTRA_SETTING"],
            raw["EXTRA_SETTING"]
        );
        fs::write(&path, b"{broken").unwrap();
        assert!(save_setup_at(&path, &edited).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{broken");
        fs::remove_file(path).unwrap();
    }
}
