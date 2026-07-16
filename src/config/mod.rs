use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process;

// 使いまわし用の定数定義
const DEFAULT_CLIENT_ID: &str = "YOUR_CLIENT_ID_HERE";
const DEFAULT_CLIENT_SECRET: &str = "YOUR_CLIENT_SECRET_HERE";
const DEFAULT_REDIRECT_URI: &str = "http://localhost/";
const DEFAULT_VOLUME: f64 = 0.7;

// デフォルトの効果音パス（空白は無音）
fn default_sound_path() -> String {
    "".to_string()
}

// デフォルトの音量
fn default_volume() -> f64 {
    DEFAULT_VOLUME
}

/// config.json の内容を保持する設定データ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(rename = "CLIENT_ID")]
    pub client_id: String,

    #[serde(rename = "CLIENT_SECRET")]
    pub client_secret: String,

    #[serde(rename = "REDIRECT_URI")]
    pub redirect_uri: String,

    #[serde(rename = "NOTIFICATION_SOUND", default = "default_sound_path")]
    pub notification_sound: String,

    #[serde(rename = "NOTIFICATION_VOLUME", default = "default_volume")]
    pub notification_volume: f64,

    #[serde(rename = "ACCESS_TOKEN", default)]
    pub access_token: String,

    #[serde(rename = "REFRESH_TOKEN", default)]
    pub refresh_token: String,
}

impl Default for Config {
    // 初回生成時の初期値
    fn default() -> Self {
        Self {
            client_id: DEFAULT_CLIENT_ID.to_string(),
            client_secret: DEFAULT_CLIENT_SECRET.to_string(),
            redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
            notification_sound: default_sound_path(),
            notification_volume: DEFAULT_VOLUME,
            access_token: "".to_string(),
            refresh_token: "".to_string(),
        }
    }
}

/// エラー内容などを読めるように、Enterキー入力を待ってからプロセスを終了する
pub fn safe_exit(code: i32) -> ! {
    println!("\nEnterキーを押すと終了します...");
    let mut buf = [0u8; 1];
    let _ = io::stdin().read(&mut buf);
    process::exit(code);
}

/// 実行ファイルの場所から config.json のパスを取得する
pub fn get_config_path() -> PathBuf {
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            return parent.join("config.json");
        }
    }
    PathBuf::from("config.json")
}

/// config.json を読み込む（ファイルが無ければデフォルトを自動生成する）
pub fn load_or_init() -> Config {
    let config_path = get_config_path();

    if !config_path.exists() {
        let default_config = Config::default();
        if let Ok(json_str) = serde_json::to_string_pretty(&default_config) {
            let _ = fs::write(&config_path, json_str);
            println!("📝 設定ファイル '{}' を自動生成しました。", config_path.display());
            println!("エディタで config.json を開き、CLIENT_ID と CLIENT_SECRET を入力してから再度実行してください。");
        }
        safe_exit(0);
    }

    match fs::read_to_string(&config_path) {
        Ok(content) => match serde_json::from_str::<Config>(&content) {
            Ok(config) => {
                // 定数を使いまわして初期値チェックを行う
                if config.client_id.is_empty()
                    || config.client_secret.is_empty()
                    || config.client_id == DEFAULT_CLIENT_ID
                    || config.client_secret == DEFAULT_CLIENT_SECRET
                {
                    println!("⚠️ [注意] config.json 内の CLIENT_ID または CLIENT_SECRET が未設定です。設定してから再度実行してください。");
                    safe_exit(1);
                }
                config
            }
            Err(e) => {
                println!("❌ [エラー] config.json のフォーマットが不正です: {}", e);
                safe_exit(1);
            }
        },
        Err(e) => {
            println!("❌ [エラー] config.json の読み込みに失敗しました: {}", e);
            safe_exit(1);
        }
    }
}

/// 取得したトークンを次回のために config.json へ保存する
pub fn save_tokens(access_token: &str, refresh_token: &str) {
    let config_path = get_config_path();
    let mut config = match fs::read_to_string(&config_path) {
        Ok(content) => serde_json::from_str::<Config>(&content).unwrap_or_default(),
        Err(_) => Config::default(),
    };

    config.access_token = access_token.to_string();
    if !refresh_token.is_empty() {
        config.refresh_token = refresh_token.to_string();
    }

    if let Ok(json_str) = serde_json::to_string_pretty(&config) {
        if fs::write(&config_path, json_str).is_ok() {
            println!("💾 トークン情報を config.json に自動保存しました。");
        }
    }
}
