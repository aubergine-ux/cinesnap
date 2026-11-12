//! A synthetic recording: a mock app UI being clicked and typed into, with a
//! deliberately jittery cursor. It gives the tests and the demo material a
//! deterministic `.cinesnap` bundle without needing a real screen.

use std::path::Path;

use anyhow::Result;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::events::{Button, Event, EventKind, EventWriter, Header};
use crate::ffmpeg::{Encoder, FrameWriter, Quality};
use crate::project::*;
use crate::render::{rounded_rect, ARROW};

pub struct SynthOptions {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Draw the raw cursor into the video, like an ordinary screen recorder.
    pub baked_cursor: bool,
}

impl Default for SynthOptions {
    fn default() -> Self {
        Self { width: 1920, height: 1080, fps: 60, baked_cursor: false }
    }
}

pub const DURATION_S: f64 = 12.5;

/// (arrival time s, x, y, clicks) in 1920×1080 design coordinates.
const PATH: [(f64, f64, f64, bool); 6] = [
    (0.0, 420.0, 860.0, false),
    (1.6, 150.0, 400.0, true),
    (3.4, 1010.0, 470.0, true),
    (5.4, 1180.0, 148.0, true),
    (10.2, 1640.0, 952.0, true),
    (12.0, 1480.0, 760.0, false),
];
const TYPING: (f64, f64, f64) = (5.9, 8.2, 0.115); // start, end, interval

fn rand(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 33) as f64 / (1u64 << 31) as f64) - 1.0
}

/// Ideal (jitter-free) cursor position at time `t`, design coordinates, and
/// whether the hand is currently moving.
fn ideal(t: f64) -> (f64, f64, bool) {
    for w in PATH.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.0 {
            let start = (b.0 - 1.0).max(a.0 + 0.25);
            if t <= start {
                return (a.1, a.2, false);
            }
            let u = (t - start) / (b.0 - start);
            let e = u * u * u * (u * (6.0 * u - 15.0) + 10.0);
            let (dx, dy) = (b.1 - a.1, b.2 - a.2);
            let arc = (std::f64::consts::PI * u).sin() * 0.07;
            return (a.1 + dx * e - dy * arc, a.2 + dy * e + dx * arc, true);
        }
    }
    let last = PATH[PATH.len() - 1];
    (last.1, last.2, false)
}

fn fill(pm: &mut Pixmap, x: f64, y: f64, w: f64, h: f64, r: f64, c: u32) {
    if let Some(p) = rounded_rect(x as f32, y as f32, w as f32, h as f32, r as f32) {
        let mut paint = Paint::default();
        paint.set_color_rgba8((c >> 16) as u8, (c >> 8) as u8, c as u8, 255);
        paint.anti_alias = true;
        pm.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

fn outline(pm: &mut Pixmap, x: f64, y: f64, w: f64, h: f64, r: f64, c: u32, width: f64) {
    if let Some(p) = rounded_rect(x as f32, y as f32, w as f32, h as f32, r as f32) {
        let mut paint = Paint::default();
        paint.set_color_rgba8((c >> 16) as u8, (c >> 8) as u8, c as u8, 255);
        paint.anti_alias = true;
        let stroke = Stroke { width: width as f32, ..Default::default() };
        pm.stroke_path(&p, &paint, &stroke, Transform::identity(), None);
    }
}

fn draw_ui(pm: &mut Pixmap, t: f64, k: f64) {
    const ACCENT: u32 = 0x6d5dfc;
    let f = |pm: &mut Pixmap, x: f64, y: f64, w: f64, h: f64, r: f64, c: u32| fill(pm, x * k, y * k, w * k, h * k, r * k, c);
    pm.fill(tiny_skia::Color::from_rgba8(0x0f, 0x11, 0x17, 255));

    // Sidebar.
    f(pm, 0.0, 0.0, 300.0, 1080.0, 0.0, 0x151823);
    f(pm, 32.0, 44.0, 52.0, 52.0, 26.0, ACCENT);
    f(pm, 100.0, 60.0, 130.0, 20.0, 10.0, 0xc9cede);
    let active = if t >= PATH[1].0 { 2 } else { 0 };
    for i in 0..6 {
        let y = 200.0 + i as f64 * 86.0;
        if i == active {
            f(pm, 24.0, y, 252.0, 60.0, 14.0, 0x2a2550);
        }
        f(pm, 44.0, y + 18.0, 24.0, 24.0, 7.0, if i == active { ACCENT } else { 0x39405a });
        f(pm, 86.0, y + 22.0, 90.0 + (i * 37 % 70) as f64, 16.0, 8.0, if i == active { 0xe6e8f2 } else { 0x4a5270 });
    }

    // Search box with typed "text".
    let focused = t >= PATH[3].0;
    f(pm, 360.0, 110.0, 900.0, 76.0, 18.0, 0x1b1f2d);
    if focused {
        outline(pm, 360.0 * k, 110.0 * k, 900.0 * k, 76.0 * k, 18.0 * k, ACCENT, 3.0 * k);
    }
    f(pm, 386.0, 134.0, 28.0, 28.0, 14.0, 0x4a5270);
    let typed = if t < TYPING.0 { 0 } else { (((t.min(TYPING.1) - TYPING.0) / TYPING.2) as usize) + 1 };
    let mut x = 436.0;
    for i in 0..typed {
        if i % 6 == 5 {
            x += 16.0; // a space
            continue;
        }
        let w = 12.0 + ((i * 7) % 5) as f64 * 3.0;
        f(pm, x, 137.0, w, 22.0, 5.0, 0xe6e8f2);
        x += w + 5.0;
    }
    if typed == 0 && !focused {
        f(pm, 436.0, 140.0, 220.0, 16.0, 8.0, 0x39405a);
    }
    if focused && (t * 2.0).fract() < 0.5 {
        f(pm, x + 2.0, 130.0, 3.0, 36.0, 1.5, 0xe6e8f2);
    }
    f(pm, 1790.0, 118.0, 60.0, 60.0, 30.0, 0x39405a);

    // Cards.
    let tints: [u32; 6] = if active == 2 {
        [0x2f6f5e, 0x7a4b8f, 0x8f6a2f, 0x2f5a8f, 0x8f3f55, 0x4b8f3a]
    } else {
        [0x3a3f8f, 0x8f5a2f, 0x2f7a8f, 0x6a2f8f, 0x2f8f6a, 0x8f2f4b]
    };
    let selected = t >= PATH[2].0;
    for i in 0..6 {
        let (cx, cy) = (360.0 + (i % 3) as f64 * 512.0, 250.0 + (i / 3) as f64 * 320.0);
        let sel = selected && i == 1;
        f(pm, cx, cy, 480.0, 288.0, 20.0, if sel { 0x232842 } else { 0x1b1f2d });
        f(pm, cx + 18.0, cy + 18.0, 444.0, 168.0, 12.0, tints[i]);
        f(pm, cx + 18.0, cy + 208.0, 260.0, 18.0, 9.0, 0xc9cede);
        f(pm, cx + 18.0, cy + 242.0, 180.0, 14.0, 7.0, 0x4a5270);
        if sel {
            outline(pm, cx * k, cy * k, 480.0 * k, 288.0 * k, 20.0 * k, ACCENT, 4.0 * k);
        }
    }

    // Footer button and confirmation toast.
    let done = t >= PATH[4].0;
    f(pm, 1500.0, 910.0, 280.0, 84.0, 20.0, if done { 0x2fbf71 } else { ACCENT });
    f(pm, 1570.0, 943.0, 140.0, 18.0, 9.0, 0xffffff);
    if done {
        let u = ((t - PATH[4].0 - 0.1) / 0.3).clamp(0.0, 1.0);
        let y = -90.0 + 130.0 * (1.0 - (1.0 - u).powi(3));
        f(pm, 760.0, y, 400.0, 70.0, 18.0, 0x2fbf71);
        f(pm, 786.0, y + 22.0, 26.0, 26.0, 13.0, 0xffffff);
        f(pm, 830.0, y + 27.0, 240.0, 16.0, 8.0, 0xffffff);
    }
}

fn draw_cursor(pm: &mut Pixmap, x: f64, y: f64, h: f64) {
    let mut pb = PathBuilder::new();
    for (i, (px, py)) in ARROW.iter().enumerate() {
        let p = ((x + *px as f64 * h) as f32, (y + *py as f64 * h) as f32);
        if i == 0 {
            pb.move_to(p.0, p.1)
        } else {
            pb.line_to(p.0, p.1)
        }
    }
    pb.close();
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.anti_alias = true;
        paint.set_color_rgba8(0, 0, 0, 255);
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        paint.set_color_rgba8(255, 255, 255, 255);
        let stroke = Stroke { width: (h * 0.07) as f32, ..Default::default() };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// Write a complete project bundle into `dir` (created if needed).
pub fn generate(dir: &Path, ffmpeg: &Path, opts: &SynthOptions) -> Result<Project> {
    std::fs::create_dir_all(dir)?;
    let k = opts.width as f64 / 1920.0;
    let duration_ms = DURATION_S * 1000.0;

    // Event log: 125 Hz samples with hand tremor.
    let header = Header::new(opts.width, opts.height, "exact", if opts.baked_cursor { "embedded" } else { "metadata" });
    let mut events: Vec<Event> = Vec::new();
    let mut seed = 0x5eed_u64;
    let mut samples: Vec<(f64, f64, f64)> = Vec::new();
    let mut t = 0.0;
    while t <= duration_ms {
        let (x, y, moving) = ideal(t / 1000.0);
        let amp = if moving { 3.2 } else { 0.5 };
        let (x, y) = ((x + rand(&mut seed) * amp) * k, (y + rand(&mut seed) * amp) * k);
        samples.push((t, x, y));
        events.push(Event { t, kind: EventKind::Move { x, y } });
        t += 8.0;
    }
    for &(at, x, y, click) in &PATH {
        if click {
            let (x, y) = (x * k, y * k);
            events.push(Event { t: at * 1000.0, kind: EventKind::Down { b: Button::Left, x, y, inferred: false } });
            events.push(Event { t: at * 1000.0 + 90.0, kind: EventKind::Up { b: Button::Left, x, y } });
        }
    }
    let mut kt = TYPING.0;
    while kt <= TYPING.1 {
        events.push(Event { t: kt * 1000.0, kind: EventKind::Key });
        kt += TYPING.2;
    }
    events.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mut w = EventWriter::create(&dir.join(EVENTS_FILE), &header)?;
    for e in &events {
        w.write(e)?;
    }
    w.flush()?;

    // Video.
    let frames = (DURATION_S * opts.fps as f64).round() as usize;
    let mut writer = FrameWriter::spawn(ffmpeg, opts.width, opts.height, opts.fps as f64, &[], |c| {
        c.args(Encoder::X264.codec_args(Quality::Master, true)).arg(dir.join(VIDEO_FILE));
    })?;
    let mut pm = Pixmap::new(opts.width, opts.height).expect("non-zero size");
    for i in 0..frames {
        let t = i as f64 / opts.fps as f64;
        draw_ui(&mut pm, t, k);
        if opts.baked_cursor {
            let s = samples[((t * 1000.0 / 8.0) as usize).min(samples.len() - 1)];
            draw_cursor(&mut pm, s.1, s.2, 24.0 * k);
        }
        writer.write(pm.data())?;
    }
    writer.finish()?;

    let mut project = Project::new(
        Source {
            video: VideoTrack {
                file: VIDEO_FILE.into(),
                width: opts.width,
                height: opts.height,
                fps: opts.fps as f64,
                duration_ms,
            },
            mic: None,
            webcam: None,
            events: FileRef { file: EVENTS_FILE.into() },
            platform: "synthetic".into(),
            target: "screen".into(),
        },
        "2026-01-01T00:00:00Z".into(),
    );
    let log = crate::events::EventLog { header, events };
    project.zooms = crate::motion::auto_zoom(&project, &log);
    project.save(dir)?;
    Ok(project)
}
