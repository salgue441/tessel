//! Thin wrappers over the Win32 calls tessel needs. Window handles are passed
//! around as `isize` so the rest of the code can hash and compare them freely.

use std::ffi::c_void;
use std::mem::size_of;
use std::sync::Mutex;

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DWMWA_WINDOW_CORNER_PREFERENCE,
    DwmGetWindowAttribute, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    MonitorFromRect, MonitorFromWindow,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput, VK_LBUTTON, VK_RBUTTON,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PWSTR};

use crate::layout::Rect;

pub type Hwnd = isize;

pub fn hwnd(h: Hwnd) -> HWND {
    HWND(h as *mut c_void)
}

/// Windows hidden by tessel (inactive workspaces). Kept global so a panic hook
/// can bring them back even if the window manager state is unreachable.
static HIDDEN: Mutex<Vec<Hwnd>> = Mutex::new(Vec::new());

fn hidden() -> std::sync::MutexGuard<'static, Vec<Hwnd>> {
    HIDDEN.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn enable_dpi_awareness() {
    // Without this, coordinates are virtualised on scaled displays.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub fn top_level_windows() -> Vec<Hwnd> {
    unsafe extern "system" fn collect(h: HWND, lparam: LPARAM) -> BOOL {
        let out = unsafe { &mut *(lparam.0 as *mut Vec<Hwnd>) };
        out.push(h.0 as Hwnd);
        true.into()
    }
    let mut out: Vec<Hwnd> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut out as *mut Vec<Hwnd> as isize));
    }
    out
}

fn style(h: Hwnd) -> WINDOW_STYLE {
    WINDOW_STYLE(unsafe { GetWindowLongPtrW(hwnd(h), GWL_STYLE) } as u32)
}

fn ex_style(h: Hwnd) -> WINDOW_EX_STYLE {
    WINDOW_EX_STYLE(unsafe { GetWindowLongPtrW(hwnd(h), GWL_EXSTYLE) } as u32)
}

/// Top-level windows that a user would consider "app windows".
pub fn is_manageable(h: Hwnd) -> bool {
    let w = hwnd(h);
    unsafe {
        if !IsWindowVisible(w).as_bool() || IsIconic(w).as_bool() || is_cloaked(h) {
            return false;
        }
        if GetAncestor(w, GA_ROOT) != w || GetWindow(w, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
            return false;
        }
    }
    if process_id(h) == std::process::id() {
        return false;
    }
    let (style, ex) = (style(h), ex_style(h));
    style.contains(WS_CAPTION)
        && !style.contains(WS_CHILD)
        && !ex.contains(WS_EX_TOOLWINDOW)
        && !ex.contains(WS_EX_NOACTIVATE)
        && unsafe { GetWindowTextLengthW(w) } > 0
}

/// Dialog-like windows that can't be resized should float.
pub fn is_fixed_size(h: Hwnd) -> bool {
    let s = style(h);
    !s.contains(WS_THICKFRAME) || !s.contains(WS_MAXIMIZEBOX)
}

/// True when an app made itself borderless-fullscreen (games, F11 in browsers).
pub fn is_app_fullscreen(h: Hwnd, monitor: Rect) -> bool {
    if style(h).contains(WS_CAPTION) {
        return false;
    }
    let r = window_rect(h);
    r.x <= monitor.x && r.y <= monitor.y && r.x + r.w >= monitor.x + monitor.w && r.y + r.h >= monitor.y + monitor.h
}

/// UWP/suspended apps and windows on other virtual desktops are "cloaked".
pub fn is_cloaked(h: Hwnd) -> bool {
    let mut cloaked = 0u32;
    let _ = unsafe {
        DwmGetWindowAttribute(hwnd(h), DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut c_void, size_of::<u32>() as u32)
    };
    cloaked != 0
}

pub fn is_window(h: Hwnd) -> bool {
    unsafe { IsWindow(Some(hwnd(h))).as_bool() }
}

pub fn is_visible(h: Hwnd) -> bool {
    unsafe { IsWindowVisible(hwnd(h)).as_bool() }
}

fn process_id(h: Hwnd) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd(h), Some(&mut pid)) };
    pid
}

pub fn title(h: Hwnd) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd(h), &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..len])
}

pub fn class_name(h: Hwnd) -> String {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd(h), &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..len])
}

/// Executable name without extension, e.g. "firefox".
pub fn exe_name(h: Hwnd) -> String {
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id(h)) }) else {
        return String::new();
    };
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) };
    unsafe {
        let _ = CloseHandle(process);
    }
    if ok.is_err() {
        return String::new();
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

pub struct MonitorInfo {
    pub handle: isize,
    pub rect: Rect,
    pub work: Rect,
}

/// All monitors, ordered left to right, then top to bottom.
pub fn monitors() -> Vec<MonitorInfo> {
    unsafe extern "system" fn collect(m: HMONITOR, _: HDC, _: *mut RECT, lparam: LPARAM) -> BOOL {
        let out = unsafe { &mut *(lparam.0 as *mut Vec<HMONITOR>) };
        out.push(m);
        true.into()
    }
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut handles as *mut Vec<HMONITOR> as isize));
    }
    let mut out: Vec<MonitorInfo> = handles
        .into_iter()
        .map(|m| {
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            unsafe {
                let _ = GetMonitorInfoW(m, &mut info);
            }
            MonitorInfo { handle: m.0 as isize, rect: to_rect(info.rcMonitor), work: to_rect(info.rcWork) }
        })
        .collect();
    out.sort_by_key(|m| (m.rect.x, m.rect.y));
    out
}

pub fn monitor_of(h: Hwnd) -> isize {
    unsafe { MonitorFromWindow(hwnd(h), MONITOR_DEFAULTTONEAREST) }.0 as isize
}

pub fn monitor_at(x: i32, y: i32) -> isize {
    unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) }.0 as isize
}

pub fn monitor_at_cursor() -> isize {
    let (x, y) = cursor();
    monitor_at(x, y)
}

fn window_rect(h: Hwnd) -> Rect {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd(h), &mut r);
    }
    to_rect(r)
}

/// The visible frame of a window, excluding the invisible resize borders.
pub fn frame_rect(h: Hwnd) -> Rect {
    let mut r = RECT::default();
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd(h),
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut _ as *mut c_void,
            size_of::<RECT>() as u32,
        )
    };
    if ok.is_ok() { to_rect(r) } else { window_rect(h) }
}

/// Move a window so its *visible* frame lands exactly on `target`.
/// Windows 10/11 windows have invisible resize borders that would otherwise
/// make gaps look uneven, so we compensate using the DWM frame bounds.
/// `settle` marks the final position of a move: it repaints the frame and
/// repeats the move when crossing monitors, since apps rescale on DPI change.
pub fn place(h: Hwnd, target: Rect, settle: bool) -> Result<(), ()> {
    let w = hwnd(h);
    unsafe {
        if IsZoomed(w).as_bool() {
            let _ = ShowWindow(w, SW_RESTORE);
        }
    }
    let outer = window_rect(h);
    let visible = frame_rect(h);
    let (l, t) = (visible.x - outer.x, visible.y - outer.y);
    let (r, b) = (outer.x + outer.w - visible.x - visible.w, outer.y + outer.h - visible.y - visible.h);
    let mut flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER;
    if settle {
        flags |= SWP_FRAMECHANGED;
    }
    let crosses_monitor = settle && {
        let rect = RECT { left: target.x, top: target.y, right: target.x + target.w, bottom: target.y + target.h };
        unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST) != MonitorFromWindow(w, MONITOR_DEFAULTTONEAREST) }
    };
    for _ in 0..if crosses_monitor { 2 } else { 1 } {
        unsafe {
            SetWindowPos(w, None, target.x - l, target.y - t, target.w + l + r, target.h + t + b, flags)
                .map_err(|_| ())?;
        }
    }
    Ok(())
}

pub fn raise(h: Hwnd) {
    unsafe {
        let _ = SetWindowPos(hwnd(h), Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

pub fn hide(h: Hwnd) {
    let mut hidden = hidden();
    if !hidden.contains(&h) {
        hidden.push(h);
    }
    unsafe {
        let _ = ShowWindow(hwnd(h), SW_HIDE);
    }
}

pub fn show(h: Hwnd) {
    hidden().retain(|&x| x != h);
    unsafe {
        let _ = ShowWindow(hwnd(h), SW_SHOWNA);
    }
}

pub fn is_hidden_by_us(h: Hwnd) -> bool {
    hidden().contains(&h)
}

/// Show every window tessel hid. Safe to call from a panic hook.
pub fn restore_hidden() {
    let windows = std::mem::take(&mut *hidden());
    for h in windows {
        unsafe {
            let _ = ShowWindow(hwnd(h), SW_SHOWNA);
        }
    }
}

pub fn foreground() -> Hwnd {
    unsafe { GetForegroundWindow() }.0 as Hwnd
}

pub fn focus(h: Hwnd) {
    let w = hwnd(h);
    unsafe {
        if IsIconic(w).as_bool() {
            let _ = ShowWindow(w, SW_RESTORE);
        }
        if SetForegroundWindow(w).as_bool() {
            return;
        }
        // Windows only lets the process that received the last input event
        // take focus, so send a no-op mouse move to become that process.
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: MOUSEEVENTF_MOVE, ..Default::default() } },
        };
        SendInput(&[input], size_of::<INPUT>() as i32);
        let _ = SetForegroundWindow(w);
    }
}

/// Give focus to the desktop, used when switching to an empty workspace.
pub fn focus_desktop() {
    unsafe {
        let shell = GetShellWindow();
        if !shell.is_invalid() {
            let _ = SetForegroundWindow(shell);
        }
    }
}

pub fn close(h: Hwnd) {
    unsafe {
        let _ = PostMessageW(Some(hwnd(h)), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

/// Window border colour as a COLORREF (Windows 11 only; ignored elsewhere).
pub fn set_border_color(h: Hwnd, color: u32) {
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd(h), DWMWA_BORDER_COLOR, &color as *const _ as *const c_void, 4);
    }
}

/// A DWM_WINDOW_CORNER_PREFERENCE value (Windows 11 only).
pub fn set_corners(h: Hwnd, preference: i32) {
    unsafe {
        let _ =
            DwmSetWindowAttribute(hwnd(h), DWMWA_WINDOW_CORNER_PREFERENCE, &preference as *const _ as *const c_void, 4);
    }
}

pub fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

pub fn set_cursor(x: i32, y: i32) {
    unsafe {
        let _ = SetCursorPos(x, y);
    }
}

/// The top-level window under the cursor.
pub fn window_at_cursor() -> Hwnd {
    let (x, y) = cursor();
    unsafe { GetAncestor(WindowFromPoint(POINT { x, y }), GA_ROOT) }.0 as Hwnd
}

pub fn mouse_button_down() -> bool {
    unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 || GetAsyncKeyState(VK_RBUTTON.0 as i32) < 0 }
}

pub fn set_timer(owner: Hwnd, id: usize, ms: u32) {
    unsafe { SetTimer(Some(hwnd(owner)), id, ms, None) };
}

pub fn kill_timer(owner: Hwnd, id: usize) {
    unsafe {
        let _ = KillTimer(Some(hwnd(owner)), id);
    }
}

fn to_rect(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}
