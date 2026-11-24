//! The Win32 side of the app: hidden window, message loop, event hooks,
//! hotkeys, timers and the tray menu. All window manager calls happen here, on
//! the UI thread.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::SystemTime;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

use crate::command::Command;
use crate::config::{self, Config};
use crate::win::{self, Hwnd};
use crate::wm::{Effect, TIMER_ANIM, Wm};
use crate::{startup, tray};

pub const CLASS_NAME: PCWSTR = w!("TesselWindow");
/// `dwData` tag for commands sent from the CLI with WM_COPYDATA.
pub const COPYDATA_COMMAND: usize = 0x7E55;

const WM_APP_EVENTS: u32 = WM_APP + 1;
const WM_APP_TRAY: u32 = WM_APP + 2;
const WM_APP_COMMANDS: u32 = WM_APP + 3;

const TIMER_CONFIG: usize = 2;
const TIMER_DISPLAY: usize = 3;
const TIMER_MOUSE: usize = 4;

const MENU_PAUSE: usize = 1;
const MENU_RELOAD: usize = 2;
const MENU_EDIT: usize = 3;
const MENU_LOG: usize = 4;
const MENU_STARTUP: usize = 5;
const MENU_QUIT: usize = 6;

/// The main window, reachable from other threads (console Ctrl+C handler).
pub static MAIN_WINDOW: AtomicIsize = AtomicIsize::new(0);

#[derive(Default)]
struct AppState {
    hotkeys: Vec<Command>,
    config_mtime: Option<SystemTime>,
    taskbar_created: u32,
}

thread_local! {
    static WM: RefCell<Option<Wm>> = const { RefCell::new(None) };
    /// Win events are queued and processed from the message loop, so a hook
    /// firing while the window manager is busy never re-enters it.
    static EVENTS: RefCell<VecDeque<(u32, Hwnd)>> = const { RefCell::new(VecDeque::new()) };
    static COMMANDS: RefCell<VecDeque<Command>> = const { RefCell::new(VecDeque::new()) };
    static STATE: RefCell<AppState> = RefCell::new(AppState::default());
}

fn main_hwnd() -> HWND {
    win::hwnd(MAIN_WINDOW.load(Ordering::Relaxed))
}

/// Run `f` on the window manager, or return None if it is busy further up the stack.
fn with_wm<R>(f: impl FnOnce(&mut Wm) -> R) -> Option<R> {
    WM.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

fn post(msg: u32) {
    unsafe {
        let _ = PostMessageW(Some(main_hwnd()), msg, WPARAM(0), LPARAM(0));
    }
}

pub fn run() -> Result<(), String> {
    win::enable_dpi_awareness();
    let path = config::path();
    if let Err(e) = config::ensure_exists(&path) {
        log!("can't create {}: {e}", path.display());
    }
    let (config, config_error) = match config::load(&path) {
        Ok(c) => (c, None),
        Err(e) => (Config::default(), Some(e)),
    };

    let hwnd = unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS_NAME,
            hIcon: tray::icon(),
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS_NAME,
            w!("tessel"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|e| e.to_string())?
    };
    MAIN_WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
    STATE.with_borrow_mut(|s| {
        s.taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
        s.config_mtime = mtime(&path);
    });
    tray::add(hwnd, WM_APP_TRAY);

    apply_bindings(&config);
    let wm = Wm::new(config, hwnd.0 as Hwnd);
    WM.with_borrow_mut(|cell| *cell = Some(wm));
    if let Some(e) = config_error {
        tray::notify(hwnd, "tessel: config error, using defaults", &e, true);
        log!("config error: {e}");
    }

    let hooks = install_hooks();
    win::set_timer(hwnd.0 as Hwnd, TIMER_CONFIG, 1000);
    log!("tessel started");

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        for hook in hooks {
            let _ = UnhookWinEvent(hook);
        }
    }
    cleanup();
    Ok(())
}

fn cleanup() {
    unregister_hotkeys();
    if let Some(mut wm) = WM.with_borrow_mut(Option::take) {
        wm.shutdown();
    }
    win::restore_hidden();
    tray::remove(main_hwnd());
}

fn install_hooks() -> Vec<HWINEVENTHOOK> {
    let ranges = [
        (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
        (EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MOVESIZEEND),
        (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
        (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
        (EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE),
        (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
    ];
    ranges
        .into_iter()
        .map(|(min, max)| unsafe {
            SetWinEventHook(min, max, None, Some(win_event), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)
        })
        .collect()
}

unsafe extern "system" fn win_event(
    _: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _: u32,
    _: u32,
) {
    // Only whole windows, not their children or accessibility sub-objects.
    if hwnd.is_invalid() || id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 {
        return;
    }
    let was_empty = EVENTS.with_borrow_mut(|q| {
        let was_empty = q.is_empty();
        q.push_back((event, hwnd.0 as Hwnd));
        was_empty
    });
    if was_empty {
        post(WM_APP_EVENTS);
    }
}

fn apply_bindings(config: &Config) -> Vec<String> {
    unregister_hotkeys();
    let hwnd = main_hwnd();
    let mut failed = Vec::new();
    let bindings = config.bindings().unwrap_or_default();
    let commands = bindings
        .into_iter()
        .enumerate()
        .map(|(id, b)| {
            let mods = HOT_KEY_MODIFIERS(b.mods) | MOD_NOREPEAT;
            if unsafe { RegisterHotKey(Some(hwnd), id as i32, mods, b.vk) }.is_err() {
                failed.push(b.keys);
            }
            b.command
        })
        .collect();
    STATE.with_borrow_mut(|s| s.hotkeys = commands);
    if config.general.focus_follows_mouse {
        win::set_timer(hwnd.0 as Hwnd, TIMER_MOUSE, 50);
    } else {
        win::kill_timer(hwnd.0 as Hwnd, TIMER_MOUSE);
    }
    if !failed.is_empty() {
        let list = failed.join(", ");
        log!("hotkeys already taken by another program: {list}");
        tray::notify(hwnd, "tessel: some hotkeys are taken", &list, true);
    }
    failed
}

fn unregister_hotkeys() {
    let count = STATE.with_borrow(|s| s.hotkeys.len());
    for id in 0..count {
        unsafe {
            let _ = UnregisterHotKey(Some(main_hwnd()), id as i32);
        }
    }
}

fn mtime(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn reload(explicit: bool) {
    let path = config::path();
    STATE.with_borrow_mut(|s| s.config_mtime = mtime(&path));
    match config::load(&path) {
        Ok(config) => {
            apply_bindings(&config);
            with_wm(|wm| wm.set_config(config));
            log!("config reloaded");
            if explicit {
                tray::notify(main_hwnd(), "tessel", "Configuration reloaded", false);
            }
        }
        Err(e) => {
            log!("config error: {e}");
            tray::notify(main_hwnd(), "tessel: config error", &e, true);
        }
    }
}

fn run_command(cmd: Command) {
    match with_wm(|wm| wm.dispatch(cmd.clone())) {
        None => {
            // Busy: retry from the message loop.
            COMMANDS.with_borrow_mut(|q| q.push_back(cmd));
            post(WM_APP_COMMANDS);
        }
        Some(Effect::Reload) => reload(true),
        Some(Effect::Quit) => unsafe {
            let _ = DestroyWindow(main_hwnd());
        },
        Some(Effect::None) => {}
    }
}

fn open(path: &std::path::Path) {
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(path.as_os_str()), None, None, SW_SHOWNORMAL);
    }
}

fn tray_menu(hwnd: HWND) {
    let paused = with_wm(|wm| wm.paused).unwrap_or(false);
    let startup = startup::is_enabled();
    let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
    let choice = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING | check(paused), MENU_PAUSE, w!("Pause tiling"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_RELOAD, w!("Reload config"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_EDIT, w!("Edit config"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_LOG, w!("Open log"));
        let _ = AppendMenuW(menu, MF_STRING | check(startup), MENU_STARTUP, w!("Start with Windows"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, w!("Quit"));
        let (x, y) = win::cursor();
        // Required so the menu closes when clicking elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let choice = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, x, y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        choice.0 as usize
    };
    match choice {
        MENU_PAUSE => run_command(Command::TogglePause),
        MENU_RELOAD => run_command(Command::Reload),
        MENU_EDIT => open(&config::path()),
        MENU_LOG => open(&config::dir().join("tessel.log")),
        MENU_STARTUP => {
            if let Err(e) = startup::set(!startup) {
                tray::notify(hwnd, "tessel", &format!("Couldn't change startup setting: {e}"), true);
            }
        }
        MENU_QUIT => run_command(Command::Quit),
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_EVENTS => {
            let done = with_wm(|wm| {
                while let Some((event, h)) = EVENTS.with_borrow_mut(VecDeque::pop_front) {
                    wm.on_event(event, h);
                }
            });
            if done.is_none() {
                post(WM_APP_EVENTS);
            }
        }
        WM_APP_COMMANDS => {
            let pending: Vec<Command> = COMMANDS.with_borrow_mut(|q| q.drain(..).collect());
            for cmd in pending {
                run_command(cmd);
            }
        }
        WM_HOTKEY => {
            if let Some(cmd) = STATE.with_borrow(|s| s.hotkeys.get(wparam.0).cloned()) {
                run_command(cmd);
            }
        }
        WM_COPYDATA => {
            // Sent synchronously by the CLI; queue it rather than run it here.
            let data = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
            if data.dwData != COPYDATA_COMMAND || data.lpData.is_null() {
                return LRESULT(0);
            }
            let bytes = unsafe { std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize) };
            let text = String::from_utf8_lossy(bytes);
            return match Command::parse(&text) {
                Ok(cmd) => {
                    log!("cli: {text}");
                    COMMANDS.with_borrow_mut(|q| q.push_back(cmd));
                    post(WM_APP_COMMANDS);
                    LRESULT(1)
                }
                Err(e) => {
                    log!("cli: {e}");
                    LRESULT(0)
                }
            };
        }
        WM_APP_TRAY => {
            let event = (lparam.0 & 0xFFFF) as u32;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                tray_menu(hwnd);
            }
        }
        WM_TIMER => match wparam.0 {
            TIMER_ANIM => {
                if with_wm(Wm::tick) == Some(false) {
                    win::kill_timer(hwnd.0 as Hwnd, TIMER_ANIM);
                }
            }
            TIMER_CONFIG => {
                let path = config::path();
                let changed = STATE.with_borrow(|s| s.config_mtime) != mtime(&path);
                if changed {
                    reload(false);
                }
            }
            TIMER_DISPLAY => {
                win::kill_timer(hwnd.0 as Hwnd, TIMER_DISPLAY);
                with_wm(Wm::refresh_monitors);
            }
            TIMER_MOUSE => {
                with_wm(Wm::mouse_tick);
            }
            _ => {}
        },
        // Monitors or the taskbar changed; wait for things to settle, then re-layout.
        WM_DISPLAYCHANGE => win::set_timer(hwnd.0 as Hwnd, TIMER_DISPLAY, 500),
        WM_SETTINGCHANGE if wparam.0 as u32 == SPI_SETWORKAREA.0 => win::set_timer(hwnd.0 as Hwnd, TIMER_DISPLAY, 500),
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                cleanup();
            }
        }
        WM_CLOSE => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        _ if msg == STATE.with_borrow(|s| s.taskbar_created) && msg != 0 => tray::add(hwnd, WM_APP_TRAY),
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}
