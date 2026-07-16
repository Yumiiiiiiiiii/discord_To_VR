use super::ipc::{DiscordIPC, IpcError};
use crate::config::Config;
use crate::vr::XsOverlaySender;
use reqwest::Client;
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;
use tokio::time::sleep;

// 使いまわし用の定数定義
const DISCORD_TOKEN_URL: &str = "https://discord.com/api/oauth2/token";
const CMD_DISPATCH: &str = "DISPATCH";
const EVT_NOTIFICATION_CREATE: &str = "NOTIFICATION_CREATE";
const EVT_ERROR: &str = "ERROR";
const EVT_READY: &str = "READY";
const MAX_TITLE_LEN: usize = 35;
const MAX_CONTENT_LEN: usize = 70;
const MAX_SEEN_IDS: usize = 1000;

/// Discord との接続・OAuth2認証・メッセージイベントの監視を行うメイン処理クラス
pub struct DiscordRPCNotifier {
    ipc: DiscordIPC,
    config: Config,
    xs_sender: XsOverlaySender,
    http_client: Client,
    guild_cache: HashMap<String, (String, String)>,
    channel_cache: HashMap<String, (String, String)>,
    pending_dispatches: VecDeque<Value>,
    seen_message_ids: HashSet<String>,
    seen_message_ids_order: VecDeque<String>,
    current_user_id: Option<String>,
}

impl DiscordRPCNotifier {
    pub fn new(config: Config, xs_sender: XsOverlaySender, http_client: Client) -> Self {
        Self {
            ipc: DiscordIPC::new(),
            config,
            xs_sender,
            http_client,
            guild_cache: HashMap::new(),
            channel_cache: HashMap::new(),
            pending_dispatches: VecDeque::new(),
            seen_message_ids: HashSet::new(),
            seen_message_ids_order: VecDeque::new(),
            current_user_id: None,
        }
    }

    /// パケットを1フレーム受信する（保留中のイベントキューがあればそれを優先して返す）
    async fn recv_async(&mut self, timeout: Duration) -> Result<Option<(i32, Value)>, IpcError> {
        let start = tokio::time::Instant::now();
        loop {
            if !self.ipc.is_connected() {
                return Err(IpcError::Disconnected);
            }
            if let Some(dispatch) = self.pending_dispatches.pop_front() {
                return Ok(Some((1, dispatch)));
            }
            let remaining = if let Some(rem) = timeout.checked_sub(start.elapsed()) {
                rem
            } else {
                return Ok(None);
            };
            let step_timeout = remaining.min(Duration::from_millis(100));
            match self.ipc.recv_timeout(step_timeout).await {
                Ok(Some(frame)) => return Ok(Some(frame)),
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        return Ok(None);
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// トークンを使って現在のセッション認証を行う
    async fn authenticate_with_token(&mut self, access_token: &str) -> bool {
        if access_token.is_empty() {
            return false;
        }
        let payload = serde_json::json!({
            "cmd": "AUTHENTICATE",
            "args": {
                "access_token": access_token
            },
            "nonce": uuid::Uuid::new_v4().to_string()
        });
        if !self.ipc.send(1, &payload).await {
            return false;
        }
        match self.recv_async(Duration::from_secs(5)).await {
            Ok(Some((opcode, res))) => {
                if opcode == -1 || res.get("evt").and_then(|v| v.as_str()) == Some(EVT_ERROR) {
                    return false;
                }
                if let Some(user_data) = res.get("data").and_then(|d| d.get("user")) {
                    if let Some(id) = user_data.get("id").and_then(|i| i.as_str()) {
                        self.current_user_id = Some(id.to_string());
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// トークン認証・自動更新・初回ログインのブラウザ承認までを行う認証まとめ処理
    async fn authorize_and_authenticate(&mut self) -> bool {
        let access_token = self.config.access_token.clone();
        if !access_token.is_empty() {
            println!("🔄 保存済みのアクセストークンで自動認証を試みます...");
            if self.authenticate_with_token(&access_token).await {
                println!("🚀 [Discord RPC] 保存済みトークンでのセッション認証に成功しました！");
                return true;
            }
            println!("⚠️ 保存済みのアクセストークンが無効・期限切れです。");
        }

        if !self.config.refresh_token.is_empty() {
            println!("🔄 リフレッシュトークンを使用して新しいアクセストークンを自動取得中...");
            let params = [
                ("client_id", self.config.client_id.as_str()),
                ("client_secret", self.config.client_secret.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", self.config.refresh_token.as_str()),
            ];
            if let Ok(resp) = self
                .http_client
                .post(DISCORD_TOKEN_URL)
                .form(&params)
                .send()
                .await
            {
                if resp.status().is_success() {
                    if let Ok(token_data) = resp.json::<Value>().await {
                        let new_access = token_data
                            .get("access_token")
                            .and_then(|a| a.as_str())
                            .unwrap_or("")
                            .to_string();
                        let new_refresh = token_data
                            .get("refresh_token")
                            .and_then(|r| r.as_str())
                            .unwrap_or(&self.config.refresh_token)
                            .to_string();
                        crate::config::save_tokens(&new_access, &new_refresh);
                        self.config.access_token = new_access.clone();
                        self.config.refresh_token = new_refresh;
                        if self.authenticate_with_token(&new_access).await {
                            println!("🚀 [Discord RPC] トークン自動更新およびセッション認証に成功しました！");
                            return true;
                        }
                    }
                } else {
                    println!("⚠️ リフレッシュトークンの期限も切れているため、再承認（ポップアップ）を行います。");
                }
            }
        }

        println!("🔄 [Discord RPC] 認証画面（ポップアップ）を要求しています。Discord画面を確認してください...");
        let scopes = ["rpc", "rpc.notifications.read", "messages.read", "guilds"];
        let auth_payload = serde_json::json!({
            "cmd": "AUTHORIZE",
            "args": {
                "client_id": self.config.client_id,
                "scopes": scopes
            },
            "nonce": uuid::Uuid::new_v4().to_string()
        });
        if !self.ipc.send(1, &auth_payload).await {
            return false;
        }

        let auth_resp = match self.recv_async(Duration::from_secs(300)).await {
            Ok(Some((opcode, resp))) => {
                if opcode == -1 || resp.get("evt").and_then(|v| v.as_str()) == Some(EVT_ERROR) {
                    println!("❌ [エラー] 認証拒否または失敗: {}", resp);
                    return false;
                }
                resp
            }
            _ => {
                println!("❌ [エラー] 認証タイムアウトまたは切断");
                return false;
            }
        };

        let auth_code = match auth_resp
            .get("data")
            .and_then(|d| d.get("code"))
            .and_then(|c| c.as_str())
        {
            Some(code) => code.to_string(),
            None => {
                println!("❌ [エラー] 認証コードが取得できませんでした: {}", auth_resp);
                return false;
            }
        };
        println!("🔑 認証コードを取得しました。アクセストークンへ交換中...");

        let params = [
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("code", auth_code.as_str()),
            ("redirect_uri", self.config.redirect_uri.as_str()),
        ];

        let resp = match self
            .http_client
            .post(DISCORD_TOKEN_URL)
            .form(&params)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                println!("❌ [エラー] トークン取得通信エラー: {}", e);
                return false;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            println!("❌ [エラー] トークン取得失敗 ({}): {}", status, text);
            return false;
        }

        let token_data = match resp.json::<Value>().await {
            Ok(d) => d,
            Err(e) => {
                println!("❌ [エラー] トークン応答の解析失敗: {}", e);
                return false;
            }
        };

        let access_token = token_data
            .get("access_token")
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .to_string();
        let refresh_token = token_data
            .get("refresh_token")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();

        crate::config::save_tokens(&access_token, &refresh_token);
        self.config.access_token = access_token.clone();
        self.config.refresh_token = refresh_token;

        println!("🔄 アクセストークンを送信してセッションを認証中...");
        if !self.authenticate_with_token(&access_token).await {
            println!("❌ [エラー] 認証 (AUTHENTICATE) 失敗");
            return false;
        }

        println!("🚀 [Discord RPC] OAuth2セッションの完全認証に成功しました！");
        true
    }

    /// チャンネルIDから所属サーバーIDとチャンネル名を取得（またはキャッシュから返却）する
    async fn get_channel_info(&mut self, channel_id: &str) -> Option<(String, String)> {
        if channel_id.is_empty() || !self.ipc.is_connected() {
            return None;
        }
        if let Some(info) = self.channel_cache.get(channel_id) {
            return Some(info.clone());
        }

        let get_payload = serde_json::json!({
            "cmd": "GET_CHANNEL",
            "args": {"channel_id": channel_id},
            "nonce": uuid::Uuid::new_v4().to_string()
        });
        if !self.ipc.send(1, &get_payload).await {
            return None;
        }

        for _ in 0..30 {
            match self.recv_async(Duration::from_millis(100)).await {
                Ok(Some((_, resp))) => {
                    let cmd = resp.get("cmd").and_then(|c| c.as_str());
                    if cmd == Some("GET_CHANNEL") {
                        let chan_data = resp.get("data").unwrap_or(&Value::Null);
                        let mut g_id = chan_data
                            .get("guild_id")
                            .and_then(|g| g.as_str())
                            .unwrap_or("")
                            .to_string();
                        let c_name = chan_data
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        let c_type = chan_data.get("type").and_then(|t| t.as_i64()).unwrap_or(-1);

                        if g_id.is_empty() && [0, 2, 5, 11, 12, 13, 14, 15].contains(&c_type) {
                            g_id = "UNKNOWN_GUILD".to_string();
                        }
                        let info = (g_id, c_name);
                        self.channel_cache
                            .insert(channel_id.to_string(), info.clone());
                        return Some(info);
                    } else if cmd == Some(CMD_DISPATCH) {
                        self.pending_dispatches.push_back(resp);
                    }
                }
                _ => break,
            }
        }
        None
    }

    /// サーバーIDからサーバー名とアイコンURLを取得（またはキャッシュから返却）する
    async fn get_guild_info(&mut self, guild_id: &str) -> Option<(String, String)> {
        if guild_id.is_empty() || !self.ipc.is_connected() {
            return None;
        }
        if let Some(info) = self.guild_cache.get(guild_id) {
            return Some(info.clone());
        }

        let get_payload = serde_json::json!({
            "cmd": "GET_GUILD",
            "args": {"guild_id": guild_id},
            "nonce": uuid::Uuid::new_v4().to_string()
        });
        if !self.ipc.send(1, &get_payload).await {
            return None;
        }

        for _ in 0..10 {
            match self.recv_async(Duration::from_millis(100)).await {
                Ok(Some((_, resp))) => {
                    let cmd = resp.get("cmd").and_then(|c| c.as_str());
                    if cmd == Some("GET_GUILD") {
                        let guild_data = resp.get("data").unwrap_or(&Value::Null);
                        let name = guild_data
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        let icon_hash = guild_data
                            .get("icon_url")
                            .and_then(|i| i.as_str())
                            .unwrap_or("");
                        let icon_url = if !icon_hash.is_empty() && !icon_hash.starts_with("http") {
                            format!("https://cdn.discordapp.com/icons/{}/{}.png", guild_id, icon_hash)
                        } else {
                            icon_hash.to_string()
                        };
                        let info = (name, icon_url);
                        self.guild_cache
                            .insert(guild_id.to_string(), info.clone());
                        return Some(info);
                    } else if cmd == Some(CMD_DISPATCH) {
                        self.pending_dispatches.push_back(resp);
                    }
                }
                _ => break,
            }
        }
        None
    }

    /// 受信したメッセージイベントをパースして、タイトル・本文・アイコンをまとめ、XSOverlayへ転送する
    async fn handle_notification(&mut self, data: Value, evt_type: &str) {
        let message = if evt_type == "MESSAGE_CREATE" || data.get("author").is_some() {
            data.clone()
        } else {
            data.get("message").cloned().unwrap_or(data.clone())
        };

        // メッセージIDを取得し、ヒープアロケーション前に重複チェックを行って無駄な生成を防止する
        let msg_id_val = message
            .get("id")
            .or_else(|| data.get("id"))
            .or_else(|| data.get("message_id"));

        if let Some(v) = msg_id_val {
            if let Some(id_str) = v.as_str() {
                if self.seen_message_ids.contains(id_str) {
                    return;
                }
                let id_owned = id_str.to_string();
                self.seen_message_ids.insert(id_owned.clone());
                self.seen_message_ids_order.push_back(id_owned);
            } else if let Some(id_num) = v.as_u64() {
                let id_owned = id_num.to_string();
                if self.seen_message_ids.contains(&id_owned) {
                    return;
                }
                self.seen_message_ids.insert(id_owned.clone());
                self.seen_message_ids_order.push_back(id_owned);
            }
            if self.seen_message_ids.len() > MAX_SEEN_IDS {
                if let Some(old_id) = self.seen_message_ids_order.pop_front() {
                    self.seen_message_ids.remove(&old_id);
                }
            }
        }

        let author = message.get("author").unwrap_or(&Value::Null);
        let author_id = author.get("id").and_then(|i| i.as_str());
        if let (Some(a_id), Some(curr_id)) = (author_id, &self.current_user_id) {
            if a_id == curr_id {
                return;
            }
        }

        let sender_name = author
            .get("global_name")
            .or_else(|| author.get("username"))
            .and_then(|n| n.as_str())
            .unwrap_or("ユーザー");

        let mut guild_id = message
            .get("guild_id")
            .or_else(|| data.get("guild_id"))
            .and_then(|g| g.as_str())
            .map(|s| s.to_string());
        let channel_id = message
            .get("channel_id")
            .or_else(|| data.get("channel_id"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());

        let mut channel_name = None;
        if let Some(ref c_id) = channel_id {
            if let Some((chan_guild_id, chan_name)) = self.get_channel_info(c_id).await {
                if guild_id.is_none() {
                    guild_id = Some(chan_guild_id);
                }
                channel_name = Some(chan_name);
            }
        }

        let is_server = guild_id
            .as_ref()
            .map(|g| !g.is_empty())
            .unwrap_or(false);

        let icon_url = if let (Some(id), Some(avatar)) = (
            author.get("id").and_then(|i| i.as_str()),
            author.get("avatar").and_then(|a| a.as_str()),
        ) {
            format!("https://cdn.discordapp.com/avatars/{}/{}.png", id, avatar)
        } else {
            data.get("icon_url")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string()
        };

        let mut title = if is_server {
            let g_id_str = guild_id.as_deref().unwrap_or("");
            let mut guild_name = if let Some((g_name, _)) = self.get_guild_info(g_id_str).await {
                g_name
            } else {
                String::new()
            };
            if guild_name.is_empty() || guild_name == "UNKNOWN_GUILD" {
                guild_name = "サーバー".to_string();
            }
            let c_name = channel_name.unwrap_or_else(|| "チャンネル".to_string());
            format!("{}(#{},{})", sender_name, c_name, guild_name)
        } else {
            sender_name.to_string()
        };

        let content_opt = message
            .get("content")
            .or_else(|| data.get("body"))
            .and_then(|c| c.as_str());

        let mut content = if let Some(c) = content_opt {
            if c.is_empty() {
                String::new()
            } else {
                c.to_string()
            }
        } else {
            String::new()
        };

        if content.is_empty() {
            if message
                .get("attachments")
                .and_then(|a| a.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false)
            {
                content = "📎 ファイルが送信されました".to_string();
            } else if message.get("sticker_items").is_some() || message.get("stickers").is_some() {
                content = "🎨 スタンプが送信されました".to_string();
            } else if let Some(embeds) = message.get("embeds").and_then(|e| e.as_array()) {
                if !embeds.is_empty() {
                    let emb = &embeds[0];
                    content = emb
                        .get("title")
                        .or_else(|| emb.get("description"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("🔗 埋め込みリンク/メッセージ")
                        .to_string();
                }
            } else if message.get("poll").is_some() {
                content = "📊 アンケートが送信されました".to_string();
            } else if let (Some(t), Some(b)) = (
                data.get("title").and_then(|t| t.as_str()),
                data.get("body").and_then(|b| b.as_str()),
            ) {
                if !t.is_empty() && !b.is_empty() {
                    content = b.to_string();
                }
            }
        }

        if content.is_empty() {
            return;
        }

        // Vec<char> の無駄な割り当てを行わずに、インプレースで文字数を切り詰める高速最適化
        if let Some((idx, _)) = title.char_indices().nth(MAX_TITLE_LEN) {
            title.truncate(idx);
            title.push_str("...");
        }
        if let Some((idx, _)) = content.char_indices().nth(MAX_CONTENT_LEN) {
            content.truncate(idx);
            content.push_str("...");
        }

        let location_str = if is_server { "Server" } else { "DM" };
        println!("📩 [受信] ({}) {}: {}", location_str, title, content);
        self.xs_sender
            .send_notification(
                &title,
                &content,
                Some(&icon_url),
                &self.config.notification_sound,
                self.config.notification_volume,
            )
            .await;
    }

    /// Discord との接続・認証・通知購読・イベント受信をループで回し続けるメインエンジン
    pub async fn start(&mut self) {
        loop {
            if !self.ipc.connect().await {
                println!("⚠️ [Discord RPC] IPCパイプが見つかりません。5秒後に再試行します...");
                sleep(Duration::from_secs(5)).await;
                continue;
            }

            let handshake_data = serde_json::json!({
                "v": 1,
                "client_id": self.config.client_id
            });
            if !self.ipc.send(0, &handshake_data).await {
                self.ipc.close();
                sleep(Duration::from_secs(3)).await;
                continue;
            }

            match self.recv_async(Duration::from_secs(5)).await {
                Ok(Some((opcode, resp))) => {
                    if opcode == -1
                        || resp.get("evt").and_then(|v| v.as_str()) != Some(EVT_READY)
                    {
                        println!("❌ [エラー] Discordハンドシェイク失敗: {}", resp);
                        self.ipc.close();
                        sleep(Duration::from_secs(3)).await;
                        continue;
                    }
                    if let Some(user_info) = resp.get("data").and_then(|d| d.get("user")) {
                        if let Some(id) = user_info.get("id").and_then(|i| i.as_str()) {
                            self.current_user_id = Some(id.to_string());
                        }
                        let username = user_info
                            .get("username")
                            .and_then(|u| u.as_str())
                            .unwrap_or("Unknown");
                        println!("✅ [Discord RPC] ハンドシェイク成功！ユーザー: {}", username);
                    }
                }
                _ => {
                    println!("❌ [エラー] Discordハンドシェイク応答タイムアウトまたは切断");
                    self.ipc.close();
                    sleep(Duration::from_secs(3)).await;
                    continue;
                }
            }

            if !self.authorize_and_authenticate().await {
                self.ipc.close();
                sleep(Duration::from_secs(5)).await;
                continue;
            }

            let nonce = uuid::Uuid::new_v4().to_string();
            let sub_payload = serde_json::json!({
                "cmd": "SUBSCRIBE",
                "evt": EVT_NOTIFICATION_CREATE,
                "args": {},
                "nonce": nonce
            });
            if !self.ipc.send(1, &sub_payload).await {
                self.ipc.close();
                sleep(Duration::from_secs(3)).await;
                continue;
            }
            println!("🔔 [Discord RPC] NOTIFICATION_CREATE の購読を開始しました。");

            let mut last_ping_time = tokio::time::Instant::now();
            while self.ipc.is_connected() {
                match self.recv_async(Duration::from_millis(200)).await {
                    Err(_) => {
                        println!("\n⚠️ [Discord RPC] パイプ切断を検知しました。再接続を試みます...");
                        break;
                    }
                    Ok(res_opt) => {
                        if last_ping_time.elapsed() > Duration::from_secs(15) {
                            last_ping_time = tokio::time::Instant::now();
                            if !self.ipc.send(3, &serde_json::json!({"v": 1})).await {
                                println!("\n⚠️ [Discord RPC] 生存確認（PING）送信エラー。再接続を試みます...");
                                break;
                            }
                        }

                        if let Some((opcode, frame)) = res_opt {
                            if opcode == 4 {
                                continue;
                            }
                            let cmd = frame.get("cmd").and_then(|c| c.as_str());
                            let evt = frame.get("evt").and_then(|e| e.as_str());

                            if cmd == Some(CMD_DISPATCH) && evt == Some(EVT_ERROR) {
                                println!("⚠️ [Discordエラー]: {:?}", frame.get("data"));
                            } else if cmd == Some(CMD_DISPATCH) && evt == Some(EVT_NOTIFICATION_CREATE) {
                                if let Some(data) = frame.get("data") {
                                    self.handle_notification(data.clone(), EVT_NOTIFICATION_CREATE).await;
                                }
                            }
                        }
                    }
                }
            }
            self.ipc.close();
            sleep(Duration::from_secs(3)).await;
        }
    }
}
