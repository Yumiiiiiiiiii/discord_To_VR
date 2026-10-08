use super::ipc::{DiscordIPC, IpcError};
use super::receive::ReceiveControl;
use crate::config::Config;
use crate::diagnostics::{Outcome, Trace};
use crate::vr::XsOverlaySender;
use reqwest::Client;
use serde_json::Value;
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

// 使いまわし用の定数定義
const DISCORD_TOKEN_URL: &str = "https://discord.com/api/oauth2/token";
const CMD_DISPATCH: &str = "DISPATCH";
const EVT_NOTIFICATION_CREATE: &str = "NOTIFICATION_CREATE";
const EVT_MESSAGE_CREATE: &str = "MESSAGE_CREATE";
const EVT_ERROR: &str = "ERROR";
const EVT_READY: &str = "READY";
const MAX_SEEN_IDS: usize = 1000;
const MAX_PENDING_DISPATCHES: usize = 128;
const RETRY_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthFailure {
    Retry(Duration),
    InvalidToken,
    MissingScope,
    ActionRequired,
}

fn token_failure(
    status: reqwest::StatusCode,
    body: &Value,
    retry_after: Option<&str>,
) -> AuthFailure {
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        let seconds = retry_after
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(5)
            .clamp(1, 300);
        return AuthFailure::Retry(Duration::from_secs(seconds));
    }
    if status == reqwest::StatusCode::BAD_REQUEST
        && body.get("error").and_then(Value::as_str) == Some("invalid_grant")
    {
        AuthFailure::InvalidToken
    } else {
        AuthFailure::ActionRequired
    }
}

fn notification_content(message: &Value, data: &Value) -> String {
    for value in [data.get("body"), message.get("content")] {
        if let Some(text) = value.and_then(Value::as_str).filter(|s| !s.is_empty()) {
            return text.to_string();
        }
    }
    let has_items = |key| {
        message
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    };
    if has_items("attachments") {
        return "📎 ファイルが送信されました".into();
    }
    if has_items("sticker_items") || has_items("stickers") {
        return "🎨 スタンプが送信されました".into();
    }
    if let Some(embed) = message
        .get("embeds")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        for key in ["title", "description"] {
            if let Some(text) = embed
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                return text.to_string();
            }
        }
        return "🔗 埋め込みリンク/メッセージ".into();
    }
    if message.get("poll").is_some_and(|v| !v.is_null()) {
        return "📊 アンケートが送信されました".into();
    }
    String::new()
}

fn safe_text(text: &str, max_chars: usize) -> String {
    // Prevent terminal control sequences and XSOverlay rich-text instructions.
    let mut chars = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t');
    let mut output: String = chars
        .by_ref()
        .take(max_chars)
        .map(|c| match c {
            '<' => '＜',
            '>' => '＞',
            _ => c,
        })
        .collect();
    if chars.next().is_some() {
        output.push_str("...");
    }
    output
}

/// Discord との接続・OAuth2認証・メッセージイベントの監視を行うメイン処理クラス
pub struct DiscordRPCNotifier {
    ipc: DiscordIPC,
    config: Config,
    xs_sender: XsOverlaySender,
    http_client: Client,
    pending_dispatches: VecDeque<Value>,
    seen_message_ids: HashSet<String>,
    seen_message_ids_order: VecDeque<String>,
    current_user_id: Option<String>,
    authorization_attempted: bool,
    status: Arc<AtomicU8>,
    receive_control: ReceiveControl,
    whitelist_status: Arc<AtomicU8>,
    trace: Option<Trace>,
}

impl DiscordRPCNotifier {
    pub fn with_trace(mut self, trace: Trace) -> Self {
        self.xs_sender = self.xs_sender.with_trace(trace.clone());
        self.trace = Some(trace);
        self
    }
    fn record(&self, outcome: Outcome) {
        if let Some(trace) = &self.trace {
            trace.record(outcome);
        }
    }
    /// Fixed synthetic content, routed through the normal message handler. Never connects to RPC.
    pub async fn inject_test_notification(&mut self, raw: bool, channel: &str) {
        self.handle_notification(serde_json::json!({"channel_id": channel, "title":"Discord 受信テスト", "message":{"id":uuid::Uuid::new_v4().to_string(), "author":{"id":"diagnostic-dummy","username":"テスト用サンプル"}, "content":"これはダミーの受信通知です。"}}), if raw { EVT_MESSAGE_CREATE } else { EVT_NOTIFICATION_CREATE }).await;
    }
    pub fn new(config: Config, xs_sender: XsOverlaySender, http_client: Client) -> Self {
        Self {
            receive_control: ReceiveControl::new(&config),
            whitelist_status: Arc::new(AtomicU8::new(0)),
            trace: None,
            ipc: DiscordIPC::new(),
            config,
            xs_sender,
            http_client,
            pending_dispatches: VecDeque::new(),
            seen_message_ids: HashSet::new(),
            seen_message_ids_order: VecDeque::new(),
            current_user_id: None,
            authorization_attempted: false,
            status: Arc::new(AtomicU8::new(0)),
        }
    }

    pub fn with_status(mut self, status: Arc<AtomicU8>) -> Self {
        self.status = status;
        self
    }
    pub fn with_receive_control(
        mut self,
        control: ReceiveControl,
        whitelist_status: Arc<AtomicU8>,
    ) -> Self {
        self.receive_control = control;
        self.whitelist_status = whitelist_status;
        self
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

    /// Match responses to requests, retaining dispatches for the event loop.
    async fn rpc_request(
        &mut self,
        payload: &Value,
        timeout: Duration,
    ) -> Result<Value, AuthFailure> {
        let nonce = payload
            .get("nonce")
            .and_then(Value::as_str)
            .ok_or(AuthFailure::ActionRequired)?;
        let command = payload
            .get("cmd")
            .and_then(Value::as_str)
            .ok_or(AuthFailure::ActionRequired)?;
        if !self.ipc.send(1, payload).await {
            return Err(AuthFailure::Retry(RETRY_DELAY));
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .ok_or(AuthFailure::Retry(RETRY_DELAY))?;
            match self.ipc.recv_timeout(remaining).await {
                Ok(Some((_, response))) => {
                    if response.get("nonce").and_then(Value::as_str) == Some(nonce)
                        && response.get("cmd").and_then(Value::as_str) == Some(command)
                    {
                        if response.get("evt").and_then(Value::as_str) == Some(EVT_ERROR) {
                            let code = response
                                .get("data")
                                .and_then(|d| d.get("code"))
                                .and_then(Value::as_i64);
                            eprintln!(
                                "⚠️ [Discord RPC] {command} が拒否されました（コード: {code:?}）。"
                            );
                            return Err(if code == Some(4009) {
                                AuthFailure::InvalidToken
                            } else {
                                AuthFailure::ActionRequired
                            });
                        }
                        return Ok(response);
                    }
                    if response.get("cmd").and_then(Value::as_str) == Some(CMD_DISPATCH)
                        && matches!(
                            response.get("evt").and_then(Value::as_str),
                            Some(EVT_NOTIFICATION_CREATE | EVT_MESSAGE_CREATE)
                        )
                    {
                        if self.pending_dispatches.len() == MAX_PENDING_DISPATCHES {
                            self.pending_dispatches.pop_front();
                            eprintln!("⚠️ 通知が集中したため、古い保留通知を破棄しました。");
                        }
                        self.pending_dispatches.push_back(response);
                    }
                }
                _ => return Err(AuthFailure::Retry(RETRY_DELAY)),
            }
        }
    }

    async fn authenticate_with_token(&mut self, access_token: &str) -> Result<(), AuthFailure> {
        let payload = serde_json::json!({
            "cmd": "AUTHENTICATE", "args": { "access_token": access_token },
            "nonce": uuid::Uuid::new_v4().to_string()
        });
        let response = self.rpc_request(&payload, Duration::from_secs(5)).await?;
        let id = response
            .get("data")
            .and_then(|d| d.get("user"))
            .and_then(|u| u.get("id"))
            .and_then(Value::as_str)
            .ok_or(AuthFailure::Retry(RETRY_DELAY))?;
        self.current_user_id = Some(id.to_string());
        self.receive_control.set_rpc_user(Some(id.to_string()));
        if !self.config.whitelist_channel_ids.is_empty()
            && !response
                .pointer("/data/scopes")
                .and_then(Value::as_array)
                .is_some_and(|scopes| {
                    scopes
                        .iter()
                        .any(|scope| scope.as_str() == Some("messages.read"))
                })
        {
            return Err(AuthFailure::MissingScope);
        }
        Ok(())
    }

    async fn exchange_tokens(
        &self,
        params: &[(&str, &str)],
    ) -> Result<(String, String), AuthFailure> {
        let response = self
            .http_client
            .post(DISCORD_TOKEN_URL)
            .form(params)
            .send()
            .await
            .map_err(|_| AuthFailure::Retry(RETRY_DELAY))?;
        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        // Never log the response body: successful replies contain credentials.
        let body = response.json::<Value>().await;
        if !status.is_success() {
            let failure = token_failure(
                status,
                body.as_ref().unwrap_or(&Value::Null),
                retry_after.as_deref(),
            );
            eprintln!(
                "⚠️ [Discord OAuth] トークン処理に失敗しました（HTTP {}）。",
                status.as_u16()
            );
            return Err(failure);
        }
        let body = body.map_err(|_| AuthFailure::Retry(RETRY_DELAY))?;
        let access = body
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or(AuthFailure::ActionRequired)?
            .to_string();
        let refresh = body
            .get("refresh_token")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok((access, refresh))
    }

    fn remember_tokens(&mut self, access: String, refresh: String) {
        if let Err(e) = crate::config::save_tokens(&access, &refresh) {
            eprintln!(
                "⚠️ 認証情報を保存できませんでした。次回は再認証が必要になる場合があります: {e}"
            );
        }
        self.config.access_token = access;
        self.config.refresh_token = refresh;
    }

    async fn authorize_and_authenticate(&mut self) -> Result<(), AuthFailure> {
        let access = self.config.access_token.clone();
        let mut scope_upgrade = false;
        if !access.is_empty() {
            match self.authenticate_with_token(&access).await {
                Ok(()) => {
                    println!("🚀 [Discord RPC] 保存済みトークンで認証しました。");
                    return Ok(());
                }
                Err(AuthFailure::InvalidToken) => println!("🔄 保存済みトークンを更新します。"),
                Err(AuthFailure::MissingScope) => scope_upgrade = true,
                Err(error) => return Err(error),
            }
        }
        if !scope_upgrade && !self.config.refresh_token.is_empty() {
            let params = [
                ("client_id", self.config.client_id.as_str()),
                ("client_secret", self.config.client_secret.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", self.config.refresh_token.as_str()),
            ];
            match self.exchange_tokens(&params).await {
                Ok((access, refresh)) => {
                    let refresh = if refresh.is_empty() {
                        self.config.refresh_token.clone()
                    } else {
                        refresh
                    };
                    self.remember_tokens(access.clone(), refresh);
                    match self.authenticate_with_token(&access).await {
                        Ok(()) => {
                            println!("🚀 [Discord RPC] トークンを更新して認証しました。");
                            return Ok(());
                        }
                        Err(AuthFailure::MissingScope) => {}
                        Err(error) => return Err(error),
                    }
                }
                Err(AuthFailure::InvalidToken) => {
                    println!("🔄 更新トークンが無効のため、再承認が必要です。")
                }
                Err(error) => return Err(error),
            }
        }
        // One popup attempt per run: denial, timeout and failed exchange must not
        // repeatedly interrupt the user on each reconnect.
        if self.authorization_attempted {
            return Err(AuthFailure::ActionRequired);
        }
        self.authorization_attempted = true;
        println!("🔄 [Discord RPC] Discord の承認画面を確認してください。");
        let request = serde_json::json!({
            "cmd": "AUTHORIZE",
            "args": { "client_id": self.config.client_id, "scopes": self.required_scopes() },
            "nonce": uuid::Uuid::new_v4().to_string()
        });
        let response = self
            .rpc_request(&request, Duration::from_secs(300))
            .await
            .map_err(|_| AuthFailure::ActionRequired)?;
        let code = response
            .get("data")
            .and_then(|d| d.get("code"))
            .and_then(Value::as_str)
            .ok_or(AuthFailure::ActionRequired)?;
        let params = [
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", self.config.redirect_uri.as_str()),
        ];
        let (access, refresh) = self.exchange_tokens(&params).await?;
        self.remember_tokens(access.clone(), refresh);
        self.authenticate_with_token(&access).await?;
        println!("🚀 [Discord RPC] OAuth2 認証に成功しました。");
        Ok(())
    }
    /// 受信したメッセージイベントをパースして、タイトル・本文・アイコンをまとめ、XSOverlayへ転送する
    fn required_scopes(&self) -> Vec<&'static str> {
        let mut scopes = vec!["rpc", "rpc.notifications.read"];
        if !self.config.whitelist_channel_ids.is_empty() {
            scopes.push("messages.read");
        }
        scopes
    }
    async fn handle_notification(&mut self, data: Value, evt_type: &str) {
        self.record(if evt_type == EVT_MESSAGE_CREATE {
            Outcome::ReceivedMessage
        } else {
            Outcome::ReceivedNotification
        });
        // Both RPC notification and message events wrap the message in `data.message`.
        // Accept a flat message too, without losing the author or message ID.
        let message = data.get("message").unwrap_or(&data);
        let channel = data
            .get("channel_id")
            .or_else(|| message.get("channel_id"))
            .and_then(Value::as_str);
        let gate = self
            .receive_control
            .gate(channel, evt_type == EVT_MESSAGE_CREATE);
        if !gate.allows() {
            self.record(Outcome::FilteredMode);
            return;
        }
        let sender = self.xs_sender.with_notification_gate(gate);
        if message.get("blocked").and_then(Value::as_bool) == Some(true) {
            self.record(Outcome::FilteredBlocked);
            return;
        }

        // メッセージIDを取得し、ヒープアロケーション前に重複チェックを行って無駄な生成を防止する
        let msg_id_val = message
            .get("id")
            .or_else(|| data.get("id"))
            .or_else(|| data.get("message_id"));

        if let Some(v) = msg_id_val {
            if let Some(id_str) = v.as_str() {
                if self.seen_message_ids.contains(id_str) {
                    self.record(Outcome::FilteredDuplicate);
                    return;
                }
                let id_owned = id_str.to_string();
                self.seen_message_ids.insert(id_owned.clone());
                self.seen_message_ids_order.push_back(id_owned);
            } else if let Some(id_num) = v.as_u64() {
                let id_owned = id_num.to_string();
                if self.seen_message_ids.contains(&id_owned) {
                    self.record(Outcome::FilteredDuplicate);
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
                self.record(Outcome::FilteredSelf);
                return;
            }
        }

        if self.xs_sender.privacy_mode() {
            println!("📩 [受信] Discord 通知（配信用モード）");
            sender
                .send_notification(
                    "Discord",
                    "新しいメッセージがあります",
                    None,
                    &self.config.notification_sound,
                    self.config.notification_volume,
                    self.config.notification_timeout,
                )
                .await;
            return;
        }

        let sender_name = author
            .get("global_name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .or_else(|| author.get("username").and_then(Value::as_str))
            .unwrap_or("ユーザー");

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

        let mut title = data
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .unwrap_or(sender_name)
            .to_string();
        let mut content = notification_content(message, &data);
        if content.is_empty() {
            self.record(Outcome::FilteredEmpty);
            return;
        }
        title = safe_text(&title, self.config.max_title_length);
        content = safe_text(&content, self.config.max_content_length);

        if self.config.log_message_content && !self.xs_sender.privacy_mode() {
            println!("📩 [受信] {}: {}", title, content);
        } else {
            println!("📩 [受信] Discord 通知");
        }
        sender
            .send_notification(
                &title,
                &content,
                Some(&icon_url),
                &self.config.notification_sound,
                self.config.notification_volume,
                self.config.notification_timeout,
            )
            .await;
    }

    async fn handle_dispatch(&mut self, frame: &Value) {
        let cmd = frame.get("cmd").and_then(Value::as_str);
        let evt = frame.get("evt").and_then(Value::as_str);
        if cmd == Some(CMD_DISPATCH) && evt == Some(EVT_ERROR) {
            println!("⚠️ [Discord RPC] エラー通知を受信しました。");
        } else if cmd == Some(CMD_DISPATCH)
            && matches!(evt, Some(EVT_NOTIFICATION_CREATE | EVT_MESSAGE_CREATE))
        {
            if let Some(data) = frame.get("data") {
                self.handle_notification(data.clone(), evt.unwrap_or(EVT_NOTIFICATION_CREATE))
                    .await;
            }
        }
    }

    /// Discord との接続・認証・通知購読・イベント受信をループで回し続けるメインエンジン
    pub async fn start(&mut self) {
        'connection: loop {
            self.status.store(1, Ordering::SeqCst);
            self.whitelist_status.store(
                if self.config.whitelist_channel_ids.is_empty() {
                    0
                } else {
                    1
                },
                Ordering::SeqCst,
            );
            self.receive_control.set_rpc_user(None);
            self.pending_dispatches.clear();
            self.current_user_id = None;
            if !self.ipc.connect().await {
                self.status.store(0, Ordering::SeqCst);
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
                    if opcode == -1 || resp.get("evt").and_then(|v| v.as_str()) != Some(EVT_READY) {
                        println!("❌ [エラー] Discordハンドシェイク失敗。");
                        self.ipc.close();
                        sleep(Duration::from_secs(3)).await;
                        continue;
                    }
                    if let Some(user_info) = resp.get("data").and_then(|d| d.get("user")) {
                        if let Some(id) = user_info.get("id").and_then(|i| i.as_str()) {
                            self.current_user_id = Some(id.to_string());
                        }
                        println!("✅ [Discord RPC] ハンドシェイク成功！");
                    }
                }
                _ => {
                    println!("❌ [エラー] Discordハンドシェイク応答タイムアウトまたは切断");
                    self.ipc.close();
                    sleep(Duration::from_secs(3)).await;
                    continue;
                }
            }

            match self.authorize_and_authenticate().await {
                Ok(()) => {}
                Err(AuthFailure::Retry(delay)) => {
                    self.status.store(3, Ordering::SeqCst);
                    self.ipc.close();
                    eprintln!(
                        "⚠️ 認証通信が一時的に利用できません。{} 秒後に再試行します。",
                        delay.as_secs()
                    );
                    sleep(delay).await;
                    continue;
                }
                Err(_) => {
                    self.status.store(4, Ordering::SeqCst);
                    self.ipc.close();
                    eprintln!("❌ 認証を停止しました。Discord の承認・アプリ設定を確認し、Ctrl+C で終了して --setup または再起動してください。");
                    std::future::pending::<()>().await;
                }
            }

            let nonce = uuid::Uuid::new_v4().to_string();
            let sub_payload = serde_json::json!({
                "cmd": "SUBSCRIBE",
                "evt": EVT_NOTIFICATION_CREATE,
                "args": {},
                "nonce": nonce
            });
            match self.rpc_request(&sub_payload, Duration::from_secs(5)).await {
                Ok(_) => {}
                Err(AuthFailure::Retry(delay)) => {
                    self.status.store(3, Ordering::SeqCst);
                    self.ipc.close();
                    sleep(delay).await;
                    continue;
                }
                Err(_) => {
                    self.status.store(4, Ordering::SeqCst);
                    self.ipc.close();
                    eprintln!("❌ 通知の購読が拒否されました。Discord の承認を確認して再起動してください。");
                    std::future::pending::<()>().await;
                }
            }
            println!("🔔 [Discord RPC] NOTIFICATION_CREATE の購読を開始しました。");
            let mut rejected = false;
            for channel in self.config.whitelist_channel_ids.clone() {
                let subscription = serde_json::json!({"cmd":"SUBSCRIBE", "evt": EVT_MESSAGE_CREATE, "args":{"channel_id":channel}, "nonce":uuid::Uuid::new_v4().to_string()});
                match self
                    .rpc_request(&subscription, Duration::from_secs(5))
                    .await
                {
                    Ok(_) => {}
                    Err(AuthFailure::Retry(delay)) => {
                        self.status.store(3, Ordering::SeqCst);
                        self.ipc.close();
                        sleep(delay).await;
                        continue 'connection;
                    }
                    Err(_) => rejected = true,
                }
            }
            self.whitelist_status.store(
                if self.config.whitelist_channel_ids.is_empty() {
                    0
                } else if rejected {
                    3
                } else {
                    2
                },
                Ordering::SeqCst,
            );
            self.status.store(2, Ordering::SeqCst);

            let mut last_ping_time = tokio::time::Instant::now();
            while self.ipc.is_connected() {
                match self.recv_async(Duration::from_millis(200)).await {
                    Err(_) => {
                        println!(
                            "\n⚠️ [Discord RPC] パイプ切断を検知しました。再接続を試みます..."
                        );
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
                            self.handle_dispatch(&frame).await;
                        }
                    }
                }
            }
            self.ipc.close();
            self.receive_control.set_rpc_user(None);
            self.status.store(3, Ordering::SeqCst);
            sleep(Duration::from_secs(3)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

    #[tokio::test]
    async fn ipc_receive_diagnostics_work_without_an_overlay_connection() {
        use crate::diagnostics::{Diagnostics, Outcome, Source};
        use crate::discord::receive::ReceiveMode;
        let name = format!(r"\\.\pipe\discord-vr-receive-test-{}", uuid::Uuid::new_v4());
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .unwrap();
        let client = ClientOptions::new().open(&name).unwrap();
        server.connect().await.unwrap();
        let peer = tokio::spawn(async move {
            let mut header = [0u8; 8];
            server.read_exact(&mut header).await.unwrap();
            let mut bytes = vec![0; u32::from_le_bytes(header[4..].try_into().unwrap()) as usize];
            server.read_exact(&mut bytes).await.unwrap();
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(request["cmd"], "SUBSCRIBE");
            let mut frames =
                vec![serde_json::json!({"cmd":"SUBSCRIBE", "nonce":request["nonce"], "data":{}})];
            for (index, event, channel, author) in [
                (1, EVT_NOTIFICATION_CREATE, "100", "201"),
                (2, EVT_MESSAGE_CREATE, "100", "201"),
                (3, EVT_MESSAGE_CREATE, "100", "201"),
                (4, EVT_MESSAGE_CREATE, "101", "201"),
                (5, EVT_MESSAGE_CREATE, "100", "200"),
            ] {
                frames.push(serde_json::json!({"cmd":"DISPATCH", "evt":event, "data":{"channel_id":channel,"message":{"id":format!("mock-{index}"),"author":{"id":author,"username":"private-mock-name"},"content":"private-mock-message"}}}));
            }
            for frame in frames {
                let bytes = serde_json::to_vec(&frame).unwrap();
                server.write_all(&1i32.to_le_bytes()).await.unwrap();
                server
                    .write_all(&(bytes.len() as u32).to_le_bytes())
                    .await
                    .unwrap();
                server.write_all(&bytes).await.unwrap();
            }
            sleep(Duration::from_secs(1)).await;
        });
        let cfg = Config {
            whitelist_channel_ids: vec!["100".into()],
            ..Config::default()
        };
        let control = ReceiveControl::new(&cfg);
        let log = Diagnostics::default();
        let privacy = Arc::new(AtomicBool::new(false));
        let (sender, rx) = crate::vr::create_diagnostic_channel(Client::new(), privacy.clone());
        let worker = tokio::spawn(crate::vr::receive_diagnostic_loop(rx, privacy));
        let mut notifier = DiscordRPCNotifier::new(cfg.clone(), sender, Client::new())
            .with_receive_control(control.clone(), Arc::new(AtomicU8::new(0)))
            .with_trace(log.trace(Source::Live));
        notifier.ipc = DiscordIPC::from_stream(client);
        notifier.current_user_id = Some("200".into());
        notifier.rpc_request(&serde_json::json!({"cmd":"SUBSCRIBE","evt":EVT_NOTIFICATION_CREATE,"args":{},"nonce":"receive-test"}), Duration::from_secs(2)).await.unwrap();
        for index in 0..5 {
            if index == 2 {
                control.configure(&Config {
                    receive_mode: ReceiveMode::Whitelist,
                    ..cfg.clone()
                });
            }
            let (_, frame) = notifier
                .recv_async(Duration::from_secs(2))
                .await
                .unwrap()
                .unwrap();
            notifier.handle_dispatch(&frame).await;
            // Complete each local check before changing policy for the next event.
            if index == 0 || index == 2 {
                let expected = if index == 0 { 1 } else { 2 };
                tokio::time::timeout(Duration::from_secs(2), async {
                    while log.snapshot().live.validated < expected {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            }
        }
        let snapshot = log.snapshot();
        assert_eq!(snapshot.live.received, 5);
        assert_eq!(snapshot.live.validated, 2);
        assert_eq!(snapshot.live.filtered, 3);
        assert_eq!(snapshot.live.sent, 0);
        assert_eq!(snapshot.live.dropped, 0);
        assert_eq!(snapshot.tests.received, 0);
        assert!(snapshot
            .events
            .iter()
            .any(|e| e.outcome == Outcome::FilteredSelf));
        assert!(snapshot.live.last_received_at_ms.is_some());
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(!json.contains("private-mock"));
        assert!(!json.contains("mock-1"));
        worker.abort();
        let _ = worker.await;
        peer.await.unwrap();
    }

    #[test]
    fn transient_refresh_errors_do_not_trigger_reauthorization() {
        use reqwest::StatusCode;
        let invalid = serde_json::json!({"error": "invalid_grant"});
        assert_eq!(
            token_failure(StatusCode::TOO_MANY_REQUESTS, &invalid, Some("120")),
            AuthFailure::Retry(Duration::from_secs(120))
        );
        assert_eq!(
            token_failure(StatusCode::BAD_GATEWAY, &Value::Null, None),
            AuthFailure::Retry(RETRY_DELAY)
        );
        assert_eq!(
            token_failure(StatusCode::BAD_REQUEST, &invalid, None),
            AuthFailure::InvalidToken
        );
        assert_eq!(
            token_failure(
                StatusCode::BAD_REQUEST,
                &serde_json::json!({"error": "invalid_client"}),
                None
            ),
            AuthFailure::ActionRequired
        );
        assert_eq!(
            token_failure(StatusCode::FORBIDDEN, &invalid, None),
            AuthFailure::ActionRequired
        );
    }

    #[tokio::test]
    async fn failed_popup_is_not_requested_again_after_reconnect() {
        let (sender, _, _) =
            crate::vr::create_xsoverlay_channel(Client::new(), Arc::new(AtomicBool::new(false)));
        let mut notifier = DiscordRPCNotifier::new(Config::default(), sender, Client::new());
        notifier.authorization_attempted = true;
        assert_eq!(
            notifier.authorize_and_authenticate().await,
            Err(AuthFailure::ActionRequired)
        );
    }

    #[test]
    fn empty_arrays_do_not_hide_polls_and_null_names_have_fallbacks() {
        let content = notification_content(
            &serde_json::json!({
                "content": "", "sticker_items": [], "embeds": [],
                "poll": {"question": {"text": "test"}}
            }),
            &Value::Null,
        );
        assert!(content.contains("アンケート"));
        assert_eq!(
            notification_content(
                &serde_json::json!({"content": null}),
                &serde_json::json!({"body": "fallback"})
            ),
            "fallback"
        );
    }

    #[test]
    fn text_cannot_inject_rendering_tags_or_terminal_controls() {
        assert_eq!(safe_text("<size=999>\u{1b}hello", 100), "＜size=999＞hello");
        assert_eq!(safe_text("日本語🙂テスト", 4), "日本語🙂...");
    }

    #[tokio::test]
    async fn privacy_toggle_hides_identity_and_duplicate_notifications() {
        let privacy = Arc::new(AtomicBool::new(false));
        let (sender, mut rx, connected) =
            crate::vr::create_xsoverlay_channel(Client::new(), privacy.clone());
        connected.store(true, Ordering::SeqCst);
        let mut notifier = DiscordRPCNotifier::new(Config::default(), sender, Client::new());
        let data = serde_json::json!({
            "message": {"id": "1", "author": {"id": "2", "global_name": null, "username": "Alice"},
                "content": "secret"}
        });
        notifier
            .handle_notification(data.clone(), EVT_NOTIFICATION_CREATE)
            .await;
        let normal = rx.recv().await.unwrap();
        assert_eq!(normal.title, "Alice");
        assert_eq!(normal.content, "secret");
        notifier
            .handle_notification(data, EVT_NOTIFICATION_CREATE)
            .await;
        assert!(rx.try_recv().is_err());
        privacy.store(true, Ordering::SeqCst);
        notifier.handle_notification(serde_json::json!({
            "message": {"id": "3", "author": {"id": "2", "username": "Alice"}, "content": "secret"}
        }), EVT_NOTIFICATION_CREATE).await;
        let hidden = rx.recv().await.unwrap();
        assert_eq!(hidden.title, "Discord");
        assert_eq!(hidden.content, "新しいメッセージがあります");
    }

    #[tokio::test]
    async fn rpc_message_wrappers_keep_authors_deduplicate_and_skip_blocked_messages() {
        let (sender, mut rx, connected) =
            crate::vr::create_xsoverlay_channel(Client::new(), Arc::new(AtomicBool::new(false)));
        connected.store(true, Ordering::SeqCst);
        let mut notifier = DiscordRPCNotifier::new(
            Config {
                whitelist_channel_ids: vec!["100".into()],
                receive_mode: super::super::receive::ReceiveMode::Whitelist,
                ..Config::default()
            },
            sender,
            Client::new(),
        );
        let data = serde_json::json!({
            "channel_id": "100",
            "message": {"id": "101", "author": {"id": "102", "username": "Example"}, "content": "example"}
        });
        notifier
            .handle_notification(data.clone(), "MESSAGE_CREATE")
            .await;
        let received = rx.recv().await.unwrap();
        assert_eq!(received.title, "Example");
        assert_eq!(received.content, "example");
        notifier
            .handle_notification(data, EVT_NOTIFICATION_CREATE)
            .await;
        assert!(rx.try_recv().is_err());
        notifier
            .handle_notification(
                serde_json::json!({
                    "channel_id": "100",
                    "message": {"id": "103", "blocked": true, "content": "blocked"}
                }),
                "MESSAGE_CREATE",
            )
            .await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn filtered_raw_events_do_not_poison_normal_notifications_and_queue_rechecks_policy() {
        use crate::discord::receive::ReceiveMode;
        let (sender, mut rx, connected) =
            crate::vr::create_xsoverlay_channel(Client::new(), Arc::new(AtomicBool::new(false)));
        connected.store(true, Ordering::SeqCst);
        let cfg = Config {
            whitelist_channel_ids: vec!["100".into()],
            ..Config::default()
        };
        let mut notifier = DiscordRPCNotifier::new(cfg, sender, Client::new());
        assert!(notifier.required_scopes().contains(&"messages.read"));
        let allowed = serde_json::json!({"channel_id":"100","message":{"id":"101","author":{"id":"102","username":"Example"},"content":"test"}});
        notifier
            .handle_notification(allowed.clone(), EVT_MESSAGE_CREATE)
            .await;
        assert!(rx.try_recv().is_err());
        notifier
            .handle_notification(allowed, EVT_NOTIFICATION_CREATE)
            .await;
        assert!(rx.recv().await.unwrap().allowed());
        let other =
            serde_json::json!({"channel_id":"200","message":{"id":"201","content":"other"}});
        notifier
            .handle_notification(other.clone(), EVT_NOTIFICATION_CREATE)
            .await;
        let queued = rx.recv().await.unwrap();
        notifier.receive_control.set_mode(ReceiveMode::Whitelist);
        assert!(!queued.allowed());
        notifier
            .handle_notification(other, EVT_NOTIFICATION_CREATE)
            .await;
        assert!(rx.try_recv().is_err());
        notifier
            .handle_notification(
                serde_json::json!({"channel_id":"100","message":{"id":"103","content":"allowed"}}),
                EVT_MESSAGE_CREATE,
            )
            .await;
        assert!(rx.recv().await.unwrap().allowed());
    }

    #[tokio::test]
    async fn missing_message_scope_requests_upgrade_instead_of_refreshing_old_token() {
        let name = format!(r"\\.\pipe\discord-vr-scope-test-{}", uuid::Uuid::new_v4());
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .unwrap();
        let client = ClientOptions::new().open(&name).unwrap();
        server.connect().await.unwrap();
        let peer = tokio::spawn(async move {
            for (command, data, rejected) in [
                (
                    "AUTHENTICATE",
                    serde_json::json!({"user":{"id":"200"},"scopes":["rpc","rpc.notifications.read"]}),
                    false,
                ),
                ("AUTHORIZE", serde_json::json!({"code":4006}), true),
                (
                    "AUTHENTICATE",
                    serde_json::json!({"user":{"id":"200"},"scopes":["rpc","rpc.notifications.read","messages.read"]}),
                    false,
                ),
            ] {
                let mut header = [0u8; 8];
                tokio::time::timeout(Duration::from_secs(2), server.read_exact(&mut header))
                    .await
                    .unwrap()
                    .unwrap();
                let mut bytes =
                    vec![0; u32::from_le_bytes(header[4..].try_into().unwrap()) as usize];
                server.read_exact(&mut bytes).await.unwrap();
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(request["cmd"], command);
                if command == "AUTHORIZE" {
                    assert_eq!(
                        request["args"]["scopes"],
                        serde_json::json!(["rpc", "rpc.notifications.read", "messages.read"])
                    );
                }
                let response = serde_json::json!({"cmd":command,"nonce":request["nonce"],"evt":if rejected {Value::String("ERROR".into())} else {Value::Null},"data":data});
                let bytes = serde_json::to_vec(&response).unwrap();
                server.write_all(&1i32.to_le_bytes()).await.unwrap();
                server
                    .write_all(&(bytes.len() as u32).to_le_bytes())
                    .await
                    .unwrap();
                server.write_all(&bytes).await.unwrap();
            }
            sleep(Duration::from_millis(100)).await;
        });
        // Even a regression must not send dummy OAuth credentials to Discord.
        let http = Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap())
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        let (sender, _rx, _) =
            crate::vr::create_xsoverlay_channel(http.clone(), Arc::new(AtomicBool::new(false)));
        let mut notifier = DiscordRPCNotifier::new(
            Config {
                whitelist_channel_ids: vec!["100".into()],
                access_token: "dummy-access".into(),
                refresh_token: "dummy-refresh".into(),
                ..Config::default()
            },
            sender,
            http,
        );
        notifier.ipc = DiscordIPC::from_stream(client);
        assert_eq!(
            notifier.authorize_and_authenticate().await,
            Err(AuthFailure::ActionRequired)
        );
        assert!(notifier.authorization_attempted);
        assert_eq!(
            notifier.authenticate_with_token("dummy-upgraded").await,
            Ok(())
        );
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn rpc_matches_nonce_and_keeps_notifications_pending() {
        let name = format!(r"\\.\pipe\discord-vr-test-{}", uuid::Uuid::new_v4());
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .unwrap();
        let client = ClientOptions::new().open(&name).unwrap();
        server.connect().await.unwrap();
        let peer = tokio::spawn(async move {
            let mut header = [0u8; 8];
            server.read_exact(&mut header).await.unwrap();
            let len = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
            let mut payload = vec![0; len];
            server.read_exact(&mut payload).await.unwrap();
            for value in [
                serde_json::json!({"cmd": "DISPATCH", "evt": EVT_NOTIFICATION_CREATE, "data": {"id": "notification"}}),
                serde_json::json!({"cmd": "DISPATCH", "evt": EVT_MESSAGE_CREATE, "data": {"id": "message"}}),
                serde_json::json!({"cmd": "GET_CHANNEL", "nonce": "wrong", "data": {"name": "wrong"}}),
                serde_json::json!({"cmd": "GET_CHANNEL", "nonce": "expected", "data": {"name": "correct"}}),
            ] {
                let bytes = serde_json::to_vec(&value).unwrap();
                server.write_all(&1i32.to_le_bytes()).await.unwrap();
                server
                    .write_all(&(bytes.len() as u32).to_le_bytes())
                    .await
                    .unwrap();
                server.write_all(&bytes).await.unwrap();
            }
            // Keep the peer alive until the client consumes its response.
            sleep(Duration::from_millis(100)).await;
        });
        let (sender, _rx, _connected) =
            crate::vr::create_xsoverlay_channel(Client::new(), Arc::new(AtomicBool::new(false)));
        let mut notifier = DiscordRPCNotifier::new(Config::default(), sender, Client::new());
        notifier.ipc = DiscordIPC::from_stream(client);
        let response = notifier
            .rpc_request(
                &serde_json::json!({
                    "cmd": "GET_CHANNEL", "args": {}, "nonce": "expected"
                }),
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert_eq!(response["data"]["name"], "correct");
        assert_eq!(notifier.pending_dispatches.len(), 2);
        let (_, event) = notifier
            .recv_async(Duration::from_secs(1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event["data"]["id"], "notification");
        peer.await.unwrap();
    }
}
