//! Bot Gateway monitor. No message intents, message handlers, or outbound chat APIs.
use super::receive::{BotStatus, ReceiveControl};
use crate::config::Config;
use futures_util::{SinkExt, StreamExt};
use reqwest::{
    header::{HeaderValue, AUTHORIZATION},
    Client, Url,
};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::{
    net::TcpStream,
    time::{sleep, sleep_until, timeout, Instant},
};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
};

const INTENTS: u64 = 1 | (1 << 8); // GUILDS + GUILD_PRESENCES only.
const IO_TIMEOUT: Duration = Duration::from_secs(15);
static LAST_IDENTIFY: tokio::sync::Mutex<Option<Instant>> = tokio::sync::Mutex::const_new(None);
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
enum Failure {
    Retry(Duration),
    Fatal,
}
struct DisconnectGuard(ReceiveControl);
impl Drop for DisconnectGuard {
    fn drop(&mut self) {
        self.0.set_bot_status(BotStatus::Unknown);
    }
}
#[derive(Default)]
struct Session {
    id: Option<String>,
    sequence: Option<u64>,
    url: Option<String>,
}
struct Monitor<'a> {
    guild: &'a str,
    user: &'a str,
    control: &'a ReceiveControl,
    request_nonce: Option<String>,
    request_sequence: Option<u64>,
    last_presence_sequence: Option<u64>,
}
impl Monitor<'_> {
    fn request(&mut self, sequence: Option<u64>) -> Value {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        self.request_nonce = Some(nonce.clone());
        self.request_sequence = sequence;
        json!({"op":8,"d":{"guild_id":self.guild,"user_ids":[self.user],"presences":true,"nonce":nonce}})
    }
    fn apply_presence(&self, data: &Value) -> bool {
        if data.pointer("/user/id").and_then(Value::as_str) != Some(self.user) {
            return false;
        }
        self.control
            .set_bot_status(match data.get("status").and_then(Value::as_str) {
                Some("dnd") => BotStatus::Dnd,
                Some("online") => BotStatus::Online,
                Some("idle") => BotStatus::Idle,
                Some("offline" | "invisible") => BotStatus::Offline,
                _ => BotStatus::Unknown,
            });
        true
    }
    fn dispatch(&mut self, event: &str, data: &Value, sequence: Option<u64>) -> Option<Value> {
        match event {
            "READY" => {
                if !data
                    .get("guilds")
                    .and_then(Value::as_array)
                    .is_some_and(|guilds| {
                        guilds
                            .iter()
                            .any(|g| g.get("id").and_then(Value::as_str) == Some(self.guild))
                    })
                {
                    self.control.set_bot_status(BotStatus::MissingGuild);
                }
            }
            "RESUMED" => return Some(self.request(sequence)),
            "GUILD_CREATE" if data.get("id").and_then(Value::as_str) == Some(self.guild) => {
                self.control.set_bot_status(BotStatus::Unknown);
                if let Some(presences) = data.get("presences").and_then(Value::as_array) {
                    for presence in presences {
                        if self.apply_presence(presence) {
                            self.last_presence_sequence = sequence;
                            break;
                        }
                    }
                }
                return Some(self.request(sequence));
            }
            "GUILD_DELETE" if data.get("id").and_then(Value::as_str) == Some(self.guild) => {
                self.request_nonce = None;
                self.control.set_bot_status(
                    if data.get("unavailable").and_then(Value::as_bool) == Some(true) {
                        BotStatus::Unknown
                    } else {
                        BotStatus::MissingGuild
                    },
                );
            }
            "PRESENCE_UPDATE"
                if data.get("guild_id").and_then(Value::as_str) == Some(self.guild) =>
            {
                if self.apply_presence(data) {
                    self.last_presence_sequence = sequence;
                }
            }
            "GUILD_MEMBERS_CHUNK"
                if data.get("guild_id").and_then(Value::as_str) == Some(self.guild)
                    && self.request_nonce.as_deref()
                        == data.get("nonce").and_then(Value::as_str)
                    && self.request_nonce.is_some() =>
            {
                self.request_nonce = None;
                if self.last_presence_sequence > self.request_sequence {
                    return None;
                }
                if let Some(presences) = data.get("presences").and_then(Value::as_array) {
                    for presence in presences {
                        if self.apply_presence(presence) {
                            self.last_presence_sequence = sequence;
                            return None;
                        }
                    }
                }
                let member_exists =
                    data.get("members")
                        .and_then(Value::as_array)
                        .is_some_and(|members| {
                            members.iter().any(|m| {
                                m.pointer("/user/id").and_then(Value::as_str) == Some(self.user)
                            })
                        });
                self.control.set_bot_status(if member_exists {
                    BotStatus::Offline
                } else {
                    BotStatus::MissingUser
                });
            }
            _ => {}
        }
        None
    }
}
fn trusted_gateway(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    let host = url.host_str()?;
    if url.scheme() != "wss"
        || !(host == "gateway.discord.gg"
            || (host.starts_with("gateway-") && host.ends_with(".discord.gg")))
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_query(Some("v=10&encoding=json"));
    url.set_fragment(None);
    Some(url.into())
}
async fn gateway_url(http: &Client, token: &str) -> Result<String, Failure> {
    let mut authorization =
        HeaderValue::from_str(&format!("Bot {token}")).map_err(|_| Failure::Fatal)?;
    authorization.set_sensitive(true);
    let response = http
        .get("https://discord.com/api/v10/gateway/bot")
        .header(AUTHORIZATION, authorization)
        .send()
        .await
        .map_err(|_| Failure::Retry(Duration::from_secs(10)))?;
    let status = response.status();
    if status.as_u16() == 429 {
        let seconds = response
            .json::<Value>()
            .await
            .ok()
            .and_then(|data| data.get("retry_after").and_then(Value::as_f64))
            .unwrap_or(30.0);
        return Err(Failure::Retry(Duration::from_secs_f64(
            if seconds.is_finite() {
                seconds.clamp(1.0, 86400.0)
            } else {
                30.0
            },
        )));
    }
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(Failure::Fatal);
    }
    if !status.is_success() {
        return Err(Failure::Retry(Duration::from_secs(30)));
    }
    let data: Value = response
        .json()
        .await
        .map_err(|_| Failure::Retry(Duration::from_secs(30)))?;
    if data
        .get("shards")
        .and_then(Value::as_u64)
        .is_some_and(|count| count > 1)
    {
        return Err(Failure::Fatal);
    }
    if data
        .pointer("/session_start_limit/remaining")
        .and_then(Value::as_u64)
        == Some(0)
    {
        let reset = data
            .pointer("/session_start_limit/reset_after")
            .and_then(Value::as_u64)
            .unwrap_or(60000);
        return Err(Failure::Retry(Duration::from_millis(
            reset.clamp(5000, 86400000),
        )));
    }
    data.get("url")
        .and_then(Value::as_str)
        .and_then(trusted_gateway)
        .ok_or(Failure::Fatal)
}
async fn send(socket: &mut Socket, data: Value) -> Result<(), Failure> {
    timeout(
        IO_TIMEOUT,
        socket.send(Message::Text(data.to_string().into())),
    )
    .await
    .map_err(|_| Failure::Retry(Duration::from_secs(10)))?
    .map_err(|_| Failure::Retry(Duration::from_secs(10)))
}
async fn connection(
    url: &str,
    cfg: &Config,
    control: &ReceiveControl,
    session: &mut Session,
) -> Result<(), Failure> {
    let websocket = WebSocketConfig::default()
        .max_message_size(Some(2 * 1024 * 1024))
        .max_frame_size(Some(2 * 1024 * 1024));
    let (mut socket, _) = timeout(
        IO_TIMEOUT,
        connect_async_with_config(url, Some(websocket), false),
    )
    .await
    .map_err(|_| Failure::Retry(Duration::from_secs(10)))?
    .map_err(|_| Failure::Retry(Duration::from_secs(10)))?;
    let hello = timeout(IO_TIMEOUT, socket.next())
        .await
        .ok()
        .flatten()
        .and_then(Result::ok)
        .ok_or(Failure::Retry(Duration::from_secs(10)))?;
    let hello: Value = serde_json::from_slice(&hello.into_data())
        .map_err(|_| Failure::Retry(Duration::from_secs(10)))?;
    let interval = hello
        .pointer("/d/heartbeat_interval")
        .and_then(Value::as_u64)
        .filter(|ms| (1000..=300000).contains(ms))
        .filter(|_| hello.get("op").and_then(Value::as_u64) == Some(10))
        .ok_or(Failure::Retry(Duration::from_secs(10)))?;
    let identify = if let (Some(id), Some(sequence)) = (&session.id, session.sequence) {
        json!({"op":6,"d":{"token":cfg.bot_token,"session_id":id,"seq":sequence}})
    } else {
        json!({"op":2,"d":{"token":cfg.bot_token,"intents":INTENTS,"properties":{"os":"windows","browser":"discord-to-vr","device":"discord-to-vr"},"large_threshold":50}})
    };
    send(&mut socket, identify).await?;
    let mut monitor = Monitor {
        guild: &cfg.bot_guild_id,
        user: &cfg.bot_user_id,
        control,
        request_nonce: None,
        request_sequence: None,
        last_presence_sequence: None,
    };
    let mut heartbeat = Instant::now()
        + Duration::from_millis((uuid::Uuid::new_v4().as_u128() % interval as u128) as u64);
    let mut pending_ack = false;
    loop {
        tokio::select! {
            _ = sleep_until(heartbeat) => {
                if pending_ack { return Err(Failure::Retry(Duration::from_secs(10))); }
                send(&mut socket, json!({"op":1,"d":session.sequence})).await?;
                pending_ack = true;
                heartbeat = Instant::now() + Duration::from_millis(interval);
            }
            frame = socket.next() => {
                match frame {
                    Some(Ok(Message::Close(close))) => {
                        if let Some(close) = close {
                            match u16::from(close.code) {
                                4004 | 4010..=4014 => return Err(Failure::Fatal),
                                4007 | 4009 => *session = Session::default(),
                                _ => {}
                            }
                        }
                        return Err(Failure::Retry(Duration::from_secs(10)));
                    }
                    Some(Ok(Message::Ping(bytes))) => { timeout(IO_TIMEOUT, socket.send(Message::Pong(bytes))).await.map_err(|_| Failure::Retry(Duration::from_secs(10)))?.map_err(|_| Failure::Retry(Duration::from_secs(10)))?; }
                    Some(Ok(Message::Text(bytes))) => {
                        let data: Value = serde_json::from_str(&bytes).map_err(|_| Failure::Retry(Duration::from_secs(10)))?;
                        if let Some(sequence) = data.get("s").and_then(Value::as_u64) { session.sequence = Some(sequence); }
                        match data.get("op").and_then(Value::as_u64) {
                            Some(0) => {
                                let event = data.get("t").and_then(Value::as_str).unwrap_or("");
                                let body = &data["d"];
                                if event == "READY" {
                                    session.id = body.get("session_id").and_then(Value::as_str).map(str::to_string);
                                    session.url = body.get("resume_gateway_url").and_then(Value::as_str).and_then(trusted_gateway);
                                }
                                if let Some(request) = monitor.dispatch(event, body, session.sequence) { send(&mut socket, request).await?; }
                            }
                            Some(1) => {
                                send(&mut socket, json!({"op":1,"d":session.sequence})).await?;
                                pending_ack = true; heartbeat = Instant::now() + Duration::from_millis(interval);
                            }
                            Some(7) => return Err(Failure::Retry(Duration::from_secs(5))),
                            Some(9) => { if data.get("d").and_then(Value::as_bool) != Some(true) { *session = Session::default(); } return Err(Failure::Retry(Duration::from_secs(5))); }
                            Some(11) => pending_ack = false,
                            _ => {}
                        }
                    }
                    Some(Ok(_)) => {},
                    _ => return Err(Failure::Retry(Duration::from_secs(10))),
                }
            }
        }
    }
}
pub async fn run(cfg: Config, http: Client, control: ReceiveControl) {
    let _guard = DisconnectGuard(control.clone());
    let mut session = Session::default();
    let mut cached_url = None;
    loop {
        control.set_bot_status(BotStatus::Connecting);
        let result = async {
            // Recheck limits before each new identify; resume does not consume one.
            if cached_url.is_none() || session.id.is_none() {
                cached_url = Some(gateway_url(&http, &cfg.bot_token).await?);
            }
            let url = session
                .url
                .clone()
                .or_else(|| cached_url.clone())
                .ok_or(Failure::Fatal)?;
            if session.id.is_none() {
                let mut last = LAST_IDENTIFY.lock().await;
                if let Some(previous) = *last {
                    sleep_until(previous + Duration::from_secs(5)).await;
                }
                *last = Some(Instant::now());
            }
            connection(&url, &cfg, &control, &mut session).await
        }
        .await;
        control.set_bot_status(BotStatus::Unknown);
        match result {
            Err(Failure::Fatal) => {
                control.set_bot_status(BotStatus::Error);
                std::future::pending::<()>().await;
            }
            Err(Failure::Retry(delay)) => sleep(delay).await,
            Ok(()) => sleep(Duration::from_secs(10)).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::receive::ReceiveMode;
    #[tokio::test]
    async fn missing_heartbeat_ack_disconnects_and_restricts_auto() {
        use tokio::net::TcpListener;
        use tokio_tungstenite::accept_async;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            socket
                .send(Message::Text(
                    json!({"op":10,"d":{"heartbeat_interval":1000}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            let identify = socket.next().await.unwrap().unwrap();
            let identify: Value = serde_json::from_slice(&identify.into_data()).unwrap();
            assert_eq!(identify["op"], 2);
            // Deliberately withhold the heartbeat ACK.
            while let Some(Ok(_)) = socket.next().await {}
        });
        let cfg = Config {
            bot_token: "dummy-bot-token".into(),
            bot_user_id: "200".into(),
            receive_mode: ReceiveMode::Auto,
            ..Config::default()
        };
        let control = ReceiveControl::new(&cfg);
        control.set_rpc_user(Some("200".into()));
        control.set_bot_status(BotStatus::Online);
        let guard = DisconnectGuard(control.clone());
        let mut session = Session::default();
        assert!(matches!(
            timeout(
                Duration::from_secs(4),
                connection(&format!("ws://{address}"), &cfg, &control, &mut session)
            )
            .await
            .unwrap(),
            Err(Failure::Retry(_))
        ));
        drop(guard);
        assert!(control.whitelist_only());
        timeout(Duration::from_secs(2), peer)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn mock_gateway_identifies_resumes_heartbeats_and_refreshes_presence() {
        use tokio::net::TcpListener;
        use tokio_tungstenite::{
            accept_async,
            tungstenite::protocol::{frame::coding::CloseCode, CloseFrame},
        };
        async fn expected(socket: &mut WebSocketStream<TcpStream>, op: u64) -> Value {
            loop {
                let frame = timeout(Duration::from_secs(5), socket.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                let data: Value = serde_json::from_slice(&frame.into_data()).unwrap();
                if data["op"] == 1 && op != 1 {
                    socket
                        .send(Message::Text(json!({"op":11}).to_string().into()))
                        .await
                        .unwrap();
                    continue;
                }
                assert_eq!(data["op"], op);
                return data;
            }
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            for resume in [false, true] {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = accept_async(stream).await.unwrap();
                socket
                    .send(Message::Text(
                        json!({"op":10,"d":{"heartbeat_interval":1000}})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                let identify = expected(&mut socket, if resume { 6 } else { 2 }).await;
                assert_eq!(identify["d"]["token"], "dummy-bot-token");
                if resume {
                    assert_eq!(identify["d"]["session_id"], "test-session");
                    assert_eq!(identify["d"]["seq"], 4);
                    socket
                        .send(Message::Text(
                            json!({"op":0,"t":"RESUMED","s":5,"d":{}})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                } else {
                    assert_eq!(identify["d"]["intents"], 257);
                    for payload in [
                        json!({"op":0,"t":"READY","s":1,"d":{"session_id":"test-session","resume_gateway_url":"wss://gateway.discord.gg","guilds":[{"id":"300"}]}}),
                        json!({"op":0,"t":"GUILD_CREATE","s":2,"d":{"id":"300","presences":[]}}),
                    ] {
                        socket
                            .send(Message::Text(payload.to_string().into()))
                            .await
                            .unwrap();
                    }
                }
                let request = expected(&mut socket, 8).await;
                assert_eq!(request["d"]["guild_id"], "300");
                assert_eq!(request["d"]["user_ids"], json!(["200"]));
                socket.send(Message::Text(json!({"op":0,"t":"GUILD_MEMBERS_CHUNK","s":if resume {6} else {3},"d":{"guild_id":"300","nonce":request["d"]["nonce"],"presences":[{"user":{"id":"200"},"status":"online"}]}}).to_string().into())).await.unwrap();
                if resume {
                    socket
                        .send(Message::Text(json!({"op":1}).to_string().into()))
                        .await
                        .unwrap();
                    let heartbeat = expected(&mut socket, 1).await;
                    assert_eq!(heartbeat["d"], 6);
                    socket
                        .send(Message::Text(json!({"op":11}).to_string().into()))
                        .await
                        .unwrap();
                    socket
                        .send(Message::Close(Some(CloseFrame {
                            code: CloseCode::Library(4009),
                            reason: "expired".into(),
                        })))
                        .await
                        .unwrap();
                } else {
                    socket.send(Message::Text(json!({"op":0,"t":"PRESENCE_UPDATE","s":4,"d":{"guild_id":"300","user":{"id":"200"},"status":"dnd"}}).to_string().into())).await.unwrap();
                    socket
                        .send(Message::Text(json!({"op":7}).to_string().into()))
                        .await
                        .unwrap();
                }
            }
        });
        let cfg = Config {
            bot_token: "dummy-bot-token".into(),
            bot_guild_id: "300".into(),
            bot_user_id: "200".into(),
            receive_mode: ReceiveMode::Auto,
            ..Config::default()
        };
        let control = ReceiveControl::new(&cfg);
        control.set_rpc_user(Some("200".into()));
        let mut session = Session::default();
        let url = format!("ws://{address}"); // Test-only localhost; production URLs are restricted to Discord WSS.
        assert!(matches!(
            timeout(
                Duration::from_secs(5),
                connection(&url, &cfg, &control, &mut session)
            )
            .await
            .unwrap(),
            Err(Failure::Retry(_))
        ));
        assert_eq!(control.bot_status(), BotStatus::Dnd);
        control.set_bot_status(BotStatus::Connecting);
        assert!(control.whitelist_only());
        assert!(matches!(
            timeout(
                Duration::from_secs(5),
                connection(&url, &cfg, &control, &mut session)
            )
            .await
            .unwrap(),
            Err(Failure::Retry(_))
        ));
        assert_eq!(control.bot_status(), BotStatus::Online);
        assert!(session.id.is_none());
        peer.await.unwrap();
    }

    #[test]
    fn snapshots_and_updates_only_use_the_configured_guild_and_user() {
        let cfg = Config {
            receive_mode: ReceiveMode::Auto,
            bot_user_id: "200".into(),
            whitelist_channel_ids: vec!["100".into()],
            ..Config::default()
        };
        let control = ReceiveControl::new(&cfg);
        control.set_rpc_user(Some("200".into()));
        let mut monitor = Monitor {
            guild: "300",
            user: "200",
            control: &control,
            request_nonce: None,
            request_sequence: None,
            last_presence_sequence: None,
        };
        monitor.dispatch(
            "PRESENCE_UPDATE",
            &json!({"guild_id":"301","user":{"id":"200"},"status":"online"}),
            Some(1),
        );
        assert!(control.whitelist_only());
        monitor.dispatch(
            "PRESENCE_UPDATE",
            &json!({"guild_id":"300","user":{"id":"201"},"status":"online"}),
            Some(2),
        );
        assert!(control.whitelist_only());
        let request = monitor
            .dispatch(
                "GUILD_CREATE",
                &json!({"id":"300","presences":[{"user":{"id":"200"},"status":"dnd"}]}),
                Some(3),
            )
            .unwrap();
        assert!(control.whitelist_only());
        assert_eq!(request["op"], 8);
        assert_eq!(request["d"]["user_ids"], json!(["200"]));
        monitor.dispatch(
            "PRESENCE_UPDATE",
            &json!({"guild_id":"300","user":{"id":"200"},"status":"online"}),
            Some(4),
        );
        assert!(!control.whitelist_only());
        monitor.dispatch("GUILD_MEMBERS_CHUNK", &json!({"guild_id":"300","nonce":request["d"]["nonce"],"presences":[{"user":{"id":"200"},"status":"dnd"}]}), Some(5));
        assert!(!control.whitelist_only()); // Late snapshots cannot undo a newer presence event.
        monitor.dispatch("GUILD_DELETE", &json!({"id":"300"}), Some(6));
        assert!(control.whitelist_only());
        let guard = DisconnectGuard(control.clone());
        control.set_bot_status(BotStatus::Online);
        drop(guard);
        assert!(control.whitelist_only());
    }
    #[test]
    fn gateway_tokens_only_go_to_discord_tls_hosts() {
        assert!(trusted_gateway("wss://gateway.discord.gg").is_some());
        assert!(trusted_gateway("wss://gateway-us-east1-b.discord.gg").is_some());
        for url in [
            "ws://gateway.discord.gg",
            "wss://example.com",
            "wss://gateway.discord.gg.evil.example",
            "wss://user:pass@gateway.discord.gg",
            "wss://gateway.discord.gg:123",
        ] {
            assert!(trusted_gateway(url).is_none());
        }
        assert_eq!(INTENTS, 257); // No messages, DM, content, or full-member-list intent.
    }
}
