//! Thin wrappers over the Win32 calls tessel needs.

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::BOOL;

use crate::layout::Rect;

pub fn enable_dpi_awareness() {
    // Without this, coordinates are virtualised on scaled displays.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

#[derive(Debug, Clone)]
pub struct Window {
    pub hwnd: HWND,
    pub title: String,
    pub monitor: HMONITOR,
}

/// Top-level windows that a user would consider "app windows".
pub fn manageable_windows() -> Vec<Window> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let out = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        out.push(hwnd);
        true.into()
    }

    let mut hwnds: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut hwnds as *mut _ as isize));
    }
    hwnds
        .into_iter()
        .filter(|&h| is_manageable(h))
        .map(|hwnd| Window {
            hwnd,
            title: title(hwnd),
            monitor: unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) },
        })
        .collect()
}

fn is_manageable(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }
        if GetWindow(hwnd, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
            return false;
        }
        let style = WINDOW_STYLE(GetWindowLongPtrW(hwnd, GWL_STYLE) as u32);
        let ex = WINDOW_EX_STYLE(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32);
        if !style.contains(WS_CAPTION) || ex.contains(WS_EX_TOOLWINDOW) || ex.contains(WS_EX_NOACTIVATE) {
            return false;
        }
        // UWP/suspended apps and windows on other virtual desktops are "cloaked".
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut c_void, size_of::<u32>() as u32);
        cloaked == 0 && GetWindowTextLengthW(hwnd) > 0
    }
}

fn title(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..len])
}

pub fn work_area(monitor: HMONITOR) -> Rect {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(monitor, &mut info);
    }
    to_rect(info.rcWork)
}

/// Move a window so its *visible* frame lands exactly on `target`.
/// Windows 10/11 windows have invisible resize borders that would otherwise
/// make gaps look uneven, so we compensate using the DWM frame bounds.
pub fn place(hwnd: HWND, target: Rect) {
    unsafe {
        if IsZoomed(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let mut outer = RECT::default();
        let mut visible = RECT::default();
        let _ = GetWindowRect(hwnd, &mut outer);
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut visible as *mut _ as *mut c_void,
            size_of::<RECT>() as u32,
        );
        let (l, t) = (visible.left - outer.left, visible.top - outer.top);
        let (r, b) = (outer.right - visible.right, outer.bottom - visible.bottom);
        let _ = SetWindowPos(
            hwnd,
            None,
            target.x - l,
            target.y - t,
            target.w + l + r,
            target.h + t + b,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

fn to_rect(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}
