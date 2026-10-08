#![cfg_attr(not(test), windows_subsystem = "windows")]

use discord_vr_core::{config, discord, system, vr};

use discord::notifier::DiscordRPCNotifier;
use reqwest::Client;
use std::sync::atomic::{AtomicBool, AtomicU8};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() {
    system::init_display_scaling();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .iter()
        .any(|a| matches!(a.as_str(), "--no-gui" | "--check-config" | "--help" | "-h"))
    {
        system::attach_console();
    }
    system::init_console_encoding();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "discord_To_VR\n\
            --check-config  接続せずに設定を確認（ファイル変更なし）\n\
            --setup         案内付き設定画面を開く\n\
            --no-gui        コンソールのみで実行\n\
            --privacy       配信用モードで起動（名前・本文・アイコンを非表示）\n\
            --normal        通常モードで起動\n\
            起動中: 通常表示／配信用表示を切替（次回も維持）。--no-gui では p + Enter。\n\
            ×でトレイに収納。終了は画面・トレイの「終了」、または Ctrl+C。\n\
            設定ファイル: {}",
            config::get_config_path().display()
        );
        return;
    }
    if args.iter().any(|arg| {
        !matches!(
            arg.as_str(),
            "--check-config" | "--privacy" | "--normal" | "--setup" | "--no-gui"
        )
    }) || (args.iter().any(|a| a == "--privacy") && args.iter().any(|a| a == "--normal"))
        || (args.iter().any(|a| a == "--setup") && args.len() != 1)
    {
        system::report_error("引数が不正です。--help で使い方を確認してください。");
        std::process::exit(1);
    }
    if args.iter().any(|arg| arg == "--check-config") {
        let path = config::get_config_path();
        match config::load_from_path(&path, false) {
            Ok(_) => println!("✅ 設定は正常です: {}", path.display()),
            Err(e) => {
                eprintln!(
                    "❌ 設定を確認できませんでした: {e}\n設定ファイル: {}",
                    path.display()
                );
                std::process::exit(1);
            }
        }
        return;
    }

    let _instance = match system::acquire_instance() {
        Ok(Some(guard)) => guard,
        Ok(None) => {
            system::report_error("既に起動しています。操作画面、またはトレイの VR アイコンから設定を開いてください。");
            config::safe_exit(0);
        }
        Err(e) => {
            system::report_error(&format!("起動状態を確認できませんでした: {e}"));
            config::safe_exit(1);
        }
    };
    if args.iter().any(|a| a == "--setup") {
        match system::setup::run(&config::get_config_path()) {
            Ok(true) => println!("✅ 設定を保存しました。アプリを起動すると反映されます。"),
            Ok(false) => println!("設定をキャンセルしました。"),
            Err(e) => {
                system::report_error(&format!("設定画面を開けませんでした: {e}"));
                config::safe_exit(1);
            }
        }
        return;
    }

    let mut apply_launch_mode = true;
    loop {
        let mut config = config::load_or_init();
        if apply_launch_mode && args.iter().any(|a| a == "--privacy") {
            config.privacy_mode = true;
        }
        if apply_launch_mode && args.iter().any(|a| a == "--normal") {
            config.privacy_mode = false;
        }
        if apply_launch_mode
            && args
                .iter()
                .any(|a| matches!(a.as_str(), "--normal" | "--privacy"))
        {
            if let Err(e) = config::save_privacy_at(&config::get_config_path(), config.privacy_mode)
            {
                system::report_error(&format!("表示モードを保存できませんでした: {e}"));
            }
        }
        apply_launch_mode = false;
        if !run_session(config, !args.iter().any(|a| a == "--no-gui")).await {
            break;
        }
        // All network tasks have stopped before configuration can be saved.
        if let Err(e) = system::setup::run(&config::get_config_path()) {
            system::report_error(&format!("設定画面を開けませんでした: {e}"));
        }
    }
}

async fn run_session(config: config::Config, show_gui: bool) -> bool {
    let privacy = Arc::new(AtomicBool::new(config.privacy_mode));
    system::print_privacy_mode(config.privacy_mode);
    let auto_exit = config.auto_exit_with_vr;

    let http_client = match Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            system::report_error(&format!("HTTP クライアントを作成できませんでした: {e}"));
            config::safe_exit(1);
        }
    };

    if !show_gui {
        system::start_privacy_controls(privacy.clone());
    }
    let discord_status = Arc::new(AtomicU8::new(0));
    let (xs_sender, xs_rx, xs_is_connected) =
        vr::create_xsoverlay_channel(http_client.clone(), privacy.clone());
    let (mut window, mut ui_rx) = if show_gui {
        match system::ui::start(
            privacy,
            discord_status.clone(),
            xs_is_connected.clone(),
            xs_sender.clone(),
            config::get_config_path(),
        ) {
            Ok((window, rx)) => (Some(window), Some(rx)),
            Err(e) => {
                system::report_error(&format!("操作画面を開けませんでした: {e}"));
                return false;
            }
        }
    } else {
        (None, None)
    };
    let xs_sender_clone = xs_sender.clone();
    let mut xs_task = tokio::spawn(async move {
        vr::xsoverlay_loop(xs_rx, xs_is_connected, xs_sender_clone).await;
    });

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
    let monitor_task = if auto_exit {
        Some(tokio::spawn(system::steamvr_monitor_loop(shutdown_tx)))
    } else {
        None
    };
    let receive_control = discord_vr_core::discord::receive::ReceiveControl::new(&config);
    let bot_task = if config.receive_mode == discord_vr_core::discord::receive::ReceiveMode::Auto {
        Some(tokio::spawn(discord_vr_core::discord::presence::run(
            config.clone(),
            http_client.clone(),
            receive_control.clone(),
        )))
    } else {
        None
    };
    let mut notifier = DiscordRPCNotifier::new(config, xs_sender, http_client)
        .with_status(discord_status)
        .with_receive_control(receive_control, Arc::new(AtomicU8::new(0)));
    let mut settings_requested = false;
    let mut overlay_finished = false;
    tokio::select! {
        _ = notifier.start() => {}
        _ = shutdown_rx.recv(), if auto_exit => {
            println!("\n🛑 VR の終了を検知しました。");
        }
        signal = tokio::signal::ctrl_c() => {
            if let Err(e) = signal { eprintln!("❌ 終了シグナルを監視できませんでした: {e}"); }
            println!("\n🛑 終了します。");
        }
        result = &mut xs_task => {
            overlay_finished = true;
            eprintln!("❌ XSOverlay の送信処理が終了しました: {result:?}");
        }
        result = async {
            match ui_rx.as_mut() {
                Some(rx) => rx.await.unwrap_or(false),
                None => std::future::pending::<bool>().await,
            }
        } => { settings_requested = result; }
    }
    xs_task.abort();
    if !overlay_finished {
        let _ = xs_task.await;
    }
    if let Some(task) = monitor_task {
        task.abort();
        let _ = task.await;
    }
    if let Some(task) = bot_task {
        task.abort();
        let _ = task.await;
    }
    if let Some(window) = window.as_mut() {
        window.close();
    }
    settings_requested
}
