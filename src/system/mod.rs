use std::io::{self, IsTerminal};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Console::{
    AllocConsole, AttachConsole, GetStdHandle, SetConsoleCP, SetConsoleOutputCP,
    ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

const CP_UTF8: u32 = 65001;

static CONSOLE_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn attach_console() {
    CONSOLE_REQUESTED.store(true, Ordering::SeqCst);
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
            if stdout.is_null() || stdout == INVALID_HANDLE_VALUE {
                AllocConsole();
            }
        }
    }
}

pub fn report_error(message: &str) {
    if CONSOLE_REQUESTED.load(Ordering::SeqCst)
        || io::stderr().is_terminal()
        || io::stdout().is_terminal()
    {
        eprintln!("❌ {message}");
    } else {
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                desktop::wide(message).as_ptr(),
                desktop::wide("Discord → VR").as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

pub struct InstanceGuard(HANDLE);
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Windows' Local namespace keeps this guard within the current logon session.
pub fn acquire_instance() -> io::Result<Option<InstanceGuard>> {
    let name: Vec<u16> = "Local\\discord_To_VR.single-instance.v1"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    let error = unsafe { GetLastError() };
    if handle.is_null() {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    if error == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(handle);
        }
        Ok(None)
    } else {
        Ok(Some(InstanceGuard(handle)))
    }
}

/// コンソールの文字コードを UTF-8 に設定し、日本語の文字化けを防ぐ
pub fn init_console_encoding() {
    unsafe {
        SetConsoleOutputCP(CP_UTF8);
        SetConsoleCP(CP_UTF8);
    }
}

pub fn init_display_scaling() {
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
}

pub fn print_privacy_mode(enabled: bool) {
    println!(
        "🔒 配信用モード: {}",
        if enabled {
            "ON（名前・本文・アイコンを非表示）"
        } else {
            "OFF（本文を表示）"
        }
    );
}

pub fn start_privacy_controls(privacy: Arc<AtomicBool>) {
    if !io::stdin().is_terminal() {
        return;
    }
    println!("💡 コンソールで p + Enter: 配信用モード切替 / Ctrl+C: 終了");
    // A detached OS thread avoids blocking Tokio shutdown on a stdin read.
    let _ = std::thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match io::stdin().read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if line.trim().eq_ignore_ascii_case("p") {
                        let enabled = !privacy.fetch_xor(true, Ordering::SeqCst);
                        if let Err(e) = crate::config::save_privacy_at(
                            &crate::config::get_config_path(),
                            enabled,
                        ) {
                            eprintln!("⚠️ 表示モードを保存できませんでした: {e}");
                        }
                        print_privacy_mode(enabled);
                    }
                }
            }
        }
    });
}

/// SteamVR や XSOverlay のプロセスが現在起動しているか確認する
pub fn is_steamvr_running() -> bool {
    // 監視対象プロセス名（小文字ASCII）
    let target_procs = [
        "vrserver.exe",
        "vrmonitor.exe",
        "vrcompositor.exe",
        "xsoverlay.exe",
    ];
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
                        exe_chars[i] <= 0x7f && (exe_chars[i] as u8).to_ascii_lowercase() == b
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
mod desktop;
pub mod setup;
pub mod ui;
