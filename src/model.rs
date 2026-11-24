//! Workspace bookkeeping, generic over the window handle type so it can be
//! tested without Win32.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fullscreen {
    /// Fills the work area, keeping the outer gaps and the taskbar.
    Maximized,
    /// Covers the entire monitor.
    Full,
}

#[derive(Debug, Clone)]
pub struct Workspace<W> {
    /// Tiled windows in dwindle order.
    pub tiled: Vec<W>,
    pub floating: Vec<W>,
    pub fullscreen: Option<(W, Fullscreen)>,
    /// Ratio of each dwindle split, indexed by split position.
    pub ratios: Vec<f32>,
    pub last_focus: Option<W>,
}

impl<W> Default for Workspace<W> {
    fn default() -> Self {
        Self { tiled: Vec::new(), floating: Vec::new(), fullscreen: None, ratios: Vec::new(), last_focus: None }
    }
}

impl<W: Copy + PartialEq> Workspace<W> {
    pub fn contains(&self, w: W) -> bool {
        self.tiled.contains(&w) || self.floating.contains(&w)
    }

    pub fn is_floating(&self, w: W) -> bool {
        self.floating.contains(&w)
    }

    pub fn windows(&self) -> impl Iterator<Item = W> + '_ {
        self.tiled.iter().chain(&self.floating).copied()
    }

    pub fn tiled_index(&self, w: W) -> Option<usize> {
        self.tiled.iter().position(|&t| t == w)
    }

    /// Insert after `after` (the focused window, like Hyprland), or at the end.
    pub fn insert_tiled(&mut self, w: W, after: Option<W>) {
        let pos = after.and_then(|a| self.tiled_index(a)).map_or(self.tiled.len(), |i| i + 1);
        self.tiled.insert(pos, w);
    }

    pub fn insert_tiled_at(&mut self, w: W, pos: usize) {
        self.tiled.insert(pos.min(self.tiled.len()), w);
    }

    pub fn remove(&mut self, w: W) -> bool {
        let before = self.tiled.len() + self.floating.len();
        self.retain(|x| x != w);
        before != self.tiled.len() + self.floating.len()
    }

    pub fn retain(&mut self, keep: impl Fn(W) -> bool) {
        self.tiled.retain(|&w| keep(w));
        self.floating.retain(|&w| keep(w));
        if self.fullscreen.is_some_and(|(w, _)| !keep(w)) {
            self.fullscreen = None;
        }
        if self.last_focus.is_some_and(|w| !keep(w)) {
            self.last_focus = None;
        }
    }

    /// Returns `Some(true)` if the window is now floating, `Some(false)` if it
    /// is now tiled, and `None` if it isn't on this workspace.
    pub fn toggle_float(&mut self, w: W) -> Option<bool> {
        if let Some(i) = self.tiled_index(w) {
            self.tiled.remove(i);
            self.floating.push(w);
            Some(true)
        } else if let Some(i) = self.floating.iter().position(|&f| f == w) {
            self.floating.remove(i);
            self.tiled.push(w);
            Some(false)
        } else {
            None
        }
    }

    pub fn swap(&mut self, a: W, b: W) {
        if let (Some(i), Some(j)) = (self.tiled_index(a), self.tiled_index(b)) {
            self.tiled.swap(i, j);
        }
    }

    /// Swap with the first (master) window; the master swaps with the second.
    pub fn promote(&mut self, w: W) {
        match self.tiled_index(w) {
            Some(0) if self.tiled.len() > 1 => self.tiled.swap(0, 1),
            Some(i) => self.tiled.swap(0, i),
            None => {}
        }
    }

    pub fn ratio(&self, split: usize, default: f32) -> f32 {
        self.ratios.get(split).copied().unwrap_or(default)
    }

    pub fn set_ratio(&mut self, split: usize, ratio: f32, default: f32) {
        if self.ratios.len() <= split {
            self.ratios.resize(split + 1, default);
        }
        self.ratios[split] = ratio;
    }

    /// Move all windows from `other` into this workspace.
    pub fn absorb(&mut self, other: Workspace<W>) {
        self.tiled.extend(other.tiled);
        self.floating.extend(other.floating);
    }

    /// The window to focus when this workspace becomes active.
    pub fn focus_candidate(&self) -> Option<W> {
        self.last_focus
            .filter(|&w| self.contains(w))
            .or_else(|| self.tiled.first().copied())
            .or_else(|| self.floating.first().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(tiled: &[u32]) -> Workspace<u32> {
        Workspace { tiled: tiled.to_vec(), ..Default::default() }
    }

    #[test]
    fn insert_after_focused() {
        let mut w = ws(&[1, 2, 3]);
        w.insert_tiled(9, Some(1));
        assert_eq!(w.tiled, [1, 9, 2, 3]);
        w.insert_tiled(8, None);
        assert_eq!(w.tiled, [1, 9, 2, 3, 8]);
    }

    #[test]
    fn remove_clears_fullscreen_and_focus() {
        let mut w = ws(&[1, 2]);
        w.fullscreen = Some((2, Fullscreen::Full));
        w.last_focus = Some(2);
        assert!(w.remove(2));
        assert_eq!(w.fullscreen, None);
        assert_eq!(w.last_focus, None);
        assert!(!w.remove(2));
    }

    #[test]
    fn toggle_float_round_trip() {
        let mut w = ws(&[1, 2]);
        assert_eq!(w.toggle_float(1), Some(true));
        assert_eq!(w.tiled, [2]);
        assert_eq!(w.floating, [1]);
        assert_eq!(w.toggle_float(1), Some(false));
        assert_eq!(w.tiled, [2, 1]);
        assert_eq!(w.toggle_float(7), None);
    }

    #[test]
    fn promote_swaps_with_master() {
        let mut w = ws(&[1, 2, 3]);
        w.promote(3);
        assert_eq!(w.tiled, [3, 2, 1]);
        w.promote(3);
        assert_eq!(w.tiled, [2, 3, 1]);
    }

    #[test]
    fn ratios_default_until_set() {
        let mut w = ws(&[]);
        assert_eq!(w.ratio(2, 0.5), 0.5);
        w.set_ratio(2, 0.7, 0.5);
        assert_eq!(w.ratios, [0.5, 0.5, 0.7]);
    }

    #[test]
    fn focus_candidate_prefers_last_focus() {
        let mut w = ws(&[1, 2]);
        assert_eq!(w.focus_candidate(), Some(1));
        w.last_focus = Some(2);
        assert_eq!(w.focus_candidate(), Some(2));
        w.tiled.clear();
        w.floating.push(5);
        assert_eq!(w.focus_candidate(), Some(5));
    }
}
