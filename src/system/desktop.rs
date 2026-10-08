//! Shared native rendering, typography and Windows theme/DPI handling.
use std::io;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::Controls::{
    SetScrollInfo, DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED,
};
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn rgb(value: u32) -> u32 {
    ((value & 0xff) << 16) | (value & 0xff00) | (value >> 16)
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub background: u32,
    pub surface: u32,
    pub text: u32,
    pub muted: u32,
    pub border: u32,
    pub accent: u32,
    pub accent_text: u32,
    pub tint: u32,
    pub good: u32,
    pub warning: u32,
}

impl Palette {
    pub fn system() -> Self {
        let mut light = 1u32;
        let mut size = 4;
        unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
                wide("AppsUseLightTheme").as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                &mut light as *mut _ as _,
                &mut size,
            );
        }
        let mut palette = Self::for_dark(light == 0);
        let mut contrast: HIGHCONTRASTW = unsafe { std::mem::zeroed() };
        contrast.cbSize = std::mem::size_of::<HIGHCONTRASTW>() as u32;
        if unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                contrast.cbSize,
                &mut contrast as *mut _ as _,
                0,
            )
        } != 0
            && contrast.dwFlags & HCF_HIGHCONTRASTON != 0
        {
            unsafe {
                palette.background = GetSysColor(COLOR_WINDOW);
                palette.surface = palette.background;
                palette.text = GetSysColor(COLOR_WINDOWTEXT);
                palette.muted = palette.text;
                palette.border = palette.text;
                palette.accent = GetSysColor(COLOR_HIGHLIGHT);
                palette.accent_text = GetSysColor(COLOR_HIGHLIGHTTEXT);
                palette.tint = palette.surface;
                palette.good = palette.text;
                palette.warning = palette.text;
            }
        }
        palette
    }

    pub fn for_dark(dark: bool) -> Self {
        if dark {
            Self {
                dark,
                background: rgb(0x1c1c1c),
                surface: rgb(0x292929),
                text: rgb(0xf5f5f5),
                muted: rgb(0xb5bac1),
                border: rgb(0x484848),
                accent: rgb(0x60cdff),
                accent_text: rgb(0x101010),
                tint: rgb(0x213743),
                good: rgb(0x6ccb5f),
                warning: rgb(0xffcf70),
            }
        } else {
            Self {
                dark,
                background: rgb(0xf5f7fa),
                surface: rgb(0xffffff),
                text: rgb(0x182536),
                muted: rgb(0x5b6776),
                border: rgb(0xd9dfe7),
                accent: rgb(0x0067c0),
                accent_text: rgb(0xffffff),
                tint: rgb(0xeaf3fc),
                good: rgb(0x107c41),
                warning: rgb(0x895400),
            }
        }
    }
}

pub struct Font(HFONT, bool);
impl Font {
    fn new(dpi: u32, size: i32, weight: i32) -> Self {
        let handle = unsafe {
            CreateFontW(
                -(size * dpi as i32 / 96),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                0,
                wide("Segoe UI").as_ptr(),
            )
        };
        if handle.is_null() {
            Self(unsafe { GetStockObject(DEFAULT_GUI_FONT) } as _, false)
        } else {
            Self(handle, true)
        }
    }
    pub fn handle(&self) -> HFONT {
        self.0
    }
}
impl Drop for Font {
    fn drop(&mut self) {
        if self.1 {
            unsafe {
                DeleteObject(self.0);
            }
        }
    }
}

pub struct Skin {
    pub dpi: u32,
    pub colors: Palette,
    pub body: Font,
    pub small: Font,
    pub strong: Font,
    pub heading: Font,
    pub background: HBRUSH,
    pub surface: HBRUSH,
}
impl Skin {
    pub fn new(dpi: u32) -> Self {
        Self::with_palette(dpi, Palette::system())
    }
    pub fn with_palette(dpi: u32, colors: Palette) -> Self {
        Self {
            dpi: dpi.max(96),
            colors,
            body: Font::new(dpi, 14, 400),
            small: Font::new(dpi, 12, 400),
            strong: Font::new(dpi, 15, 600),
            heading: Font::new(dpi, 24, 600),
            background: unsafe { CreateSolidBrush(colors.background) },
            surface: unsafe { CreateSolidBrush(colors.surface) },
        }
    }
    pub fn px(&self, value: i32) -> i32 {
        value * self.dpi as i32 / 96
    }
    pub fn rect(&self, bounds: [i32; 4]) -> RECT {
        RECT {
            left: self.px(bounds[0]),
            top: self.px(bounds[1]),
            right: self.px(bounds[0] + bounds[2]),
            bottom: self.px(bounds[1] + bounds[3]),
        }
    }
}
impl Drop for Skin {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.background);
            DeleteObject(self.surface);
        }
    }
}

pub unsafe fn create_control(
    hwnd: HWND,
    class: &str,
    text: &str,
    id: i32,
    style: u32,
    font: HFONT,
) -> HWND {
    let child = CreateWindowExW(
        0,
        wide(class).as_ptr(),
        wide(text).as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        0,
        0,
        1,
        1,
        hwnd,
        id as usize as _,
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null(),
    );
    if !child.is_null() {
        SendMessageW(child, WM_SETFONT, font as usize, 1);
    }
    child
}

pub unsafe fn place(hwnd: HWND, id: i32, skin: &Skin, bounds: [i32; 4]) {
    let r = skin.rect(bounds);
    SetWindowPos(
        GetDlgItem(hwnd, id),
        std::ptr::null_mut(),
        r.left,
        r.top,
        r.right - r.left,
        r.bottom - r.top,
        SWP_NOZORDER | SWP_NOACTIVATE,
    );
}

pub unsafe fn caption(hwnd: HWND, id: i32, value: &str) {
    SetDlgItemTextW(hwnd, id, wide(value).as_ptr());
}

pub unsafe fn text(hwnd: HWND, id: i32) -> String {
    let child = GetDlgItem(hwnd, id);
    let length = GetWindowTextLengthW(child).max(0) as usize;
    let mut value = vec![0; length + 1];
    let count = GetWindowTextW(child, value.as_mut_ptr(), value.len() as i32).max(0) as usize;
    String::from_utf16_lossy(&value[..count])
}

pub unsafe fn client_size(hwnd: HWND, skin: &Skin) -> (i32, i32) {
    let mut rect = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    (
        rect.right * 96 / skin.dpi as i32,
        rect.bottom * 96 / skin.dpi as i32,
    )
}

/// A short monitor or large text scale must never make the last buttons unreachable.
pub unsafe fn scroll_range(hwnd: HWND, skin: &Skin, content: i32, requested: i32) -> i32 {
    let (_, height) = client_size(hwnd, skin);
    let position = requested.clamp(0, (content - height).max(0));
    let info = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: content - 1,
        nPage: height.max(1) as u32,
        nPos: position,
        nTrackPos: 0,
    };
    SetScrollInfo(hwnd, SB_VERT, &info, 1);
    position
}

pub unsafe fn scroll_target(hwnd: HWND, position: i32, message: u32, wp: usize) -> i32 {
    if message == WM_MOUSEWHEEL {
        return position - ((wp >> 16) as u16 as i16 as i32) * 48 / 120;
    }
    let mut info: SCROLLINFO = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<SCROLLINFO>() as u32;
    info.fMask = SIF_ALL;
    GetScrollInfo(hwnd, SB_VERT, &mut info);
    match (wp & 0xffff) as i32 {
        SB_LINEUP => position - 32,
        SB_LINEDOWN => position + 32,
        SB_PAGEUP => position - info.nPage as i32,
        SB_PAGEDOWN => position + info.nPage as i32,
        SB_THUMBTRACK | SB_THUMBPOSITION => info.nTrackPos,
        SB_TOP => 0,
        SB_BOTTOM => info.nMax,
        _ => position,
    }
}

pub unsafe fn focus_scroll(hwnd: HWND, skin: &Skin, child: HWND, position: i32) -> i32 {
    let mut rect: RECT = std::mem::zeroed();
    GetWindowRect(child, &mut rect);
    MapWindowPoints(std::ptr::null_mut(), hwnd, &mut rect as *mut _ as _, 2);
    let (_, height) = client_size(hwnd, skin);
    let top = rect.top * 96 / skin.dpi as i32;
    let bottom = rect.bottom * 96 / skin.dpi as i32;
    if top < 8 {
        position + top - 8
    } else if bottom > height - 8 {
        position + bottom - height + 8
    } else {
        position
    }
}

pub unsafe fn set_client_size(hwnd: HWND, width: i32, height: i32) {
    let dpi = GetDpiForWindow(hwnd).max(96);
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: width * dpi as i32 / 96,
        bottom: height * dpi as i32 / 96,
    };
    AdjustWindowRectExForDpi(
        &mut rect,
        GetWindowLongW(hwnd, GWL_STYLE) as u32,
        0,
        GetWindowLongW(hwnd, GWL_EXSTYLE) as u32,
        dpi,
    );
    let mut monitor: MONITORINFO = std::mem::zeroed();
    monitor.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(
        MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
        &mut monitor,
    ) != 0
    {
        let work = monitor.rcWork;
        let w = (rect.right - rect.left).min(work.right - work.left - 16);
        let h = (rect.bottom - rect.top).min(work.bottom - work.top - 16);
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            work.left + (work.right - work.left - w) / 2,
            work.top + (work.bottom - work.top - h) / 2,
            w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

pub unsafe fn title_theme(hwnd: HWND, dark: bool) {
    let enabled: i32 = if dark { 1 } else { 0 };
    DwmSetWindowAttribute(hwnd, 20, &enabled as *const _ as _, 4);
}

pub unsafe fn card(dc: HDC, skin: &Skin, bounds: [i32; 4], fill: u32, border: u32) {
    let saved = SaveDC(dc);
    let brush = CreateSolidBrush(fill);
    let pen = CreatePen(PS_SOLID, skin.px(1).max(1), border);
    SelectObject(dc, brush);
    SelectObject(dc, pen);
    let rect = skin.rect(bounds);
    RoundRect(
        dc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        skin.px(10),
        skin.px(10),
    );
    RestoreDC(dc, saved);
    DeleteObject(brush);
    DeleteObject(pen);
}

pub enum Button {
    Normal,
    Primary,
    Selection(bool),
    Check(bool),
}

pub unsafe fn draw_button(draw: &DRAWITEMSTRUCT, skin: &Skin, kind: Button, on_surface: bool) {
    let saved = SaveDC(draw.hDC);
    let colors = skin.colors;
    let selected = matches!(kind, Button::Selection(true) | Button::Check(true));
    let primary = matches!(kind, Button::Primary);
    let disabled = draw.itemState & ODS_DISABLED != 0;
    let fill = if primary && !disabled {
        colors.accent
    } else if selected || draw.itemState & ODS_SELECTED != 0 {
        colors.tint
    } else {
        colors.surface
    };
    let border = if selected || primary {
        colors.accent
    } else {
        colors.border
    };
    let brush = CreateSolidBrush(fill);
    let pen = CreatePen(PS_SOLID, skin.px(if selected { 2 } else { 1 }), border);
    SelectObject(draw.hDC, brush);
    SelectObject(draw.hDC, pen);
    let r = draw.rcItem;
    FillRect(
        draw.hDC,
        &r,
        if on_surface {
            skin.surface
        } else {
            skin.background
        },
    );
    RoundRect(
        draw.hDC,
        r.left,
        r.top,
        r.right,
        r.bottom,
        skin.px(8),
        skin.px(8),
    );
    SetBkMode(draw.hDC, TRANSPARENT as i32);
    let foreground = if disabled {
        colors.muted
    } else if primary {
        colors.accent_text
    } else {
        colors.text
    };
    SetTextColor(draw.hDC, foreground);
    let label = text(GetParent(draw.hwndItem), draw.CtlID as i32);
    let (heading, detail) = label.split_once('\n').unwrap_or((&label, ""));
    let mut rect = r;
    rect.left += skin.px(14);
    rect.right -= skin.px(14);
    if let Button::Check(checked) = kind {
        let mark = if checked { "✓" } else { "○" };
        SelectObject(draw.hDC, skin.strong.handle());
        let mut check_rect = rect;
        check_rect.right = check_rect.left + skin.px(24);
        DrawTextW(
            draw.hDC,
            wide(mark).as_ptr(),
            -1,
            &mut check_rect,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
        rect.left += skin.px(28);
    }
    SelectObject(draw.hDC, skin.strong.handle());
    if detail.is_empty() {
        DrawTextW(
            draw.hDC,
            wide(heading).as_ptr(),
            -1,
            &mut rect,
            DT_SINGLELINE
                | DT_VCENTER
                | if matches!(kind, Button::Check(_)) {
                    DT_LEFT
                } else {
                    DT_CENTER
                }
                | DT_NOPREFIX
                | DT_END_ELLIPSIS,
        );
    } else {
        rect.top += skin.px(12);
        rect.bottom = rect.top + skin.px(22);
        DrawTextW(
            draw.hDC,
            wide(heading).as_ptr(),
            -1,
            &mut rect,
            DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        rect.top += skin.px(25);
        rect.bottom = r.bottom - skin.px(8);
        SelectObject(draw.hDC, skin.small.handle());
        SetTextColor(draw.hDC, if primary { foreground } else { colors.muted });
        DrawTextW(
            draw.hDC,
            wide(detail).as_ptr(),
            -1,
            &mut rect,
            DT_WORDBREAK | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
    }
    if draw.itemState & ODS_FOCUS != 0 {
        let mut focus = r;
        InflateRect(&mut focus, -skin.px(5), -skin.px(5));
        DrawFocusRect(draw.hDC, &focus);
    }
    RestoreDC(draw.hDC, saved);
    DeleteObject(brush);
    DeleteObject(pen);
}

/// A dialog supplies Windows keyboard navigation; controls and layout use DIPs.
pub unsafe fn show_dialog(
    title: &str,
    resizable: bool,
    procedure: DLGPROC,
    parameter: isize,
) -> io::Result<isize> {
    let mut style = WS_POPUP
        | WS_CAPTION
        | WS_SYSMENU
        | WS_CLIPCHILDREN
        | WS_VSCROLL
        | DS_SETFONT as u32
        | DS_MODALFRAME as u32;
    if resizable {
        style |= WS_THICKFRAME | WS_MINIMIZEBOX;
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&style.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    for value in [0u16, 0, 0, 320, 250, 0, 0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in wide(title) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&9u16.to_le_bytes());
    for value in wide("Segoe UI") {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    let template: Vec<u32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect();
    let result = DialogBoxIndirectParamW(
        GetModuleHandleW(std::ptr::null()),
        template.as_ptr().cast(),
        std::ptr::null_mut(),
        procedure,
        parameter,
    );
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

/// Capture only a window created by our test harness; never the desktop.
#[cfg(test)]
pub unsafe fn capture_test_window(hwnd: HWND, name: &str) -> io::Result<()> {
    let Ok(directory) = std::env::var("DISCORD_VR_CAPTURE_DIR") else {
        return Ok(());
    };
    let mut rect: RECT = std::mem::zeroed();
    GetWindowRect(hwnd, &mut rect);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return Err(io::Error::other("No capture surface"));
    }
    let mut info: BITMAPINFO = std::mem::zeroed();
    info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = width;
    info.bmiHeader.biHeight = -height;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    let mut bits = std::ptr::null_mut();
    let dc = CreateCompatibleDC(std::ptr::null_mut());
    let bitmap = CreateDIBSection(
        dc,
        &info,
        DIB_RGB_COLORS,
        &mut bits,
        std::ptr::null_mut(),
        0,
    );
    if dc.is_null() || bitmap.is_null() {
        if !dc.is_null() {
            DeleteDC(dc);
        }
        return Err(io::Error::last_os_error());
    }
    let previous = SelectObject(dc, bitmap);
    let printed = windows_sys::Win32::Storage::Xps::PrintWindow(hwnd, dc, 2) != 0;
    let mut bytes = Vec::new();
    if printed {
        let size = width as usize * height as usize * 4;
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&((size + 54) as u32).to_le_bytes());
        bytes.extend_from_slice(&[0u8; 4]);
        bytes.extend_from_slice(&54u32.to_le_bytes());
        let header = &info.bmiHeader as *const _ as *const u8;
        bytes.extend_from_slice(std::slice::from_raw_parts(header, 40));
        bytes.extend_from_slice(std::slice::from_raw_parts(bits as *const u8, size));
    }
    SelectObject(dc, previous);
    DeleteObject(bitmap);
    DeleteDC(dc);
    if !printed {
        return Err(io::Error::last_os_error());
    }
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        std::path::Path::new(&directory).join(format!("{name}.bmp")),
        bytes,
    )
}
