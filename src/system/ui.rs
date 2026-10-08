//! Daily controls and tray. Network work stays on Tokio.
use super::desktop::{self, *};
use crate::{config, vr::XsOverlaySender};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
use std::sync::Arc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::DRAWITEMSTRUCT;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const DISCORD: i32 = 201;
const OVERLAY: i32 = 202;
const PRIVACY: i32 = 203;
const NORMAL: i32 = 204;
const TEST: i32 = 205;
const TEST_STATUS: i32 = 206;
const SETTINGS: i32 = 207;
const EXIT: i32 = 208;
const OPEN: i32 = 209;
const HEADER: i32 = 220;
const SUBTITLE: i32 = 221;
const READY: i32 = 222;
const DISCORD_TITLE: i32 = 223;
const OVERLAY_TITLE: i32 = 224;
const DISPLAY_TITLE: i32 = 225;
const DISPLAY_HELP: i32 = 226;
const PREVIEW_TITLE: i32 = 227;
const PREVIEW: i32 = 228;
const MODE_STATUS: i32 = 229;
const CAUTION: i32 = 230;
const TRAY_HINT: i32 = 231;
const TRAY_MESSAGE: u32 = WM_APP + 1;
const FORCE_CLOSE: u32 = WM_APP + 2;
// Shell's keyboard activation is NIN_SELECT | NINF_KEY (not exported by 0.59).
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;

pub struct Window {
    handle: Arc<AtomicIsize>,
    closing: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Window {
    pub fn close(&mut self) {
        self.closing.store(true, Ordering::SeqCst);
        let handle = self.handle.load(Ordering::SeqCst);
        if handle != 0 {
            unsafe {
                PostMessageW(handle as HWND, FORCE_CLOSE, 0, 0);
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        self.close();
    }
}
struct State {
    privacy: Arc<AtomicBool>,
    discord: Arc<AtomicU8>,
    overlay: Arc<AtomicBool>,
    sender: XsOverlaySender,
    runtime: tokio::runtime::Handle,
    path: PathBuf,
    handle: Arc<AtomicIsize>,
    closing: Arc<AtomicBool>,
    test_status: Arc<AtomicU8>,
    settings_requested: bool,
    failed: bool,
    skin: Option<Skin>,
    initialized: bool,
    tray: bool,
    icon: HICON,
    taskbar_created: u32,
    last_status: Option<(u8, bool, bool, u8)>,
    tray_tip: String,
    scroll: i32,
}
unsafe fn layout(hwnd: HWND, state: *mut State) {
    let Some(skin) = (*state).skin.as_ref() else {
        return;
    };
    (*state).scroll = scroll_range(hwnd, skin, 628, (*state).scroll);
    let (w, _) = client_size(hwnd, skin);
    let width = w - 48;
    let half = (width - 16) / 2;
    for (id, mut rect) in [
        (HEADER, [24, 20, width, 32]),
        (SUBTITLE, [24, 56, width, 20]),
        (READY, [36, 94, width - 24, 24]),
        (DISCORD_TITLE, [40, 150, half - 32, 22]),
        (DISCORD, [40, 180, half - 32, 42]),
        (OVERLAY_TITLE, [40 + half + 16, 150, half - 32, 22]),
        (OVERLAY, [40 + half + 16, 180, half - 32, 42]),
        (DISPLAY_TITLE, [24, 252, width, 22]),
        (DISPLAY_HELP, [24, 278, width, 20]),
        (NORMAL, [24, 308, half, 76]),
        (PRIVACY, [24 + half + 16, 308, half, 76]),
        (PREVIEW_TITLE, [40, 412, 112, 20]),
        (
            PREVIEW,
            [
                if (*state).privacy.load(Ordering::SeqCst) {
                    160
                } else {
                    208
                },
                404,
                width - 200,
                42,
            ],
        ),
        (MODE_STATUS, [24, 464, width, 18]),
        (CAUTION, [24, 487, width, 18]),
        (TEST, [24, 520, 150, 36]),
        (SETTINGS, [w - 216, 520, 88, 36]),
        (EXIT, [w - 116, 520, 92, 36]),
        (TEST_STATUS, [24, 564, width, 20]),
        (TRAY_HINT, [24, 593, width, 18]),
    ] {
        rect[1] -= (*state).scroll;
        place(hwnd, id, skin, rect);
    }
    InvalidateRect(hwnd, std::ptr::null(), 1);
}
unsafe fn apply_skin(hwnd: HWND, state: *mut State) {
    install_skin(hwnd, state, Skin::new(GetDpiForWindow(hwnd).max(96)));
}
unsafe fn install_skin(hwnd: HWND, state: *mut State, replacement: Skin) {
    let previous = (*state).skin.replace(replacement);
    let skin = (*state).skin.as_ref().unwrap();
    title_theme(hwnd, skin.colors.dark);
    for id in HEADER..=TRAY_HINT {
        let font = if id == HEADER {
            skin.heading.handle()
        } else if matches!(id, DISCORD_TITLE | OVERLAY_TITLE | DISPLAY_TITLE) {
            skin.strong.handle()
        } else if matches!(
            id,
            CAUTION | TRAY_HINT | MODE_STATUS | DISPLAY_HELP | TEST_STATUS
        ) {
            skin.small.handle()
        } else {
            skin.body.handle()
        };
        SendDlgItemMessageW(hwnd, id, WM_SETFONT, font as usize, 1);
    }
    for id in [DISCORD, OVERLAY, NORMAL, PRIVACY, TEST, SETTINGS, EXIT] {
        SendDlgItemMessageW(hwnd, id, WM_SETFONT, skin.body.handle() as usize, 1);
    }
    layout(hwnd, state);
    drop(previous);
}
unsafe fn make_icon() -> HICON {
    let mut pixels = vec![0u8; 32 * 32 * 4];
    let v = [0b10001u8, 0b10001, 0b10001, 0b01010, 0b00100];
    let r = [0b11110u8, 0b10001, 0b11110, 0b10100, 0b10010];
    for y in 0..32 {
        for x in 0..32 {
            let glyph = if (11..21).contains(&y) {
                let row = (y - 11) / 2;
                if (5..15).contains(&x) {
                    v[row] & (1 << (4 - (x - 5) / 2)) != 0
                } else if (17..27).contains(&x) {
                    r[row] & (1 << (4 - (x - 17) / 2)) != 0
                } else {
                    false
                }
            } else {
                false
            };
            let color = if glyph {
                [255, 255, 255, 255]
            } else if (2..30).contains(&x) && (2..30).contains(&y) {
                [192, 103, 0, 255]
            } else {
                [0, 0, 0, 0]
            };
            pixels[(y * 32 + x) * 4..(y * 32 + x + 1) * 4].copy_from_slice(&color);
        }
    }
    CreateIcon(
        std::ptr::null_mut(),
        32,
        32,
        1,
        32,
        [0u8; 128].as_ptr(),
        pixels.as_ptr(),
    )
}
unsafe fn tray_data(hwnd: HWND, state: *const State) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = std::mem::zeroed();
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = TRAY_MESSAGE;
    data.hIcon = (*state).icon;
    let tip = wide(&(*state).tray_tip);
    let count = (tip.len() - 1).min(data.szTip.len() - 1);
    data.szTip[..count].copy_from_slice(&tip[..count]);
    data
}
unsafe fn install_tray(hwnd: HWND, state: *mut State) {
    let mut data = tray_data(hwnd, state);
    (*state).tray = !data.hIcon.is_null() && Shell_NotifyIconW(NIM_ADD, &data) != 0;
    if (*state).tray {
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        if Shell_NotifyIconW(NIM_SETVERSION, &data) == 0 {
            Shell_NotifyIconW(NIM_DELETE, &data);
            (*state).tray = false;
        }
    }
    caption(
        hwnd,
        TRAY_HINT,
        if (*state).tray {
            "×でトレイに収納して転送を続けます。停止するには「終了」。"
        } else {
            "トレイを利用できません。画面を開いたまま使用してください。"
        },
    );
    if !(*state).tray {
        ShowWindow(hwnd, SW_SHOW);
    }
}
unsafe fn refresh(hwnd: HWND, state: *mut State) {
    let discord = (*state).discord.load(Ordering::SeqCst);
    let overlay = (*state).overlay.load(Ordering::SeqCst);
    let privacy = (*state).privacy.load(Ordering::SeqCst);
    let test = (*state).test_status.load(Ordering::SeqCst);
    let status = (discord, overlay, privacy, test);
    if (*state).last_status == Some(status) {
        return;
    }
    (*state).last_status = Some(status);
    caption(
        hwnd,
        READY,
        if discord == 2 && overlay {
            "準備完了  ·  Discord の通知を VR に転送できます"
        } else {
            "接続待ち  ·  Discord と XSOverlay を確認してください"
        },
    );
    caption(
        hwnd,
        DISCORD,
        match discord {
            1 => "接続・認証中\r\nDiscord の承認画面を確認",
            2 => "接続済み\r\n通知を受信できます",
            3 => "再接続中\r\n通信の回復を待っています",
            4 => "確認が必要\r\n「設定」で認証情報を確認",
            _ => "起動待ち\r\nDiscord を起動してください",
        },
    );
    caption(
        hwnd,
        OVERLAY,
        if overlay {
            "接続済み\r\nテスト通知で VR 内の表示を確認できます"
        } else {
            "接続待ち · XSOverlay を起動\r\n通知用 WebSocket を有効に"
        },
    );
    caption(
        hwnd,
        NORMAL,
        if privacy {
            "通常表示\n名前・本文・アイコンを表示"
        } else {
            "通常表示  ✓ 選択中\n名前・本文・アイコンを表示"
        },
    );
    caption(
        hwnd,
        PRIVACY,
        if privacy {
            "配信用表示  ✓ 選択中\n名前・本文・アイコンを隠す"
        } else {
            "配信用表示\n名前・本文・アイコンを隠す"
        },
    );
    caption(
        hwnd,
        PREVIEW,
        if privacy {
            "Discord\r\n新しいメッセージがあります"
        } else {
            "サンプルさん\r\nこんにちは！ これは表示例です。"
        },
    );
    caption(
        hwnd,
        TEST_STATUS,
        match test {
            1 => "テスト通知を送信しています…",
            2 => "送信待ちに追加しました。VR 内の表示を確認してください。",
            3 => "送信できませんでした。XSOverlay の接続を確認してください。",
            _ if !overlay => "XSOverlay に接続するとテスト通知を送れます。",
            _ => "テスト通知には会話や個人情報を含みません。",
        },
    );
    EnableWindow(GetDlgItem(hwnd, TEST), i32::from(overlay && test != 1));
    (*state).tray_tip = format!(
        "Discord → VR | {} | Discord: {} / XSOverlay: {}",
        if privacy { "配信用" } else { "通常" },
        if discord == 2 {
            "接続済み"
        } else {
            "待機"
        },
        if overlay { "接続済み" } else { "待機" }
    );
    if (*state).tray && Shell_NotifyIconW(NIM_MODIFY, &tray_data(hwnd, state)) == 0 {
        (*state).tray = false;
        install_tray(hwnd, state);
    }
    layout(hwnd, state);
}
unsafe fn restore(hwnd: HWND) {
    ShowWindow(hwnd, SW_RESTORE);
    SetForegroundWindow(hwnd);
}
unsafe fn tray_menu(hwnd: HWND, state: *mut State) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        restore(hwnd);
        return;
    }
    let privacy = (*state).privacy.load(Ordering::SeqCst);
    for (id, label) in [
        (OPEN, "操作画面を開く"),
        (NORMAL, "通常表示"),
        (PRIVACY, "配信用表示"),
        (TEST, "テスト通知"),
        (SETTINGS, "設定"),
        (EXIT, "終了"),
    ] {
        if matches!(id, NORMAL | TEST | SETTINGS | EXIT) {
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        }
        let checked = (id == PRIVACY && privacy) || (id == NORMAL && !privacy);
        let disabled = id == TEST
            && (!(*state).overlay.load(Ordering::SeqCst)
                || (*state).test_status.load(Ordering::SeqCst) == 1);
        AppendMenuW(
            menu,
            MF_STRING | if checked { MF_CHECKED } else { 0 } | if disabled { MF_GRAYED } else { 0 },
            id as usize,
            wide(label).as_ptr(),
        );
    }
    let mut point = POINT { x: 0, y: 0 };
    GetCursorPos(&mut point);
    SetForegroundWindow(hwnd);
    let command = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        point.x,
        point.y,
        0,
        hwnd,
        std::ptr::null(),
    );
    DestroyMenu(menu);
    PostMessageW(hwnd, WM_NULL, 0, 0);
    if command != 0 {
        PostMessageW(hwnd, WM_COMMAND, command as usize, 0);
    }
}
unsafe extern "system" fn dialog_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> isize {
    if message == WM_INITDIALOG {
        let state = lp as *mut State;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, lp);
        if (*state).closing.load(Ordering::SeqCst) {
            EndDialog(hwnd, 0);
            return 1;
        }
        (*state).skin = Some(Skin::new(GetDpiForWindow(hwnd).max(96)));
        let skin = (*state).skin.as_ref().unwrap();
        let mut failed = false;
        for (id, value) in [
            (HEADER, "Discord → VR"),
            (SUBTITLE, "いつもの通知を、VR の中でも。"),
            (READY, ""),
            (DISCORD_TITLE, "Discord"),
            (DISCORD, ""),
            (OVERLAY_TITLE, "XSOverlay"),
            (OVERLAY, ""),
            (DISPLAY_TITLE, "通知の表示"),
            (DISPLAY_HELP, "選択したモードを次回の起動でも使用します。"),
            (PREVIEW_TITLE, "表示例（ダミー）"),
            (PREVIEW, ""),
            (MODE_STATUS, ""),
            (
                CAUTION,
                "切り替え前に送信した通知や、送信中の通知は消去できません。",
            ),
            (TEST_STATUS, ""),
            (TRAY_HINT, ""),
        ] {
            failed |= create_control(hwnd, "STATIC", value, id, 0, skin.body.handle()).is_null();
        }
        for (id, value) in [
            (NORMAL, "通常表示"),
            (PRIVACY, "配信用表示"),
            (TEST, "テスト通知"),
            (SETTINGS, "設定"),
            (EXIT, "終了"),
        ] {
            failed |= create_control(
                hwnd,
                "BUTTON",
                value,
                id,
                WS_TABSTOP | BS_OWNERDRAW as u32 | BS_NOTIFY as u32,
                skin.body.handle(),
            )
            .is_null();
        }
        if failed || SetTimer(hwnd, 1, 500, None) == 0 {
            (*state).failed = true;
            EndDialog(hwnd, 0);
            return 1;
        }
        (*state).initialized = true;
        (*state).icon = make_icon();
        if !(*state).icon.is_null() {
            SendMessageW(
                hwnd,
                WM_SETICON,
                ICON_SMALL as usize,
                (*state).icon as isize,
            );
        }
        (*state).taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        set_client_size(hwnd, 680, 628);
        apply_skin(hwnd, state);
        refresh(hwnd, state);
        install_tray(hwnd, state);
        (*state).handle.store(hwnd as isize, Ordering::SeqCst);
        if (*state).closing.load(Ordering::SeqCst) {
            EndDialog(hwnd, 0);
        }
        return 1;
    }
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if state.is_null() {
        return 0;
    }
    #[cfg(test)]
    if message == WM_APP + 10 {
        install_skin(
            hwnd,
            state,
            Skin::with_palette(GetDpiForWindow(hwnd).max(96), Palette::for_dark(wp != 0)),
        );
        return 1;
    }
    if message == (*state).taskbar_created && message != 0 {
        install_tray(hwnd, state);
        return 1;
    }
    match message {
        WM_TIMER => {
            refresh(hwnd, state);
            1
        }
        WM_CLOSE => {
            if (*state).tray {
                ShowWindow(hwnd, SW_HIDE);
            } else {
                caption(
                    hwnd,
                    TRAY_HINT,
                    "トレイを利用できません。停止するには「終了」を押してください。",
                );
            }
            1
        }
        FORCE_CLOSE => {
            EndDialog(hwnd, 0);
            1
        }
        WM_COMMAND => {
            if (wp >> 16) as u32 == BN_SETFOCUS {
                if let Some(skin) = (*state).skin.as_ref() {
                    (*state).scroll = focus_scroll(hwnd, skin, lp as HWND, (*state).scroll);
                    layout(hwnd, state);
                }
                return 1;
            }
            if wp >> 16 != 0 {
                return 0;
            }
            match (wp & 0xffff) as i32 {
                NORMAL | PRIVACY => {
                    let enabled = (wp & 0xffff) as i32 == PRIVACY;
                    (*state).privacy.store(enabled, Ordering::SeqCst);
                    let result = config::save_privacy_at(&(*state).path, enabled);
                    caption(
                        hwnd,
                        MODE_STATUS,
                        if result.is_ok() {
                            "表示モードを保存しました。"
                        } else {
                            "切り替えましたが保存できません。設定ファイルと保存先を確認してください。"
                        },
                    );
                    refresh(hwnd, state);
                }
                TEST => {
                    if (*state).overlay.load(Ordering::SeqCst)
                        && (*state).test_status.swap(1, Ordering::SeqCst) != 1
                    {
                        let sender = (*state).sender.clone();
                        let status = (*state).test_status.clone();
                        (*state).runtime.spawn(async move {
                            let sent = sender
                                .send_notification(
                                    "Discord → VR",
                                    "テスト通知です",
                                    None,
                                    "",
                                    0.0,
                                    5.0,
                                )
                                .await;
                            status.store(if sent { 2 } else { 3 }, Ordering::SeqCst);
                        });
                    }
                    refresh(hwnd, state);
                }
                OPEN => restore(hwnd),
                SETTINGS => {
                    (*state).settings_requested = true;
                    EndDialog(hwnd, 1);
                }
                EXIT => {
                    EndDialog(hwnd, 0);
                }
                IDCANCEL => {
                    SendMessageW(hwnd, WM_CLOSE, 0, 0);
                }
                _ => return 0,
            }
            1
        }
        TRAY_MESSAGE => {
            match lp as u32 & 0xffff {
                NIN_SELECT | NIN_KEYSELECT => restore(hwnd),
                WM_CONTEXTMENU => tray_menu(hwnd, state),
                _ => {}
            }
            1
        }
        WM_SIZE if (*state).initialized => {
            layout(hwnd, state);
            0
        }
        WM_VSCROLL | WM_MOUSEWHEEL => {
            (*state).scroll = scroll_target(hwnd, (*state).scroll, message, wp);
            layout(hwnd, state);
            1
        }
        WM_GETMINMAXINFO => {
            let dpi = GetDpiForWindow(hwnd).max(96) as i32;
            let limits = &mut *(lp as *mut MINMAXINFO);
            limits.ptMinTrackSize.x = 620 * dpi / 96;
            limits.ptMinTrackSize.y = 360 * dpi / 96;
            0
        }
        WM_DPICHANGED => {
            let rect = &*(lp as *const windows_sys::Win32::Foundation::RECT);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            apply_skin(hwnd, state);
            1
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED if (*state).initialized => {
            apply_skin(hwnd, state);
            1
        }
        WM_CTLCOLORDLG => (*state).skin.as_ref().map_or(0, |s| s.background as isize),
        WM_CTLCOLORSTATIC => {
            let Some(skin) = (*state).skin.as_ref() else {
                return 0;
            };
            let id = GetDlgCtrlID(lp as HWND);
            let surface = matches!(
                id,
                READY | DISCORD_TITLE | DISCORD | OVERLAY_TITLE | OVERLAY | PREVIEW_TITLE | PREVIEW
            );
            let color = if id == READY {
                if (*state).discord.load(Ordering::SeqCst) == 2
                    && (*state).overlay.load(Ordering::SeqCst)
                {
                    skin.colors.good
                } else {
                    skin.colors.warning
                }
            } else if matches!(
                id,
                SUBTITLE
                    | DISPLAY_HELP
                    | CAUTION
                    | TRAY_HINT
                    | MODE_STATUS
                    | TEST_STATUS
                    | PREVIEW_TITLE
            ) {
                skin.colors.muted
            } else {
                skin.colors.text
            };
            SetTextColor(wp as HDC, color);
            SetBkColor(
                wp as HDC,
                if surface {
                    skin.colors.surface
                } else {
                    skin.colors.background
                },
            );
            if surface {
                skin.surface as isize
            } else {
                skin.background as isize
            }
        }
        WM_DRAWITEM => {
            let Some(skin) = (*state).skin.as_ref() else {
                return 0;
            };
            let draw = &*(lp as *const DRAWITEMSTRUCT);
            let privacy = (*state).privacy.load(Ordering::SeqCst);
            draw_button(
                draw,
                skin,
                match draw.CtlID as i32 {
                    NORMAL => Button::Selection(!privacy),
                    PRIVACY => Button::Selection(privacy),
                    TEST => Button::Primary,
                    _ => Button::Normal,
                },
                false,
            );
            1
        }
        WM_PAINT if (*state).initialized => {
            let Some(skin) = (*state).skin.as_ref() else {
                return 0;
            };
            let mut paint = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut paint);
            let (w, _) = client_size(hwnd, skin);
            let half = (w - 64) / 2;
            let offset = (*state).scroll;
            card(
                dc,
                skin,
                [24, 86 - offset, w - 48, 44],
                skin.colors.surface,
                skin.colors.border,
            );
            card(
                dc,
                skin,
                [24, 140 - offset, half, 96],
                skin.colors.surface,
                skin.colors.border,
            );
            card(
                dc,
                skin,
                [40 + half, 140 - offset, half, 96],
                skin.colors.surface,
                skin.colors.border,
            );
            card(
                dc,
                skin,
                [24, 396 - offset, w - 48, 62],
                skin.colors.surface,
                skin.colors.border,
            );
            if !(*state).privacy.load(Ordering::SeqCst) {
                let saved = SaveDC(dc);
                let brush = CreateSolidBrush(skin.colors.accent);
                SelectObject(dc, brush);
                SelectObject(dc, GetStockObject(NULL_PEN));
                let rect = skin.rect([160, 410 - offset, 34, 34]);
                Ellipse(dc, rect.left, rect.top, rect.right, rect.bottom);
                let person = CreateSolidBrush(skin.colors.accent_text);
                SelectObject(dc, person);
                let head = skin.rect([173, 417 - offset, 8, 8]);
                Ellipse(dc, head.left, head.top, head.right, head.bottom);
                let body = skin.rect([168, 427 - offset, 18, 10]);
                RoundRect(
                    dc,
                    body.left,
                    body.top,
                    body.right,
                    body.bottom,
                    skin.px(8),
                    skin.px(8),
                );
                RestoreDC(dc, saved);
                DeleteObject(brush);
                DeleteObject(person);
            }
            EndPaint(hwnd, &paint);
            1
        }
        WM_DESTROY => {
            KillTimer(hwnd, 1);
            if (*state).tray {
                Shell_NotifyIconW(NIM_DELETE, &tray_data(hwnd, state));
                (*state).tray = false;
            }
            0
        }
        WM_NCDESTROY => {
            (*state).handle.store(0, Ordering::SeqCst);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            0
        }
        _ => 0,
    }
}

pub fn start(
    privacy: Arc<AtomicBool>,
    discord: Arc<AtomicU8>,
    overlay: Arc<AtomicBool>,
    sender: XsOverlaySender,
    path: PathBuf,
) -> std::io::Result<(Window, tokio::sync::oneshot::Receiver<bool>)> {
    let handle = Arc::new(AtomicIsize::new(0));
    let thread_handle = handle.clone();
    let closing = Arc::new(AtomicBool::new(false));
    let thread_closing = closing.clone();
    let runtime = tokio::runtime::Handle::current();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let thread = std::thread::Builder::new()
        .name("discord-vr-ui".into())
        .spawn(move || {
            let mut state = State {
                privacy,
                discord,
                overlay,
                sender,
                runtime,
                path,
                handle: thread_handle,
                closing: thread_closing,
                test_status: Arc::new(AtomicU8::new(0)),
                settings_requested: false,
                failed: false,
                skin: None,
                initialized: false,
                tray: false,
                icon: std::ptr::null_mut(),
                taskbar_created: 0,
                last_status: None,
                tray_tip: "Discord → VR".into(),
                scroll: 0,
            };
            let result = unsafe {
                desktop::show_dialog(
                    "Discord → VR",
                    true,
                    Some(dialog_proc),
                    &mut state as *mut State as isize,
                )
            };
            state.handle.store(0, Ordering::SeqCst);
            unsafe {
                if !state.icon.is_null() {
                    DestroyIcon(state.icon);
                }
            }
            if result.is_err() || state.failed {
                super::report_error("操作画面を開けませんでした。");
            }
            let _ = tx.send(state.settings_requested);
        })?;
    Ok((
        Window {
            handle,
            closing,
            thread: Some(thread),
        },
        rx,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
    async fn wait_for_window(window: &Window) -> HWND {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while window.handle.load(Ordering::SeqCst) == 0
            || unsafe { IsWindowVisible(window.handle.load(Ordering::SeqCst) as HWND) == 0 }
        {
            assert!(tokio::time::Instant::now() < deadline, "GUI did not open");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        window.handle.load(Ordering::SeqCst) as HWND
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn gui_remembers_mode_hides_restores_and_opens_settings() {
        let path =
            std::env::temp_dir().join(format!("discord-vr-ui-{}.json", uuid::Uuid::new_v4()));
        config::save_setup_at(
            &path,
            &config::Config {
                client_id: "123456789".into(),
                client_secret: "test-secret".into(),
                ..config::Config::default()
            },
        )
        .unwrap();
        let privacy = Arc::new(AtomicBool::new(false));
        let (sender, _, connected) =
            crate::vr::create_xsoverlay_channel(reqwest::Client::new(), privacy.clone());
        let (mut window, mut result) = start(
            privacy.clone(),
            Arc::new(AtomicU8::new(0)),
            connected,
            sender,
            path.clone(),
        )
        .unwrap();
        let hwnd = wait_for_window(&window).await;
        unsafe {
            assert_eq!(IsWindowEnabled(GetDlgItem(hwnd, TEST)), 0);
            SendDlgItemMessageW(hwnd, PRIVACY, BM_CLICK, 0, 0);
        }
        assert!(privacy.load(Ordering::SeqCst));
        assert!(config::load_from_path(&path, false).unwrap().privacy_mode);
        unsafe {
            SendMessageW(hwnd, WM_CLOSE, 0, 0);
        }
        assert!(matches!(
            result.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        let mut identifier: NOTIFYICONIDENTIFIER = unsafe { std::mem::zeroed() };
        identifier.cbSize = std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32;
        identifier.hWnd = hwnd;
        identifier.uID = 1;
        let mut tray_rect = unsafe { std::mem::zeroed() };
        if unsafe { Shell_NotifyIconGetRect(&identifier, &mut tray_rect) } == 0 {
            unsafe {
                assert_eq!(
                    IsWindowVisible(hwnd),
                    0,
                    "Close should hide when the tray exists"
                );
            }
        }
        // CI may lack Explorer. Keeping the window visible is the safe fallback.
        unsafe {
            SendMessageW(hwnd, WM_COMMAND, OPEN as usize, 0);
            assert_ne!(IsWindowVisible(hwnd), 0);
            PostMessageW(hwnd, WM_COMMAND, SETTINGS as usize, 0);
        }
        assert!(tokio::time::timeout(Duration::from_secs(5), result)
            .await
            .unwrap()
            .unwrap());
        window.close();
        unsafe {
            assert_eq!(IsWindow(hwnd), 0);
            assert_ne!(Shell_NotifyIconGetRect(&identifier, &mut tray_rect), 0);
        }
        std::fs::remove_file(path).unwrap();
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn explicit_exit_and_startup_shutdown_complete_the_session() {
        for immediate in [true, false] {
            let privacy = Arc::new(AtomicBool::new(false));
            let (sender, _, connected) =
                crate::vr::create_xsoverlay_channel(reqwest::Client::new(), privacy.clone());
            let (mut window, result) = start(
                privacy,
                Arc::new(AtomicU8::new(0)),
                connected,
                sender,
                std::env::temp_dir().join("unused-discord-vr-config.json"),
            )
            .unwrap();
            if immediate {
                window.close();
            } else {
                let hwnd = wait_for_window(&window).await;
                unsafe {
                    PostMessageW(hwnd, WM_COMMAND, EXIT as usize, 0);
                }
            }
            assert!(!tokio::time::timeout(Duration::from_secs(5), result)
                .await
                .unwrap()
                .unwrap());
            window.close();
            assert_eq!(window.handle.load(Ordering::SeqCst), 0);
        }
    }

    /// Optional native preview for visual checks; it opens no Discord/XSOverlay connection.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "opens a synthetic native window for visual checks"]
    async fn native_preview_without_network() {
        let path =
            std::env::temp_dir().join(format!("discord-vr-preview-{}.json", uuid::Uuid::new_v4()));
        config::save_setup_at(
            &path,
            &config::Config {
                client_id: "123456789".into(),
                client_secret: "test-secret".into(),
                ..config::Config::default()
            },
        )
        .unwrap();
        let privacy = Arc::new(AtomicBool::new(false));
        let (sender, _, connected) =
            crate::vr::create_xsoverlay_channel(reqwest::Client::new(), privacy.clone());
        connected.store(true, Ordering::SeqCst);
        let (mut window, _) = start(
            privacy,
            Arc::new(AtomicU8::new(2)),
            connected,
            sender,
            path.clone(),
        )
        .unwrap();
        let hwnd = wait_for_window(&window).await;
        unsafe {
            SendMessageW(hwnd, WM_APP + 10, 0, 0);
            capture_test_window(hwnd, "home-normal").unwrap();
            SendMessageW(hwnd, WM_APP + 10, 1, 0);
            capture_test_window(hwnd, "home-dark").unwrap();
            SendDlgItemMessageW(hwnd, PRIVACY, BM_CLICK, 0, 0);
            capture_test_window(hwnd, "home-privacy").unwrap();
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                700,
                430,
                SWP_NOMOVE | SWP_NOZORDER,
            );
            SendMessageW(hwnd, WM_VSCROLL, SB_BOTTOM as usize, 0);
            capture_test_window(hwnd, "home-short").unwrap();
        }
        if std::env::var_os("DISCORD_VR_CAPTURE_DIR").is_none() {
            tokio::time::sleep(Duration::from_secs(20)).await;
        }
        window.close();
        std::fs::remove_file(path).unwrap();
    }
}
