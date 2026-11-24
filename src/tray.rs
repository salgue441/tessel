//! Notification-area icon, its balloon notifications and the generated icon image.

use std::mem::size_of;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

static ICON: AtomicIsize = AtomicIsize::new(0);

fn copy_into(dst: &mut [u16], s: &str) {
    let max = dst.len() - 1;
    for (d, c) in dst.iter_mut().zip(s.encode_utf16().take(max)) {
        *d = c;
    }
}

fn base(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() }
}

/// A 32x32 icon showing a dwindle layout: one tall tile and two stacked ones.
pub fn icon() -> HICON {
    let cached = ICON.load(Ordering::Relaxed);
    if cached != 0 {
        return HICON(cached as *mut _);
    }
    const SIZE: i32 = 32;
    const ACCENT: u32 = 0xFF33CCFF;
    const LIGHT: u32 = 0xFFE6E6E6;
    let tiles = [(2, 2, 15, 30, ACCENT), (18, 2, 30, 15, LIGHT), (18, 17, 30, 30, LIGHT)];
    let mut pixels = vec![0u32; (SIZE * SIZE) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            if let Some(&(.., c)) = tiles.iter().find(|&&(x0, y0, x1, y1, _)| x >= x0 && x < x1 && y >= y0 && y < y1) {
                pixels[(y * SIZE + x) as usize] = c;
            }
        }
    }
    let mask = vec![0u8; (SIZE * SIZE / 8) as usize];
    let icon = unsafe {
        let color = CreateBitmap(SIZE, SIZE, 1, 32, Some(pixels.as_ptr().cast()));
        let mask = CreateBitmap(SIZE, SIZE, 1, 1, Some(mask.as_ptr().cast()));
        let icon =
            CreateIconIndirect(&ICONINFO { fIcon: true.into(), hbmMask: mask, hbmColor: color, ..Default::default() });
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon.unwrap_or_default()
    };
    ICON.store(icon.0 as isize, Ordering::Relaxed);
    icon
}

pub fn add(hwnd: HWND, callback_message: u32) {
    let mut data = base(hwnd);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = callback_message;
    data.hIcon = icon();
    copy_into(&mut data.szTip, "tessel");
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &data);
    }
}

pub fn remove(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &base(hwnd));
    }
}

pub fn notify(hwnd: HWND, title: &str, body: &str, warning: bool) {
    let mut data = base(hwnd);
    data.uFlags = NIF_INFO;
    data.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
    copy_into(&mut data.szInfoTitle, title);
    copy_into(&mut data.szInfo, body);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}
