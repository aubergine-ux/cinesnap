//! The motion model: everything that decides *where things are* in each
//! output frame. Both the live preview and the exporter consume its output.

pub mod autozoom;
pub mod camera;
pub mod cursor;
pub mod reframe;
pub mod timeline;

use std::path::Path;

use serde::Serialize;

use crate::activity::Activity;
use crate::events::{EventKind, EventLog};
use crate::project::{ClipBody, Project, TransitionKind, Zoom};
use crate::transcript::{caption_lines, CaptionLine};

pub use timeline::Kept;

/// Per-output-frame motion, flattened for cheap transfer to the frontend.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MotionTracks {
    pub fps: u32,
    pub frames: usize,
    pub duration_ms: f64,
    /// Timeline time shown by each output frame.
    pub src_ms: Vec<f64>,
    /// Index of the clip each output frame belongs to.
    pub clip: Vec<u16>,
    /// Timeline time at which every clip starts.
    pub clip_starts: Vec<f64>,
    /// `cx, cy, scale` per frame; centre of the visible rect in source pixels.
    pub cam: Vec<f32>,
    /// `x, y, alpha, press` per frame; position in source pixels.
    pub cursor: Vec<f32>,
    /// `x, y, size, alpha` per frame: centre of the webcam bubble as
    /// fractions of the output, side as a fraction of its shorter side.
    pub webcam: Vec<f32>,
    pub ripples: Vec<Ripple>,
    pub events: EventMarks,
    /// Shortcuts to show, by output time.
    pub keys: Vec<KeyMark>,
    /// Caption lines, in output time.
    pub captions: Vec<CaptionLine>,
    pub transitions: Vec<TransitionMark>,
    /// Per clip: whether the cursor should be redrawn.
    pub cursor_drawn: Vec<bool>,
    /// `metadata`, `embedded` or `none`, from the first recording's log.
    pub cursor_mode: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Ripple {
    pub out_ms: f64,
    pub x: f64,
    pub y: f64,
    pub clip: u16,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeyMark {
    pub out_ms: f64,
    pub combo: String,
}

/// A blend between two clips, centred on the first frame of the incoming one.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TransitionMark {
    /// First output frame of the incoming clip.
    pub frame: usize,
    /// Frames of blending on each side of it.
    pub half: usize,
    pub kind: TransitionKind,
}

impl TransitionMark {
    /// Progress 0..1 through the blend at an output frame, if it is inside.
    pub fn progress(&self, frame: usize) -> Option<f64> {
        let start = self.frame - self.half;
        (frame >= start && frame < self.frame + self.half)
            .then(|| (frame - start) as f64 + 0.5)
            .map(|f| f / (2 * self.half) as f64)
    }
}

/// Event times in timeline milliseconds, for timeline tick marks.
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
    pub fn webcam_at(&self, frame: usize) -> (f64, f64, f64, f64) {
        let c = &self.webcam[frame * 4..frame * 4 + 4];
        (c[0] as f64, c[1] as f64, c[2] as f64, c[3] as f64)
    }
    /// The transition an output frame is part of, with its progress.
    pub fn transition_at(&self, frame: usize) -> Option<(TransitionMark, f64)> {
        self.transitions
            .iter()
            .find_map(|t| t.progress(frame).map(|p| (*t, p)))
    }
}

/// Camera states and cursor positions across a shutter interval.
pub type BlurSamples = (Vec<(f64, f64, f64)>, Vec<(f64, f64)>);

/// Output pixels of movement per extra motion blur sample.
pub const BLUR_PX_PER_SAMPLE: f64 = 2.5;
pub const BLUR_MAX_SAMPLES: usize = 12;

/// Samples across the shutter of output frame `i`: camera states and
/// cursor positions, each empty when it moves too little to blur.
/// `frame_w` is the frame width in output pixels and `view_w` the source
/// width shown at zoom 1.
pub fn blur_samples(
    tracks: &MotionTracks,
    i: usize,
    strength: f64,
    frame_w: f64,
    view_w: f64,
) -> BlurSamples {
    if strength <= 0.0 || tracks.frames < 2 || i >= tracks.frames {
        return (vec![], vec![]);
    }
    let clip = tracks.clip[i];
    // The frames of the same clip around `i`.
    let (mut lo, mut hi) = (i, i);
    while lo > 0 && tracks.clip[lo - 1] == clip && i - lo < 2 {
        lo -= 1;
    }
    while hi + 1 < tracks.frames && tracks.clip[hi + 1] == clip && hi - i < 2 {
        hi += 1;
    }
    let at = |f: f64, get: &dyn Fn(usize) -> Vec<f64>| -> Vec<f64> {
        let f = f.clamp(lo as f64, hi as f64);
        let (a, b) = (f.floor() as usize, (f.ceil() as usize).min(hi));
        let t = f - a as f64;
        get(a)
            .iter()
            .zip(get(b))
            .map(|(x, y)| x + (y - x) * t)
            .collect()
    };
    let cam = |j: usize| {
        let (x, y, s) = tracks.cam_at(j);
        vec![x, y, s]
    };
    let cur = |j: usize| {
        let (x, y, _, _) = tracks.cursor_at(j);
        vec![x, y]
    };
    let (f0, f1) = (i as f64 - strength / 2.0, i as f64 + strength / 2.0);
    let samples = |n: usize, get: &dyn Fn(usize) -> Vec<f64>| -> Vec<Vec<f64>> {
        (0..n)
            .map(|k| at(f0 + (f1 - f0) * (k as f64 + 0.5) / n as f64, get))
            .collect()
    };
    let count = |d: f64| ((d / BLUR_PX_PER_SAMPLE).ceil() as usize).min(BLUR_MAX_SAMPLES);

    let (a, b) = (at(f0, &cam), at(f1, &cam));
    let px_per_src = |s: f64| frame_w * s.max(1.0) / view_w;
    let k = px_per_src(a[2].max(b[2]));
    let pan = (a[0] - b[0]).abs().max((a[1] - b[1]).abs()) * k;
    let zoom = (a[2] - b[2]).abs() / a[2].max(b[2]) * frame_w / 2.0;
    let n = count(pan.max(zoom));
    let cams = if n >= 2 {
        samples(n, &cam)
            .into_iter()
            .map(|v| (v[0], v[1], v[2]))
            .collect()
    } else {
        vec![]
    };

    let (a, b) = (at(f0, &cur), at(f1, &cur));
    let n = count((a[0] - b[0]).hypot(a[1] - b[1]) * k / 1.6);
    let trail = if n >= 2 {
        samples(n, &cur).into_iter().map(|v| (v[0], v[1])).collect()
    } else {
        vec![]
    };
    (cams, trail)
}

/// The event log of every clip, in clip order (empty for cards and for
/// recordings whose log cannot be read).
pub fn load_logs(dir: &Path, project: &Project) -> Vec<EventLog> {
    project
        .clips
        .iter()
        .map(|clip| match &clip.body {
            ClipBody::Recording(r) => {
                EventLog::load(&Project::clip_dir(dir, r).join(&r.source.events.file))
                    .unwrap_or_else(|_| {
                        EventLog::empty(r.source.video.width, r.source.video.height)
                    })
            }
            ClipBody::Card(_) => EventLog::empty(2, 2),
        })
        .collect()
}

/// Regenerate automatic zooms for every recording on the timeline. Manual
/// segments are kept. Returns the full, sorted list in timeline time.
/// `activity` (per clip, may be shorter) places typing zooms on the text.
pub fn auto_zoom(project: &Project, logs: &[EventLog], activity: &[Option<Activity>]) -> Vec<Zoom> {
    let mut out = Vec::new();
    for span in project.spans() {
        let (Some(local), Some(log)) = (project.local(span), logs.get(span.index)) else {
            continue;
        };
        let id = &project.clips[span.index].id;
        let act = activity
            .get(span.index)
            .and_then(Option::as_ref)
            .filter(|_| project.auto_zoom.follow_text);
        for mut z in autozoom::auto_zoom(&local, log, act) {
            z.start_ms += span.start_ms;
            z.end_ms += span.start_ms;
            if !z.manual && id != "main" {
                z.id = format!("{id}-{}", z.id);
            }
            out.push(z);
        }
    }
    out.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
    out
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How long the webcam bubble takes to move to a new keyframe.
pub const WEBCAM_MOVE_MS: f64 = 450.0;

/// `(x, y, size, alpha)` of the webcam bubble at a timeline time.
pub fn webcam_state(project: &Project, ratio: f64, t: f64) -> (f64, f64, f64, f64) {
    let w = &project.webcam;
    let (bx, by) = w.center(ratio);
    let base = (bx, by, w.size, 1.0);
    let state = |i: usize| {
        let k = &project.webcam_keys[i];
        (k.x, k.y, k.size, if k.show { 1.0 } else { 0.0 })
    };
    let n = project.webcam_keys.partition_point(|k| k.at_ms <= t);
    if n == 0 {
        return base;
    }
    let to = state(n - 1);
    let from = if n >= 2 { state(n - 2) } else { base };
    let p = smoothstep((t - project.webcam_keys[n - 1].at_ms) / WEBCAM_MOVE_MS);
    let mix = |a: f64, b: f64| a + (b - a) * p;
    // A hidden bubble fades where it stands instead of flying off.
    let (fx, fy, fs) = if from.3 == 0.0 {
        (to.0, to.1, to.2)
    } else {
        (from.0, from.1, from.2)
    };
    let (tx, ty, ts) = if to.3 == 0.0 {
        (fx, fy, fs)
    } else {
        (to.0, to.1, to.2)
    };
    (mix(fx, tx), mix(fy, ty), mix(fs, ts), mix(from.3, to.3))
}

/// What the camera follows: the cursor, except while typing, when it is
/// wherever the typed text appears.
pub fn follow_series(
    cursor: &cursor::CursorPath,
    log: &EventLog,
    activity: Option<&Activity>,
    duration_ms: f64,
) -> camera::Follow {
    let mut f = camera::Follow::cursor(cursor);
    let Some(activity) = activity else {
        return f;
    };
    let n = f.x.len();
    let mut changed = false;
    for burst in autozoom::typing_bursts(log) {
        for (k, &t) in burst.iter().enumerate() {
            let Some((x, y)) =
                activity.local_change(t, t + autozoom::ECHO_MS, autozoom::TYPED_SHARE)
            else {
                continue;
            };
            let until = burst
                .get(k + 1)
                .copied()
                .unwrap_or(t + 1200.0)
                .min(duration_ms);
            let i0 = (t / cursor::STEP_MS).max(0.0) as usize;
            let i1 = ((until / cursor::STEP_MS) as usize).min(n.saturating_sub(1));
            for i in i0..=i1.max(i0).min(n.saturating_sub(1)) {
                f.x[i] = x;
                f.y[i] = y;
                f.keep_x[i] = x;
                f.keep_y[i] = y;
                changed = true;
            }
        }
    }
    if changed {
        cursor::smooth_zero_phase(&mut f.x, 220.0);
        cursor::smooth_zero_phase(&mut f.y, 220.0);
    }
    f
}

pub fn compute_motion(
    project: &Project,
    logs: &[EventLog],
    activity: &[Option<Activity>],
) -> MotionTracks {
    let duration = project.duration_ms();
    let fps = crate::layout::output_fps(project);
    let kept = Kept::new(duration, &project.cuts);
    let out_duration = kept.duration_ms();
    let frames = (out_duration * fps as f64 / 1000.0).round() as usize;
    let spans = project.spans();
    let ratio = crate::layout::output_ratio(project);

    // Cursor and camera paths per recording, in the clip's own time.
    let empty = EventLog::empty(2, 2);
    let paths: Vec<_> = spans
        .iter()
        .map(|&span| {
            let local = project.local(span)?;
            let log = logs.get(span.index).unwrap_or(&empty);
            let cursor = cursor::cursor_path(
                log,
                span.duration_ms,
                project.cursor.smoothing,
                project.cursor.hide_when_idle_ms,
            );
            let act = activity.get(span.index).and_then(Option::as_ref);
            let text = act.filter(|_| project.auto_zoom.follow_text);
            let follow = follow_series(&cursor, log, text, span.duration_ms);
            let layout = crate::layout::Layout::for_crop(project, local.crop);
            let reframed =
                layout.view_w < local.crop.width - 0.5 || layout.view_h < local.crop.height - 0.5;
            let (rx, ry) = if reframed {
                reframe::rest_path(
                    &reframe::Reframing {
                        crop: local.crop,
                        width: layout.view_w,
                        height: layout.view_h,
                        follow: (&follow.x, &follow.y),
                        activity: act,
                        segments: &local.reframe.segments,
                    },
                    span.duration_ms,
                )
            } else {
                (vec![], vec![])
            };
            let view = camera::View {
                crop: local.crop,
                width: layout.view_w,
                height: layout.view_h,
                rest_x: &rx,
                rest_y: &ry,
            };
            let camera = camera::camera_path(&local.zooms, &follow, &view, span.duration_ms);
            Some((cursor, camera))
        })
        .collect();

    let mut src_ms = Vec::with_capacity(frames);
    let mut clip = Vec::with_capacity(frames);
    let mut cam = Vec::with_capacity(frames * 3);
    let mut cur = Vec::with_capacity(frames * 4);
    let mut webcam = Vec::with_capacity(frames * 4);
    let mut at = 0;
    for i in 0..frames {
        let t = kept.out_to_src(i as f64 * 1000.0 / fps as f64);
        while at + 1 < spans.len() && t >= spans[at].end_ms() {
            at += 1;
        }
        src_ms.push(t);
        clip.push(at as u16);
        let local = t - spans.get(at).map_or(0.0, |s| s.start_ms);
        let has_webcam = match (paths.get(at), project.clips.get(at)) {
            (Some(Some((cursor, camera))), Some(c)) => {
                let (cx, cy, s) = camera.sample(local);
                cam.extend([cx as f32, cy as f32, s as f32]);
                let (x, y, a, p) = cursor.sample(local);
                cur.extend([x as f32, y as f32, a as f32, p as f32]);
                c.recording().is_some_and(|r| r.source.webcam.is_some())
            }
            _ => {
                cam.extend([0.0, 0.0, 1.0]);
                cur.extend([0.0; 4]);
                false
            }
        };
        let (x, y, size, alpha) = webcam_state(project, ratio, t);
        let alpha = if has_webcam && project.webcam.show {
            alpha
        } else {
            0.0
        };
        webcam.extend([x as f32, y as f32, size as f32, alpha as f32]);
    }

    let mut ripples = Vec::new();
    let mut events = EventMarks::default();
    let mut keys = Vec::new();
    for span in &spans {
        let Some(log) = logs.get(span.index) else {
            continue;
        };
        for e in &log.events {
            if e.t < 0.0 || e.t >= span.duration_ms {
                continue;
            }
            let t = span.start_ms + e.t;
            match &e.kind {
                EventKind::Down { x, y, inferred, .. } => {
                    events.clicks.push(t);
                    if !inferred {
                        if let Some(out_ms) = kept.src_to_out(t) {
                            ripples.push(Ripple {
                                out_ms,
                                x: *x,
                                y: *y,
                                clip: span.index as u16,
                            });
                        }
                    }
                }
                EventKind::Key { combo } => {
                    events.keys.push(t);
                    if let (Some(combo), Some(out_ms)) = (combo, kept.src_to_out(t)) {
                        keys.push(KeyMark {
                            out_ms,
                            combo: combo.clone(),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    // Blends where the clip changes between two consecutive frames.
    let mut transitions = Vec::new();
    let mut run_start = 0;
    let mut pending: Option<TransitionMark> = None;
    for i in 1..=frames {
        if i < frames && clip[i] == clip[i - 1] {
            continue;
        }
        // The run [run_start, i) just ended; cap the blend that led into it.
        let run = i - run_start;
        if let Some(mut t) = pending.take() {
            t.half = t.half.min(run / 2);
            if t.half >= 1 {
                transitions.push(t);
            }
        }
        if i < frames {
            let tr = project.clips[clip[i] as usize].transition;
            if tr.kind != TransitionKind::Cut {
                let half = (tr.duration_ms / 2.0 * fps as f64 / 1000.0).round() as usize;
                pending = Some(TransitionMark {
                    frame: i,
                    half: half.min(run / 2),
                    kind: tr.kind,
                });
            }
            run_start = i;
        }
    }

    let captions = match &project.transcript {
        Some(t) => caption_lines(&t.words, &kept, project.captions.max_chars),
        None => vec![],
    };

    let first_log = project
        .clips
        .iter()
        .position(|c| c.recording().is_some())
        .and_then(|i| logs.get(i));
    MotionTracks {
        fps,
        frames,
        duration_ms: out_duration,
        src_ms,
        clip,
        clip_starts: spans.iter().map(|s| s.start_ms).collect(),
        cam,
        cursor: cur,
        webcam,
        ripples,
        events,
        keys,
        captions,
        transitions,
        cursor_drawn: (0..project.clips.len())
            .map(|i| project.cursor.show && logs.get(i).is_some_and(|l| l.header.can_draw_cursor()))
            .collect(),
        cursor_mode: first_log.map_or("none".into(), |l| l.header.cursor.clone()),
    }
}

#[cfg(test)]
mod tests {

    fn tracks(
        cam_x: impl Fn(usize) -> f32,
        cur_x: impl Fn(usize) -> f32,
        clips: &[u16],
    ) -> MotionTracks {
        let n = clips.len();
        MotionTracks {
            fps: 60,
            frames: n,
            duration_ms: n as f64 * 1000.0 / 60.0,
            src_ms: (0..n).map(|i| i as f64 * 1000.0 / 60.0).collect(),
            clip: clips.to_vec(),
            clip_starts: vec![0.0],
            cam: (0..n).flat_map(|i| [cam_x(i), 540.0, 2.0]).collect(),
            cursor: (0..n).flat_map(|i| [cur_x(i), 500.0, 1.0, 0.0]).collect(),
            webcam: vec![0.0; n * 4],
            ripples: vec![],
            events: EventMarks::default(),
            keys: vec![],
            captions: vec![],
            transitions: vec![],
            cursor_drawn: vec![true],
            cursor_mode: "metadata".into(),
        }
    }

    // The same cases as `blurSamples` in src/render/renderer.ts.
    #[test]
    fn blur_leaves_still_frames_alone() {
        let t = tracks(|_| 960.0, |_| 400.0, &[0; 10]);
        let (cams, trail) = blur_samples(&t, 5, 0.5, 1920.0, 1920.0);
        assert!(cams.is_empty() && trail.is_empty());
    }

    #[test]
    fn blur_takes_more_samples_the_further_things_move() {
        let t = tracks(|i| 900.0 + 40.0 * i as f32, |_| 400.0, &[0; 10]);
        let (cams, _) = blur_samples(&t, 5, 0.5, 1920.0, 1920.0);
        assert_eq!(cams.len(), BLUR_MAX_SAMPLES);
        assert!((cams[0].0 - (900.0 + 40.0 * (4.75 + 0.5 / 24.0))).abs() < 1e-3);
        let t = tracks(|i| 900.0 + 6.0 * i as f32, |_| 400.0, &[0; 10]);
        assert_eq!(blur_samples(&t, 5, 0.5, 1920.0, 1920.0).0.len(), 3);
    }

    #[test]
    fn blur_stays_inside_the_clip() {
        let t = tracks(
            |_| 960.0,
            |i| 100.0 + 30.0 * i as f32,
            &[0, 0, 0, 0, 0, 1, 1, 1, 1, 1],
        );
        let (_, trail) = blur_samples(&t, 5, 1.0, 1920.0, 1920.0);
        assert!(trail.iter().all(|&(x, _)| x >= 100.0 + 30.0 * 5.0 - 1e-3));
    }

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
                    kind: EventKind::Key { combo: None },
                },
            ],
        };
        let logs = [log];
        p.zooms = auto_zoom(&p, &logs, &[]);
        let m = compute_motion(&p, &logs, &[]);
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
        assert_eq!(m.cursor_drawn, vec![true]);
    }
}
