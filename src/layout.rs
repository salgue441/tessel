//! Pure layout math. No Win32 here, so everything is unit-testable.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// Shrink the rect by `n` pixels on every side.
    pub fn inset(self, n: i32) -> Self {
        Self::new(self.x + n, self.y + n, (self.w - 2 * n).max(0), (self.h - 2 * n).max(0))
    }

    /// Split into two halves separated by `gap` pixels. Wide rects split
    /// side-by-side, tall rects split top/bottom (Hyprland's dwindle rule).
    pub fn split(self, ratio: f32, gap: i32) -> (Self, Self) {
        if self.w >= self.h {
            let first = ((self.w - gap) as f32 * ratio).round() as i32;
            let a = Self::new(self.x, self.y, first, self.h);
            let b = Self::new(self.x + first + gap, self.y, self.w - first - gap, self.h);
            (a, b)
        } else {
            let first = ((self.h - gap) as f32 * ratio).round() as i32;
            let a = Self::new(self.x, self.y, self.w, first);
            let b = Self::new(self.x, self.y + first + gap, self.w, self.h - first - gap);
            (a, b)
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Gaps {
    /// Gap between adjacent windows.
    pub inner: i32,
    /// Gap between windows and the monitor edge.
    pub outer: i32,
}

/// Dwindle layout: each new window takes half of the space left by the
/// previous one, alternating direction according to the remaining aspect ratio.
/// One window fills the whole work area (minus outer gaps).
pub fn dwindle(area: Rect, count: usize, gaps: Gaps, ratio: f32) -> Vec<Rect> {
    let mut out = Vec::with_capacity(count);
    let mut rest = area.inset(gaps.outer);
    for i in 0..count {
        if i == count - 1 {
            out.push(rest);
        } else {
            let (a, b) = rest.split(ratio, gaps.inner);
            out.push(a);
            rest = b;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 0, 1920, 1080);
    const GAPS: Gaps = Gaps { inner: 10, outer: 20 };

    #[test]
    fn single_window_fills_area_minus_outer_gaps() {
        assert_eq!(dwindle(AREA, 1, GAPS, 0.5), vec![Rect::new(20, 20, 1880, 1040)]);
    }

    #[test]
    fn two_windows_side_by_side_on_wide_monitor() {
        let r = dwindle(AREA, 2, GAPS, 0.5);
        assert_eq!(r[0], Rect::new(20, 20, 935, 1040));
        assert_eq!(r[1], Rect::new(965, 20, 935, 1040));
        assert_eq!(r[1].x - (r[0].x + r[0].w), GAPS.inner);
    }

    #[test]
    fn third_window_splits_right_column_vertically() {
        let r = dwindle(AREA, 3, GAPS, 0.5);
        assert_eq!(r[1].x, r[2].x);
        assert_eq!(r[2].y - (r[1].y + r[1].h), GAPS.inner);
    }

    #[test]
    fn empty_workspace_yields_no_rects() {
        assert!(dwindle(AREA, 0, GAPS, 0.5).is_empty());
    }
}
