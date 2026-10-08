use discord_vr_core::{
    config::{self, Config, ThemePreference},
    diagnostics::{Diagnostics, Snapshot, Source},
    discord::{
        notifier::DiscordRPCNotifier,
        presence,
        receive::{BotStatus, ReceiveControl, ReceiveMode},
    },
    system,
    vr::{self, XsOverlaySender},
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State,
};
use tokio::sync::Mutex;

type TrayModes = Option<(CheckMenuItem<tauri::Wry>, CheckMenuItem<tauri::Wry>)>;

#[derive(Clone)]
pub struct AppState {
    path: Arc<PathBuf>,
    privacy: Arc<AtomicBool>,
    discord: Arc<AtomicU8>,
    tray: Arc<AtomicBool>,
    controller: Arc<Mutex<Controller>>,
    preview: bool,
    start_settings: bool,
    tray_modes: Arc<std::sync::Mutex<TrayModes>>,
    receive_control: ReceiveControl,
    whitelist_status: Arc<AtomicU8>,
    diagnostics: Option<Diagnostics>,
    updating: Arc<AtomicBool>,
}
struct Controller {
    config: Config,
    services: Option<Services>,
    paused: bool,
    problem: Option<String>,
}
struct Services {
    sender: XsOverlaySender,
    overlay: Arc<AtomicBool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    bot_task: Option<tokio::task::JoinHandle<()>>,
    http: reqwest::Client,
}
impl Services {
    async fn stop(self) {
        if let Some(task) = self.bot_task {
            task.abort();
            let _ = task.await;
        }
        for task in &self.tasks {
            task.abort();
        }
        for task in self.tasks {
            let _ = task.await;
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    theme: ThemePreference,
    discord: u8,
    overlay: bool,
    privacy: bool,
    configured: bool,
    paused: bool,
    tray: bool,
    problem: Option<String>,
    preview: bool,
    start_settings: bool,
    receive_mode: ReceiveMode,
    whitelist_only: bool,
    whitelist_count: usize,
    whitelist_status: u8,
    bot_status: BotStatus,
    bot_configured: bool,
    onboarding_pending: bool,
    debug_enabled: bool,
}
/// Neither Client Secret nor OAuth tokens ever travel to the WebView.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    theme: ThemePreference,
    client_id: String,
    redirect_uri: String,
    secret_set: bool,
    notification_timeout: f64,
    volume_percent: f64,
    max_title_length: usize,
    max_content_length: usize,
    notification_sound: String,
    privacy_mode: bool,
    auto_exit_with_vr: bool,
    whitelist_channel_ids: Vec<String>,
    receive_mode: ReceiveMode,
    bot_token_set: bool,
    bot_guild_id: String,
    bot_user_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsInput {
    theme: ThemePreference,
    client_id: String,
    client_secret: Option<String>,
    redirect_uri: String,
    notification_timeout: f64,
    volume_percent: f64,
    max_title_length: usize,
    max_content_length: usize,
    notification_sound: String,
    privacy_mode: bool,
    auto_exit_with_vr: bool,
    whitelist_channel_ids: Vec<String>,
    receive_mode: ReceiveMode,
    bot_token: Option<String>,
    clear_bot_token: bool,
    bot_guild_id: String,
    bot_user_id: String,
}
#[derive(Serialize)]
pub struct FormError {
    field: Option<&'static str>,
    message: String,
}
impl FormError {
    fn storage() -> Self {
        Self {
            field: None,
            message: "保存できませんでした。設定ファイルと保存先の書き込み権限を確認してください。"
                .into(),
        }
    }
    fn validation(message: &'static str) -> Self {
        let field = match message.split_whitespace().next().unwrap_or("") {
            "CLIENT_ID" => "clientId",
            "CLIENT_SECRET" => "clientSecret",
            "REDIRECT_URI" => "redirectUri",
            "NOTIFICATION_VOLUME" => "volumePercent",
            "NOTIFICATION_TIMEOUT" => "notificationTimeout",
            "MAX_TITLE_LENGTH" => "maxTitleLength",
            "WHITELIST_CHANNEL_IDS" => "whitelistChannelIds",
            "BOT_TOKEN" => "botToken",
            "BOT_GUILD_ID" => "botGuildId",
            "BOT_USER_ID" => "botUserId",
            _ => "maxContentLength",
        };
        let message = match field {
            "clientId" => "Developer Portal の数字の Client ID を入力してください。",
            "clientSecret" => "自分のアプリの Client Secret を入力してください。",
            "redirectUri" => {
                "有効な HTTP(S) URL を入力し、Portal の Redirects と一致させてください。"
            }
            "volumePercent" => "音量は 0 ～ 100% にしてください。",
            "notificationTimeout" => "表示時間は 1 ～ 30 秒にしてください。",
            "maxTitleLength" => "タイトル文字数は 10 ～ 200 にしてください。",
            "whitelistChannelIds" => {
                "許可リストには重複しない数字のチャンネル ID を32件以内で登録してください。"
            }
            "botToken" => "自動判別用 Bot の Token を入力してください。空白は使用できません。",
            "botGuildId" => "Bot と自分が参加しているサーバーの数字の ID を入力してください。",
            "botUserId" => "Discord にログインしている自分の数字のユーザー ID を入力してください。",
            _ => "本文文字数は 10 ～ 2000 にしてください。",
        };
        Self {
            field: Some(field),
            message: message.into(),
        }
    }
}
fn public_settings(cfg: &Config, privacy: bool) -> Settings {
    Settings {
        theme: cfg.theme,
        client_id: if cfg.client_id == "YOUR_CLIENT_ID_HERE" {
            String::new()
        } else {
            cfg.client_id.clone()
        },
        redirect_uri: cfg.redirect_uri.clone(),
        secret_set: !cfg.client_secret.is_empty() && cfg.client_secret != "YOUR_CLIENT_SECRET_HERE",
        notification_timeout: cfg.notification_timeout,
        volume_percent: cfg.notification_volume * 100.0,
        max_title_length: cfg.max_title_length,
        max_content_length: cfg.max_content_length,
        notification_sound: cfg.notification_sound.clone(),
        privacy_mode: privacy,
        auto_exit_with_vr: cfg.auto_exit_with_vr,
        whitelist_channel_ids: cfg.whitelist_channel_ids.clone(),
        receive_mode: cfg.receive_mode,
        bot_token_set: !cfg.bot_token.is_empty(),
        bot_guild_id: cfg.bot_guild_id.clone(),
        bot_user_id: cfg.bot_user_id.clone(),
    }
}
impl AppState {
    pub fn new(
        path: PathBuf,
        preview: bool,
        start_settings: bool,
        mode: Option<bool>,
        migration_problem: Option<String>,
        debug_enabled: bool,
    ) -> Self {
        let loaded = if preview {
            Ok(Config {
                client_id: "123456789012345678".into(),
                client_secret: "preview-only".into(),
                ..Config::default()
            })
        } else {
            config::setup_defaults(&path)
        };
        let (mut cfg,problem) = match loaded { Ok(cfg)=>(cfg,None), Err(_)=>(Config::default(),Some("設定を読み込めませんでした。設定ファイルを確認してください。元のファイルは上書きしません。".into())) };
        if let Some(mode) = mode {
            cfg.privacy_mode = mode;
            if !preview && cfg.validate().is_ok() {
                let _ = config::save_privacy_at(&path, mode);
            }
        }
        let receive_control = ReceiveControl::new(&cfg);
        Self {
            path: Arc::new(path),
            privacy: Arc::new(AtomicBool::new(cfg.privacy_mode)),
            discord: Arc::new(AtomicU8::new(if preview { 2 } else { 0 })),
            tray: Arc::new(AtomicBool::new(false)),
            controller: Arc::new(Mutex::new(Controller {
                config: cfg,
                services: None,
                paused: start_settings,
                problem: problem.or(migration_problem),
            })),
            preview,
            start_settings,
            tray_modes: Arc::new(std::sync::Mutex::new(None)),
            receive_control,
            whitelist_status: Arc::new(AtomicU8::new(0)),
            diagnostics: debug_enabled.then(Diagnostics::default),
            updating: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn tray_available(&self) -> bool {
        self.tray.load(Ordering::SeqCst)
    }
    pub async fn start(&self, app: &AppHandle) {
        let mut controller = self.controller.lock().await;
        apply_theme(app, controller.config.theme);
        self.start_locked(app, &mut controller).await;
    }
    pub async fn resume(&self, app: &AppHandle) {
        if self.updating.load(Ordering::SeqCst) {
            return;
        }
        let mut controller = self.controller.lock().await;
        apply_theme(app, controller.config.theme);
        controller.paused = false;
        self.start_locked(app, &mut controller).await;
    }
    /// Atomically postpone installation while a settings form is open.
    pub async fn prepare_update(&self) -> bool {
        let mut controller = self.controller.lock().await;
        if controller.paused {
            return false;
        }
        self.updating.store(true, Ordering::SeqCst);
        controller.paused = true;
        if let Some(services) = controller.services.take() {
            services.stop().await;
        }
        true
    }
    pub async fn cancel_update(&self, app: &AppHandle) {
        self.updating.store(false, Ordering::SeqCst);
        self.resume(app).await;
    }
    async fn start_locked(&self, app: &AppHandle, controller: &mut Controller) {
        if self.preview
            || self.updating.load(Ordering::SeqCst)
            || controller.paused
            || controller.services.is_some()
            || controller.config.validate().is_err()
        {
            return;
        }
        let mut cfg = match config::load_from_path(&self.path, true) {
            Ok(cfg) => cfg,
            Err(_) => {
                controller.problem =
                    Some("設定を読み込めませんでした。認証情報と保存先を確認してください。".into());
                return;
            }
        };
        controller.config = cfg.clone();
        self.receive_control.configure(&cfg);
        let diagnostic_only = self.diagnostics.is_some();
        if diagnostic_only {
            cfg.log_message_content = false;
        }
        let http = match reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(true)
            .build()
        {
            Ok(client) => client,
            Err(_) => {
                controller.problem =
                    Some("接続の準備ができませんでした。アプリを再起動してください。".into());
                return;
            }
        };
        let (sender, rx, overlay) = if diagnostic_only {
            let (sender, rx) = vr::create_diagnostic_channel(http.clone(), self.privacy.clone());
            (sender, rx, Arc::new(AtomicBool::new(false)))
        } else {
            vr::create_xsoverlay_channel(http.clone(), self.privacy.clone())
        };
        let sender = if let Some(log) = &self.diagnostics {
            sender.with_trace(log.trace(Source::System))
        } else {
            sender
        };
        let overlay_sender = sender.clone();
        let overlay_status = overlay.clone();
        let privacy = self.privacy.clone();
        let mut tasks = vec![tokio::spawn(async move {
            if diagnostic_only {
                vr::receive_diagnostic_loop(rx, privacy).await;
            } else {
                vr::xsoverlay_loop(rx, overlay_status, overlay_sender).await;
            }
        })];
        let mut notifier = DiscordRPCNotifier::new(cfg.clone(), sender.clone(), http.clone())
            .with_status(self.discord.clone())
            .with_receive_control(self.receive_control.clone(), self.whitelist_status.clone());
        if let Some(log) = &self.diagnostics {
            notifier = notifier.with_trace(log.trace(Source::Live));
        }
        tasks.push(tokio::spawn(async move {
            notifier.start().await;
        }));
        if cfg.auto_exit_with_vr && !diagnostic_only {
            let (tx, mut rx) = tokio::sync::mpsc::channel(1);
            tasks.push(tokio::spawn(system::steamvr_monitor_loop(tx)));
            let app = app.clone();
            tasks.push(tokio::spawn(async move {
                if rx.recv().await.is_some() {
                    app.exit(0);
                }
            }));
        }
        controller.services = Some(Services {
            sender,
            overlay,
            tasks,
            bot_task: if cfg.receive_mode == ReceiveMode::Auto {
                Some(tokio::spawn(presence::run(
                    cfg,
                    http.clone(),
                    self.receive_control.clone(),
                )))
            } else {
                None
            },
            http,
        });
    }
    async fn pause(&self) {
        let mut controller = self.controller.lock().await;
        controller.paused = true;
        if let Some(services) = controller.services.take() {
            services.stop().await;
        }
        self.discord
            .store(if self.preview { 2 } else { 0 }, Ordering::SeqCst);
    }
    async fn status(&self) -> Status {
        let controller = self.controller.lock().await;
        Status {
            theme: controller.config.theme,
            discord: self.discord.load(Ordering::SeqCst),
            overlay: if self.preview {
                self.diagnostics.is_none() && !controller.paused
            } else {
                controller
                    .services
                    .as_ref()
                    .is_some_and(|s| s.overlay.load(Ordering::SeqCst))
            },
            privacy: self.privacy.load(Ordering::SeqCst),
            configured: controller.config.validate().is_ok(),
            paused: controller.paused,
            tray: self.tray_available(),
            problem: controller.problem.clone(),
            preview: self.preview,
            start_settings: self.start_settings,
            receive_mode: self.receive_control.mode(),
            whitelist_only: self.receive_control.whitelist_only(),
            whitelist_count: controller.config.whitelist_channel_ids.len(),
            whitelist_status: if self.preview && !controller.config.whitelist_channel_ids.is_empty()
            {
                2
            } else {
                self.whitelist_status.load(Ordering::SeqCst)
            },
            bot_status: self.receive_control.bot_status(),
            bot_configured: controller.config.bot_configured(),
            onboarding_pending: controller.config.needs_onboarding()
                && controller.problem.is_none(),
            debug_enabled: self.diagnostics.is_some(),
        }
    }
    fn sync_tray(&self, app: &AppHandle) {
        let privacy = self.privacy.load(Ordering::SeqCst);
        let menus = self.tray_modes.lock().ok().and_then(|m| m.clone());
        if let Some((normal, private)) = menus {
            let _ = normal.set_checked(!privacy);
            let _ = private.set_checked(privacy);
        }
        if let Some(tray) = app.tray_by_id("main-tray") {
            let _ = tray.set_tooltip(Some(if privacy {
                "Discord → VR · 配信用表示"
            } else {
                "Discord → VR · 通常表示"
            }));
        }
    }
    async fn privacy(&self, app: &AppHandle, enabled: bool) -> Result<(), String> {
        self.privacy.store(enabled, Ordering::SeqCst);
        self.sync_tray(app);
        if self.preview {
            return Ok(());
        }
        config::save_privacy_at(&self.path, enabled).map_err(|_| {
            "切り替えましたが保存できません。設定ファイルと保存先を確認してください。".into()
        })
    }
}
fn debug_log(state: &AppState) -> Result<&Diagnostics, String> {
    state
        .diagnostics
        .as_ref()
        .ok_or_else(|| "この操作は -debug を付けて起動した場合だけ使えます。".into())
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestPresence {
    Online,
    Dnd,
    Offline,
    Unknown,
}

async fn simulate_receive(
    mut cfg: Config,
    log: &Diagnostics,
    raw: bool,
    registered: bool,
    presence: TestPresence,
) -> Result<(), String> {
    let channel = if registered {
        cfg.whitelist_channel_ids.first().cloned().ok_or_else(|| {
            "登録済みの会話を試すには、先に許可リストへ1件以上追加してください。".to_string()
        })?
    } else {
        let mut id = 1u64;
        while cfg.whitelist_channel_ids.contains(&id.to_string()) {
            id += 1;
        }
        id.to_string()
    };
    cfg.log_message_content = false;
    // A separate policy prevents synthetic presence from changing live forwarding.
    if cfg.bot_user_id.is_empty() {
        cfg.bot_user_id = "200".into();
    }
    let control = ReceiveControl::new(&cfg);
    control.set_rpc_user(Some(cfg.bot_user_id.clone()));
    control.set_bot_status(match presence {
        TestPresence::Online => BotStatus::Online,
        TestPresence::Dnd => BotStatus::Dnd,
        TestPresence::Offline => BotStatus::Offline,
        TestPresence::Unknown => BotStatus::Unknown,
    });
    let privacy = cfg.privacy_mode;
    let http = reqwest::Client::new();
    let (sender, mut rx) =
        vr::create_diagnostic_channel(http.clone(), Arc::new(AtomicBool::new(privacy)));
    let mut notifier = DiscordRPCNotifier::new(cfg, sender, http)
        .with_receive_control(control, Arc::new(AtomicU8::new(0)))
        .with_trace(log.trace(Source::Simulation));
    notifier.inject_test_notification(raw, &channel).await;
    while let Ok(note) = rx.try_recv() {
        note.validate_without_sending(privacy);
    }
    Ok(())
}

#[tauri::command]
pub async fn get_debug_snapshot(state: State<'_, AppState>) -> Result<Snapshot, String> {
    Ok(debug_log(&state)?.snapshot())
}
#[tauri::command]
pub async fn clear_debug_events(state: State<'_, AppState>) -> Result<(), String> {
    debug_log(&state)?.clear();
    Ok(())
}
#[tauri::command]
pub async fn run_debug_receive_test(
    state: State<'_, AppState>,
    raw: bool,
    registered: bool,
    presence: TestPresence,
) -> Result<Snapshot, String> {
    let log = debug_log(&state)?;
    let mut cfg = state.controller.lock().await.config.clone();
    cfg.privacy_mode = state.privacy.load(Ordering::SeqCst);
    simulate_receive(cfg, log, raw, registered, presence).await?;
    Ok(log.snapshot())
}

#[tauri::command]
pub async fn get_status(app: AppHandle, state: State<'_, AppState>) -> Result<Status, String> {
    state.sync_tray(&app);
    Ok(state.status().await)
}
#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    if state.updating.load(Ordering::SeqCst) {
        return Err("更新を適用しています。再起動後に設定を開いてください。".into());
    }
    state.pause().await;
    if state.updating.load(Ordering::SeqCst) {
        return Err("更新を適用しています。再起動後に設定を開いてください。".into());
    }
    let controller = state.controller.lock().await;
    Ok(public_settings(
        &controller.config,
        state.privacy.load(Ordering::SeqCst),
    ))
}
#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SettingsInput,
) -> Result<Settings, FormError> {
    let mut controller = state.controller.lock().await;
    if !controller.paused {
        return Err(FormError {
            field: None,
            message: "設定画面を開き直してください。".into(),
        });
    }
    let mut cfg = if state.preview {
        controller.config.clone()
    } else {
        config::setup_defaults(&state.path).map_err(|_| FormError::storage())?
    };
    cfg.client_id = input.client_id.trim().into();
    cfg.theme = input.theme;
    cfg.redirect_uri = input.redirect_uri.trim().into();
    if let Some(secret) = input.client_secret.filter(|v| !v.trim().is_empty()) {
        cfg.client_secret = secret.trim().into();
    }
    cfg.notification_timeout = input.notification_timeout;
    cfg.notification_volume = input.volume_percent / 100.0;
    cfg.max_title_length = input.max_title_length;
    cfg.max_content_length = input.max_content_length;
    cfg.notification_sound = input.notification_sound;
    cfg.privacy_mode = input.privacy_mode;
    cfg.auto_exit_with_vr = input.auto_exit_with_vr;
    cfg.whitelist_channel_ids = input.whitelist_channel_ids;
    cfg.receive_mode = input.receive_mode;
    cfg.bot_guild_id = input.bot_guild_id.trim().into();
    cfg.bot_user_id = input.bot_user_id.trim().into();
    if controller.config.needs_onboarding() {
        cfg.onboarding_completed = Some(false);
    }
    if input.clear_bot_token {
        cfg.bot_token.clear();
    }
    if let Some(token) = input.bot_token.filter(|token| !token.trim().is_empty()) {
        cfg.bot_token = token
            .trim()
            .strip_prefix("Bot ")
            .unwrap_or(token.trim())
            .into();
    }
    cfg.validate().map_err(FormError::validation)?;
    if !cfg.notification_sound.is_empty()
        && cfg.notification_sound != "default"
        && (!PathBuf::from(&cfg.notification_sound).is_absolute()
            || !PathBuf::from(&cfg.notification_sound).is_file())
    {
        return Err(FormError {
            field: Some("notificationSound"),
            message: "通知音に使うファイルを「参照」から選んでください。".into(),
        });
    }
    if !state.preview {
        config::save_setup_at(&state.path, &cfg).map_err(|_| FormError::storage())?;
    }
    state.privacy.store(cfg.privacy_mode, Ordering::SeqCst);
    state.sync_tray(&app);
    controller.config = cfg;
    apply_theme(&app, controller.config.theme);
    state.receive_control.configure(&controller.config);
    controller.problem = None;
    controller.paused = false;
    state.start_locked(&app, &mut controller).await;
    Ok(public_settings(
        &controller.config,
        state.privacy.load(Ordering::SeqCst),
    ))
}
#[tauri::command]
pub async fn complete_onboarding(state: State<'_, AppState>) -> Result<(), String> {
    let mut controller = state.controller.lock().await;
    controller
        .config
        .validate()
        .map_err(|_| "Discord の接続設定を保存してから完了してください。".to_string())?;
    if !state.preview {
        config::complete_onboarding_at(&state.path).map_err(|_| {
            "案内の完了を保存できませんでした。保存先を確認してください。".to_string()
        })?;
    }
    controller.config.onboarding_completed = Some(true);
    Ok(())
}

#[tauri::command]
pub async fn preview_reset_onboarding(state: State<'_, AppState>) -> Result<(), String> {
    if !state.preview {
        return Err("この操作は画面プレビュー専用です。".into());
    }
    let mut controller = state.controller.lock().await;
    controller.config = Config::default();
    controller.paused = false;
    controller.problem = None;
    state.privacy.store(false, Ordering::SeqCst);
    state.receive_control.configure(&controller.config);
    Ok(())
}

#[tauri::command]
pub async fn cancel_settings(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let mut controller = state.controller.lock().await;
    apply_theme(&app, controller.config.theme);
    controller.paused = false;
    state.start_locked(&app, &mut controller).await;
    Ok(())
}
#[tauri::command]
pub async fn set_receive_mode(state: State<'_, AppState>, mode: ReceiveMode) -> Result<(), String> {
    let mut controller = state.controller.lock().await;
    if controller.paused {
        return Err("設定を保存またはキャンセルしてから切り替えてください。".into());
    }
    let mut cfg = controller.config.clone();
    cfg.receive_mode = mode;
    cfg.validate().map_err(|_| "自動判別を使うには、許可リストの設定で Bot Token・サーバー ID・自分のユーザー ID を保存してください。".to_string())?;
    state.receive_control.set_mode(mode);
    controller.config.receive_mode = mode;
    if !state.preview {
        if let Some(services) = controller.services.as_mut() {
            if let Some(task) = services.bot_task.take() {
                task.abort();
                let _ = task.await;
            }
            state.receive_control.set_bot_status(BotStatus::Disabled);
            if mode == ReceiveMode::Auto {
                services.bot_task = Some(tokio::spawn(presence::run(
                    cfg,
                    services.http.clone(),
                    state.receive_control.clone(),
                )));
            }
        }
        config::save_receive_mode_at(&state.path, mode).map_err(|_| {
            "受信モードを切り替えましたが保存できませんでした。保存先を確認してください。"
                .to_string()
        })?;
    }
    Ok(())
}
#[tauri::command]
pub async fn preview_set_presence(
    state: State<'_, AppState>,
    status: String,
) -> Result<(), String> {
    if !state.preview {
        return Err("この操作は画面プレビュー専用です。".into());
    }
    let controller = state.controller.lock().await;
    state
        .receive_control
        .set_rpc_user(Some(controller.config.bot_user_id.clone()));
    state.receive_control.set_bot_status(match status.as_str() {
        "online" => BotStatus::Online,
        "dnd" => BotStatus::Dnd,
        "offline" => BotStatus::Offline,
        _ => BotStatus::Unknown,
    });
    Ok(())
}
#[tauri::command]
pub async fn set_privacy(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    state.privacy(&app, enabled).await
}
#[tauri::command]
pub async fn send_test(state: State<'_, AppState>) -> Result<(), String> {
    if state.diagnostics.is_some() {
        return Err(
            "-debug では Discord の受信だけを確認します。VR 送信は通常起動で試してください。"
                .into(),
        );
    }
    if state.preview {
        return Err("プレビューでは通知を送信しません。".into());
    }
    let sender = {
        let controller = state.controller.lock().await;
        controller
            .services
            .as_ref()
            .filter(|s| s.overlay.load(Ordering::SeqCst))
            .map(|s| s.sender.clone())
    };
    let Some(sender) = sender else {
        return Err("XSOverlay に接続してから試してください。".into());
    };
    let sender = if let Some(log) = &state.diagnostics {
        sender.with_trace(log.trace(Source::OverlayTest))
    } else {
        sender
    };
    if sender
        .send_notification("Discord → VR", "これはテスト通知です", None, "", 0.0, 5.0)
        .await
    {
        Ok(())
    } else {
        Err("送信できませんでした。XSOverlay の接続を確認してください。".into())
    }
}
#[tauri::command]
pub fn open_portal(window: tauri::WebviewWindow) -> Result<(), String> {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let hwnd = window.hwnd().map_err(|_| "画面を取得できませんでした。")?.0 as _;
    let result = unsafe {
        ShellExecuteW(
            hwnd,
            wide("open").as_ptr(),
            wide("https://discord.com/developers/applications").as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize > 32 {
        Ok(())
    } else {
        Err(
            "ブラウザを開けませんでした。discord.com/developers/applications を開いてください。"
                .into(),
        )
    }
}
#[tauri::command]
pub async fn browse_sound(window: tauri::WebviewWindow) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let owner = window.clone();
    window
        .run_on_main_thread(move || {
            use windows_sys::Win32::UI::Controls::Dialogs::*;
            let result = (|| {
                let hwnd = owner
                    .hwnd()
                    .map_err(|_| "画面を取得できませんでした。".to_string())?
                    .0 as _;
                let mut buffer = vec![0u16; 32768];
                let filter = "音声ファイル (*.wav;*.ogg;*.mp3)\0*.wav;*.ogg;*.mp3\0\0"
                    .encode_utf16()
                    .collect::<Vec<_>>();
                let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
                dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
                dialog.hwndOwner = hwnd;
                dialog.lpstrFile = buffer.as_mut_ptr();
                dialog.nMaxFile = buffer.len() as u32;
                dialog.lpstrFilter = filter.as_ptr();
                dialog.Flags = OFN_EXPLORER
                    | OFN_FILEMUSTEXIST
                    | OFN_PATHMUSTEXIST
                    | OFN_NOCHANGEDIR
                    | OFN_DONTADDTORECENT;
                if unsafe { GetOpenFileNameW(&mut dialog) } != 0 {
                    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
                    Ok(Some(String::from_utf16_lossy(&buffer[..end])))
                } else if unsafe { CommDlgExtendedError() } == 0 {
                    Ok(None)
                } else {
                    Err("ファイル選択を開けませんでした。".into())
                }
            })();
            let _ = tx.send(result);
        })
        .map_err(|_| "ファイル選択を開けませんでした。")?;
    rx.await
        .map_err(|_| "ファイル選択を開けませんでした。".to_string())?
}
#[tauri::command]
pub async fn hide_to_tray(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    if !state.tray_available() {
        return Err("トレイを利用できないため、画面を開いたままにします。".into());
    }
    state.resume(&app).await;
    let _ = app.emit("navigate-home", ());
    if let Some(window) = app.get_webview_window("main") {
        window
            .hide()
            .map_err(|_| "画面を収納できませんでした。".to_string())?;
    }
    Ok(())
}
#[tauri::command]
pub fn exit_app(app: AppHandle) {
    app.exit(0);
}
fn apply_theme(app: &AppHandle, theme: ThemePreference) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_theme(match theme {
            ThemePreference::System => None,
            ThemePreference::Light => Some(tauri::Theme::Light),
            ThemePreference::Dark => Some(tauri::Theme::Dark),
        });
    }
}
#[tauri::command]
pub async fn preview_theme(
    app: AppHandle,
    state: State<'_, AppState>,
    theme: ThemePreference,
) -> Result<(), String> {
    let controller = state.controller.lock().await;
    // A stale preview request cannot replace the theme after Save or Cancel.
    apply_theme(
        &app,
        if controller.paused {
            theme
        } else {
            controller.config.theme
        },
    );
    Ok(())
}
pub fn restore(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
pub fn install_tray(app: &AppHandle, state: &AppState) {
    let result = (|| -> tauri::Result<()> {
        let open = MenuItem::with_id(app, "open", "操作画面を開く", true, None::<&str>)?;
        let normal = CheckMenuItem::with_id(
            app,
            "normal",
            "通常表示",
            true,
            !state.privacy.load(Ordering::SeqCst),
            None::<&str>,
        )?;
        let private = CheckMenuItem::with_id(
            app,
            "privacy",
            "配信用表示",
            true,
            state.privacy.load(Ordering::SeqCst),
            None::<&str>,
        )?;
        let test = MenuItem::with_id(app, "test", "テスト通知", true, None::<&str>)?;
        let settings = MenuItem::with_id(app, "settings", "設定", true, None::<&str>)?;
        let exit = MenuItem::with_id(app, "exit", "終了", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&open, &normal, &private, &test, &settings, &exit])?;
        if let Ok(mut modes) = state.tray_modes.lock() {
            *modes = Some((normal, private));
        }
        let mut builder = TrayIconBuilder::with_id("main-tray")
            .menu(&menu)
            .tooltip("Discord → VR")
            .show_menu_on_left_click(false)
            .on_tray_icon_event(|tray, event| {
                if matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }
                ) {
                    restore(tray.app_handle());
                }
            })
            .on_menu_event(|app, event| match event.id.as_ref() {
                "exit" => app.exit(0),
                "open" => restore(app),
                "normal" | "privacy" => {
                    let enabled = event.id.as_ref() == "privacy";
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app.state::<AppState>();
                        if let Err(error) = state.privacy(&app, enabled).await {
                            let _ = app.emit("operation-error", error);
                        }
                    });
                }
                "settings" => {
                    restore(app);
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        app.state::<AppState>().pause().await;
                        let _ = app.emit("navigate-settings", ());
                    });
                }
                "test" => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let result = send_test(app.state::<AppState>()).await;
                        let _ = app.emit(
                            "operation-result",
                            result.map(|_| "送信待ちに追加しました。VR 内で確認してください。"),
                        );
                    });
                }
                _ => {}
            });
        if let Some(icon) = app.default_window_icon() {
            builder = builder.icon(icon.clone());
        }
        builder.build(app)?;
        Ok(())
    })();
    state.tray.store(result.is_ok(), Ordering::SeqCst);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn debug_is_opt_in_and_local_simulations_remain_separate_from_live_receives() {
        let state = AppState::new(PathBuf::new(), true, false, None, None, false);
        assert!(debug_log(&state).is_err());
        assert!(state.status().await.overlay);
        let debug_state = AppState::new(PathBuf::new(), true, false, None, None, true);
        let status = debug_state.status().await;
        assert!(status.debug_enabled);
        assert!(!status.overlay);
        let log = Diagnostics::default();
        let cfg = Config {
            whitelist_channel_ids: vec!["100".into()],
            receive_mode: ReceiveMode::Auto,
            bot_user_id: "200".into(),
            ..Config::default()
        };
        simulate_receive(cfg.clone(), &log, true, true, TestPresence::Dnd)
            .await
            .unwrap();
        simulate_receive(cfg.clone(), &log, false, false, TestPresence::Dnd)
            .await
            .unwrap();
        simulate_receive(cfg, &log, false, false, TestPresence::Online)
            .await
            .unwrap();
        let snapshot = log.snapshot();
        assert_eq!(snapshot.live.received, 0);
        assert_eq!(snapshot.live.sent, 0);
        assert_eq!(snapshot.tests.received, 3);
        assert_eq!(snapshot.tests.filtered, 1);
        assert_eq!(snapshot.tests.validated, 2);
        assert_eq!(snapshot.tests.sent, 0);
        assert!(!serde_json::to_string(&snapshot)
            .unwrap()
            .contains("テスト用サンプル"));
    }
    #[tokio::test]
    async fn update_installation_waits_for_settings_and_blocks_resuming_services() {
        let editing = AppState::new(PathBuf::new(), true, true, None, None, false);
        assert!(!editing.prepare_update().await);
        assert!(!editing.updating.load(Ordering::SeqCst));
        let idle = AppState::new(PathBuf::new(), true, false, None, None, false);
        assert!(idle.prepare_update().await);
        assert!(idle.updating.load(Ordering::SeqCst));
        assert!(idle.status().await.paused);
    }
    #[test]
    fn webview_settings_never_contain_secrets_or_tokens() {
        let cfg = Config {
            client_id: "123456789".into(),
            client_secret: "never-expose-secret".into(),
            access_token: "never-expose-access".into(),
            refresh_token: "never-expose-refresh".into(),
            bot_token: "never-expose-bot".into(),
            ..Config::default()
        };
        let text = serde_json::to_string(&public_settings(&cfg, false)).unwrap();
        assert!(!text.contains("never-expose"));
        assert!(!text.contains("clientSecret"));
        assert!(!text.contains("accessToken"));
        assert!(!text.contains("\"botToken\""));
        assert!(text.contains("secretSet"));
    }
}
