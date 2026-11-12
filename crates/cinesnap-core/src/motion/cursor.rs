//! Cursor path smoothing.
//!
//! The raw samples are resampled onto a fixed grid and low-pass filtered
//! forwards and backwards. Running the filter in both directions cancels its
//! phase delay, so the smoothed cursor never lags behind the real one, and near
//! clicks the result is pulled back to the raw position so the cursor is
//! exactly on the thing being clicked.

use crate::events::{EventKind, EventLog};

/// Grid step for all sampled motion, in milliseconds (120 Hz).
pub const STEP_MS: f64 = 1000.0 / 120.0;

#[derive(Clone, Debug)]
pub struct CursorPath {
    /// Position per grid step.
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    /// Opacity per grid step, 0..1.
    pub alpha: Vec<f64>,
    /// How "pressed" the button is per grid step, 0..1.
    pub press: Vec<f64>,
}

impl CursorPath {
    pub fn len(&self) -> usize {
        self.x.len()
    }
    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    /// Linearly interpolated `(x, y, alpha, press)` at a source time.
    pub fn sample(&self, t_ms: f64) -> (f64, f64, f64, f64) {
        (
            sample(&self.x, t_ms),
            sample(&self.y, t_ms),
            sample(&self.alpha, t_ms),
            sample(&self.press, t_ms),
        )
    }
}

/// Linear interpolation into a grid-sampled series.
pub fn sample(series: &[f64], t_ms: f64) -> f64 {
    if series.is_empty() {
        return 0.0;
    }
    let f = (t_ms / STEP_MS).clamp(0.0, (series.len() - 1) as f64);
    let i = f.floor() as usize;
    let j = (i + 1).min(series.len() - 1);
    let a = f - i as f64;
    series[i] * (1.0 - a) + series[j] * a
}

pub fn grid_len(duration_ms: f64) -> usize {
    (duration_ms.max(0.0) / STEP_MS).ceil() as usize + 1
}

/// Zero-phase one-pole low-pass with time constant `tau_ms`.
pub fn smooth_zero_phase(v: &mut [f64], tau_ms: f64) {
    if tau_ms <= 0.0 || v.len() < 2 {
        return;
    }
    let a = 1.0 - (-STEP_MS / tau_ms).exp();
    for i in 1..v.len() {
        v[i] = v[i - 1] + a * (v[i] - v[i - 1]);
    }
    for i in (0..v.len() - 1).rev() {
        v[i] = v[i + 1] + a * (v[i] - v[i + 1]);
    }
}

/// Build the drawn cursor path.
///
/// `smoothing` is 0 (raw) to 1 (very smooth). `hide_when_idle_ms` fades the
/// cursor out after that long without movement (0 disables it).
pub fn cursor_path(log: &EventLog, duration_ms: f64, smoothing: f64, hide_when_idle_ms: f64) -> CursorPath {
    let n = grid_len(duration_ms);
    let raw: Vec<(f64, f64, f64)> = log
        .events
        .iter()
        .filter_map(|e| e.position().map(|(x, y)| (e.t, x, y)))
        .collect();

    // Resample raw positions onto the grid (hold before the first and after
    // the last sample).
    let mut x = vec![0.0; n];
    let mut y = vec![0.0; n];
    let mut k = 0;
    for i in 0..n {
        let t = i as f64 * STEP_MS;
        while k + 1 < raw.len() && raw[k + 1].0 <= t {
            k += 1;
        }
        let (px, py) = match raw.get(k) {
            None => (log.header.width as f64 / 2.0, log.header.height as f64 / 2.0),
            Some(&(t0, x0, y0)) => match raw.get(k + 1) {
                Some(&(t1, x1, y1)) if t >= t0 && t1 > t0 => {
                    let a = (t - t0) / (t1 - t0);
                    (x0 + (x1 - x0) * a, y0 + (y1 - y0) * a)
                }
                _ => (x0, y0),
            },
        };
        x[i] = px;
        y[i] = py;
    }
    let (raw_x, raw_y) = (x.clone(), y.clone());

    // 0..1 maps to a 0..140 ms time constant, with a curve that gives the
    // lower half of the slider finer control.
    let tau = 140.0 * smoothing.clamp(0.0, 1.0).powf(1.5);
    smooth_zero_phase(&mut x, tau);
    smooth_zero_phase(&mut y, tau);

    // Pin the path to the raw position around each button press.
    let mut press = vec![0.0f64; n];
    let mut down_at: Option<f64> = None;
    let mut presses: Vec<(f64, f64)> = Vec::new();
    for e in &log.events {
        match e.kind {
            EventKind::Down { inferred, .. } => {
                if inferred {
                    presses.push((e.t, e.t + 90.0));
                } else {
                    down_at = Some(e.t);
                }
            }
            EventKind::Up { .. } => {
                if let Some(d) = down_at.take() {
                    presses.push((d, e.t.max(d + 60.0)));
                }
            }
            _ => {}
        }
    }
    if let Some(d) = down_at {
        presses.push((d, d + 90.0));
    }
    const PIN_MS: f64 = 90.0;
    for &(d, u) in &presses {
        let lo = (((d - 3.0 * PIN_MS) / STEP_MS).floor().max(0.0)) as usize;
        let hi = (((u + 3.0 * PIN_MS) / STEP_MS).ceil() as usize).min(n.saturating_sub(1));
        for i in lo..=hi.max(lo).min(n.saturating_sub(1)) {
            let t = i as f64 * STEP_MS;
            let dist = if t < d { d - t } else if t > u { t - u } else { 0.0 };
            let w = (-(dist / PIN_MS).powi(2)).exp();
            x[i] += (raw_x[i] - x[i]) * w;
            y[i] += (raw_y[i] - y[i]) * w;
            // Press envelope: quick in, slower out.
            let p = if t < d {
                1.0 - ((d - t) / 60.0).min(1.0)
            } else if t > u {
                1.0 - ((t - u) / 140.0).min(1.0)
            } else {
                1.0
            };
            press[i] = press[i].max(p);
        }
    }

    // Visibility: explicit events, plus optional idle fade.
    let mut alpha = vec![1.0f64; n];
    let mut visible = true;
    let mut ev = log.events.iter().peekable();
    let mut last_move = f64::NEG_INFINITY;
    let mut prev = (f64::INFINITY, f64::INFINITY);
    for i in 0..n {
        let t = i as f64 * STEP_MS;
        while let Some(e) = ev.peek() {
            if e.t > t {
                break;
            }
            if let EventKind::Visible { v } = e.kind {
                visible = v;
            }
            ev.next();
        }
        if (raw_x[i] - prev.0).abs() > 0.5 || (raw_y[i] - prev.1).abs() > 0.5 || press[i] > 0.0 {
            last_move = t;
            prev = (raw_x[i], raw_y[i]);
        }
        let idle = hide_when_idle_ms > 0.0 && t - last_move > hide_when_idle_ms;
        alpha[i] = if visible && !idle { 1.0 } else { 0.0 };
    }
    // Soften visibility changes into ~120 ms fades.
    smooth_zero_phase(&mut alpha, 40.0);

    CursorPath { x, y, alpha, press }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Button, Event, Header};

    fn log(events: Vec<Event>) -> EventLog {
        EventLog { header: Header::new(1000, 1000, "exact", "metadata"), events }
    }
    fn mv(t: f64, x: f64, y: f64) -> Event {
        Event { t, kind: EventKind::Move { x, y } }
    }

    #[test]
    fn smoothing_reduces_jitter_without_lag() {
        // A straight line with alternating ±6 px jitter.
        let events: Vec<Event> = (0..300)
            .map(|i| {
                let t = i as f64 * 8.0;
                mv(t, t / 4.0, 500.0 + if i % 2 == 0 { 6.0 } else { -6.0 })
            })
            .collect();
        let l = log(events);
        let raw = cursor_path(&l, 2400.0, 0.0, 0.0);
        let smooth = cursor_path(&l, 2400.0, 0.8, 0.0);
        let rough = |p: &CursorPath| -> f64 {
            p.y.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / p.len() as f64
        };
        assert!(rough(&smooth) < rough(&raw) * 0.2, "{} vs {}", rough(&smooth), rough(&raw));
        // No lag: mid-way along the line the smoothed x matches the true x.
        let (x, y, ..) = smooth.sample(1200.0);
        assert!((x - 300.0).abs() < 3.0, "x = {x}");
        assert!((y - 500.0).abs() < 2.0, "y = {y}");
    }

    #[test]
    fn cursor_is_exact_at_click() {
        let mut events = vec![mv(0.0, 0.0, 0.0), mv(400.0, 800.0, 600.0)];
        events.push(Event { t: 500.0, kind: EventKind::Down { b: Button::Left, x: 800.0, y: 600.0, inferred: false } });
        events.push(Event { t: 580.0, kind: EventKind::Up { b: Button::Left, x: 800.0, y: 600.0 } });
        events.push(mv(700.0, 100.0, 100.0));
        let p = cursor_path(&log(events), 1500.0, 1.0, 0.0);
        let (x, y, _, press) = p.sample(540.0);
        assert!((x - 800.0).abs() < 1.0 && (y - 600.0).abs() < 1.0, "({x}, {y})");
        assert!(press > 0.99);
        assert_eq!(p.sample(1400.0).3, 0.0);
    }

    #[test]
    fn idle_and_visibility_fade() {
        let events = vec![
            mv(0.0, 10.0, 10.0),
            mv(100.0, 50.0, 50.0),
            Event { t: 3000.0, kind: EventKind::Visible { v: false } },
        ];
        let l = log(events);
        let p = cursor_path(&l, 4000.0, 0.5, 1000.0);
        assert!(p.sample(500.0).2 > 0.95);
        assert!(p.sample(2000.0).2 < 0.05, "idle fade");
        let p = cursor_path(&l, 4000.0, 0.5, 0.0);
        assert!(p.sample(2000.0).2 > 0.95);
        assert!(p.sample(3800.0).2 < 0.05, "left the capture area");
    }

    #[test]
    fn empty_log_is_centered() {
        let p = cursor_path(&log(vec![]), 100.0, 0.5, 0.0);
        assert_eq!(p.sample(50.0).0, 500.0);
    }
}
