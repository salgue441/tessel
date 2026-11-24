// Release builds are GUI apps (no console window); the CLI attaches to the
// parent console when there is one.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod log;

mod anim;
mod app;
mod command;
mod config;
mod keys;
mod layout;
mod model;
mod startup;
mod tray;
mod win;
mod wm;

use std::ffi::c_void;
use std::process::ExitCode;
use std::sync::atomic::Ordering;

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, LPARAM, WPARAM};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, SetConsoleCtrlHandler};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, MB_ICONINFORMATION, MB_OK, MessageBoxW, PostMessageW, SendMessageW, WM_CLOSE, WM_COPYDATA,
};
use windows::core::{BOOL, w};

use command::Command;

fn help() -> String {
    format!(
        "tessel {} - a dynamic tiling window manager for Windows

USAGE:
  tessel                 start the window manager
  tessel <command>       send a command to the running instance
  tessel --config-path   print the config file location
  tessel --help          show this help

COMMANDS:
{}

Config: {}",
        env!("CARGO_PKG_VERSION"),
        command::HELP,
        config::path().display()
    )
}

fn main() -> ExitCode {
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => start(),
        Some("-h" | "--help" | "help") => {
            println!("{}", help());
            ExitCode::SUCCESS
        }
        Some("-V" | "--version") => {
            println!("tessel {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--config-path") => {
            println!("{}", config::path().display());
            ExitCode::SUCCESS
        }
        Some(_) => match send(&args.join(" ")) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("tessel: {e}");
                ExitCode::FAILURE
            }
        },
    }
}

/// Forward a command to the running instance, like `hyprctl dispatch`.
fn send(text: &str) -> Result<(), String> {
    Command::parse(text)?;
    let target = unsafe { FindWindowW(app::CLASS_NAME, None) }.map_err(|_| "tessel is not running".to_string())?;
    let data = COPYDATASTRUCT {
        dwData: app::COPYDATA_COMMAND,
        cbData: text.len() as u32,
        lpData: text.as_ptr() as *mut c_void,
    };
    let accepted = unsafe { SendMessageW(target, WM_COPYDATA, None, Some(LPARAM(&data as *const _ as isize))) };
    if accepted.0 == 1 { Ok(()) } else { Err("the running instance rejected the command".into()) }
}

unsafe extern "system" fn on_console_ctrl(_: u32) -> BOOL {
    // Ctrl+C in a debug console: shut down cleanly on the UI thread.
    let main = app::MAIN_WINDOW.load(Ordering::Relaxed);
    unsafe {
        let _ = PostMessageW(Some(win::hwnd(main)), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    true.into()
}

fn start() -> ExitCode {
    let _mutex = unsafe { CreateMutexW(None, false, w!("Local\\tessel.single-instance")) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        eprintln!("tessel is already running");
        unsafe {
            MessageBoxW(None, w!("tessel is already running."), w!("tessel"), MB_OK | MB_ICONINFORMATION);
        }
        return ExitCode::FAILURE;
    }

    let _ = std::fs::create_dir_all(config::dir());
    log::init(&config::dir().join("tessel.log"));

    // If anything goes badly wrong, never leave windows hidden on other workspaces.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log!("panic: {info}");
        win::restore_hidden();
        default_hook(info);
    }));
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(on_console_ctrl), true);
    }

    match app::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log!("fatal: {e}");
            win::restore_hidden();
            ExitCode::FAILURE
        }
    }
}
