mod config;
mod discord;
mod system;
mod vr;

use discord::notifier::DiscordRPCNotifier;
use reqwest::Client;

#[tokio::main]
async fn main() {
    // 1. コンソール出力の文字化けを防ぐ設定
    system::init_console_encoding();

    // 2. config.json の読み取りまたは新規自動生成
    let config = config::load_or_init();

    // 3. アイコン画像のダウンロードなどで使用する HTTP クライアント
    let http_client = Client::builder().build().unwrap_or_default();

    // 4. XSOverlay (VRオーバーレイ) と接続・送信を行う WebSocket タスクを起動
    let (xs_sender, xs_rx, xs_is_connected) = vr::create_xsoverlay_channel(http_client.clone());
    let xs_sender_clone = xs_sender.clone();
    tokio::spawn(async move {
        vr::xsoverlay_loop(xs_rx, xs_is_connected, xs_sender_clone).await;
    });

    // 5. SteamVR の終了監視タスクを起動 (VR終了時にアプリを自動終了するため)
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
    tokio::spawn(async move {
        system::steamvr_monitor_loop(shutdown_tx).await;
    });

    // 6. Discord 通知監視エンジンの初期化
    let mut rpc_notifier = DiscordRPCNotifier::new(config, xs_sender, http_client);

    // 7. メインイベントループ（Discord監視、SteamVR終了検知、Ctrl+C を待機）
    tokio::select! {
        _ = rpc_notifier.start() => {}
        _ = shutdown_rx.recv() => {
            println!("\n🛑 [SteamVR終了検知] アプリケーションを自動終了します...");
            std::process::exit(0);
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\n🛑 キーボード割込 (Ctrl+C) を検知しました。終了します...");
            std::process::exit(0);
        }
    }

    std::process::exit(0);
}
