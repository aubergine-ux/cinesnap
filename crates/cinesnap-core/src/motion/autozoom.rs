//! Turn the event log into zoom segments.

use crate::events::{EventKind, EventLog};
use crate::motion::timeline::Kept;
use crate::project::{default_ease_in, default_ease_out, Easing, Project, Zoom, ZoomMode};

/// Keys closer together than this belong to one typing burst.
const TYPING_GAP_MS: f64 = 1200.0;
/// A burst needs at least this many keys to count as typing.
const TYPING_MIN_KEYS: usize = 3;
/// How long the zoom holds after the last interest point before easing out.
const HOLD_AFTER_MS: f64 = 900.0;
/// Fraction of the ease-in that happens before the first interest point.
const LEAD_IN: f64 = 0.75;
/// Segments whose gap is smaller than this are joined rather than bouncing
/// out and straight back in.
const MIN_GAP_MS: f64 = 300.0;

#[derive(Clone, Copy, Debug)]
struct Point {
    /// Start of the activity (a click is instantaneous, typing has a span).
    t: f64,
    end: f64,
    x: f64,
    y: f64,
    inferred: bool,
}

fn interest_points(project: &Project, log: &EventLog) -> Vec<Point> {
    let kept = Kept::new(project.source.video.duration_ms, &project.cuts);
    let crop = project.crop;
    let mut points = Vec::new();
    let mut last_pos = crop.center();
    let mut burst: Vec<f64> = Vec::new();
    let mut burst_pos = last_pos;

    let flush = |burst: &mut Vec<f64>, pos: (f64, f64), points: &mut Vec<Point>| {
        if burst.len() >= TYPING_MIN_KEYS {
            points.push(Point { t: burst[0], end: *burst.last().unwrap(), x: pos.0, y: pos.1, inferred: false });
        }
        burst.clear();
    };

    for e in &log.events {
        if let Some(p) = e.position() {
            last_pos = p;
        }
        match e.kind {
            EventKind::Down { x, y, inferred, .. } => {
                flush(&mut burst, burst_pos, &mut points);
                points.push(Point { t: e.t, end: e.t, x, y, inferred });
            }
            EventKind::Key if project.auto_zoom.on_typing => {
                if burst.last().is_some_and(|&t| e.t - t > TYPING_GAP_MS) {
                    flush(&mut burst, burst_pos, &mut points);
                }
                if burst.is_empty() {
                    burst_pos = last_pos;
                }
                burst.push(e.t);
            }
            _ => {}
        }
    }
    flush(&mut burst, burst_pos, &mut points);

    points.retain(|p| p.t >= 0.0 && kept.contains(p.t) && crop.contains(p.x, p.y));
    points.sort_by(|a, b| a.t.total_cmp(&b.t));
    points
}

/// Regenerate automatic zoom segments. Manual segments are kept untouched and
/// automatic ones never overlap them. Returns the full, sorted segment list.
pub fn auto_zoom(project: &Project, log: &EventLog) -> Vec<Zoom> {
    let mut result: Vec<Zoom> = project.zooms.iter().filter(|z| z.manual).cloned().collect();
    if !project.auto_zoom.enabled {
        return result;
    }
    let duration = project.source.video.duration_ms;
    let (ease_in, ease_out) = (default_ease_in(), default_ease_out());
    let merge_gap = project.auto_zoom.merge_gap_ms.max(0.0);

    // Group points that are close in time.
    let last_end = |g: &[Point]| g.iter().map(|p| p.end).fold(f64::MIN, f64::max);
    let mut groups: Vec<Vec<Point>> = Vec::new();
    for p in interest_points(project, log) {
        match groups.last_mut() {
            Some(g) if p.t - last_end(g) < merge_gap => g.push(p),
            _ => groups.push(vec![p]),
        }
    }
    // A lone inferred click is too weak a signal to move the camera for.
    groups.retain(|g| g.len() >= 2 || !g[0].inferred);

    let mut segments: Vec<(f64, f64, Point)> = Vec::new();
    for g in groups {
        let start = (g[0].t - ease_in * LEAD_IN).max(0.0);
        let end = (last_end(&g) + HOLD_AFTER_MS + ease_out).min(duration);
        match segments.last_mut() {
            Some(prev) if start - prev.1 < MIN_GAP_MS => prev.1 = end.max(prev.1),
            _ => segments.push((start, end, g[0])),
        }
    }

    let manual = result.clone();
    let mut n = 0;
    for (start, end, first) in segments {
        if end - start < ease_in + ease_out {
            continue; // squeezed against the end of the recording
        }
        if manual.iter().any(|m| start < m.end_ms && end > m.start_ms) {
            continue;
        }
        n += 1;
        result.push(Zoom {
            id: format!("auto-{n}"),
            start_ms: start,
            end_ms: end,
            scale: project.auto_zoom.scale.clamp(1.0, 8.0),
            ease_in_ms: ease_in,
            ease_out_ms: ease_out,
            easing: Easing::Smooth,
            mode: ZoomMode::Follow,
            x: first.x,
            y: first.y,
            manual: false,
        });
    }
    result.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Button, Event, Header};
    use crate::project::Cut;
    use crate::testutil::project;

    fn click(t: f64, x: f64, y: f64) -> Event {
        Event { t, kind: EventKind::Down { b: Button::Left, x, y, inferred: false } }
    }
    fn key(t: f64) -> Event {
        Event { t, kind: EventKind::Key }
    }
    fn log(events: Vec<Event>) -> EventLog {
        EventLog { header: Header::new(1920, 1080, "exact", "metadata"), events }
    }

    #[test]
    fn nearby_clicks_merge_distant_ones_do_not() {
        let p = project(1920, 1080, 30_000.0);
        let l = log(vec![click(2000.0, 100.0, 100.0), click(3500.0, 300.0, 200.0), click(12_000.0, 900.0, 500.0)]);
        let z = auto_zoom(&p, &l);
        assert_eq!(z.len(), 2);
        assert_eq!(z[0].start_ms, 2000.0 - 450.0);
        assert_eq!(z[0].end_ms, 3500.0 + 900.0 + 700.0);
        assert_eq!((z[0].x, z[0].y), (100.0, 100.0));
        assert_eq!(z[1].id, "auto-2");
        assert!(z.iter().all(|z| !z.manual && z.scale == 2.0));
    }

    #[test]
    fn typing_burst_extends_the_zoom() {
        let p = project(1920, 1080, 30_000.0);
        let mut ev = vec![click(2000.0, 400.0, 400.0)];
        ev.extend((0..20).map(|i| key(2500.0 + i as f64 * 300.0)));
        let z = auto_zoom(&p, &log(ev.clone()));
        assert_eq!(z.len(), 1);
        assert_eq!(z[0].end_ms, 2500.0 + 19.0 * 300.0 + 1600.0);

        let mut no_typing = p.clone();
        no_typing.auto_zoom.on_typing = false;
        let z = auto_zoom(&no_typing, &log(ev));
        assert_eq!(z[0].end_ms, 2000.0 + 1600.0);
    }

    #[test]
    fn a_couple_of_stray_keys_are_not_typing() {
        let p = project(1920, 1080, 30_000.0);
        assert!(auto_zoom(&p, &log(vec![key(5000.0), key(5100.0)])).is_empty());
    }

    #[test]
    fn manual_segments_survive_and_block_overlaps() {
        let mut p = project(1920, 1080, 30_000.0);
        p.zooms.push(Zoom {
            id: "mine".into(), start_ms: 1000.0, end_ms: 5000.0, scale: 3.0,
            ease_in_ms: 100.0, ease_out_ms: 100.0, easing: Easing::Linear, mode: ZoomMode::Fixed,
            x: 5.0, y: 5.0, manual: true,
        });
        p.zooms.push(Zoom { id: "auto-9".into(), manual: false, start_ms: 20_000.0, end_ms: 22_000.0, ..p.zooms[0].clone() });
        let z = auto_zoom(&p, &log(vec![click(2000.0, 100.0, 100.0), click(12_000.0, 900.0, 500.0)]));
        assert_eq!(z.iter().map(|z| z.id.as_str()).collect::<Vec<_>>(), ["mine", "auto-1"]);
        assert_eq!(z[1].start_ms, 11_550.0);

        p.auto_zoom.enabled = false;
        assert_eq!(auto_zoom(&p, &log(vec![click(12_000.0, 1.0, 1.0)])).len(), 1);
    }

    #[test]
    fn ignores_cut_out_of_crop_and_lone_inferred_clicks() {
        let mut p = project(1920, 1080, 30_000.0);
        p.cuts.push(Cut { start_ms: 0.0, end_ms: 4000.0 });
        p.crop.width = 960.0;
        let lone = Event { t: 20_000.0, kind: EventKind::Down { b: Button::Left, x: 10.0, y: 10.0, inferred: true } };
        let z = auto_zoom(&p, &log(vec![click(2000.0, 100.0, 100.0), click(10_000.0, 1500.0, 100.0), lone]));
        assert!(z.is_empty(), "{z:?}");
    }

    #[test]
    fn segments_never_overlap() {
        let mut p = project(1920, 1080, 60_000.0);
        p.auto_zoom.merge_gap_ms = 500.0;
        let ev: Vec<Event> = (0..30).map(|i| click(1000.0 + i as f64 * 1700.0, 500.0, 500.0)).collect();
        let z = auto_zoom(&p, &log(ev));
        for w in z.windows(2) {
            assert!(w[0].end_ms + MIN_GAP_MS <= w[1].start_ms, "{w:?}");
        }
    }
}
