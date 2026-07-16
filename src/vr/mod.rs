use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::{interval, sleep};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

// 使いまわし用の文字列定数
const XSOVERLAY_WS_URL: &str = "ws://127.0.0.1:42070/?client=discord_To_VR";
const SENDER_NAME: &str = "discord_To_VR";
const TARGET_NAME: &str = "xsoverlay";
const CMD_SEND_NOTIFICATION: &str = "SendNotification";
const DEFAULT_ICON_VAL: &str = "default";

/// XSOverlay に送る通知データの構造体
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NotificationData<'a> {
    #[serde(rename = "type")]
    msg_type: i32,
    timeout: f64,
    height: i32,
    opacity: f64,
    volume: f64,
    audio_path: &'a str,
    title: &'a str,
    content: &'a str,
    use_base64_icon: bool,
    icon: &'a str,
}

/// WebSocket メッセージフレームの構造体
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WsMessage<'a> {
    sender: &'a str,
    target: &'a str,
    command: &'a str,
    json_data: String,
}

/// アプリ内から XSOverlay へ通知を送信するためのハンドル
#[derive(Clone)]
pub struct XsOverlaySender {
    tx: UnboundedSender<String>,
    http_client: Client,
    is_connected: Arc<AtomicBool>,
}

impl XsOverlaySender {
    /// 画像URLをダウンロードして Base64 文字列へ変換する
    pub async fn get_base64_icon(&self, url: &str) -> String {
        if url.is_empty() {
            return String::new();
        }
        match self
            .http_client
            .get(url)
            .timeout(Duration::from_secs(3))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                Ok(bytes) => base64::engine::general_purpose::STANDARD.encode(&bytes),
                Err(_) => String::new(),
            },
            _ => String::new(),
        }
    }

    /// 通知データをJSONに変換し、WebSocketタスクの送信キューへ入れる
    pub async fn send_notification(
        &self,
        title: &str,
        content: &str,
        icon_url: Option<&str>,
        sound_path: &str,
        volume: f64,
    ) {
        if !self.is_connected.load(Ordering::SeqCst) {
            return;
        }

        let icon_url_str = icon_url.unwrap_or("");
        let base64_icon = self.get_base64_icon(icon_url_str).await;
        let use_base64 = !base64_icon.is_empty();
        let icon_val = if use_base64 {
            base64_icon.as_str()
        } else {
            DEFAULT_ICON_VAL
        };

        let audio_path = if sound_path.is_empty() || volume <= 0.0 {
            ""
        } else {
            sound_path
        };
        let vol = if audio_path.is_empty() { 0.0 } else { volume };

        let notification_data = NotificationData {
            msg_type: 1,
            timeout: 5.0,
            height: 175,
            opacity: 1.0,
            volume: vol,
            audio_path,
            title,
            content,
            use_base64_icon: use_base64,
            icon: icon_val,
        };

        let json_data = match serde_json::to_string(&notification_data) {
            Ok(j) => j,
            Err(_) => return,
        };

        let ws_msg = WsMessage {
            sender: SENDER_NAME,
            target: TARGET_NAME,
            command: CMD_SEND_NOTIFICATION,
            json_data,
        };

        if let Ok(ws_json) = serde_json::to_string(&ws_msg) {
            let _ = self.tx.send(ws_json);
        }
    }
}

/// XSOverlay と通信するためのハンドルと受信チャネルを作成する
pub fn create_xsoverlay_channel(
    http_client: Client,
) -> (
    XsOverlaySender,
    UnboundedReceiver<String>,
    Arc<AtomicBool>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    let is_connected = Arc::new(AtomicBool::new(false));
    let sender = XsOverlaySender {
        tx,
        http_client,
        is_connected: is_connected.clone(),
    };
    (sender, rx, is_connected)
}

/// XSOverlay との WebSocket 接続を維持・再接続し、メッセージの送受信を行うメインループ
pub async fn xsoverlay_loop(
    mut rx: UnboundedReceiver<String>,
    is_connected: Arc<AtomicBool>,
    sender: XsOverlaySender,
) {
    loop {
        match connect_async(XSOVERLAY_WS_URL).await {
            Ok((ws_stream, _)) => {
                is_connected.store(true, Ordering::SeqCst);
                println!("✅ [XSOverlay] WebSocketへの接続が完了しました！");

                sender
                    .send_notification(
                        SENDER_NAME,
                        "🚀 接続完了！Discordのメッセージ通知を監視します。",
                        Some("https://cdn.discordapp.com/embed/avatars/0.png"),
                        "",
                        0.0,
                    )
                    .await;
                println!("🔔 [XSOverlay] 接続確認用テスト通知をVRに送信しました。");

                let (mut ws_tx, mut ws_rx) = ws_stream.split();
                let mut ping_interval = interval(Duration::from_secs(10));

                loop {
                    tokio::select! {
                        msg_opt = rx.recv() => {
                            match msg_opt {
                                Some(msg) => {
                                    if let Err(_) = ws_tx.send(Message::Text(msg.into())).await {
                                        break;
                                    }
                                }
                                None => return,
                            }
                        }
                        _ = ping_interval.tick() => {
                            if let Err(_) = ws_tx.send(Message::Ping(vec![].into())).await {
                                break;
                            }
                        }
                        ws_evt = ws_rx.next() => {
                            match ws_evt {
                                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                }

                is_connected.store(false, Ordering::SeqCst);
            }
            Err(_) => {
                is_connected.store(false, Ordering::SeqCst);
            }
        }
        sleep(Duration::from_secs(3)).await;
    }
}
