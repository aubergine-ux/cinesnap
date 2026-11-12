//! The motion model: everything that decides *where things are* in each
//! output frame. Both the live preview and the exporter consume its output.

pub mod autozoom;
pub mod camera;
pub mod cursor;
pub mod timeline;

use serde::Serialize;

use crate::events::{EventKind, EventLog};
use crate::project::Project;

pub use autozoom::auto_zoom;
pub use timeline::Kept;

/// Per-output-frame motion, flattened for cheap transfer to the frontend.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MotionTracks {
    pub fps: u32,
    pub frames: usize,
    pub duration_ms: f64,
    /// Source time shown by each output frame.
    pub src_ms: Vec<f64>,
    /// `cx, cy, scale` per frame; centre of the visible rect in source pixels.
    pub cam: Vec<f32>,
    /// `x, y, alpha, press` per frame; position in source pixels.
    pub cursor: Vec<f32>,
    pub ripples: Vec<Ripple>,
    pub events: EventMarks,
    /// Whether the cursor should be drawn at all for this recording.
    pub draw_cursor: bool,
    /// `metadata`, `embedded` or `none`, from the event log header.
    pub cursor_mode: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Ripple {
    pub out_ms: f64,
    pub x: f64,
    pub y: f64,
}

/// Event times in source milliseconds, for timeline tick marks.
#[derive(Serialize, Clone, Debug, Default)]
pub struct EventMarks {
    pub clicks: Vec<f64>,
    pub keys: Vec<f64>,
}

impl MotionTracks {
    pub fn cam_at(&self, frame: usize) -> (f64, f64, f64) {
        let c = &self.cam[frame * 3..frame * 3 + 3];
        (c[0] as f64, c[1] as f64, c[2] as f64)
    }
    pub fn cursor_at(&self, frame: usize) -> (f64, f64, f64, f64) {
        let c = &self.cursor[frame * 4..frame * 4 + 4];
        (c[0] as f64, c[1] as f64, c[2] as f64, c[3] as f64)
    }
}

pub fn compute_motion(project: &Project, log: &EventLog) -> MotionTracks {
    let duration = project.source.video.duration_ms;
    let fps = crate::layout::output_fps(project);
    let kept = Kept::new(duration, &project.cuts);
    let out_duration = kept.duration_ms();
    let frames = (out_duration * fps as f64 / 1000.0).round() as usize;

    let cursor = cursor::cursor_path(
        log,
        duration,
        project.cursor.smoothing,
        project.cursor.hide_when_idle_ms,
    );
    let camera = camera::camera_path(&project.zooms, &cursor, project.crop, duration);

    let mut src_ms = Vec::with_capacity(frames);
    let mut cam = Vec::with_capacity(frames * 3);
    let mut cur = Vec::with_capacity(frames * 4);
    for i in 0..frames {
        let t = kept.out_to_src(i as f64 * 1000.0 / fps as f64);
        src_ms.push(t);
        let (cx, cy, s) = camera.sample(t);
        cam.extend([cx as f32, cy as f32, s as f32]);
        let (x, y, a, p) = cursor.sample(t);
        cur.extend([x as f32, y as f32, a as f32, p as f32]);
    }

    let mut ripples = Vec::new();
    let mut events = EventMarks::default();
    for e in &log.events {
        match e.kind {
            EventKind::Down { x, y, inferred, .. } => {
                events.clicks.push(e.t);
                if !inferred {
                    if let Some(out_ms) = kept.src_to_out(e.t) {
                        ripples.push(Ripple { out_ms, x, y });
                    }
                }
            }
            EventKind::Key => events.keys.push(e.t),
            _ => {}
        }
    }

    MotionTracks {
        fps,
        frames,
        duration_ms: out_duration,
        src_ms,
        cam,
        cursor: cur,
        ripples,
        events,
        draw_cursor: project.cursor.show && log.header.can_draw_cursor(),
        cursor_mode: log.header.cursor.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Button, Event, Header};
    use crate::project::Cut;
    use crate::testutil::project;

    #[test]
    fn tracks_follow_cuts() {
        let mut p = project(1920, 1080, 10_000.0);
        p.export.fps = 30;
        p.cuts = vec![
            Cut {
                start_ms: 0.0,
                end_ms: 2000.0,
            },
            Cut {
                start_ms: 5000.0,
                end_ms: 6000.0,
            },
        ];
        let log = EventLog {
            header: Header::new(1920, 1080, "exact", "metadata"),
            events: vec![
                Event {
                    t: 1000.0,
                    kind: EventKind::Down {
                        b: Button::Left,
                        x: 1.0,
                        y: 1.0,
                        inferred: false,
                    },
                },
                Event {
                    t: 7000.0,
                    kind: EventKind::Down {
                        b: Button::Left,
                        x: 500.0,
                        y: 500.0,
                        inferred: false,
                    },
                },
                Event {
                    t: 7100.0,
                    kind: EventKind::Key,
                },
            ],
        };
        p.zooms = auto_zoom(&p, &log);
        let m = compute_motion(&p, &log);
        assert_eq!(m.frames, 210);
        assert_eq!(m.duration_ms, 7000.0);
        assert_eq!(m.src_ms[0], 2000.0);
        assert!((m.src_ms[90] - 6000.0).abs() < 1e-6, "{}", m.src_ms[90]);
        assert_eq!(m.cam.len(), 630);
        assert_eq!(m.cursor.len(), 840);
        // The click inside the cut makes a tick mark but no ripple.
        assert_eq!(m.events.clicks.len(), 2);
        assert_eq!(m.ripples.len(), 1);
        assert_eq!(m.ripples[0].out_ms, 4000.0);
        // Zoomed in around the surviving click.
        let f = (4200.0 * 30.0 / 1000.0) as usize;
        assert!(m.cam_at(f).2 > 1.9);
        assert!(m.draw_cursor);
    }
}
