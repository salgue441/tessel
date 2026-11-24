# tessel

A dynamic tiling window manager for Windows, inspired by Hyprland.

Windows tile automatically as they open, using Hyprland's **dwindle** layout:
one window fills the screen, two sit side by side, and each new window splits
the focused one in half, alternating direction by aspect ratio. Gaps separate
the windows and the screen edges, and windows animate into place.

```
┌──────────────┬──────────────┐
│              │      2       │
│      1       ├──────┬───────┤
│              │  3   │   4   │
└──────────────┴──────┴───────┘
```

## Features

- **Automatic dwindle tiling** with inner and outer gaps
- **Animations** with configurable duration and curve (`overshoot`, `ease-out`, `linear`)
- **Workspaces**, up to 20 per monitor, with move-to-workspace
- **Multi-monitor**: independent layouts, moving windows across monitors with
  the keyboard or the mouse, and handling of monitors being plugged in or out
- **Floating windows**: toggle per window; dialogs and fixed-size windows float automatically
- **Fullscreen** (whole monitor) and **maximize** (work area with gaps) toggles
- **Mouse support**: drag a tile onto another to swap them, onto another
  monitor to move it there, or drag a tile's edge to resize its split
- **Keyboard resizing** of splits, plus a promote-to-master command
- **Window rules** by exe, class or title: float, ignore, or pin to a workspace
- **Active / inactive border colours** and corner rounding (Windows 11)
- **Focus follows mouse** and **cursor follows focus**, both optional
- **TOML config with hot reload**; errors show as a notification and the
  previous config stays active
- **Tray icon**: pause, reload, edit config, open log, start with Windows, quit
- **CLI control** of a running instance (`tessel workspace 3`), similar to `hyprctl dispatch`
- **Safe exit**: quitting, logging off or even a crash puts all hidden windows back

## Install

Download `tessel.exe` from the [releases](https://github.com/salgue441/tessel/releases),
or build it from source:

```powershell
cargo build --release
.\target\release\tessel.exe
```

On first run tessel writes its config to `%APPDATA%\tessel\tessel.toml`.
Use the tray icon's **Start with Windows** to launch it at login.

## Default key bindings

| Keys                       | Action                                   |
| -------------------------- | ---------------------------------------- |
| `Alt + H/J/K/L`            | Focus left / down / up / right           |
| `Alt + Shift + H/J/K/L`    | Move window (swap, or to the next monitor) |
| `Alt + 1…9`                | Switch workspace                         |
| `Alt + Shift + 1…9`        | Move window to workspace                 |
| `Alt + Shift + Space`      | Toggle floating                          |
| `Alt + Shift + F`          | Toggle fullscreen                        |
| `Alt + Shift + M`          | Toggle maximize (keeps gaps and taskbar) |
| `Alt + Shift + Enter`      | Promote to master                        |
| `Alt + =` / `Alt + -`      | Grow / shrink the focused split          |
| `Alt + Shift + Q`          | Close window                             |
| `Alt + Enter`              | Open Windows Terminal                    |
| `Alt + Shift + R`          | Reload config                            |
| `Alt + Shift + P`          | Pause / resume tiling                    |
| `Alt + Shift + E`          | Quit                                     |

Alt is the default modifier because Windows reserves most `Win + key`
combinations. The trade-off is that these bindings take over the same Alt
shortcuts inside apps. Rebind anything in the config; if another program
already owns a hotkey, tessel tells you which.

## Configuration

```toml
[general]
gaps_in = 5
gaps_out = 20
split_ratio = 0.5
workspaces = 9
focus_follows_mouse = false
cursor_follows_focus = false

[decoration]
active_border = "#33ccff"     # "#rrggbb", "default" or "none"
inactive_border = "#595959"
rounding = "round"            # round | small | square | default

[animations]
enabled = true
duration_ms = 180
curve = "overshoot"           # linear | ease-out | overshoot

[[rules]]
exe = "Spotify"
workspace = 9

[[rules]]
title = "Picture-in-Picture"
float = true

[binds]
"alt+enter" = "exec wt"
"alt+b" = "exec firefox"
```

`exe` and `class` match exactly (case-insensitive), and `title` matches a
substring. UWP apps such as Calculator all report `ApplicationFrameHost`, so
match them by title. See [`config/default.toml`](config/default.toml) for the
full annotated defaults.

## Commands

The same commands work in `[binds]` and from the command line:

```
tessel workspace 3
tessel move-to-workspace next
tessel resize +0.05
tessel exec notepad
```

| Command                              | Effect                                              |
| ------------------------------------ | --------------------------------------------------- |
| `focus <left/right/up/down>`         | Focus the neighbouring window (or monitor)          |
| `move <left/right/up/down>`          | Swap with the neighbour, or move to the next monitor |
| `workspace <1-20/next/prev>`         | Switch workspace on the focused monitor             |
| `move-to-workspace <1-20/next/prev>` | Send the focused window to another workspace        |
| `toggle-float`                       | Float or tile the focused window                    |
| `toggle-fullscreen`                  | Cover the whole monitor                             |
| `toggle-maximize`                    | Fill the work area, keeping gaps and the taskbar    |
| `promote`                            | Swap the focused window with the master             |
| `resize <delta>`                     | Grow (`+0.05`) or shrink (`-0.05`) the focused split |
| `close`                              | Close the focused window                            |
| `retile`                             | Re-scan windows and re-apply the layout             |
| `reload`                             | Reload the config                                   |
| `toggle-pause`                       | Stop or resume tiling                               |
| `quit`                               | Restore all windows and exit                        |
| `exec <command line>`                | Launch a program                                    |

## Known limitations

- **Elevated windows** (Task Manager, admin terminals) can't be moved by a
  non-elevated process. tessel detects this and leaves them alone; run tessel
  as administrator if you want them tiled.
- Windows on inactive workspaces are hidden, so they disappear from the
  taskbar until you switch back. This is the same approach GlazeWM takes.
- Border colours and rounding need Windows 11; on Windows 10 they are skipped.
- Some apps enforce a minimum size and can overlap their neighbours when a
  tile gets very small.

## Architecture

```
src/
  main.rs     CLI, single-instance guard, panic safety
  app.rs      hidden window, message loop, WinEvent hooks, hotkeys, timers, tray menu
  wm.rs       window manager state machine: events, commands, retiling
  model.rs    workspace bookkeeping (generic, unit-tested)
  layout.rs   dwindle layout, split ratios, directional neighbours (pure, unit-tested)
  anim.rs     tweens and easing curves
  config.rs   TOML schema, validation, window rules
  command.rs  command parser shared by binds and the CLI
  keys.rs     hotkey string parser
  win.rs      Win32 wrappers (window filtering, DWM frame compensation, focus)
  tray.rs     tray icon and notifications
  startup.rs  "start with Windows" registry entry
```

WinEvent hooks only queue events; the window manager processes them from the
message loop, so a hook firing during a `SetWindowPos` call can never re-enter
it. Placement uses `DWMWA_EXTENDED_FRAME_BOUNDS` to compensate for the invisible
resize borders of Windows 10/11, which keeps gaps pixel-exact. The process is
per-monitor DPI aware.

## License

MIT
