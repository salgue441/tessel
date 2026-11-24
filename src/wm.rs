//! The window manager: owns all workspace state and reacts to window events
//! and commands. Runs entirely on the UI thread.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_CLOAKED, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_SHOW,
    EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART,
    EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART,
};

use crate::anim::Tween;
use crate::command::{Command, WsTarget};
use crate::config::{BORDER_DEFAULT, Config, WindowInfo};
use crate::layout::{self, Direction, Gaps, Layout, Rect};
use crate::model::{Fullscreen, Workspace};
use crate::win::{self, Hwnd};

pub const TIMER_ANIM: usize = 1;
const FRAME_MS: u32 = 8;

pub enum Effect {
    None,
    Reload,
    Quit,
}

pub struct Monitor {
    pub handle: isize,
    pub rect: Rect,
    pub work: Rect,
    pub workspaces: Vec<Workspace<Hwnd>>,
    pub active: usize,
}

impl Monitor {
    fn new(info: &win::MonitorInfo, count: usize) -> Self {
        Self {
            handle: info.handle,
            rect: info.rect,
            work: info.work,
            workspaces: (0..count).map(|_| Workspace::default()).collect(),
            active: 0,
        }
    }

    fn ws(&self) -> &Workspace<Hwnd> {
        &self.workspaces[self.active]
    }
}

pub struct Wm {
    pub config: Config,
    pub monitors: Vec<Monitor>,
    pub paused: bool,
    /// Our hidden message window, which owns the animation timer.
    main: Hwnd,
    focused_mon: usize,
    /// The managed window currently wearing the active border.
    focused: Option<Hwnd>,
    ignored: HashSet<Hwnd>,
    tweens: HashMap<Hwnd, Tween>,
    animating: bool,
    failed: Vec<Hwnd>,
    drag: Option<(Hwnd, Rect)>,
    /// Set when we focus the desktop ourselves, so that focus event doesn't
    /// move the focused monitor to wherever the cursor happens to be.
    keep_monitor_on_focus: bool,
}

fn center_in(area: Rect, w: i32, h: i32) -> Rect {
    let (w, h) = (w.min(area.w), h.min(area.h));
    Rect::new(area.x + (area.w - w) / 2, area.y + (area.h - h) / 2, w, h)
}

impl Wm {
    pub fn new(config: Config, main: Hwnd) -> Self {
        let mut wm = Self {
            config,
            monitors: Vec::new(),
            paused: false,
            main,
            focused_mon: 0,
            focused: None,
            ignored: HashSet::new(),
            tweens: HashMap::new(),
            animating: false,
            failed: Vec::new(),
            drag: None,
            keep_monitor_on_focus: false,
        };
        wm.refresh_monitors();
        wm.sync(true);
        wm.retile_all();
        wm.on_focus(win::foreground());
        wm
    }

    fn gaps(&self) -> Gaps {
        Gaps { inner: self.config.general.gaps_in, outer: self.config.general.gaps_out }
    }

    // ---- bookkeeping ----------------------------------------------------

    pub fn refresh_monitors(&mut self) {
        let infos = win::monitors();
        if infos.is_empty() {
            return;
        }
        let count = self.config.general.workspaces;
        let mut old = std::mem::take(&mut self.monitors);
        let mut monitors: Vec<Monitor> = infos
            .iter()
            .map(|info| match old.iter().position(|m| m.handle == info.handle) {
                Some(i) => {
                    let mut m = old.remove(i);
                    m.rect = info.rect;
                    m.work = info.work;
                    m
                }
                None => Monitor::new(info, count),
            })
            .collect();
        // Windows on a disconnected monitor move to the first one.
        for orphan in old {
            for (i, ws) in orphan.workspaces.into_iter().enumerate() {
                monitors[0].workspaces[i.min(count - 1)].absorb(ws);
            }
        }
        log!("{} monitor(s)", monitors.len());
        self.monitors = monitors;
        self.focused_mon = self.focused_mon.min(self.monitors.len() - 1);
        self.sync_visibility();
        self.retile_all();
    }

    /// Show windows on active workspaces and hide all others.
    fn sync_visibility(&self) {
        for m in &self.monitors {
            for (i, ws) in m.workspaces.iter().enumerate() {
                for h in ws.windows() {
                    let hidden = win::is_hidden_by_us(h);
                    if i == m.active && hidden {
                        win::show(h);
                    } else if i != m.active && !hidden {
                        win::hide(h);
                    }
                }
            }
        }
    }

    fn set_workspace_count(&mut self, count: usize) {
        for m in &mut self.monitors {
            while m.workspaces.len() > count {
                let extra = m.workspaces.pop().unwrap_or_default();
                if let Some(last) = m.workspaces.last_mut() {
                    last.absorb(extra);
                }
            }
            m.workspaces.resize_with(count, Workspace::default);
            m.active = m.active.min(count - 1);
        }
        self.sync_visibility();
    }

    /// Drop windows that no longer exist and adopt any we don't know yet.
    fn sync(&mut self, initial: bool) {
        for m in &mut self.monitors {
            for ws in &mut m.workspaces {
                ws.retain(win::is_window);
            }
        }
        self.ignored.retain(|&h| win::is_window(h));
        for h in win::top_level_windows() {
            if self.locate(h).is_none() && !self.ignored.contains(&h) && win::is_manageable(h) {
                self.manage(h, initial);
            }
        }
    }

    fn locate(&self, h: Hwnd) -> Option<(usize, usize)> {
        self.monitors
            .iter()
            .enumerate()
            .find_map(|(mi, m)| m.workspaces.iter().position(|ws| ws.contains(h)).map(|wi| (mi, wi)))
    }

    fn ws_mut(&mut self, (m, w): (usize, usize)) -> &mut Workspace<Hwnd> {
        &mut self.monitors[m].workspaces[w]
    }

    fn monitor_index(&self, handle: isize) -> usize {
        self.monitors.iter().position(|m| m.handle == handle).unwrap_or(self.focused_mon)
    }

    /// The focused window, if tessel manages it.
    fn focused_managed(&self) -> Option<(Hwnd, (usize, usize))> {
        let h = win::foreground();
        self.locate(h).map(|loc| (h, loc))
    }

    /// Start managing a window. `initial` windows stay on their own monitor;
    /// new ones open on the focused monitor, like Hyprland. Returns the
    /// monitor that needs a retile.
    fn manage(&mut self, h: Hwnd, initial: bool) -> Option<usize> {
        let (exe, class, title) = (win::exe_name(h), win::class_name(h), win::title(h));
        let rule = self.config.match_rules(&WindowInfo { exe: &exe, class: &class, title: &title });
        if rule.ignore {
            log!("ignoring {exe} \"{title}\" (rule)");
            self.ignored.insert(h);
            return None;
        }
        let own = self.monitor_index(win::monitor_of(h));
        let m = if initial { own } else { self.focused_mon };
        let count = self.monitors[m].workspaces.len();
        let wi = rule.workspace.map_or(self.monitors[m].active, |n| (n - 1).min(count - 1));
        let float = rule.float || win::is_fixed_size(h);

        win::set_corners(h, self.config.corner_preference());
        win::set_border_color(h, self.config.inactive_border());
        let work = self.monitors[m].work;
        let ws = self.ws_mut((m, wi));
        if float {
            ws.floating.push(h);
            if own != m {
                let r = win::frame_rect(h);
                let _ = win::place(h, center_in(work, r.w, r.h), true);
            }
        } else {
            let after = ws.last_focus;
            ws.insert_tiled(h, after);
        }
        if wi != self.monitors[m].active {
            win::hide(h);
        }
        let kind = if float { "floating" } else { "tiled" };
        log!("managing {exe} \"{title}\" on monitor {m}, workspace {} ({kind})", wi + 1);
        Some(m)
    }

    fn unmanage(&mut self, h: Hwnd) -> Option<usize> {
        let loc = self.locate(h)?;
        self.ws_mut(loc).remove(h);
        self.tweens.remove(&h);
        if self.focused == Some(h) {
            self.focused = None;
        }
        Some(loc.0)
    }

    // ---- events ---------------------------------------------------------

    pub fn on_event(&mut self, event: u32, h: Hwnd) {
        if self.paused {
            return;
        }
        match event {
            EVENT_OBJECT_SHOW | EVENT_OBJECT_UNCLOAKED | EVENT_SYSTEM_MINIMIZEEND | EVENT_OBJECT_NAMECHANGE => {
                if self.locate(h).is_none() && !self.ignored.contains(&h) && win::is_manageable(h) {
                    if let Some(m) = self.manage(h, false) {
                        self.retile(m);
                    }
                    if win::foreground() == h {
                        self.on_focus(h);
                    }
                }
            }
            EVENT_OBJECT_DESTROY => {
                self.ignored.remove(&h);
                if let Some(m) = self.unmanage(h) {
                    self.retile(m);
                }
            }
            EVENT_OBJECT_HIDE | EVENT_OBJECT_CLOAKED | EVENT_SYSTEM_MINIMIZESTART => {
                let Some((m, wi)) = self.locate(h) else { return };
                // Windows on inactive workspaces are hidden by us on purpose.
                if wi != self.monitors[m].active {
                    return;
                }
                let gone = match event {
                    EVENT_OBJECT_HIDE => !win::is_visible(h),
                    EVENT_OBJECT_CLOAKED => win::is_cloaked(h),
                    _ => true,
                };
                if gone && self.unmanage(h).is_some() {
                    self.retile(m);
                }
            }
            EVENT_SYSTEM_FOREGROUND => self.on_focus(h),
            EVENT_SYSTEM_MOVESIZESTART => {
                if self.locate(h).is_some() {
                    self.tweens.remove(&h);
                    self.drag = Some((h, win::frame_rect(h)));
                }
            }
            EVENT_SYSTEM_MOVESIZEEND => self.on_drag_end(h),
            _ => {}
        }
    }

    fn on_focus(&mut self, h: Hwnd) {
        let keep_monitor = std::mem::take(&mut self.keep_monitor_on_focus);
        if let Some((m, wi)) = self.locate(h) {
            self.monitors[m].workspaces[wi].last_focus = Some(h);
            // An app on a hidden workspace activated itself: follow it there.
            if wi != self.monitors[m].active {
                self.switch_workspace(m, wi, false);
            }
            self.focused_mon = m;
            self.set_focused(Some(h));
        } else {
            if !keep_monitor && h != 0 {
                self.focused_mon = self.monitor_index(win::monitor_at_cursor());
            }
            self.set_focused(None);
        }
    }

    fn set_focused(&mut self, new: Option<Hwnd>) {
        if self.focused == new {
            return;
        }
        if let Some(old) = self.focused
            && win::is_window(old)
        {
            win::set_border_color(old, self.config.inactive_border());
        }
        if let Some(h) = new {
            win::set_border_color(h, self.config.active_border());
        }
        self.focused = new;
    }

    /// The user finished dragging or resizing a window with the mouse.
    fn on_drag_end(&mut self, h: Hwnd) {
        let Some((m, wi)) = self.locate(h) else { return };
        let start = self.drag.take().filter(|d| d.0 == h).map(|d| d.1);
        let now = win::frame_rect(h);
        let (cx, cy) = win::cursor();
        let target_m = self.monitor_index(win::monitor_at(cx, cy));

        if self.monitors[m].workspaces[wi].is_floating(h) {
            if target_m != m {
                self.move_to_monitor(h, (m, wi), target_m, None, false);
            }
            return;
        }

        let resized = start.is_some_and(|s| (s.w - now.w).abs() > 2 || (s.h - now.h).abs() > 2);
        if resized {
            self.resize_from_drag(h, (m, wi), now);
        } else {
            // Dropped onto another tile: swap, or move to that monitor.
            let target_ws = self.monitors[target_m].active;
            let layout = self.layout_for(target_m, target_ws);
            let tiled = &self.monitors[target_m].workspaces[target_ws].tiled;
            let under = tiled.iter().zip(&layout.rects).position(|(&t, r)| t != h && r.contains(cx, cy));
            if target_m == m {
                if let Some(i) = under {
                    let other = tiled[i];
                    self.ws_mut((m, wi)).swap(h, other);
                }
            } else {
                self.move_to_monitor(h, (m, wi), target_m, Some(under.unwrap_or(usize::MAX)), false);
                return;
            }
        }
        self.retile(m);
    }

    fn resize_from_drag(&mut self, h: Hwnd, loc: (usize, usize), size: Rect) {
        let layout = self.layout_for(loc.0, loc.1);
        let gap = self.config.general.gaps_in;
        let default = self.config.general.split_ratio;
        let ws = self.ws_mut(loc);
        let n = ws.tiled.len();
        let Some(i) = ws.tiled_index(h) else { return };
        if n < 2 {
            return;
        }
        // A window's own split is the one it heads; the last window belongs to the previous one.
        let s = if i < n - 1 { i } else { i - 1 };
        let ratio = layout::ratio_for_size(&layout.splits[s], i == s, size, gap);
        ws.set_ratio(s, ratio, default);
    }

    // ---- layout ---------------------------------------------------------

    fn layout_for(&self, m: usize, wi: usize) -> Layout {
        let mon = &self.monitors[m];
        let ws = &mon.workspaces[wi];
        layout::dwindle(mon.work, ws.tiled.len(), self.gaps(), &ws.ratios, self.config.general.split_ratio)
    }

    pub fn retile_all(&mut self) {
        for m in 0..self.monitors.len() {
            self.retile(m);
        }
    }

    fn retile(&mut self, m: usize) {
        if self.paused || m >= self.monitors.len() {
            return;
        }
        let mon = &self.monitors[m];
        let ws = mon.ws();
        let layout = self.layout_for(m, mon.active);
        let monitor_rect = mon.rect;
        let mut targets: Vec<(Hwnd, Rect)> = ws
            .tiled
            .iter()
            .copied()
            .zip(layout.rects)
            .filter(|&(h, _)| !win::is_app_fullscreen(h, monitor_rect))
            .collect();
        let fullscreen = ws.fullscreen;
        if let Some((f, mode)) = fullscreen {
            let r = match mode {
                Fullscreen::Full => mon.rect,
                Fullscreen::Maximized => mon.work.inset(self.config.general.gaps_out),
            };
            targets.retain(|&(h, _)| h != f);
            targets.push((f, r));
        }
        for (h, r) in targets {
            self.move_window(h, r);
        }
        if let Some((f, _)) = fullscreen {
            win::raise(f);
        }
        self.handle_failures();
    }

    fn move_window(&mut self, h: Hwnd, to: Rect) {
        let anim = &self.config.animations;
        if !anim.enabled || anim.duration_ms == 0 {
            if win::place(h, to, true).is_err() {
                self.failed.push(h);
            }
            return;
        }
        let now = Instant::now();
        let from = match self.tweens.get(&h) {
            Some(t) if t.to == to => return,
            Some(t) => t.sample(now).0,
            None => win::frame_rect(h),
        };
        if from == to {
            self.tweens.remove(&h);
            return;
        }
        let duration = Duration::from_millis(anim.duration_ms);
        self.tweens.insert(h, Tween { from, to, start: now, duration, curve: self.config.curve() });
        if !self.animating {
            self.animating = true;
            win::set_timer(self.main, TIMER_ANIM, FRAME_MS);
        }
    }

    /// Advance animations by one frame. Returns false once all are done.
    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        let mut finished = Vec::new();
        for (&h, t) in &self.tweens {
            let (r, done) = t.sample(now);
            if win::place(h, r, done).is_err() {
                self.failed.push(h);
                finished.push(h);
            } else if done {
                finished.push(h);
            }
        }
        for h in finished {
            self.tweens.remove(&h);
        }
        self.handle_failures();
        self.animating = !self.tweens.is_empty();
        self.animating
    }

    /// Windows we can't move (usually elevated apps, since tessel isn't) get ignored.
    fn handle_failures(&mut self) {
        let failed = std::mem::take(&mut self.failed);
        let mut touched = Vec::new();
        for h in failed {
            if self.ignored.insert(h) {
                log!("can't move \"{}\" (elevated?); ignoring it", win::title(h));
            }
            if let Some(m) = self.unmanage(h) {
                touched.push(m);
            }
        }
        touched.dedup();
        for m in touched {
            self.retile(m);
        }
    }

    // ---- commands -------------------------------------------------------

    pub fn dispatch(&mut self, cmd: Command) -> Effect {
        if self.paused && !matches!(cmd, Command::TogglePause | Command::Reload | Command::Quit | Command::Exec(_)) {
            return Effect::None;
        }
        match cmd {
            Command::Focus(d) => self.focus_direction(d),
            Command::Move(d) => self.move_direction(d),
            Command::Workspace(t) => {
                let m = self.focused_mon;
                let wi = self.resolve_workspace(m, t);
                self.switch_workspace(m, wi, true);
            }
            Command::MoveToWorkspace(t) => self.move_to_workspace(t),
            Command::ToggleFloat => self.toggle_float(),
            Command::ToggleFullscreen => self.toggle_fullscreen(Fullscreen::Full),
            Command::ToggleMaximize => self.toggle_fullscreen(Fullscreen::Maximized),
            Command::Close => {
                let h = win::foreground();
                // Never send WM_CLOSE to the desktop or taskbar.
                if self.locate(h).is_some() || win::is_manageable(h) {
                    win::close(h);
                }
            }
            Command::Promote => {
                if let Some((h, loc)) = self.focused_managed() {
                    self.ws_mut(loc).promote(h);
                    self.retile(loc.0);
                }
            }
            Command::Resize(delta) => self.resize(delta),
            Command::Retile => {
                self.sync(false);
                self.retile_all();
            }
            Command::TogglePause => self.toggle_pause(),
            Command::Exec(cmdline) => spawn(&cmdline),
            Command::Reload => return Effect::Reload,
            Command::Quit => return Effect::Quit,
        }
        Effect::None
    }

    fn resolve_workspace(&self, m: usize, target: WsTarget) -> usize {
        let count = self.monitors[m].workspaces.len();
        let active = self.monitors[m].active;
        match target {
            WsTarget::Index(i) => i,
            WsTarget::Next => (active + 1) % count,
            WsTarget::Prev => (active + count - 1) % count,
        }
    }

    /// Windows currently on screen, across all monitors.
    fn visible_windows(&self) -> Vec<Hwnd> {
        self.monitors
            .iter()
            .flat_map(|m| {
                let ws = m.ws();
                match ws.fullscreen {
                    Some((f, _)) => vec![f],
                    None => ws.windows().collect(),
                }
            })
            .collect()
    }

    fn monitor_neighbor(&self, from: usize, d: Direction) -> Option<usize> {
        let candidates: Vec<(usize, Rect)> =
            self.monitors.iter().enumerate().filter(|&(i, _)| i != from).map(|(i, m)| (i, m.rect)).collect();
        layout::neighbor(self.monitors[from].rect, &candidates, d)
    }

    fn focus_window(&self, h: Hwnd) {
        win::focus(h);
        if self.config.general.cursor_follows_focus {
            let (x, y) = win::frame_rect(h).center();
            win::set_cursor(x, y);
        }
    }

    /// Focus the remembered window of a monitor's active workspace, or the desktop.
    fn focus_workspace(&mut self, m: usize) {
        match self.monitors[m].ws().focus_candidate() {
            Some(h) => self.focus_window(h),
            None => {
                self.keep_monitor_on_focus = true;
                win::focus_desktop();
                self.set_focused(None);
                if self.config.general.cursor_follows_focus {
                    let (x, y) = self.monitors[m].work.center();
                    win::set_cursor(x, y);
                }
            }
        }
    }

    fn focus_direction(&mut self, d: Direction) {
        let fg = win::foreground();
        let from = if self.locate(fg).is_some() {
            win::frame_rect(fg)
        } else {
            let (x, y) = self.monitors[self.focused_mon].work.center();
            Rect::new(x, y, 0, 0)
        };
        let candidates: Vec<(Hwnd, Rect)> =
            self.visible_windows().into_iter().filter(|&h| h != fg).map(|h| (h, win::frame_rect(h))).collect();
        if let Some(h) = layout::neighbor(from, &candidates, d) {
            self.focus_window(h);
        } else if let Some(m) = self.monitor_neighbor(self.focused_mon, d) {
            // Nothing in that direction but an empty monitor.
            self.focused_mon = m;
            self.focus_workspace(m);
        }
    }

    fn move_direction(&mut self, d: Direction) {
        let Some((h, (m, wi))) = self.focused_managed() else { return };
        let work = self.monitors[m].work;
        if self.monitors[m].workspaces[wi].is_floating(h) {
            let mut r = win::frame_rect(h);
            let (dx, dy) = (work.w / 20, work.h / 20);
            match d {
                Direction::Left => r.x -= dx,
                Direction::Right => r.x += dx,
                Direction::Up => r.y -= dy,
                Direction::Down => r.y += dy,
            }
            self.move_window(h, r);
            return;
        }
        let layout = self.layout_for(m, wi);
        let tiled = &self.monitors[m].workspaces[wi].tiled;
        let from = tiled.iter().position(|&t| t == h).map(|i| layout.rects[i]);
        let candidates: Vec<(Hwnd, Rect)> =
            tiled.iter().copied().zip(layout.rects.iter().copied()).filter(|&(t, _)| t != h).collect();
        match from.and_then(|r| layout::neighbor(r, &candidates, d)) {
            Some(other) => {
                self.ws_mut((m, wi)).swap(h, other);
                self.retile(m);
            }
            None => {
                if let Some(target) = self.monitor_neighbor(m, d) {
                    self.move_to_monitor(h, (m, wi), target, None, true);
                }
            }
        }
        if self.config.general.cursor_follows_focus {
            self.focus_window(h);
        }
    }

    /// Move a window to another monitor's active workspace, at tiled position
    /// `pos` (end if None). `recenter` re-centres floating windows.
    fn move_to_monitor(&mut self, h: Hwnd, from: (usize, usize), to: usize, pos: Option<usize>, recenter: bool) {
        let floating = self.monitors[from.0].workspaces[from.1].is_floating(h);
        self.ws_mut(from).remove(h);
        let to_loc = (to, self.monitors[to].active);
        let work = self.monitors[to].work;
        let ws = self.ws_mut(to_loc);
        if floating {
            ws.floating.push(h);
            if recenter {
                let r = win::frame_rect(h);
                self.move_window(h, center_in(work, r.w, r.h));
            }
        } else {
            match pos {
                Some(p) => ws.insert_tiled_at(h, p),
                None => ws.tiled.push(h),
            }
        }
        self.ws_mut(to_loc).last_focus = Some(h);
        self.focused_mon = to;
        self.retile(from.0);
        self.retile(to);
    }

    fn switch_workspace(&mut self, m: usize, wi: usize, focus: bool) {
        let mon = &self.monitors[m];
        if wi >= mon.workspaces.len() || wi == mon.active {
            return;
        }
        let old: Vec<Hwnd> = mon.ws().windows().collect();
        self.monitors[m].active = wi;
        // Show the new workspace before hiding the old one to avoid a flash of desktop.
        let new: Vec<Hwnd> = self.monitors[m].ws().windows().collect();
        for &h in &new {
            win::show(h);
        }
        self.retile(m);
        for h in old {
            self.tweens.remove(&h);
            win::hide(h);
        }
        self.focused_mon = m;
        if focus {
            self.focus_workspace(m);
        }
        log!("monitor {m}: workspace {}", wi + 1);
    }

    fn move_to_workspace(&mut self, target: WsTarget) {
        let Some((h, (m, wi))) = self.focused_managed() else { return };
        let to = self.resolve_workspace(m, target);
        if to == wi || to >= self.monitors[m].workspaces.len() {
            return;
        }
        let floating = self.monitors[m].workspaces[wi].is_floating(h);
        self.ws_mut((m, wi)).remove(h);
        let dest = self.ws_mut((m, to));
        if floating {
            dest.floating.push(h);
        } else {
            dest.tiled.push(h);
        }
        dest.last_focus = Some(h);
        self.tweens.remove(&h);
        win::hide(h);
        self.set_focused(None);
        self.retile(m);
        self.focus_workspace(m);
    }

    fn toggle_float(&mut self) {
        let Some((h, (m, wi))) = self.focused_managed() else { return };
        let work = self.monitors[m].work;
        if self.ws_mut((m, wi)).toggle_float(h) == Some(true) {
            self.move_window(h, center_in(work, work.w * 3 / 5, work.h * 3 / 5));
            win::raise(h);
        }
        self.retile(m);
    }

    fn toggle_fullscreen(&mut self, mode: Fullscreen) {
        let Some((h, (m, wi))) = self.focused_managed() else { return };
        let work = self.monitors[m].work;
        let ws = self.ws_mut((m, wi));
        let leaving = ws.fullscreen == Some((h, mode));
        ws.fullscreen = if leaving { None } else { Some((h, mode)) };
        let floating = ws.is_floating(h);
        if leaving && floating {
            self.move_window(h, center_in(work, work.w * 3 / 5, work.h * 3 / 5));
        }
        self.retile(m);
    }

    fn resize(&mut self, delta: f32) {
        let Some((h, (m, wi))) = self.focused_managed() else { return };
        let work = self.monitors[m].work;
        let default = self.config.general.split_ratio;
        let ws = self.ws_mut((m, wi));
        if ws.is_floating(h) {
            let r = win::frame_rect(h);
            let (dw, dh) = ((work.w as f32 * delta) as i32, (work.h as f32 * delta) as i32);
            let r = Rect::new(r.x - dw / 2, r.y - dh / 2, (r.w + dw).max(100), (r.h + dh).max(100));
            self.move_window(h, r);
            return;
        }
        let n = ws.tiled.len();
        let Some(i) = ws.tiled_index(h) else { return };
        if n < 2 {
            return;
        }
        let s = if i < n - 1 { i } else { i - 1 };
        let grow = if i == s { delta } else { -delta };
        let ratio = (ws.ratio(s, default) + grow).clamp(layout::MIN_RATIO, layout::MAX_RATIO);
        ws.set_ratio(s, ratio, default);
        self.retile(m);
    }

    fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        log!("tiling {}", if self.paused { "paused" } else { "resumed" });
        if self.paused {
            self.tweens.clear();
            self.set_focused(None);
        } else {
            self.sync(false);
            self.retile_all();
            self.on_focus(win::foreground());
        }
    }

    /// Focus follows mouse, polled from a timer.
    pub fn mouse_tick(&mut self) {
        if !self.config.general.focus_follows_mouse || self.paused || self.animating || win::mouse_button_down() {
            return;
        }
        let h = win::window_at_cursor();
        if h != win::foreground() && self.locate(h).is_some() {
            win::focus(h);
        }
    }

    pub fn set_config(&mut self, config: Config) {
        self.config = config;
        self.set_workspace_count(self.config.general.workspaces);
        for m in &self.monitors {
            for ws in &m.workspaces {
                for h in ws.windows() {
                    win::set_corners(h, self.config.corner_preference());
                    let color = if Some(h) == self.focused {
                        self.config.active_border()
                    } else {
                        self.config.inactive_border()
                    };
                    win::set_border_color(h, color);
                }
            }
        }
        self.retile_all();
    }

    /// Put every window back the way Windows expects it: visible, default borders.
    pub fn shutdown(&mut self) {
        self.tweens.clear();
        for m in &self.monitors {
            for ws in &m.workspaces {
                for h in ws.windows() {
                    if win::is_hidden_by_us(h) {
                        win::show(h);
                    }
                    win::set_border_color(h, BORDER_DEFAULT);
                    win::set_corners(h, 0);
                }
            }
        }
        win::restore_hidden();
        log!("shut down");
    }
}

fn spawn(cmdline: &str) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let result = std::process::Command::new("cmd")
        .raw_arg(format!("/C start \"\" {cmdline}"))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    if let Err(e) = result {
        log!("exec `{cmdline}` failed: {e}");
    }
}
