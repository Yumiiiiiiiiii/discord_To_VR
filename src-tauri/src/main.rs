#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod paths;
mod updater;
use discord_vr_core::{config, system};
use tauri::Manager;

fn main() {
    system::init_display_scaling();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help") {
        system::attach_console();
        println!("Discord → VR (Tauri)\n--setup: 設定を開く\n--privacy / --normal: 表示モードを指定\n-debug / --debug: XSOverlay を使わない Discord 受信診断とダミーテスト\n--ui-preview: 接続しない画面プレビュー\n×でトレイへ収納。停止は「終了」。");
        return;
    }
    let preview = args.iter().any(|a| a == "--ui-preview");
    let debug_enabled = args
        .iter()
        .any(|a| matches!(a.as_str(), "-debug" | "--debug"));
    let launch_mode = if args.iter().any(|a| a == "--privacy") {
        Some(true)
    } else if args.iter().any(|a| a == "--normal") {
        Some(false)
    } else {
        None
    };
    let settings = args.iter().any(|a| a == "--setup");
    let mut context = tauri::generate_context!();
    // Preview launches share only a preview instance, never a user's running app.
    if preview {
        context.config_mut().identifier.push_str(".preview");
    }
    // Opt-in inspection of dummy previews only; never present in release builds.
    #[cfg(debug_assertions)]
    if preview {
        if let Ok(port) = std::env::var("DISCORD_VR_PREVIEW_DEBUG_PORT") {
            if let Ok(port) = port.parse::<u16>() {
                context.config_mut().app.windows[0].additional_browser_args =
                    Some(format!("--remote-debugging-port={port}"));
            }
        }
    }
    let result = tauri::Builder::default()
        // Must be first: secondary launches exit before settings, services or updates start.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            app::restore(app);
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            app::get_status,
            app::get_debug_snapshot,
            app::clear_debug_events,
            app::run_debug_receive_test,
            app::get_settings,
            app::save_settings,
            app::preview_theme,
            app::complete_onboarding,
            app::preview_reset_onboarding,
            app::cancel_settings,
            app::set_privacy,
            app::set_receive_mode,
            app::preview_set_presence,
            app::send_test,
            app::open_portal,
            app::browse_sound,
            app::hide_to_tray,
            app::exit_app,
            updater::get_update_status,
            updater::check_updates,
            updater::set_auto_update,
        ])
        .setup(move |host| {
            let handle = host.handle().clone();
            let directory = handle.path().config_dir()?.join(paths::CONFIG_DIRECTORY);
            let previous_directory = handle.path().app_config_dir()?;
            let path = directory.join("config.json");
            let mut migration_problem = None;
            let mut update_migration_failed = false;
            if !preview {
                std::fs::create_dir_all(&directory)?;
                let legacy = config::get_config_path();
                if paths::migrate_config(&path, &previous_directory.join("config.json"), &legacy).is_err() {
                    migration_problem = Some("旧設定を移行できませんでした。以前の AppData フォルダー、実行ファイルと同じフォルダーの config.json、新しい保存先を確認してください。旧ファイルは変更していません。".into());
                }
                update_migration_failed = updater::migrate_preferences(
                    &previous_directory.join("updates.json"), &directory.join("updates.json"),
                ).is_err();
                config::set_config_path(path.clone())?;
            }
            let state = app::AppState::new(path, preview, settings, launch_mode, migration_problem, debug_enabled);
            host.manage(state.clone());
            let updates = updater::UpdateManager::new(
                directory.join("updates.json"), preview, debug_enabled,
            );
            if update_migration_failed {
                updates.report_migration_failure();
            }
            host.manage(updates.clone());
            updater::start(handle.clone(), updates);
            app::install_tray(&handle, &state);
            tauri::async_runtime::spawn(async move {
                state.start(&handle).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if window
                    .try_state::<app::AppState>()
                    .is_some_and(|s| s.tray_available())
                {
                    let handle = window.app_handle().clone();
                    let window = window.clone();
                    tauri::async_runtime::spawn(async move {
                        handle.state::<app::AppState>().resume(&handle).await;
                        let _ = tauri::Emitter::emit(&handle, "navigate-home", ());
                        let _ = window.hide();
                    });
                }
            }
        })
        .run(context);
    if result.is_err() {
        system::report_error("画面を開けませんでした。Microsoft Edge WebView2 Runtime とアプリの保存先を確認してください。");
    }
}
