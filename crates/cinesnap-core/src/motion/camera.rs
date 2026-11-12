//! Turn zoom segments into a camera path.

use crate::motion::cursor::{grid_len, sample, smooth_zero_phase, CursorPath, STEP_MS};
use crate::project::{Easing, Rect, Zoom, ZoomMode};

/// How much of the zoomed view the cursor may roam before the camera follows
/// (half extent, as a fraction of the view size).
const DEAD_ZONE: f64 = 0.22;
/// Natural frequency of the follow spring, rad/s. Critically damped.
const FOLLOW_OMEGA: f64 = 5.0;

#[derive(Clone, Debug)]
pub struct CameraPath {
    /// Centre of the visible rectangle in source pixels, per grid step.
    pub cx: Vec<f64>,
    pub cy: Vec<f64>,
    /// Magnification, >= 1.
    pub scale: Vec<f64>,
}

impl CameraPath {
    pub fn sample(&self, t_ms: f64) -> (f64, f64, f64) {
        (sample(&self.cx, t_ms), sample(&self.cy, t_ms), sample(&self.scale, t_ms))
    }
}

pub fn ease(kind: Easing, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match kind {
        Easing::Linear => t,
        Easing::Smooth => t * t * t * (t * (6.0 * t - 15.0) + 10.0),
        Easing::Snappy => 1.0 - (1.0 - t).powi(4),
    }
}

/// Zoom progress of a segment at time `t`, 0..1.
fn progress(z: &Zoom, t: f64) -> f64 {
    if t <= z.start_ms || t >= z.end_ms {
        return 0.0;
    }
    let len = z.end_ms - z.start_ms;
    // If the ramps do not fit, shrink both proportionally.
    let fit = (len / (z.ease_in_ms + z.ease_out_ms).max(1e-9)).min(1.0);
    let (ein, eout) = (z.ease_in_ms * fit, z.ease_out_ms * fit);
    let a = if ein > 0.0 { (t - z.start_ms) / ein } else { 1.0 };
    let b = if eout > 0.0 { (z.end_ms - t) / eout } else { 1.0 };
    ease(z.easing, a.min(b))
}

pub fn camera_path(zooms: &[Zoom], cursor: &CursorPath, crop: Rect, duration_ms: f64) -> CameraPath {
    let n = grid_len(duration_ms);
    let (ccx, ccy) = crop.center();
    let mut cx = vec![ccx; n];
    let mut cy = vec![ccy; n];
    let mut scale = vec![1.0; n];

    // The camera follows a calmer version of the cursor than the one drawn.
    let mut fx = cursor.x.clone();
    let mut fy = cursor.y.clone();
    smooth_zero_phase(&mut fx, 220.0);
    smooth_zero_phase(&mut fy, 220.0);

    let dt = STEP_MS / 1000.0;
    for z in zooms {
        if z.scale <= 1.0 || z.end_ms <= z.start_ms {
            continue;
        }
        let i0 = (z.start_ms / STEP_MS).floor().max(0.0) as usize;
        let i1 = ((z.end_ms / STEP_MS).ceil() as usize).min(n - 1);
        let (view_w, view_h) = (crop.width / z.scale, crop.height / z.scale);
        let (dzx, dzy) = (view_w * DEAD_ZONE, view_h * DEAD_ZONE);

        let mut target = (z.x, z.y);
        let mut pos = target;
        let mut vel = (0.0, 0.0);
        for i in i0..=i1 {
            let t = i as f64 * STEP_MS;
            if z.mode == ZoomMode::Follow && i < fx.len() {
                // Push the target only when the cursor leaves the dead zone.
                let (dx, dy) = (fx[i] - target.0, fy[i] - target.1);
                if dx.abs() > dzx {
                    target.0 += dx - dzx * dx.signum();
                }
                if dy.abs() > dzy {
                    target.1 += dy - dzy * dy.signum();
                }
                // Critically damped spring towards the target.
                let w = FOLLOW_OMEGA;
                vel.0 += (w * w * (target.0 - pos.0) - 2.0 * w * vel.0) * dt;
                vel.1 += (w * w * (target.1 - pos.1) - 2.0 * w * vel.1) * dt;
                pos.0 += vel.0 * dt;
                pos.1 += vel.1 * dt;
            }

            let e = progress(z, t);
            let s = (z.scale.ln() * e).exp();
            if s <= scale[i] {
                continue; // an overlapping segment already zooms further here
            }
            // Blend from the crop centre to the focus so that the motion looks
            // like zooming *towards* the focus and ends centred on it.
            let w = (1.0 - 1.0 / s) / (1.0 - 1.0 / z.scale);
            let (mut x, mut y) = (ccx + (pos.0 - ccx) * w, ccy + (pos.1 - ccy) * w);
            // Never show anything outside the recording.
            let (hw, hh) = (crop.width / s / 2.0, crop.height / s / 2.0);
            x = x.clamp(crop.x + hw, crop.x + crop.width - hw);
            y = y.clamp(crop.y + hh, crop.y + crop.height - hh);
            cx[i] = x;
            cy[i] = y;
            scale[i] = s;
        }
    }
    CameraPath { cx, cy, scale }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Event, EventKind, EventLog, Header};
    use crate::motion::cursor::cursor_path;

    const CROP: Rect = Rect { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0 };

    fn zoom(start: f64, end: f64, x: f64, y: f64, mode: ZoomMode) -> Zoom {
        Zoom {
            id: "z".into(), start_ms: start, end_ms: end, scale: 2.0, ease_in_ms: 600.0, ease_out_ms: 700.0,
            easing: Easing::Smooth, mode, x, y, manual: false,
        }
    }
    fn still_cursor(x: f64, y: f64, dur: f64) -> CursorPath {
        let log = EventLog {
            header: Header::new(1920, 1080, "exact", "metadata"),
            events: vec![Event { t: 0.0, kind: EventKind::Move { x, y } }],
        };
        cursor_path(&log, dur, 0.5, 0.0)
    }

    #[test]
    fn easing_endpoints_and_monotonic() {
        for k in [Easing::Linear, Easing::Smooth, Easing::Snappy] {
            assert_eq!(ease(k, 0.0), 0.0);
            assert!((ease(k, 1.0) - 1.0).abs() < 1e-12);
            let mut prev = 0.0;
            for i in 1..=100 {
                let v = ease(k, i as f64 / 100.0);
                assert!(v >= prev);
                prev = v;
            }
        }
    }

    #[test]
    fn rests_at_full_view_outside_segments() {
        let cur = still_cursor(600.0, 400.0, 10_000.0);
        let cam = camera_path(&[zoom(2000.0, 6000.0, 600.0, 400.0, ZoomMode::Fixed)], &cur, CROP, 10_000.0);
        assert_eq!(cam.sample(1000.0), (960.0, 540.0, 1.0));
        assert_eq!(cam.sample(8000.0), (960.0, 540.0, 1.0));
        let (x, y, s) = cam.sample(4000.0);
        assert!((s - 2.0).abs() < 1e-9 && (x - 600.0).abs() < 1e-6 && (y - 400.0).abs() < 1e-6);
        // Half-way through the ramp the scale is between the two.
        let (_, _, s) = cam.sample(2300.0);
        assert!(s > 1.0 && s < 2.0);
    }

    #[test]
    fn view_never_leaves_the_recording() {
        let cur = still_cursor(5.0, 1075.0, 10_000.0);
        let crop = Rect { x: 100.0, y: 50.0, width: 800.0, height: 600.0 };
        let cam = camera_path(&[zoom(500.0, 9000.0, 5.0, 1075.0, ZoomMode::Follow)], &cur, crop, 10_000.0);
        for i in 0..cam.cx.len() {
            let (hw, hh) = (crop.width / cam.scale[i] / 2.0, crop.height / cam.scale[i] / 2.0);
            assert!(cam.cx[i] - hw >= crop.x - 1e-6 && cam.cx[i] + hw <= crop.x + crop.width + 1e-6);
            assert!(cam.cy[i] - hh >= crop.y - 1e-6 && cam.cy[i] + hh <= crop.y + crop.height + 1e-6);
        }
    }

    #[test]
    fn follows_cursor_but_ignores_small_movements() {
        let mk = |x1: f64| {
            let log = EventLog {
                header: Header::new(1920, 1080, "exact", "metadata"),
                events: vec![
                    Event { t: 0.0, kind: EventKind::Move { x: 700.0, y: 540.0 } },
                    Event { t: 3000.0, kind: EventKind::Move { x: 700.0, y: 540.0 } },
                    Event { t: 3400.0, kind: EventKind::Move { x: x1, y: 540.0 } },
                ],
            };
            let cur = cursor_path(&log, 10_000.0, 0.5, 0.0);
            camera_path(&[zoom(1000.0, 9000.0, 700.0, 540.0, ZoomMode::Follow)], &cur, CROP, 10_000.0)
        };
        // 100 px is inside the dead zone (0.22 * 960 = 211 px): camera stays.
        assert!((mk(800.0).sample(7000.0).0 - 700.0).abs() < 1.0);
        // A long move drags the camera so the cursor sits at the dead-zone edge.
        let x = mk(1400.0).sample(7000.0).0;
        assert!((x - (1400.0 - 211.2)).abs() < 3.0, "x = {x}");
    }

    #[test]
    fn camera_motion_is_continuous() {
        let cur = still_cursor(300.0, 300.0, 6000.0);
        let cam = camera_path(&[zoom(500.0, 1200.0, 300.0, 300.0, ZoomMode::Follow)], &cur, CROP, 6000.0);
        for w in cam.scale.windows(2) {
            assert!((w[1] - w[0]).abs() < 0.05);
        }
        for w in cam.cx.windows(2) {
            assert!((w[1] - w[0]).abs() < 25.0);
        }
        assert!(cam.scale.iter().cloned().fold(0.0, f64::max) > 1.2, "short segment still zooms");
    }
}
