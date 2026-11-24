//! Window movement animations.

use std::time::{Duration, Instant};

use crate::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    Linear,
    EaseOut,
    /// Slightly overshoots the target before settling, like Hyprland's default bezier.
    Overshoot,
}

impl Curve {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "linear" => Some(Self::Linear),
            "ease-out" => Some(Self::EaseOut),
            "overshoot" => Some(Self::Overshoot),
            _ => None,
        }
    }

    pub fn apply(self, t: f32) -> f32 {
        match self {
            Self::Linear => t,
            Self::EaseOut => 1.0 - (1.0 - t).powi(3),
            Self::Overshoot => {
                const C1: f32 = 1.2;
                const C3: f32 = C1 + 1.0;
                1.0 + C3 * (t - 1.0).powi(3) + C1 * (t - 1.0).powi(2)
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Tween {
    pub from: Rect,
    pub to: Rect,
    pub start: Instant,
    pub duration: Duration,
    pub curve: Curve,
}

impl Tween {
    /// The rect at `now`, and whether the animation has finished.
    pub fn sample(&self, now: Instant) -> (Rect, bool) {
        let elapsed = now.saturating_duration_since(self.start).as_secs_f32();
        let total = self.duration.as_secs_f32();
        if total <= 0.0 || elapsed >= total {
            return (self.to, true);
        }
        (lerp(self.from, self.to, self.curve.apply(elapsed / total)), false)
    }
}

pub fn lerp(a: Rect, b: Rect, t: f32) -> Rect {
    let mix = |x: i32, y: i32| x + ((y - x) as f32 * t).round() as i32;
    Rect::new(mix(a.x, b.x), mix(a.y, b.y), mix(a.w, b.w).max(1), mix(a.h, b.h).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Rect = Rect::new(0, 0, 100, 100);
    const B: Rect = Rect::new(100, 50, 200, 300);

    #[test]
    fn lerp_endpoints_and_midpoint() {
        assert_eq!(lerp(A, B, 0.0), A);
        assert_eq!(lerp(A, B, 1.0), B);
        assert_eq!(lerp(A, B, 0.5), Rect::new(50, 25, 150, 200));
    }

    #[test]
    fn curves_start_at_zero_and_end_at_one() {
        for c in [Curve::Linear, Curve::EaseOut, Curve::Overshoot] {
            assert!(c.apply(0.0).abs() < 1e-5, "{c:?}");
            assert!((c.apply(1.0) - 1.0).abs() < 1e-5, "{c:?}");
        }
        assert!((1..10).map(|i| Curve::Overshoot.apply(i as f32 / 10.0)).any(|v| v > 1.0));
    }

    #[test]
    fn tween_finishes_after_duration() {
        let start = Instant::now();
        let t = Tween { from: A, to: B, start, duration: Duration::from_millis(100), curve: Curve::EaseOut };
        assert_eq!(t.sample(start), (A, false));
        assert_eq!(t.sample(start + Duration::from_millis(150)), (B, true));
    }
}
