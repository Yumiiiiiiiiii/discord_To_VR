//! Guided native settings. Values remain in controls when switching pages.
use super::desktop::{self, *};
use crate::config::{self, Config};
use std::io;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::{Dialogs::*, DRAWITEMSTRUCT, EM_SETLIMITTEXT, EM_SETSEL};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLIENT_ID: i32 = 101;
const SECRET: i32 = 102;
const REDIRECT: i32 = 103;
const PRIVACY: i32 = 104;
const PORTAL: i32 = 105;
const TIMEOUT: i32 = 106;
const VOLUME: i32 = 107;
const TITLE_LENGTH: i32 = 108;
const CONTENT_LENGTH: i32 = 109;
const SOUND: i32 = 110;
const AUTO_EXIT: i32 = 111;
const DISPLAY_TAB: i32 = 120;
const SOUND_TAB: i32 = 121;
const DISCORD_TAB: i32 = 122;
const SOUND_NONE: i32 = 123;
const SOUND_DEFAULT: i32 = 124;
const SOUND_FILE: i32 = 125;
const BROWSE: i32 = 126;
const HEADER: i32 = 140;
const HELP: i32 = 141;
const PAGE_TITLE: i32 = 142;
const PAGE_HELP: i32 = 143;
const ERROR: i32 = 144;
const TIMEOUT_LABEL: i32 = 150;
const TITLE_LABEL: i32 = 151;
const CONTENT_LABEL: i32 = 152;
const MODE_HELP: i32 = 153;
const SOUND_HELP: i32 = 154;
const VOLUME_LABEL: i32 = 155;
const AUTO_HELP: i32 = 156;
const STEPS: i32 = 157;
const CLIENT_LABEL: i32 = 158;
const SECRET_LABEL: i32 = 159;
const REDIRECT_LABEL: i32 = 160;
const SECURITY: i32 = 161;

const DISPLAY_CONTROLS: &[i32] = &[
    TIMEOUT,
    TITLE_LENGTH,
    CONTENT_LENGTH,
    PRIVACY,
    TIMEOUT_LABEL,
    TITLE_LABEL,
    CONTENT_LABEL,
    MODE_HELP,
];
const SOUND_CONTROLS: &[i32] = &[
    SOUND_NONE,
    SOUND_DEFAULT,
    SOUND_FILE,
    SOUND,
    BROWSE,
    VOLUME,
    AUTO_EXIT,
    SOUND_HELP,
    VOLUME_LABEL,
    AUTO_HELP,
];
const DISCORD_CONTROLS: &[i32] = &[
    STEPS,
    PORTAL,
    CLIENT_ID,
    SECRET,
    REDIRECT,
    CLIENT_LABEL,
    SECRET_LABEL,
    REDIRECT_LABEL,
    SECURITY,
];
struct Setup {
    path: PathBuf,
    config: Config,
    saved: bool,
    error: Option<io::Error>,
    skin: Option<Skin>,
    initialized: bool,
    page: i32,
    sound_kind: i32,
    privacy: bool,
    auto_exit: bool,
    scroll: i32,
}
unsafe fn show_page(hwnd: HWND, state: *mut Setup, page: i32) {
    (*state).page = page;
    for (tab, controls) in [
        (DISPLAY_TAB, DISPLAY_CONTROLS),
        (SOUND_TAB, SOUND_CONTROLS),
        (DISCORD_TAB, DISCORD_CONTROLS),
    ] {
        for id in controls {
            ShowWindow(
                GetDlgItem(hwnd, *id),
                if page == tab { SW_SHOW } else { SW_HIDE },
            );
        }
    }
    for (id, label) in [
        (DISPLAY_TAB, "通知の表示"),
        (SOUND_TAB, "通知音・動作"),
        (DISCORD_TAB, "Discord接続"),
    ] {
        caption(
            hwnd,
            id,
            &format!("{}{}", label, if page == id { "  ✓" } else { "" }),
        );
    }
    let (heading, help) = match page {
        SOUND_TAB => (
            "通知音・動作",
            "音と、VR を終了したときの動作を調整します。",
        ),
        DISCORD_TAB => (
            "自分の Discord アプリを接続",
            "利用する本人のアカウントで作成した Client ID を使います。",
        ),
        _ => (
            "通知の見え方",
            "VR 内の読みやすさに合わせて、表示時間と文字数を調整します。",
        ),
    };
    caption(hwnd, PAGE_TITLE, heading);
    caption(hwnd, PAGE_HELP, help);
    if page == SOUND_TAB {
        sound_controls(hwnd, state);
    } else {
        layout(hwnd, state);
    }
    InvalidateRect(hwnd, std::ptr::null(), 1);
}
unsafe fn sound_controls(hwnd: HWND, state: *mut Setup) {
    let file = (*state).sound_kind == SOUND_FILE;
    ShowWindow(
        GetDlgItem(hwnd, SOUND),
        if file && (*state).page == SOUND_TAB {
            SW_SHOW
        } else {
            SW_HIDE
        },
    );
    ShowWindow(
        GetDlgItem(hwnd, BROWSE),
        if file && (*state).page == SOUND_TAB {
            SW_SHOW
        } else {
            SW_HIDE
        },
    );
    EnableWindow(GetDlgItem(hwnd, SOUND), i32::from(file));
    EnableWindow(GetDlgItem(hwnd, BROWSE), i32::from(file));
    EnableWindow(
        GetDlgItem(hwnd, VOLUME),
        i32::from((*state).sound_kind != SOUND_NONE),
    );
    for (id, label) in [
        (SOUND_NONE, "無音"),
        (SOUND_DEFAULT, "標準の通知音"),
        (SOUND_FILE, "音声ファイル"),
    ] {
        caption(
            hwnd,
            id,
            &format!(
                "{}{}",
                label,
                if (*state).sound_kind == id {
                    "  ✓"
                } else {
                    ""
                }
            ),
        );
    }
    caption(
        hwnd,
        SOUND_HELP,
        if (*state).sound_kind == SOUND_NONE {
            "通知音は鳴りません。"
        } else {
            "音を鳴らすには、音量を 1% 以上にしてください。"
        },
    );
    layout(hwnd, state);
}
unsafe fn check_labels(hwnd: HWND, state: *const Setup) {
    caption(
        hwnd,
        PRIVACY,
        if (*state).privacy {
            "配信用表示を使用する（有効）"
        } else {
            "配信用表示を使用する（無効）"
        },
    );
    caption(
        hwnd,
        AUTO_EXIT,
        if (*state).auto_exit {
            "VR 終了時にこのツールも終了する（有効）"
        } else {
            "VR 終了時にこのツールも終了する（無効）"
        },
    );
}

unsafe fn layout(hwnd: HWND, state: *mut Setup) {
    let Some(skin) = (*state).skin.as_ref() else {
        return;
    };
    (*state).scroll = scroll_range(hwnd, skin, 612, (*state).scroll);
    let (w, _) = client_size(hwnd, skin);
    let width = w - 48;
    for (id, mut rect) in [
        (HEADER, [24, 18, width, 32]),
        (HELP, [24, 54, width, 20]),
        (DISPLAY_TAB, [24, 88, 180, 36]),
        (SOUND_TAB, [216, 88, 180, 36]),
        (DISCORD_TAB, [408, 88, w - 432, 36]),
        (PAGE_TITLE, [40, 148, width - 32, 24]),
        (PAGE_HELP, [40, 178, width - 32, 24]),
        (TIMEOUT_LABEL, [40, 224, 370, 22]),
        (TIMEOUT, [w - 162, 219, 98, 30]),
        (TITLE_LABEL, [40, 272, 370, 22]),
        (TITLE_LENGTH, [w - 162, 267, 98, 30]),
        (CONTENT_LABEL, [40, 320, 370, 22]),
        (CONTENT_LENGTH, [w - 162, 315, 98, 30]),
        (PRIVACY, [40, 365, width - 32, 42]),
        (MODE_HELP, [40, 418, width - 32, 30]),
        (SOUND_NONE, [40, 216, 174, 42]),
        (SOUND_DEFAULT, [226, 216, 174, 42]),
        (SOUND_FILE, [412, 216, w - 452, 42]),
        (SOUND, [40, 276, w - 212, 30]),
        (BROWSE, [w - 160, 276, 120, 32]),
        (SOUND_HELP, [40, 313, width - 32, 22]),
        (VOLUME_LABEL, [40, 352, 350, 22]),
        (VOLUME, [w - 162, 347, 98, 30]),
        (AUTO_EXIT, [40, 397, width - 32, 40]),
        (AUTO_HELP, [40, 445, width - 32, 20]),
        (STEPS, [40, 214, width - 32, 66]),
        (PORTAL, [40, 289, 230, 32]),
        (CLIENT_LABEL, [40, 340, 136, 22]),
        (CLIENT_ID, [184, 335, w - 224, 30]),
        (SECRET_LABEL, [40, 382, 136, 22]),
        (SECRET, [184, 377, w - 224, 30]),
        (REDIRECT_LABEL, [40, 424, 136, 22]),
        (REDIRECT, [184, 419, w - 224, 30]),
        (SECURITY, [40, 461, width - 32, 30]),
        (ERROR, [24, 511, width, 30]),
        (IDOK, [w - 252, 554, 132, 36]),
        (IDCANCEL, [w - 114, 554, 90, 36]),
    ] {
        if (*state).sound_kind != SOUND_FILE
            && matches!(
                id,
                SOUND_HELP | VOLUME_LABEL | VOLUME | AUTO_EXIT | AUTO_HELP
            )
        {
            rect[1] -= 36;
        }
        rect[1] -= (*state).scroll;
        place(hwnd, id, skin, rect);
    }
    InvalidateRect(hwnd, std::ptr::null(), 1);
}
unsafe fn apply_skin(hwnd: HWND, state: *mut Setup) {
    let previous = (*state)
        .skin
        .replace(Skin::new(GetDpiForWindow(hwnd).max(96)));
    let skin = (*state).skin.as_ref().unwrap();
    title_theme(hwnd, skin.colors.dark);
    for id in [HEADER, HELP, PAGE_TITLE, PAGE_HELP, ERROR]
        .into_iter()
        .chain(150..=161)
        .chain(101..=126)
        .chain([IDOK, IDCANCEL])
    {
        let font = if id == HEADER {
            skin.heading.handle()
        } else if id == PAGE_TITLE {
            skin.strong.handle()
        } else if matches!(
            id,
            HELP | PAGE_HELP | MODE_HELP | AUTO_HELP | SECURITY | ERROR | SOUND_HELP
        ) {
            skin.small.handle()
        } else {
            skin.body.handle()
        };
        SendDlgItemMessageW(hwnd, id, WM_SETFONT, font as usize, 1);
    }
    layout(hwnd, state);
    drop(previous);
}
unsafe fn invalid(hwnd: HWND, state: *mut Setup, id: i32, message: &str) {
    let page = if matches!(id, CLIENT_ID | SECRET | REDIRECT) {
        DISCORD_TAB
    } else if matches!(id, VOLUME | SOUND) {
        SOUND_TAB
    } else {
        DISPLAY_TAB
    };
    show_page(hwnd, state, page);
    caption(hwnd, ERROR, message);
    SetFocus(GetDlgItem(hwnd, id));
    if matches!(
        id,
        CLIENT_ID | SECRET | REDIRECT | VOLUME | SOUND | TIMEOUT | TITLE_LENGTH | CONTENT_LENGTH
    ) {
        SendDlgItemMessageW(hwnd, id, EM_SETSEL, 0, -1);
    }
}
unsafe fn read_form(hwnd: HWND, state: *const Setup) -> Result<Config, (i32, &'static str)> {
    let mut cfg = (*state).config.clone();
    cfg.client_id = text(hwnd, CLIENT_ID).trim().into();
    cfg.client_secret = text(hwnd, SECRET).trim().into();
    cfg.redirect_uri = text(hwnd, REDIRECT).trim().into();
    cfg.privacy_mode = (*state).privacy;
    cfg.auto_exit_with_vr = (*state).auto_exit;
    cfg.notification_sound = match (*state).sound_kind {
        SOUND_NONE => String::new(),
        SOUND_DEFAULT => "default".into(),
        _ => text(hwnd, SOUND).trim().into(),
    };
    if (*state).sound_kind == SOUND_FILE && !Path::new(&cfg.notification_sound).is_absolute() {
        return Err((SOUND, "通知音に使うファイルを「参照」から選んでください。"));
    }
    cfg.notification_timeout = text(hwnd, TIMEOUT)
        .trim()
        .parse()
        .map_err(|_| (TIMEOUT, "表示時間には 1 ～ 30 秒の数値を入力してください。"))?;
    cfg.notification_volume = text(hwnd, VOLUME)
        .trim()
        .parse::<f64>()
        .map_err(|_| (VOLUME, "音量には 0 ～ 100% の数値を入力してください。"))?
        / 100.0;
    cfg.max_title_length = text(hwnd, TITLE_LENGTH).trim().parse().map_err(|_| {
        (
            TITLE_LENGTH,
            "タイトル文字数には 10 ～ 200 の整数を入力してください。",
        )
    })?;
    cfg.max_content_length = text(hwnd, CONTENT_LENGTH).trim().parse().map_err(|_| {
        (
            CONTENT_LENGTH,
            "本文文字数には 10 ～ 2000 の整数を入力してください。",
        )
    })?;
    cfg.validate().map_err(|error| {
        if error.starts_with("CLIENT_ID") {(CLIENT_ID,"Client ID に Developer Portal の数字の ID を入力してください。")}
        else if error.starts_with("CLIENT_SECRET") {(SECRET,"Client Secret を入力してください。Bot Token は使用しません。")}
        else if error.starts_with("REDIRECT_URI") {(REDIRECT,"Redirect URI に有効な HTTP(S) URL を入力し、Portal の Redirects と一致させてください。")}
        else if error.starts_with("NOTIFICATION_VOLUME") {(VOLUME,"音量は 0 ～ 100% にしてください。")}
        else if error.starts_with("NOTIFICATION_TIMEOUT") {(TIMEOUT,"表示時間は 1 ～ 30 秒にしてください。")}
        else if error.starts_with("MAX_TITLE_LENGTH") {(TITLE_LENGTH,"タイトル文字数は 10 ～ 200 にしてください。")}
        else {(CONTENT_LENGTH,"本文文字数は 10 ～ 2000 にしてください。")}
    })?;
    Ok(cfg)
}
unsafe fn browse(hwnd: HWND) {
    let mut filename = vec![0u16; 32768];
    let filter =
        wide("音声ファイル (*.wav;*.ogg;*.mp3)\0*.wav;*.ogg;*.mp3\0すべてのファイル\0*.*\0");
    let title = wide("通知音に使うファイルを選択");
    let mut dialog: OPENFILENAMEW = std::mem::zeroed();
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = hwnd;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = filename.as_mut_ptr();
    dialog.nMaxFile = filename.len() as u32;
    dialog.lpstrTitle = title.as_ptr();
    dialog.Flags = OFN_EXPLORER
        | OFN_FILEMUSTEXIST
        | OFN_PATHMUSTEXIST
        | OFN_NOCHANGEDIR
        | OFN_DONTADDTORECENT;
    if GetOpenFileNameW(&mut dialog) != 0 {
        let end = filename
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(filename.len());
        caption(hwnd, SOUND, &String::from_utf16_lossy(&filename[..end]));
    } else if CommDlgExtendedError() != 0 {
        caption(
            hwnd,
            ERROR,
            "ファイル選択を開けませんでした。音声ファイルの絶対パスを入力してください。",
        );
    }
}
unsafe extern "system" fn dialog_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> isize {
    if message == WM_INITDIALOG {
        let state = lp as *mut Setup;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, lp);
        (*state).skin = Some(Skin::new(GetDpiForWindow(hwnd).max(96)));
        let skin = (*state).skin.as_ref().unwrap();
        let mut failed = false;
        for (id,value) in [(HEADER,"設定"),(HELP,"保存すると接続を再開して反映します。キャンセルすると変更を破棄します。"),
            (PAGE_TITLE,""),(PAGE_HELP,""),(ERROR,""),(TIMEOUT_LABEL,"表示時間（1 ～ 30 秒）"),(TITLE_LABEL,"タイトル文字数（10 ～ 200）"),
            (CONTENT_LABEL,"本文文字数（10 ～ 2000）"),(MODE_HELP,"ホームで切り替えたモードも保存されます。配信用表示は名前・本文・アイコンを隠します。"),
            (SOUND_HELP,""),(VOLUME_LABEL,"音量（0 ～ 100%）"),(AUTO_HELP,"一度検知した VR 関連アプリがすべて終了すると、このツールも終了します。"),
            (STEPS,"1. 自分のアカウントで New Application を作成\r\n2. OAuth2 で Client ID・Client Secret を確認\r\n3. Redirects に下の URI を追加し、Save Changes を押す"),
            (CLIENT_LABEL,"Client ID"),(SECRET_LABEL,"Client Secret"),(REDIRECT_LABEL,"Redirect URI"),
            (SECURITY,"Secret はこの Windows ユーザー向けに暗号化して保存します。設定ファイルを共有しないでください。Bot Token は不要です。")] {
            failed|=create_control(hwnd,"STATIC",value,id,0,skin.body.handle()).is_null();
        }
        for (id, value) in [
            (DISPLAY_TAB, "通知の表示"),
            (SOUND_TAB, "通知音・動作"),
            (DISCORD_TAB, "Discord接続"),
            (PRIVACY, "配信用表示を使用する"),
            (AUTO_EXIT, "VR 関連アプリの終了に合わせて自動終了"),
            (SOUND_NONE, "無音"),
            (SOUND_DEFAULT, "標準の通知音"),
            (SOUND_FILE, "音声ファイル"),
            (BROWSE, "参照…"),
            (PORTAL, "Developer Portal を開く"),
            (IDOK, "保存して戻る"),
            (IDCANCEL, "キャンセル"),
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
        let cfg = &(*state).config;
        for (id, value) in [
            (
                CLIENT_ID,
                if cfg.client_id == "YOUR_CLIENT_ID_HERE" {
                    String::new()
                } else {
                    cfg.client_id.clone()
                },
            ),
            (
                SECRET,
                if cfg.client_secret == "YOUR_CLIENT_SECRET_HERE" {
                    String::new()
                } else {
                    cfg.client_secret.clone()
                },
            ),
            (REDIRECT, cfg.redirect_uri.clone()),
            (TIMEOUT, cfg.notification_timeout.to_string()),
            (VOLUME, (cfg.notification_volume * 100.0).to_string()),
            (TITLE_LENGTH, cfg.max_title_length.to_string()),
            (CONTENT_LENGTH, cfg.max_content_length.to_string()),
            (
                SOUND,
                if (*state).sound_kind == SOUND_FILE {
                    cfg.notification_sound.clone()
                } else {
                    String::new()
                },
            ),
        ] {
            let style = WS_BORDER
                | WS_TABSTOP
                | ES_AUTOHSCROLL as u32
                | if id == SECRET { ES_PASSWORD as u32 } else { 0 };
            failed |= create_control(hwnd, "EDIT", &value, id, style, skin.body.handle()).is_null();
            SendDlgItemMessageW(
                hwnd,
                id,
                EM_SETLIMITTEXT,
                if id == SOUND { 32767 } else { 2048 },
                0,
            );
        }
        if failed {
            (*state).error = Some(io::Error::last_os_error());
            EndDialog(hwnd, 0);
            return 1;
        }
        (*state).initialized = true;
        set_client_size(hwnd, 680, 612);
        apply_skin(hwnd, state);
        sound_controls(hwnd, state);
        check_labels(hwnd, state);
        show_page(hwnd, state, (*state).page);
        SendMessageW(hwnd, DM_SETDEFID, IDOK as usize, 0);
        SetFocus(GetDlgItem(
            hwnd,
            if (*state).page == DISCORD_TAB {
                CLIENT_ID
            } else {
                TIMEOUT
            },
        ));
        return 0;
    }
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Setup;
    if state.is_null() {
        return 0;
    }
    match message {
        WM_COMMAND => {
            let notification = (wp >> 16) as u32;
            if notification == BN_SETFOCUS || notification == EN_SETFOCUS {
                if let Some(skin) = (*state).skin.as_ref() {
                    (*state).scroll = focus_scroll(hwnd, skin, lp as HWND, (*state).scroll);
                    layout(hwnd, state);
                }
                return 1;
            }
            if notification != 0 {
                return 0;
            }
            match (wp & 0xffff) as i32 {
                DISPLAY_TAB | SOUND_TAB | DISCORD_TAB => {
                    show_page(hwnd, state, (wp & 0xffff) as i32);
                }
                SOUND_NONE | SOUND_DEFAULT | SOUND_FILE => {
                    (*state).sound_kind = (wp & 0xffff) as i32;
                    sound_controls(hwnd, state);
                }
                PRIVACY => {
                    (*state).privacy = !(*state).privacy;
                    check_labels(hwnd, state);
                    InvalidateRect(GetDlgItem(hwnd, PRIVACY), std::ptr::null(), 1);
                }
                AUTO_EXIT => {
                    (*state).auto_exit = !(*state).auto_exit;
                    check_labels(hwnd, state);
                    InvalidateRect(GetDlgItem(hwnd, AUTO_EXIT), std::ptr::null(), 1);
                }
                BROWSE => browse(hwnd),
                PORTAL => {
                    if ShellExecuteW(
                        hwnd,
                        wide("open").as_ptr(),
                        wide("https://discord.com/developers/applications").as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        SW_SHOWNORMAL,
                    ) as isize
                        <= 32
                    {
                        caption(hwnd,ERROR,"ブラウザを開けませんでした。discord.com/developers/applications を開いてください。");
                    }
                }
                IDOK => match read_form(hwnd, state) {
                    Err((id, error)) => invalid(hwnd, state, id, error),
                    Ok(cfg) => match config::save_setup_at(&(*state).path, &cfg) {
                        Ok(()) => {
                            (*state).saved = true;
                            EndDialog(hwnd, 1);
                        }
                        Err(_) => {
                            caption(hwnd,ERROR,"保存できませんでした。保存先の書き込み権限と空き容量を確認してください。");
                        }
                    },
                },
                IDCANCEL => {
                    EndDialog(hwnd, 0);
                }
                _ => return 0,
            }
            1
        }
        WM_CLOSE => {
            EndDialog(hwnd, 0);
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
        WM_SETTINGCHANGE | WM_THEMECHANGED if (*state).initialized => {
            apply_skin(hwnd, state);
            1
        }
        WM_DPICHANGED => {
            set_client_size(hwnd, 680, 612);
            apply_skin(hwnd, state);
            1
        }
        WM_CTLCOLORDLG => (*state).skin.as_ref().map_or(0, |s| s.background as isize),
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
            let Some(skin) = (*state).skin.as_ref() else {
                return 0;
            };
            let id = GetDlgCtrlID(lp as HWND);
            let outer = matches!(id, HEADER | HELP | ERROR);
            let muted = matches!(
                id,
                HELP | PAGE_HELP | MODE_HELP | SOUND_HELP | AUTO_HELP | SECURITY
            );
            SetTextColor(
                wp as HDC,
                if id == ERROR {
                    skin.colors.warning
                } else if muted {
                    skin.colors.muted
                } else {
                    skin.colors.text
                },
            );
            SetBkColor(
                wp as HDC,
                if outer {
                    skin.colors.background
                } else {
                    skin.colors.surface
                },
            );
            if outer {
                skin.background as isize
            } else {
                skin.surface as isize
            }
        }
        WM_DRAWITEM => {
            let Some(skin) = (*state).skin.as_ref() else {
                return 0;
            };
            let draw = &*(lp as *const DRAWITEMSTRUCT);
            let kind = match draw.CtlID as i32 {
                DISPLAY_TAB | SOUND_TAB | DISCORD_TAB => {
                    Button::Selection((*state).page == draw.CtlID as i32)
                }
                SOUND_NONE | SOUND_DEFAULT | SOUND_FILE => {
                    Button::Selection((*state).sound_kind == draw.CtlID as i32)
                }
                PRIVACY => Button::Check((*state).privacy),
                AUTO_EXIT => Button::Check((*state).auto_exit),
                IDOK => Button::Primary,
                _ => Button::Normal,
            };
            draw_button(
                draw,
                skin,
                kind,
                !matches!(
                    draw.CtlID as i32,
                    DISPLAY_TAB | SOUND_TAB | DISCORD_TAB | IDOK | IDCANCEL
                ),
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
            card(
                dc,
                skin,
                [24, 134 - (*state).scroll, w - 48, 364],
                skin.colors.surface,
                skin.colors.border,
            );
            EndPaint(hwnd, &paint);
            1
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            0
        }
        _ => 0,
    }
}
pub fn run(path: &Path) -> io::Result<bool> {
    let cfg = config::setup_defaults(path)?;
    let page = if cfg.validate().is_ok() {
        DISPLAY_TAB
    } else {
        DISCORD_TAB
    };
    let sound_kind = if cfg.notification_sound.is_empty() {
        SOUND_NONE
    } else if cfg.notification_sound == "default" {
        SOUND_DEFAULT
    } else {
        SOUND_FILE
    };
    let mut setup = Setup {
        path: path.into(),
        privacy: cfg.privacy_mode,
        auto_exit: cfg.auto_exit_with_vr,
        config: cfg,
        saved: false,
        error: None,
        skin: None,
        initialized: false,
        page,
        sound_kind,
        scroll: 0,
    };
    unsafe {
        desktop::show_dialog(
            "Discord → VR 設定",
            false,
            Some(dialog_proc),
            &mut setup as *mut Setup as isize,
        )?;
    }
    if let Some(error) = setup.error {
        return Err(error);
    }
    Ok(setup.saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn tabs_preserve_edits_and_validation_keeps_the_dialog_open() {
        let path =
            std::env::temp_dir().join(format!("discord-vr-form-{}.json", uuid::Uuid::new_v4()));
        let cfg = Config {
            client_id: "123456789".into(),
            client_secret: "test-secret".into(),
            ..Config::default()
        };
        config::save_setup_at(&path, &cfg).unwrap();
        let title = format!("Settings regression {}", uuid::Uuid::new_v4());
        let thread_title = title.clone();
        let thread_path = path.clone();
        let thread = std::thread::spawn(move || {
            let mut state = Setup {
                path: thread_path,
                privacy: cfg.privacy_mode,
                auto_exit: cfg.auto_exit_with_vr,
                config: cfg,
                saved: false,
                error: None,
                skin: None,
                initialized: false,
                page: DISPLAY_TAB,
                sound_kind: SOUND_NONE,
                scroll: 0,
            };
            unsafe {
                desktop::show_dialog(
                    &thread_title,
                    false,
                    Some(dialog_proc),
                    &mut state as *mut Setup as isize,
                )
                .unwrap();
            }
            state.saved
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let hwnd = loop {
            let hwnd = unsafe { FindWindowW(std::ptr::null(), wide(&title).as_ptr()) };
            if !hwnd.is_null()
                && unsafe { IsWindowVisible(hwnd) != 0 && !GetDlgItem(hwnd, TIMEOUT).is_null() }
            {
                break hwnd;
            }
            assert!(Instant::now() < deadline, "Settings did not open");
            std::thread::sleep(Duration::from_millis(10));
        };
        // Synchronous messages run on the owning thread after initialization.
        unsafe {
            caption(hwnd, TIMEOUT, "8");
            capture_test_window(hwnd, "settings-display").unwrap();
            SendMessageW(hwnd, WM_COMMAND, SOUND_TAB as usize, 0);
            SendDlgItemMessageW(hwnd, SOUND_DEFAULT, BM_CLICK, 0, 0);
            caption(hwnd, VOLUME, "42");
            capture_test_window(hwnd, "settings-sound").unwrap();
            SendMessageW(hwnd, WM_COMMAND, DISCORD_TAB as usize, 0);
            caption(hwnd, SECRET, "edited-test-secret");
            capture_test_window(hwnd, "settings-discord").unwrap();
            SendMessageW(hwnd, WM_COMMAND, DISPLAY_TAB as usize, 0);
            assert_eq!(text(hwnd, TIMEOUT), "8");
            caption(hwnd, TIMEOUT, "0");
            SendMessageW(hwnd, WM_COMMAND, IDOK as usize, 0);
            assert_ne!(IsWindow(hwnd), 0);
            assert!(text(hwnd, ERROR).contains("1 ～ 30"));
            assert_ne!(IsWindowVisible(GetDlgItem(hwnd, TIMEOUT)), 0);
            capture_test_window(hwnd, "settings-validation").unwrap();
        }
        assert_eq!(
            config::load_from_path(&path, false)
                .unwrap()
                .notification_timeout,
            5.0
        );
        unsafe {
            caption(hwnd, TIMEOUT, "8");
            SendDlgItemMessageW(hwnd, PRIVACY, BM_CLICK, 0, 0);
            SendMessageW(hwnd, WM_COMMAND, IDOK as usize, 0);
        }
        assert!(thread.join().unwrap());
        let saved = config::load_from_path(&path, false).unwrap();
        assert!(saved.privacy_mode);
        assert_eq!(saved.notification_timeout, 8.0);
        assert_eq!(saved.notification_volume, 0.42);
        assert_eq!(saved.notification_sound, "default");
        assert_eq!(saved.client_secret, "edited-test-secret");
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("edited-test-secret"));
        std::fs::remove_file(path).unwrap();
    }

    /// Exercise the packaged GUI using a copied executable and dummy values only.
    #[test]
    #[ignore = "requires a freshly built target/release/discord_To_VR.exe"]
    fn release_setup_uses_dummy_credentials_and_never_connects() {
        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        struct Search {
            process: u32,
            hwnd: HWND,
        }
        unsafe extern "system" fn find(hwnd: HWND, parameter: isize) -> i32 {
            let state = &mut *(parameter as *mut Search);
            let mut process = 0;
            GetWindowThreadProcessId(hwnd, &mut process);
            if process == state.process
                && IsWindowVisible(hwnd) != 0
                && !GetDlgItem(hwnd, CLIENT_ID).is_null()
            {
                state.hwnd = hwnd;
                return 0;
            }
            1
        }
        unsafe fn fill(hwnd: HWND, id: i32, value: &str) {
            SendMessageW(
                GetDlgItem(hwnd, id),
                WM_SETTEXT,
                0,
                wide(value).as_ptr() as isize,
            );
        }
        let release =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("target/release/discord_To_VR.exe");
        let directory =
            std::env::temp_dir().join(format!("discord-vr-release-smoke-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let executable = directory.join("discord_To_VR.exe");
        std::fs::copy(release, &executable).unwrap();
        let mut child = ChildGuard(
            std::process::Command::new(&executable)
                .arg("--setup")
                .spawn()
                .unwrap(),
        );
        let mut search = Search {
            process: child.0.id(),
            hwnd: std::ptr::null_mut(),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while search.hwnd.is_null() {
            unsafe {
                EnumWindows(Some(find), &mut search as *mut Search as isize);
            }
            assert!(Instant::now() < deadline, "Release setup did not open");
            std::thread::sleep(Duration::from_millis(10));
        }
        let hwnd = search.hwnd;
        let path = directory.join("config.json");
        unsafe {
            fill(hwnd, CLIENT_ID, "123456789012345678");
            fill(hwnd, SECRET, "dummy-release-smoke-secret");
            fill(hwnd, REDIRECT, "http://localhost/");
            capture_test_window(hwnd, "release-settings-discord").unwrap();
            SendMessageW(hwnd, WM_COMMAND, DISPLAY_TAB as usize, 0);
            fill(hwnd, TIMEOUT, "0");
            SendMessageW(hwnd, WM_COMMAND, IDOK as usize, 0);
            assert!(!path.exists(), "Invalid form must not be saved");
            assert_ne!(IsWindow(hwnd), 0);
            fill(hwnd, TIMEOUT, "8");
            SendDlgItemMessageW(hwnd, PRIVACY, BM_CLICK, 0, 0);
            SendMessageW(hwnd, WM_COMMAND, SOUND_TAB as usize, 0);
            SendDlgItemMessageW(hwnd, SOUND_DEFAULT, BM_CLICK, 0, 0);
            fill(hwnd, VOLUME, "42");
            SendMessageW(hwnd, WM_COMMAND, IDOK as usize, 0);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "Release setup did not finish");
            std::thread::sleep(Duration::from_millis(10));
        }
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("dummy-release-smoke-secret"));
        let cfg = config::load_from_path(&path, false).unwrap();
        assert!(cfg.privacy_mode);
        assert_eq!(cfg.notification_timeout, 8.0);
        assert_eq!(cfg.notification_volume, 0.42);
        assert_eq!(cfg.notification_sound, "default");
        assert_eq!(cfg.client_secret, "dummy-release-smoke-secret");
        let before = std::fs::read(&path).unwrap();
        let check = std::process::Command::new(&executable)
            .arg("--check-config")
            .output()
            .unwrap();
        assert!(check.status.success());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        drop(child);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(executable).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
