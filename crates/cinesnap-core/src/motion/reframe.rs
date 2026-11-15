//! Where the window sits when a recording is reframed into another shape
//! (a vertical video of a wide recording).
//!
//! The window follows the cursor with a dead zone, so small movements leave
//! it still, and moves slowly enough to read. While the mouse rests it
//! follows what changes on screen instead (typing, a terminal). Manual
//! segments pin it, easing in and out of them.

use crate::activity::Activity;
use crate::motion::camera::{ease, View};
use crate::motion::cursor::{grid_len, smooth_zero_phase, STEP_MS};
use crate::project::{Easing, Rect, ReframeSegment};

/// How far the target may roam before the window follows (fraction of the
/// window size, from its centre).
const DEAD_ZONE: f64 = 0.25;
const TAU_MS: f64 = 450.0;
/// Time to ease into and out of a manual segment.
pub const SEGMENT_EASE_MS: f64 = 450.0;
/// The mouse is resting after this long within `IDLE_PX`.
const IDLE_MS: f64 = 1200.0;
const IDLE_PX: f64 = 6.0;
/// Changes bigger than this share of the screen (a page loading) are not
/// something to follow.
const MAX_FOLLOWED_CHANGE: f64 = 0.35;

pub struct Reframing<'a> {
    pub crop: Rect,
    /// Window size in source pixels.
    pub width: f64,
    pub height: f64,
    /// The calm cursor, per grid step.
    pub follow: (&'a [f64], &'a [f64]),
    pub activity: Option<&'a Activity>,
    /// In the clip's own time.
    pub segments: &'a [ReframeSegment],
}

/// Weight of a manual segment at `t`: 0 outside, 1 inside, eased between.
fn segment_weight(seg: &ReframeSegment, t: f64) -> f64 {
    if t <= seg.start_ms - SEGMENT_EASE_MS || t >= seg.end_ms + SEGMENT_EASE_MS {
        return 0.0;
    }
    let into = (t - (seg.start_ms - SEGMENT_EASE_MS)) / SEGMENT_EASE_MS;
    let out = ((seg.end_ms + SEGMENT_EASE_MS) - t) / SEGMENT_EASE_MS;
    ease(Easing::Smooth, into.min(out))
}

/// Centre of the window per grid step.
pub fn rest_path(r: &Reframing, duration_ms: f64) -> (Vec<f64>, Vec<f64>) {
    let n = grid_len(duration_ms);
    let view = View {
        crop: r.crop,
        width: r.width,
        height: r.height,
        rest_x: &[],
        rest_y: &[],
    };
    let (fx, fy) = r.follow;
    let at = |v: &[f64], i: usize| v.get(i).or(v.last()).copied();
    let center = r.crop.center();

    // What to keep in the window: the cursor, or while it rests, the
    // largest recent change on screen.
    let idle_steps = (IDLE_MS / STEP_MS) as usize;
    let mut tx = Vec::with_capacity(n);
    let mut ty = Vec::with_capacity(n);
    for i in 0..n {
        let cur = (at(fx, i).unwrap_or(center.0), at(fy, i).unwrap_or(center.1));
        let back = i.saturating_sub(idle_steps);
        let then = (at(fx, back).unwrap_or(cur.0), at(fy, back).unwrap_or(cur.1));
        let resting = i >= idle_steps && (cur.0 - then.0).hypot(cur.1 - then.1) < IDLE_PX;
        let change = r
            .activity
            .filter(|_| resting)
            .and_then(|a| a.at(i as f64 * STEP_MS))
            .filter(|c| c.amount <= MAX_FOLLOWED_CHANGE);
        let target = change.map_or(cur, |c| c.center());
        tx.push(target.0);
        ty.push(target.1);
    }

    // Dead zone, then smoothing without lag, then keep it in the recording.
    let (dzx, dzy) = (r.width * DEAD_ZONE, r.height * DEAD_ZONE);
    let mut pos = view.clamp(
        tx.first().copied().unwrap_or(center.0),
        ty.first().copied().unwrap_or(center.1),
        1.0,
    );
    let mut px = Vec::with_capacity(n);
    let mut py = Vec::with_capacity(n);
    for i in 0..n {
        let (dx, dy) = (tx[i] - pos.0, ty[i] - pos.1);
        if dx.abs() > dzx {
            pos.0 += dx - dzx * dx.signum();
        }
        if dy.abs() > dzy {
            pos.1 += dy - dzy * dy.signum();
        }
        pos = view.clamp(pos.0, pos.1, 1.0);
        px.push(pos.0);
        py.push(pos.1);
    }
    smooth_zero_phase(&mut px, TAU_MS);
    smooth_zero_phase(&mut py, TAU_MS);

    for i in 0..n {
        let t = i as f64 * STEP_MS;
        let mut p = view.clamp(px[i], py[i], 1.0);
        for seg in r.segments {
            let w = segment_weight(seg, t);
            if w > 0.0 {
                let m = view.clamp(seg.x, seg.y, 1.0);
                p = (p.0 + (m.0 - p.0) * w, p.1 + (m.1 - p.1) * w);
            }
        }
        px[i] = p.0;
        py[i] = p.1;
    }
    (px, py)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::analyze_frames;

    /// A 16:9 recording reframed to 9:16: a full-height window 607.5 px wide.
    const CROP: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    };
    const W: f64 = 1080.0 * 9.0 / 16.0;

    fn path(
        xs: &[f64],
        ys: &[f64],
        activity: Option<&Activity>,
        segments: &[ReframeSegment],
    ) -> (Vec<f64>, Vec<f64>) {
        let dur = (xs.len() - 1) as f64 * STEP_MS;
        rest_path(
            &Reframing {
                crop: CROP,
                width: W,
                height: 1080.0,
                follow: (xs, ys),
                activity,
                segments,
            },
            dur,
        )
    }

    /// A cursor at `a` that moves to `b` at `at_ms`, for `dur_ms`.
    fn jump(a: f64, b: f64, at_ms: f64, dur_ms: f64) -> (Vec<f64>, Vec<f64>) {
        let n = grid_len(dur_ms);
        let xs = (0..n)
            .map(|i| if (i as f64) * STEP_MS < at_ms { a } else { b })
            .collect();
        (xs, vec![540.0; n])
    }

    fn inside(x: f64) -> bool {
        x - W / 2.0 >= -1e-6 && x + W / 2.0 <= 1920.0 + 1e-6
    }

    #[test]
    fn the_window_follows_the_cursor_across_the_screen() {
        let (xs, ys) = jump(400.0, 1500.0, 2000.0, 8000.0);
        let (px, py) = path(&xs, &ys, None, &[]);
        // Smoothing without lag leans a little towards the move to come.
        assert!(
            (px[0] - 400.0).abs() < 10.0,
            "starts on the cursor: {}",
            px[0]
        );
        let end = *px.last().unwrap();
        // Comes to rest with the cursor at the edge of the dead zone.
        assert!((end - (1500.0 - W * DEAD_ZONE)).abs() < 2.0, "end {end}");
        assert!(px.iter().all(|&x| inside(x)));
        // The window is full height, so it never moves vertically.
        assert!(py.iter().all(|&y| (y - 540.0).abs() < 1e-9));
    }

    #[test]
    fn small_movements_leave_the_window_still() {
        let (xs, ys) = jump(900.0, 900.0 + 0.8 * W * DEAD_ZONE, 1000.0, 5000.0);
        let (px, _) = path(&xs, &ys, None, &[]);
        assert!(px.iter().all(|&x| (x - 900.0).abs() < 1e-6));
    }

    #[test]
    fn the_window_stops_at_the_edges_of_the_recording() {
        let (xs, ys) = jump(5.0, 1915.0, 2000.0, 6000.0);
        let (px, _) = path(&xs, &ys, None, &[]);
        assert!((px[0] - W / 2.0).abs() < 10.0);
        assert!((px.last().unwrap() - (1920.0 - W / 2.0)).abs() < 0.5);
        assert!(px.iter().all(|&x| inside(x)));
    }

    #[test]
    fn motion_is_smooth() {
        let (xs, ys) = jump(300.0, 1700.0, 1000.0, 6000.0);
        let (px, _) = path(&xs, &ys, None, &[]);
        let fastest = px
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f64::max);
        // Under 15 px per 120th of a second.
        assert!(fastest < 15.0, "{fastest}");
    }

    #[test]
    fn a_manual_segment_pins_the_window_and_eases_back() {
        let n = grid_len(10_000.0);
        let (xs, ys) = (vec![400.0; n], vec![540.0; n]);
        let seg = ReframeSegment {
            id: "r".into(),
            start_ms: 3000.0,
            end_ms: 6000.0,
            x: 1700.0,
            y: 100.0,
        };
        let (px, py) = path(&xs, &ys, None, std::slice::from_ref(&seg));
        let at = |ms: f64| (ms / STEP_MS) as usize;
        assert!((px[at(1000.0)] - 400.0).abs() < 1e-6);
        // Inside: where it was put, kept within the recording.
        let right = 1920.0 - W / 2.0;
        assert!((px[at(4500.0)] - right).abs() < 1e-6);
        assert!((py[at(4500.0)] - 540.0).abs() < 1e-6);
        // Half way through easing in, half way between.
        let mid = px[at(3000.0 - SEGMENT_EASE_MS / 2.0)];
        assert!((mid - (400.0 + right) / 2.0).abs() < 10.0, "{mid}");
        assert!((px[at(9000.0)] - 400.0).abs() < 1e-6);
        assert!(px.windows(2).all(|w| (w[1] - w[0]).abs() < 50.0));
    }

    #[test]
    fn while_the_mouse_rests_the_window_follows_the_screen() {
        // Mouse parked on the left; something keeps changing on the right.
        let n = grid_len(6000.0);
        let (xs, ys) = (vec![300.0; n], vec![540.0; n]);
        let (gw, gh) = (24, 12);
        let frames: Vec<Vec<u8>> = (0..61)
            .map(|i| {
                let mut f = vec![0u8; gw * gh];
                f[6 * gw + 20] = if i % 2 == 0 { 0 } else { 255 };
                f
            })
            .collect();
        let activity = analyze_frames(frames.iter().map(Vec::as_slice), gw, gh, 1920.0, 1080.0);
        let (px, _) = path(&xs, &ys, Some(&activity), &[]);
        assert!(px[0] < 400.0, "starts by the mouse: {}", px[0]);
        // Cell 20 of 24 is around x = 1640.
        let end = *px.last().unwrap();
        assert!(end > 1300.0 && inside(end), "{end}");
    }

    #[test]
    fn nothing_moves_when_the_window_is_the_whole_recording() {
        let (xs, ys) = jump(100.0, 1800.0, 1000.0, 3000.0);
        let (px, py) = rest_path(
            &Reframing {
                crop: CROP,
                width: 1920.0,
                height: 1080.0,
                follow: (&xs, &ys),
                activity: None,
                segments: &[],
            },
            3000.0,
        );
        assert!(px.iter().all(|&x| x == 960.0) && py.iter().all(|&y| y == 540.0));
    }
}
