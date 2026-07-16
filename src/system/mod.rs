use std::time::Duration;
use tokio::time::sleep;
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Console::{SetConsoleCP, SetConsoleOutputCP};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

const CP_UTF8: u32 = 65001;

/// コンソールの文字コードを UTF-8 に設定し、日本語の文字化けを防ぐ
pub fn init_console_encoding() {
    unsafe {
        SetConsoleOutputCP(CP_UTF8);
        SetConsoleCP(CP_UTF8);
    }
}

/// SteamVR や XSOverlay のプロセスが現在起動しているか確認する
pub fn is_steamvr_running() -> bool {
    // 監視対象プロセス名（小文字ASCII）
    let target_procs = ["vrserver.exe", "vrmonitor.exe", "vrcompositor.exe", "xsoverlay.exe"];
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
            return true;
        }

        let mut pe32: PROCESSENTRY32W = std::mem::zeroed();
        pe32.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        let mut running = false;
        if Process32FirstW(snapshot, &mut pe32) != 0 {
            loop {
                let len = pe32
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(pe32.szExeFile.len());
                let exe_chars = &pe32.szExeFile[..len];

                // ヒープアロケーション(String作成)を行わずに直接バイト比較で高速チェック
                let matches = target_procs.iter().any(|target| {
                    if target.len() != len {
                        return false;
                    }
                    target.bytes().enumerate().all(|(i, b)| {
                        (exe_chars[i] as u8).to_ascii_lowercase() == b
                    })
                });

                if matches {
                    running = true;
                    break;
                }
                if Process32NextW(snapshot, &mut pe32) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        running
    }
}

/// SteamVR の終了を定期監視し、終了したと判断したらアプリ自動終了シグナルを送る
pub async fn steamvr_monitor_loop(shutdown_tx: tokio::sync::mpsc::Sender<()>) {
    let mut consecutive_not_running = 0;
    let mut steamvr_was_running = tokio::task::spawn_blocking(is_steamvr_running)
        .await
        .unwrap_or(true);

    loop {
        sleep(Duration::from_secs(3)).await;
        
        let running = tokio::task::spawn_blocking(is_steamvr_running)
            .await
            .unwrap_or(true);

        if running {
            steamvr_was_running = true;
            consecutive_not_running = 0;
        } else if steamvr_was_running {
            consecutive_not_running += 1;
            if consecutive_not_running >= 2 {
                println!("\n🛑 [SteamVR終了検知] SteamVR の終了を検知しました。アプリを自動終了します...");
                let _ = shutdown_tx.send(()).await;
                break;
            }
        }
    }
}
