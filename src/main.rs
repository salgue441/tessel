mod layout;
mod win;

use std::collections::HashMap;

use layout::{Gaps, dwindle};

const GAPS: Gaps = Gaps { inner: 8, outer: 16 };
const SPLIT_RATIO: f32 = 0.5;

fn main() {
    let dry_run = std::env::args().any(|a| a == "--dry-run");
    win::enable_dpi_awareness();

    // Group windows per monitor; each monitor gets its own dwindle tree.
    let mut by_monitor: HashMap<isize, Vec<win::Window>> = HashMap::new();
    for w in win::manageable_windows() {
        by_monitor.entry(w.monitor.0 as isize).or_default().push(w);
    }

    for windows in by_monitor.values() {
        let area = win::work_area(windows[0].monitor);
        let rects = dwindle(area, windows.len(), GAPS, SPLIT_RATIO);
        for (w, r) in windows.iter().zip(rects) {
            println!("{:>5},{:>5} {:>5}x{:<5} {}", r.x, r.y, r.w, r.h, w.title);
            if !dry_run {
                win::place(w.hwnd, r);
            }
        }
    }
}
