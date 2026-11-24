# tessel

A dynamic tiling window manager for Windows, inspired by Hyprland.

Windows are tiled automatically as they open, using a dwindle layout: one window
fills the screen, two sit side by side, and each new window splits the most
recent space in half. Gaps sit between windows and around the screen edge.

> **Status:** early prototype. Today `tessel` tiles the current windows once per
> monitor and exits. See the roadmap below.

## Usage

```powershell
cargo run -- --dry-run   # print the computed layout without moving anything
cargo run                # tile all windows on every monitor
```

## Why Rust + Win32

A tiling manager is mostly Win32 calls: `SetWinEventHook` to watch windows,
`SetWindowPos`/`DeferWindowPos` to place them, `RegisterHotKey` for key
bindings, and DWM for borders and rounded corners. Rust (through Microsoft's
[`windows`](https://github.com/microsoft/windows-rs) crate) gives:

- a single native binary with no runtime, which keeps latency low
- direct, zero-cost access to the entire Win32 API surface
- memory safety around a lot of `unsafe` FFI

## Architecture

```
src/
  layout.rs   pure layout math (dwindle, gaps); no Win32, fully unit-tested
  win.rs      Win32 wrappers: window enumeration/filtering, monitors, placement
  main.rs     entry point
```

Details handled so far:

- **Per-monitor DPI awareness**, so coordinates aren't scaled on HiDPI screens
- **Invisible-border compensation**: Windows 10/11 windows have transparent
  resize borders, so tessel uses `DWMWA_EXTENDED_FRAME_BOUNDS` to make gaps
  pixel-exact
- **Window filtering** that skips tool windows, owned dialogs, minimised and
  cloaked windows (including windows on other virtual desktops)

## Roadmap

- [x] Dwindle layout with inner/outer gaps
- [x] Multi-monitor (independent layout per monitor)
- [ ] Event loop with `SetWinEventHook`, so new/closed windows retile live
- [ ] Global hotkeys (focus/move/swap directionally, toggle float, fullscreen)
- [ ] Workspaces (hide/show window sets per monitor)
- [ ] Floating rules per app (`class`/`exe`/`title`)
- [ ] Config file (TOML) with hot reload
- [ ] Active-window border colour via `DWMWA_BORDER_COLOR` (Windows 11)
- [ ] Master layout, resizable split ratios
- [ ] Animations (interpolated `SetWindowPos`)
- [ ] Tray icon and run on startup

## License

MIT
