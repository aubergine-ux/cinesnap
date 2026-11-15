//! One unbroken stretch of a recording: the video, the side tracks and the
//! input log, from start (or resume) to pause (or stop).

use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use cinesnap_core::events::{infer_clicks, Event, EventKind, EventLog, EventWriter, Header};
use cinesnap_core::ffmpeg::{self, Encoder};
use cinesnap_core::project::*;

use super::grab::{self, Grab};
use super::input::{self, Sink, Tracker};
use super::loopback::{self, Loopback};
#[cfg(target_os = "linux")]
use super::portal::PortalCapture;
use super::{RecordOptions, Source};
use crate::devices;

enum Video {
    Grab(Grab),
    #[cfg(target_os = "linux")]
    Portal(Arc<PortalCapture>),
}

/// What the computer plays: through ffmpeg (a PulseAudio monitor) or
/// captured in-process.
enum Played {
    #[allow(dead_code)]
    Grab(Grab),
    Loopback(Loopback),
}

impl Played {
    fn first_sample(&self) -> Option<Instant> {
        match self {
            Self::Grab(g) => g.first_frame(),
            Self::Loopback(l) => l.first_sample(),
        }
    }
    fn stop(self) -> Result<()> {
        match self {
            Self::Grab(g) => g.stop(),
            Self::Loopback(l) => l.stop(),
        }
    }
}

pub struct Segment {
    dir: PathBuf,
    video: Video,
    /// When the first video frame was captured: time zero for every track.
    pub t0: Instant,
    mic: Option<Grab>,
    played: Option<Played>,
    app: Option<String>,
    webcam: Option<Grab>,
    sink: Sink,
    tracker: Tracker,
    fps: u32,
    vfr: bool,
    hdr: bool,
    crop: Option<Rect>,
    /// The video size the event coordinates were computed for.
    expected: Option<(u32, u32)>,
    clicks: &'static str,
    cursor: &'static str,
    target: &'static str,
    /// Shortcut labels to leave out of the log (Cinesnap's own hotkeys).
    hidden: Vec<String>,
}

/// A segment that has been stopped, with its files complete.
pub struct Finished {
    /// The segment's folder inside the bundle (empty for the bundle itself).
    pub dir: String,
    pub source: cinesnap_core::project::Source,
    pub crop: Rect,
    pub log: EventLog,
    pub t0: Instant,
}

impl Finished {
    pub fn duration_ms(&self) -> f64 {
        self.source.video.duration_ms
    }
}

fn side_track(what: &str, start: impl FnOnce() -> Result<Grab>) -> Result<Option<Grab>> {
    start()
        .map(Some)
        .with_context(|| format!("could not start the {what}"))
}

impl Segment {
    /// Start recording `source` into `dir`. Blocks until the first frame has
    /// been captured.
    pub fn start(
        ffmpeg: &Path,
        encoder: Encoder,
        root: &Path,
        sub: &str,
        opts: &RecordOptions,
        source: &Source,
    ) -> Result<Self> {
        let dir = if sub.is_empty() {
            root.to_owned()
        } else {
            root.join(sub)
        };
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let fps = opts.fps.clamp(5, 120);
        let sink = Sink::default();
        let mut tracker = Tracker::default();
        let video_path = dir.join(VIDEO_FILE);

        let mic = match opts.mic.as_deref() {
            Some(d) => side_track("microphone", || {
                grab::microphone(ffmpeg, d, &dir.join(MIC_FILE))
            })?,
            None => None,
        };
        let played = match (opts.app_audio.as_deref(), opts.system_audio.as_deref()) {
            (Some(app), _) => Some(Played::Loopback(
                loopback::app(app, &dir.join(SYSTEM_FILE))
                    .context("could not record that app's sound")?,
            )),
            #[cfg(target_os = "linux")]
            (None, Some(out)) => Some(Played::Grab(
                grab::system_audio(ffmpeg, out, &dir.join(SYSTEM_FILE))
                    .context("could not record the system sound")?,
            )),
            #[cfg(not(target_os = "linux"))]
            (None, Some(_)) => Some(Played::Loopback(
                loopback::system(&dir.join(SYSTEM_FILE))
                    .context("could not record the system sound")?,
            )),
            (None, None) => None,
        };
        let app = opts
            .app_audio
            .as_ref()
            .map(|_| opts.app_name.clone().unwrap_or_else(|| "App".into()));
        let webcam = match opts.webcam.as_deref() {
            Some(d) => side_track("webcam", || grab::webcam(ffmpeg, d, &dir.join(WEBCAM_FILE)))?,
            None => None,
        };
        let base = |video, t0, crop, expected, clicks, cursor, target, hdr| Self {
            dir: dir.clone(),
            video,
            t0,
            mic: None,
            played: None,
            app: None,
            webcam: None,
            sink: sink.clone(),
            tracker: Tracker::default(),
            fps,
            vfr: opts.vfr,
            hdr,
            crop,
            expected,
            clicks,
            cursor,
            target,
            hidden: opts.hidden_combos.clone(),
        };

        let mut seg = match source {
            #[cfg(target_os = "linux")]
            Source::Portal { capture, region } => {
                let t0 = capture.begin(&video_path, sink.clone(), Duration::from_secs(30))?;
                let buttons = super::evdev_input::available();
                if buttons {
                    super::evdev_input::track(
                        &mut tracker,
                        sink.clone(),
                        capture.cursor(),
                        opts.shortcuts,
                    );
                }
                let cursor = if capture.cursor_metadata {
                    "metadata"
                } else {
                    "embedded"
                };
                let clicks = match (buttons, capture.cursor_metadata) {
                    (true, _) => "exact",
                    (false, true) => "inferred",
                    (false, false) => "none",
                };
                let target = if region.is_some() { "region" } else { "screen" };
                base(
                    Video::Portal(capture.clone()),
                    t0,
                    *region,
                    None,
                    clicks,
                    cursor,
                    target,
                    false,
                )
            }
            Source::Screens {
                areas,
                mapping,
                crop,
                target,
                ..
            } => {
                // Desktop Duplication is the fast path on Windows; fall back
                // to GDI where it is unavailable (remote sessions, some VMs).
                let attempt = |win| -> Result<(Grab, Instant)> {
                    let mut g =
                        grab::screen(ffmpeg, encoder, areas, fps, opts.vfr, &video_path, win)?;
                    let t0 = g.wait_first_frame(Duration::from_secs(15))?;
                    Ok((g, t0))
                };
                let (video, t0) = match attempt(grab::WinGrab::Dda) {
                    Ok(v) => v,
                    Err(e) if cfg!(target_os = "windows") => {
                        attempt(grab::WinGrab::Gdi).map_err(|e2| e2.context(format!("{e:#}")))?
                    }
                    Err(e) => return Err(e.context("could not start screen capture")),
                };
                input::poll(&mut tracker, sink.clone(), *mapping, opts.shortcuts);
                let (_, _, w, h) = grab::bounds(areas);
                base(
                    Video::Grab(video),
                    t0,
                    *crop,
                    Some((w, h)),
                    "exact",
                    "metadata",
                    target,
                    areas.iter().any(|a| a.hdr),
                )
            }
        };
        seg.mic = mic;
        seg.played = played;
        seg.app = app;
        seg.webcam = webcam;
        seg.tracker = tracker;
        Ok(seg)
    }

    pub fn elapsed_ms(&self) -> f64 {
        self.t0.elapsed().as_secs_f64() * 1000.0
    }

    /// Stop every track and write the event log.
    pub fn finish(self, ffmpeg: &Path, root: &Path) -> Result<Finished> {
        let Segment {
            dir,
            video,
            t0,
            mic,
            played,
            app,
            webcam,
            sink,
            tracker,
            fps,
            vfr,
            hdr,
            crop,
            expected,
            clicks,
            cursor,
            target,
            hidden,
        } = self;
        drop(tracker);
        let raw = sink.take();

        let video_result = match video {
            Video::Grab(g) => g.stop(),
            #[cfg(target_os = "linux")]
            Video::Portal(p) => p.end(),
        };
        // A side track only counts if it actually delivered data.
        let keep = |first: Option<Instant>, ok: bool, file: &str| -> Option<f64> {
            match first {
                Some(t) if ok => Some(signed_ms(t, t0)),
                _ => {
                    let _ = std::fs::remove_file(dir.join(file));
                    None
                }
            }
        };
        let finish = |g: Option<Grab>, file: &str| -> Option<f64> {
            let g = g?;
            let first = g.first_frame();
            let ok = g.stop().is_ok();
            keep(first, ok, file)
        };
        let mic_offset = finish(mic, MIC_FILE);
        let played_offset = played.and_then(|p| {
            let first = p.first_sample();
            let ok = p.stop().is_ok();
            keep(first, ok, SYSTEM_FILE)
        });
        let webcam_offset = finish(webcam, WEBCAM_FILE);
        video_result?;

        let info = ffmpeg::probe(ffmpeg, &dir.join(VIDEO_FILE))
            .context("the recording is empty or unreadable")?;
        if info.duration_ms < 200.0 {
            bail!("the recording is too short");
        }
        let (width, height) = (info.width as f64, info.height as f64);

        // Event log: instants → milliseconds since the first frame, positions
        // → the pixel grid of the file that was actually written.
        let (kx, ky) = expected.map_or((1.0, 1.0), |(w, h)| (width / w as f64, height / h as f64));
        let center = (width / 2.0, height / 2.0);
        let place = |x: f64, y: f64| {
            if cursor == "metadata" {
                (x * kx, y * ky)
            } else {
                center
            }
        };
        let mut events: Vec<Event> = raw
            .into_iter()
            .map(|(at, kind)| (signed_ms(at, t0), kind))
            .filter(|(t, _)| *t >= -1000.0 && *t <= info.duration_ms + 100.0)
            .filter_map(|(t, kind)| {
                let kind = match kind {
                    EventKind::Move { .. } if cursor != "metadata" => return None,
                    EventKind::Move { x, y } => {
                        let (x, y) = place(x, y);
                        EventKind::Move { x, y }
                    }
                    EventKind::Down { b, x, y, inferred } => {
                        let (x, y) = place(x, y);
                        EventKind::Down { b, x, y, inferred }
                    }
                    EventKind::Up { b, x, y } => {
                        let (x, y) = place(x, y);
                        EventKind::Up { b, x, y }
                    }
                    EventKind::Scroll { dx, dy, x, y } => {
                        let (x, y) = place(x, y);
                        EventKind::Scroll { dx, dy, x, y }
                    }
                    EventKind::Key { combo: Some(c) } if hidden.contains(&c) => return None,
                    other => other,
                };
                Some(Event {
                    t: (t * 10.0).round() / 10.0,
                    kind,
                })
            })
            .collect();
        if clicks == "inferred" {
            events.extend(infer_clicks(&events));
        }
        events.sort_by(|a, b| a.t.total_cmp(&b.t));

        let header = Header::new(info.width, info.height, clicks, cursor);
        let mut writer = EventWriter::create(&dir.join(EVENTS_FILE), &header)?;
        for e in &events {
            writer.write(e)?;
        }
        writer.flush()?;

        let webcam = webcam_offset.and_then(|offset_ms| {
            let cam = ffmpeg::probe(ffmpeg, &dir.join(WEBCAM_FILE)).ok()?;
            Some(WebcamTrack {
                file: WEBCAM_FILE.into(),
                offset_ms,
                width: cam.width,
                height: cam.height,
            })
        });
        let full = Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        };
        let crop = crop.map_or(full, |c| Rect {
            x: c.x * kx,
            y: c.y * ky,
            width: c.width * kx,
            height: c.height * ky,
        });
        let rel = dir
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        Ok(Finished {
            dir: rel,
            source: cinesnap_core::project::Source {
                video: VideoTrack {
                    file: VIDEO_FILE.into(),
                    width: info.width,
                    height: info.height,
                    fps: fps as f64,
                    duration_ms: info.duration_ms,
                    vfr,
                    hdr: hdr.then(|| "tonemapped".into()),
                },
                mic: mic_offset.map(|offset_ms| AudioTrack {
                    file: MIC_FILE.into(),
                    offset_ms,
                    app: None,
                }),
                system: played_offset.map(|offset_ms| AudioTrack {
                    file: SYSTEM_FILE.into(),
                    offset_ms,
                    app,
                }),
                webcam,
                events: FileRef {
                    file: EVENTS_FILE.into(),
                },
                platform: devices::platform().into(),
                target: target.into(),
            },
            crop,
            log: EventLog { header, events },
            t0,
        })
    }

    /// Stop everything without keeping any of it.
    pub fn cancel(self) {
        drop(self.tracker);
        match self.video {
            Video::Grab(g) => g.kill(),
            #[cfg(target_os = "linux")]
            Video::Portal(p) => {
                let _ = p.end();
            }
        }
        for g in [self.mic, self.webcam].into_iter().flatten() {
            g.kill();
        }
        drop(self.played);
    }
}

/// `a - b` in milliseconds, negative when `a` is earlier.
pub fn signed_ms(a: Instant, b: Instant) -> f64 {
    match a.checked_duration_since(b) {
        Some(d) => d.as_secs_f64() * 1000.0,
        None => -(b.duration_since(a).as_secs_f64() * 1000.0),
    }
}
