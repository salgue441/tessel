//! Minimal file logger. Also echoes to stderr in debug builds.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use windows::Win32::System::SystemInformation::GetLocalTime;

static FILE: Mutex<Option<File>> = Mutex::new(None);
const MAX_SIZE: u64 = 1024 * 1024;

pub fn init(path: &Path) {
    let too_big = std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_SIZE);
    let file = OpenOptions::new().create(true).append(!too_big).write(true).truncate(too_big).open(path);
    if let Ok(file) = file {
        *FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(file);
    }
}

pub fn write(msg: &str) {
    let t = unsafe { GetLocalTime() };
    let line = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} {msg}\n",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    );
    if cfg!(debug_assertions) {
        eprint!("{line}");
    }
    if let Some(f) = FILE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = f.write_all(line.as_bytes());
    }
}

macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}
