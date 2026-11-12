//! Recording: runs the capture, audio, webcam and input tracks side by side
//! and turns them into a `.cinesnap` project when stopped.

#[cfg(target_os = "linux")]
pub mod evdev_input;
pub mod grab;
pub mod input;
#[cfg(target_os = "linux")]
pub mod portal;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use cinesnap_core::events::{infer_clicks, Event, EventKind, EventLog, EventWriter, Header};
use cinesnap_core::ffmpeg::{self, Encoder};
use cinesnap_core::motion::auto_zoom;
use cinesnap_core::project::*;
use serde::Deserialize;

use crate::devices;
use grab::Grab;
use input::{Sink, Tracker};

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RecordOptions {
    pub target: Target,
    #[serde(default)]
    pub region: Option<Rect>,
    #[serde(default)]
    pub mic: Option<String>,
    #[serde(default)]
    pub webcam: Option<String>,
    #[serde(default = "default_fps")]
    pub fps: u32,
    /// Label shortcuts (Ctrl+K, Enter) in the event log for the keystroke
    /// overlay. Typed text is never recorded either way.
    #[serde(default = "yes")]
    pub shortcuts: bool,
}

fn yes() -> bool {
    true
}

fn default_fps() -> u32 {
    60
}

#[derive(Deserialize, Debug)]
pub struct Target {
    /// `screen`, `window` or `portal`.
    pub kind: String,
    #[serde(default)]
    pub id: Option<String>,
}

enum Video {
    Grab(Grab),
    #[cfg(target_os = "linux")]
    Portal(portal::PortalCapture),
}

pub struct Recording {
    dir: PathBuf,
    fps: u32,
    video: Video,
    /// When the first video frame was captured: time zero for every track.
    t0: Instant,
    mic: Option<Grab>,
    webcam: Option<Grab>,
    sink: Sink,
    tracker: Tracker,
    /// Region or window rectangle, in video pixels.
    crop: Option<Rect>,
    /// The video size the event coordinates were computed for.
    expected: Option<(u32, u32)>,
    clicks: &'static str,
    cursor: &'static str,
    target: String,
    /// Set when the app window came back to the front; everything after is
    /// the user reaching for the stop button and gets cut by default.
    pub tail_from: Option<Instant>,
}

impl Recording {
    pub fn elapsed_ms(&self) -> f64 {
        self.t0.elapsed().as_secs_f64() * 1000.0
    }

    /// Start recording into `dir`. Blocks until the first frame has been
    /// captured (on Wayland that includes the system's source picker).
    pub fn start(
        ffmpeg: &Path,
        encoder: Encoder,
        dir: &Path,
        opts: &RecordOptions,
    ) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let fps = opts.fps.clamp(5, 120);
        let sink = Sink::default();
        let mut tracker = Tracker::default();
        let video_path = dir.join(VIDEO_FILE);

        let side_tracks = || -> Result<(Option<Grab>, Option<Grab>)> {
            let mic = opts
                .mic
                .as_deref()
                .map(|d| grab::microphone(ffmpeg, d, &dir.join(MIC_FILE)))
                .transpose()?;
            let cam = opts
                .webcam
                .as_deref()
                .map(|d| grab::webcam(ffmpeg, d, &dir.join(WEBCAM_FILE)))
                .transpose()?;
            Ok((mic, cam))
        };

        #[cfg(target_os = "linux")]
        if devices::is_wayland() {
            let mut capture =
                portal::PortalCapture::start(ffmpeg, encoder, fps, &video_path, sink.clone())?;
            let (mic, webcam) = side_tracks()?;
            let t0 = capture.wait_first_frame(Duration::from_secs(30))?;
            let buttons = evdev_input::available();
            if buttons {
                evdev_input::track(&mut tracker, sink.clone(), capture.cursor(), opts.shortcuts);
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
            return Ok(Self {
                dir: dir.to_owned(),
                fps,
                video: Video::Portal(capture),
                t0,
                mic,
                webcam,
                sink,
                tracker,
                crop: opts.region,
                expected: None,
                clicks,
                cursor,
                target: if opts.region.is_some() {
                    "region"
                } else {
                    "screen"
                }
                .into(),
                tail_from: None,
            });
        }

        let screens = devices::screens();
        let (index, window_rect) = match (opts.target.kind.as_str(), opts.target.id.as_deref()) {
            ("window", Some(id)) => {
                let (i, rect) = devices::window_area(id, &screens)
                    .context("that window is no longer available")?;
                (i, Some(rect))
            }
            (_, id) => (id.and_then(|i| i.parse().ok()).unwrap_or(0), None),
        };
        let screen = screens
            .get(index)
            .or(screens.first())
            .context("no screen found to record")?;
        let area = grab::ScreenArea {
            index,
            x: screen.x,
            y: screen.y,
            width: screen.width,
            height: screen.height,
        };

        // Desktop Duplication is the fast path on Windows; fall back to GDI
        // where it is unavailable (remote sessions, some virtual machines).
        let attempt = |win| -> Result<(Grab, Instant)> {
            let mut g = grab::screen(ffmpeg, encoder, &area, fps, &video_path, win)?;
            let t0 = g.wait_first_frame(Duration::from_secs(15))?;
            Ok((g, t0))
        };
        let (mic, webcam) = side_tracks()?;
        let (video, t0) = match attempt(grab::WinGrab::Dda) {
            Ok(v) => v,
            Err(e) if cfg!(target_os = "windows") => {
                attempt(grab::WinGrab::Gdi).map_err(|e2| e2.context(format!("{e:#}")))?
            }
            Err(e) => return Err(e.context("could not start screen capture")),
        };
        input::poll(
            &mut tracker,
            sink.clone(),
            input::Mapping {
                origin: screen.origin,
                scale: screen.scale,
            },
            opts.shortcuts,
        );

        Ok(Self {
            dir: dir.to_owned(),
            fps,
            video: Video::Grab(video),
            t0,
            mic,
            webcam,
            sink,
            tracker,
            crop: window_rect.or(opts.region),
            expected: Some((screen.width, screen.height)),
            clicks: "exact",
            cursor: "metadata",
            target: match (window_rect.is_some(), opts.region.is_some()) {
                (true, _) => "window",
                (_, true) => "region",
                _ => "screen",
            }
            .into(),
            tail_from: None,
        })
    }

    /// Stop every track and assemble the project. Returns the project dir.
    pub fn stop(self, ffmpeg: &Path) -> Result<PathBuf> {
        let Recording {
            dir,
            fps,
            video,
            t0,
            mic,
            webcam,
            sink,
            tracker,
            crop,
            expected,
            clicks,
            cursor,
            target,
            tail_from,
        } = self;
        drop(tracker);
        let raw = sink.take();

        let video_result = match video {
            Video::Grab(g) => g.stop(),
            #[cfg(target_os = "linux")]
            Video::Portal(p) => p.stop(),
        };
        // A side track only counts if it actually delivered data.
        let finish = |g: Option<Grab>, file: &str| -> Option<f64> {
            let g = g?;
            let first = g.first_frame();
            let ok = g.stop().is_ok();
            match first {
                Some(t) if ok => Some(signed_ms(t, t0)),
                _ => {
                    let _ = std::fs::remove_file(dir.join(file));
                    None
                }
            }
        };
        let mic_offset = finish(mic, MIC_FILE);
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

        let webcam_track = webcam_offset.and_then(|offset_ms| {
            let cam = ffmpeg::probe(ffmpeg, &dir.join(WEBCAM_FILE)).ok()?;
            Some(WebcamTrack {
                file: WEBCAM_FILE.into(),
                offset_ms,
                width: cam.width,
                height: cam.height,
            })
        });
        let mut project = Project::new(
            Source {
                video: VideoTrack {
                    file: VIDEO_FILE.into(),
                    width: info.width,
                    height: info.height,
                    fps: fps as f64,
                    duration_ms: info.duration_ms,
                },
                mic: mic_offset.map(|offset_ms| AudioTrack {
                    file: MIC_FILE.into(),
                    offset_ms,
                }),
                webcam: webcam_track,
                events: FileRef {
                    file: EVENTS_FILE.into(),
                },
                platform: devices::platform().into(),
                target,
            },
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        );
        if let Some(c) = crop {
            project.crop = Rect {
                x: c.x * kx,
                y: c.y * ky,
                width: c.width * kx,
                height: c.height * ky,
            };
        }
        if let Some(tail) = tail_from {
            let start = signed_ms(tail, t0) - 150.0;
            if start > 1000.0 && start < info.duration_ms {
                project.cuts.push(Cut {
                    start_ms: start,
                    end_ms: info.duration_ms,
                });
            }
        }
        project.sanitize();
        project.zooms = auto_zoom(&project, &[EventLog { header, events }]);
        project.save(&dir)?;
        Ok(dir)
    }

    /// Stop everything and throw the recording away.
    pub fn cancel(self) {
        let dir = self.dir.clone();
        drop(self.tracker);
        match self.video {
            Video::Grab(g) => g.kill(),
            #[cfg(target_os = "linux")]
            Video::Portal(p) => drop(p),
        }
        for g in [self.mic, self.webcam].into_iter().flatten() {
            g.kill();
        }
        if dir.extension().is_some_and(|e| e == "cinesnap") {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// `a - b` in milliseconds, negative when `a` is earlier.
fn signed_ms(a: Instant, b: Instant) -> f64 {
    match a.checked_duration_since(b) {
        Some(d) => d.as_secs_f64() * 1000.0,
        None => -(b.duration_since(a).as_secs_f64() * 1000.0),
    }
}
