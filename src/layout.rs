//! Pure layout math. No Win32 here, so everything is unit-testable.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

    pub fn center(self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    /// Split into two halves separated by `gap` pixels. Wide rects split
    /// side-by-side, tall rects split top/bottom (Hyprland's dwindle rule).
    /// The returned bool is true for a side-by-side split.
    pub fn split(self, ratio: f32, gap: i32) -> (Self, Self, bool) {
        if self.w >= self.h {
            let first = ((self.w - gap) as f32 * ratio).round() as i32;
            let a = Self::new(self.x, self.y, first, self.h);
            let b = Self::new(self.x + first + gap, self.y, self.w - first - gap, self.h);
            (a, b, true)
        } else {
            let first = ((self.h - gap) as f32 * ratio).round() as i32;
            let a = Self::new(self.x, self.y, self.w, first);
            let b = Self::new(self.x, self.y + first + gap, self.w, self.h - first - gap);
            (a, b, false)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

pub const MIN_RATIO: f32 = 0.1;
pub const MAX_RATIO: f32 = 0.9;

/// One node of the dwindle tree: `parent` was divided into window `i` and
/// everything after it.
#[derive(Debug, Clone, Copy)]
pub struct Split {
    pub parent: Rect,
    pub horizontal: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub rects: Vec<Rect>,
    pub splits: Vec<Split>,
}

/// Dwindle layout: each window takes `ratio` of the space left by the previous
/// one, alternating direction according to the remaining aspect ratio.
/// One window fills the whole work area (minus outer gaps).
pub fn dwindle(area: Rect, count: usize, gaps: Gaps, ratios: &[f32], default_ratio: f32) -> Layout {
    let mut layout = Layout::default();
    let mut rest = area.inset(gaps.outer);
    for i in 0..count {
        if i == count - 1 {
            layout.rects.push(rest);
        } else {
            let ratio = ratios.get(i).copied().unwrap_or(default_ratio);
            let (a, b, horizontal) = rest.split(ratio, gaps.inner);
            layout.splits.push(Split { parent: rest, horizontal });
            layout.rects.push(a);
            rest = b;
        }
    }
    layout
}

/// The split ratio that gives a window of `size` within `split`. `first` says
/// whether the window is the first (left/top) child of that split.
pub fn ratio_for_size(split: &Split, first: bool, size: Rect, gap: i32) -> f32 {
    let (total, len) = if split.horizontal { (split.parent.w - gap, size.w) } else { (split.parent.h - gap, size.h) };
    if total <= 0 {
        return 0.5;
    }
    let ratio = len as f32 / total as f32;
    let ratio = if first { ratio } else { 1.0 - ratio };
    ratio.clamp(MIN_RATIO, MAX_RATIO)
}

fn spans_overlap(a: i32, a_len: i32, b: i32, b_len: i32) -> bool {
    a < b + b_len && b < a + a_len.max(1)
}

/// The closest candidate in `dir` from `from`. Candidates that line up with
/// `from` on the perpendicular axis win over ones that are merely closer.
pub fn neighbor<K: Copy>(from: Rect, candidates: &[(K, Rect)], dir: Direction) -> Option<K> {
    let (fx, fy) = from.center();
    candidates
        .iter()
        .filter_map(|&(k, r)| {
            let (cx, cy) = r.center();
            let (ahead, distance, aligned, offset) = match dir {
                Direction::Right => {
                    (cx > fx, r.x - (from.x + from.w), spans_overlap(from.y, from.h, r.y, r.h), (cy - fy).abs())
                }
                Direction::Left => {
                    (cx < fx, from.x - (r.x + r.w), spans_overlap(from.y, from.h, r.y, r.h), (cy - fy).abs())
                }
                Direction::Down => {
                    (cy > fy, r.y - (from.y + from.h), spans_overlap(from.x, from.w, r.x, r.w), (cx - fx).abs())
                }
                Direction::Up => {
                    (cy < fy, from.y - (r.y + r.h), spans_overlap(from.x, from.w, r.x, r.w), (cx - fx).abs())
                }
            };
            // Offsets within a few pixels count as a tie, broken toward the top-left.
            ahead.then_some((k, (!aligned, distance.max(0), offset / 8, r.y, r.x)))
        })
        .min_by_key(|&(_, score)| score)
        .map(|(k, _)| k)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 0, 1920, 1080);
    const GAPS: Gaps = Gaps { inner: 10, outer: 20 };

    fn rects(count: usize) -> Vec<Rect> {
        dwindle(AREA, count, GAPS, &[], 0.5).rects
    }

    #[test]
    fn single_window_fills_area_minus_outer_gaps() {
        assert_eq!(rects(1), vec![Rect::new(20, 20, 1880, 1040)]);
    }

    #[test]
    fn two_windows_side_by_side_on_wide_monitor() {
        let r = rects(2);
        assert_eq!(r[0], Rect::new(20, 20, 935, 1040));
        assert_eq!(r[1], Rect::new(965, 20, 935, 1040));
        assert_eq!(r[1].x - (r[0].x + r[0].w), GAPS.inner);
    }

    #[test]
    fn third_window_splits_right_column_vertically() {
        let r = rects(3);
        assert_eq!(r[1].x, r[2].x);
        assert_eq!(r[2].y - (r[1].y + r[1].h), GAPS.inner);
    }

    #[test]
    fn empty_workspace_yields_no_rects() {
        assert!(rects(0).is_empty());
    }

    #[test]
    fn custom_ratio_changes_first_split() {
        let l = dwindle(AREA, 2, GAPS, &[0.7], 0.5);
        assert_eq!(l.rects[0].w, (1870.0f32 * 0.7).round() as i32);
        assert!(l.splits[0].horizontal);
    }

    #[test]
    fn ratio_for_size_round_trips() {
        let l = dwindle(AREA, 2, GAPS, &[0.6], 0.5);
        let s = &l.splits[0];
        assert!((ratio_for_size(s, true, l.rects[0], GAPS.inner) - 0.6).abs() < 0.01);
        assert!((ratio_for_size(s, false, l.rects[1], GAPS.inner) - 0.6).abs() < 0.01);
    }

    #[test]
    fn ratio_is_clamped() {
        let l = dwindle(AREA, 2, GAPS, &[], 0.5);
        assert_eq!(ratio_for_size(&l.splits[0], true, Rect::new(0, 0, 5, 5), GAPS.inner), MIN_RATIO);
    }

    #[test]
    fn neighbor_follows_the_dwindle_layout() {
        let r = rects(3);
        let c: Vec<(usize, Rect)> = r.iter().copied().enumerate().collect();
        let others = |i: usize| c.iter().copied().filter(|&(k, _)| k != i).collect::<Vec<_>>();
        assert_eq!(neighbor(r[0], &others(0), Direction::Right), Some(1));
        assert_eq!(neighbor(r[1], &others(1), Direction::Down), Some(2));
        assert_eq!(neighbor(r[2], &others(2), Direction::Up), Some(1));
        assert_eq!(neighbor(r[2], &others(2), Direction::Left), Some(0));
        assert_eq!(neighbor(r[0], &others(0), Direction::Left), None);
    }

    #[test]
    fn neighbor_from_a_point() {
        let r = rects(2);
        let c = vec![(0, r[0]), (1, r[1])];
        let center = Rect::new(960, 540, 0, 0);
        assert_eq!(neighbor(center, &c, Direction::Right), Some(1));
        assert_eq!(neighbor(center, &c, Direction::Left), Some(0));
    }
}
