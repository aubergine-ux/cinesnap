//! Recording: a session made of segments. Pausing ends a segment and
//! resuming starts the next, so nothing is captured in between. Switching
//! screens starts a segment on the new one before the old one stops and
//! cuts the overlap, so nothing is lost either. When stopped, the segments
//! become the clips of one project, joined without transitions.

#[cfg(target_os = "linux")]
pub mod evdev_input;
pub mod grab;
pub mod input;
pub mod loopback;
#[cfg(target_os = "linux")]
pub mod portal;
mod segment;
pub mod tape;

use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use cinesnap_core::ffmpeg::Encoder;
use cinesnap_core::motion::auto_zoom;
use cinesnap_core::project::*;
use serde::Deserialize;

use crate::devices;
use segment::{signed_ms, Finished, Segment};

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RecordOptions {
    pub target: Target,
    #[serde(default)]
    pub region: Option<Rect>,
    #[serde(default)]
    pub mic: Option<String>,
    #[serde(default)]
    pub webcam: Option<String>,
    /// An output from `Sources::outputs`: record what it plays.
    #[serde(default)]
    pub system_audio: Option<String>,
    /// An app from `Sources::apps`: record only its sound.
    #[serde(default)]
    pub app_audio: Option<String>,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default = "default_fps")]
    pub fps: u32,
    /// Drop frames where nothing changed.
    #[serde(default = "yes")]
    pub vfr: bool,
    /// Seconds of countdown, cut from the start.
    #[serde(default)]
    pub countdown: u32,
    /// Label shortcuts (Ctrl+K, Enter) in the event log for the keystroke
    /// overlay. Typed text is never recorded either way.
    #[serde(default = "yes")]
    pub shortcuts: bool,
    /// Cinesnap's own hotkeys, kept out of the keystroke overlay.
    #[serde(skip)]
    pub hidden_combos: Vec<String>,
}

fn yes() -> bool {
    true
}

fn default_fps() -> u32 {
    60
}

#[derive(Deserialize, Debug, Clone)]
pub struct Target {
    /// `screen`, `window`, `all` (every screen) or `portal`.
    pub kind: String,
    #[serde(default)]
    pub id: Option<String>,
}

/// What a segment records.
#[derive(Clone)]
pub enum Source {
    Screens {
        areas: Vec<grab::ScreenArea>,
        mapping: input::Mapping,
        /// Region or window rectangle, in video pixels.
        crop: Option<Rect>,
        target: &'static str,
    },
    #[cfg(target_os = "linux")]
    Portal {
        capture: Arc<portal::PortalCapture>,
        region: Option<Rect>,
    },
}

fn area(s: &devices::Screen) -> grab::ScreenArea {
    grab::ScreenArea {
        index: s.output,
        x: s.x,
        y: s.y,
        width: s.width,
        height: s.height,
        hdr: s.hdr,
        sdr_white_nits: s.sdr_white_nits,
    }
}

impl Source {
    /// Find what `target` refers to. On Wayland this shows the system picker.
    #[allow(unused_variables)]
    pub fn resolve(
        ffmpeg: &Path,
        encoder: Encoder,
        opts: &RecordOptions,
        target: &Target,
    ) -> Result<Self> {
        #[cfg(target_os = "linux")]
        if devices::is_wayland() {
            let capture = portal::PortalCapture::open(ffmpeg, encoder, opts.fps, opts.vfr)?;
            return Ok(Self::Portal {
                capture: Arc::new(capture),
                region: opts.region,
            });
        }
        let screens = devices::screens();
        anyhow::ensure!(!screens.is_empty(), "no screen found to record");
        let by_id = |id: Option<&str>| {
            id.and_then(|i| screens.iter().find(|s| s.id == i))
                .unwrap_or(&screens[0])
        };
        let (chosen, crop, kind): (Vec<&devices::Screen>, Option<Rect>, &'static str) =
            match (target.kind.as_str(), target.id.as_deref()) {
                ("window", Some(id)) => {
                    let (i, rect) = devices::window_area(id, &screens)
                        .context("that window is no longer available")?;
                    (vec![&screens[i]], Some(rect), "window")
                }
                ("all", _) => {
                    let scale = screens[0].scale;
                    anyhow::ensure!(
                        screens.iter().all(|s| (s.scale - scale).abs() < 0.01),
                        "screens with different scaling cannot be recorded together"
                    );
                    (screens.iter().collect(), None, "screens")
                }
                (_, id) => {
                    let kind = if opts.region.is_some() {
                        "region"
                    } else {
                        "screen"
                    };
                    (vec![by_id(id)], opts.region, kind)
                }
            };
        let areas: Vec<grab::ScreenArea> = chosen.iter().map(|s| area(s)).collect();
        let origin = (
            chosen.iter().map(|s| s.origin.0).fold(f64::MAX, f64::min),
            chosen.iter().map(|s| s.origin.1).fold(f64::MAX, f64::min),
        );
        Ok(Self::Screens {
            areas,
            mapping: input::Mapping {
                origin,
                scale: chosen[0].scale,
            },
            crop,
            target: kind,
        })
    }
}

pub struct Session {
    dir: PathBuf,
    ffmpeg: PathBuf,
    encoder: Encoder,
    opts: RecordOptions,
    source: Source,
    done: Vec<Finished>,
    /// Where a segment ends early because a screen switch overlapped it.
    early_ends: Vec<Option<f64>>,
    current: Option<Segment>,
    /// Markers as (segment, instant). Markers set while paused go to the
    /// start of the next segment.
    markers: Vec<(usize, Option<Instant>)>,
    /// Set when the app came back to the front: the user reaching for the
    /// stop button, cut by default.
    pub tail_from: Option<Instant>,
}

fn segment_dir(index: usize) -> String {
    if index == 0 {
        String::new()
    } else {
        format!("clips/part{}", index + 1)
    }
}

impl Session {
    /// Start recording into `dir`. Blocks until the first frame has been
    /// captured (on Wayland that includes the system's picker).
    pub fn start(ffmpeg: &Path, encoder: Encoder, dir: &Path, opts: RecordOptions) -> Result<Self> {
        let source = Source::resolve(ffmpeg, encoder, &opts, &opts.target)?;
        let first = Segment::start(ffmpeg, encoder, dir, "", &opts, &source);
        let first = match first {
            Ok(s) => s,
            Err(e) => {
                discard(dir);
                return Err(e);
            }
        };
        Ok(Self {
            dir: dir.to_owned(),
            ffmpeg: ffmpeg.to_owned(),
            encoder,
            opts,
            source,
            done: vec![],
            early_ends: vec![],
            current: Some(first),
            markers: vec![],
            tail_from: None,
        })
    }

    /// Recorded time, pauses left out.
    pub fn elapsed_ms(&self) -> f64 {
        let banked: f64 = self
            .done
            .iter()
            .zip(&self.early_ends)
            .map(|(f, end)| end.unwrap_or_else(|| f.duration_ms()))
            .sum();
        banked + self.current.as_ref().map_or(0.0, Segment::elapsed_ms)
    }

    pub fn is_paused(&self) -> bool {
        self.current.is_none()
    }

    pub fn countdown_ms(&self) -> f64 {
        self.opts.countdown.min(10) as f64 * 1000.0
    }

    /// The screens being recorded, as `(x, y, width, height)`; empty when
    /// the system picker chose.
    pub fn screens(&self) -> Vec<(i32, i32, u32, u32)> {
        match &self.source {
            Source::Screens { areas, .. } => areas
                .iter()
                .map(|a| (a.x, a.y, a.width, a.height))
                .collect(),
            #[cfg(target_os = "linux")]
            Source::Portal { .. } => vec![],
        }
    }

    pub fn segments(&self) -> usize {
        self.done.len() + usize::from(self.current.is_some())
    }

    fn finish_current(&mut self, early_end: Option<f64>) -> Result<()> {
        if let Some(seg) = self.current.take() {
            let finished = seg.finish(&self.ffmpeg, &self.dir)?;
            self.done.push(finished);
            self.early_ends.push(early_end);
        }
        Ok(())
    }

    pub fn pause(&mut self) -> Result<()> {
        self.tail_from = None;
        self.finish_current(None)
    }

    pub fn resume(&mut self) -> Result<()> {
        if self.current.is_some() {
            return Ok(());
        }
        let index = self.done.len();
        let seg = Segment::start(
            &self.ffmpeg,
            self.encoder,
            &self.dir,
            &segment_dir(index),
            &self.opts,
            &self.source,
        )?;
        for m in &mut self.markers {
            if m.0 == index && m.1.is_none() {
                m.1 = Some(seg.t0);
            }
        }
        self.current = Some(seg);
        Ok(())
    }

    /// Record `target` from now on. The new segment starts before the old
    /// one stops, and the old one is cut where the new one begins.
    pub fn switch(&mut self, target: Target) -> Result<()> {
        let mut opts = self.opts.clone();
        opts.target = target;
        opts.region = None;
        let source = Source::resolve(&self.ffmpeg, self.encoder, &opts, &opts.target)?;
        self.opts = opts;
        if self.current.is_none() {
            self.source = source;
            return Ok(());
        }
        let index = self.done.len() + 1;
        let next = Segment::start(
            &self.ffmpeg,
            self.encoder,
            &self.dir,
            &segment_dir(index),
            &self.opts,
            &source,
        )?;
        let old_t0 = self.current.as_ref().map(|s| s.t0).unwrap();
        let end = signed_ms(next.t0, old_t0);
        self.finish_current(Some(end))?;
        self.current = Some(next);
        self.source = source;
        Ok(())
    }

    pub fn marker(&mut self) {
        let at = self.current.as_ref().map(|_| Instant::now());
        self.markers.push((self.done.len(), at));
    }

    /// Stop and assemble the project. Returns the bundle directory.
    pub fn stop(mut self) -> Result<PathBuf> {
        self.finish_current(None)?;
        if self.done.is_empty() {
            bail!("nothing was recorded");
        }
        let markers: Vec<(usize, f64)> = self
            .markers
            .iter()
            .filter_map(|&(i, at)| Some((i, signed_ms(at?, self.done.get(i)?.t0))))
            .collect();
        let tail = self
            .tail_from
            .map(|t| signed_ms(t, self.done.last().unwrap().t0));
        let project = assemble(
            &self.done,
            &self.early_ends,
            &markers,
            self.countdown_ms(),
            tail,
        );
        project.save(&self.dir)?;
        Ok(self.dir)
    }

    /// Stop everything and throw the recording away.
    pub fn cancel(mut self) {
        if let Some(seg) = self.current.take() {
            seg.cancel();
        }
        discard(&self.dir);
    }
}

fn discard(dir: &Path) {
    if dir.extension().is_some_and(|e| e == "cinesnap") {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The project for a finished session: one clip per segment, the countdown
/// and the reach for the stop button cut, markers on the timeline.
/// `markers` and `tail` are in their segment's own time.
fn assemble(
    parts: &[Finished],
    early_ends: &[Option<f64>],
    markers: &[(usize, f64)],
    countdown_ms: f64,
    tail: Option<f64>,
) -> Project {
    let first = &parts[0];
    let created = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut project = Project::new(first.source.clone(), created);
    project.crop = first.crop;
    project.main_mut().crop = first.crop;
    for (i, part) in parts.iter().enumerate().skip(1) {
        project.clips.push(Clip {
            id: format!("part{}", i + 1),
            transition: Transition::default(),
            body: ClipBody::Recording(RecordingClip {
                dir: part.dir.clone(),
                source: part.source.clone(),
                crop: part.crop,
            }),
        });
    }
    let spans = project.spans();
    let mut cut = |start: f64, end: f64| {
        if end - start > 1.0 {
            project.cuts.push(Cut {
                start_ms: start,
                end_ms: end,
            });
        }
    };
    if countdown_ms > 0.0 {
        cut(0.0, countdown_ms.min(spans[0].duration_ms - 500.0));
    }
    for (span, end) in spans.iter().zip(early_ends) {
        if let Some(end) = end {
            cut(
                span.start_ms + end.clamp(0.0, span.duration_ms),
                span.end_ms(),
            );
        }
    }
    if let (Some(t), Some(last)) = (tail, spans.last()) {
        let start = t - 150.0;
        if start > 1000.0 && start < last.duration_ms {
            cut(last.start_ms + start, last.end_ms());
        }
    }
    project.markers = markers
        .iter()
        .filter_map(|&(i, t)| {
            let span = spans.get(i)?;
            (0.0..=span.duration_ms)
                .contains(&t)
                .then_some(span.start_ms + t)
        })
        .enumerate()
        .map(|(n, at_ms)| Marker {
            id: format!("m{}", n + 1),
            at_ms,
            label: String::new(),
        })
        .collect();
    project.sanitize();
    let logs: Vec<_> = parts.iter().map(|p| p.log.clone()).collect();
    project.zooms = auto_zoom(&project, &logs, &[]);
    project
}

#[cfg(test)]
mod tests {
    use super::*;
    use cinesnap_core::events::{EventLog, Header};

    fn part(dir: &str, duration_ms: f64, width: u32) -> Finished {
        Finished {
            dir: dir.into(),
            source: cinesnap_core::project::Source {
                video: VideoTrack {
                    file: VIDEO_FILE.into(),
                    width,
                    height: 1080,
                    fps: 60.0,
                    duration_ms,
                    vfr: true,
                    hdr: None,
                },
                mic: None,
                system: None,
                webcam: None,
                events: FileRef {
                    file: EVENTS_FILE.into(),
                },
                platform: "test".into(),
                target: "screen".into(),
            },
            crop: Rect {
                x: 0.0,
                y: 0.0,
                width: width as f64,
                height: 1080.0,
            },
            log: EventLog {
                header: Header::new(width, 1080, "exact", "metadata"),
                events: vec![],
            },
            t0: Instant::now(),
        }
    }

    #[test]
    fn one_segment_is_one_clip() {
        let p = assemble(&[part("", 5000.0, 1920)], &[None], &[], 0.0, None);
        assert_eq!(p.clips.len(), 1);
        assert_eq!(p.duration_ms(), 5000.0);
        assert!(p.cuts.is_empty());
        assert_eq!(p.version, PROJECT_VERSION);
    }

    #[test]
    fn segments_follow_each_other_without_transitions() {
        let parts = [
            part("", 4000.0, 1920),
            part("clips/part2", 3000.0, 2560),
            part("clips/part3", 2000.0, 1920),
        ];
        let p = assemble(&parts, &[None, None, None], &[], 0.0, None);
        let ids: Vec<&str> = p.clips.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["main", "part2", "part3"]);
        assert!(p
            .clips
            .iter()
            .all(|c| c.transition.kind == TransitionKind::Cut));
        assert_eq!(p.clips[1].recording().unwrap().dir, "clips/part2");
        assert_eq!(p.clips[1].recording().unwrap().crop.width, 2560.0);
        assert_eq!(p.duration_ms(), 9000.0);
        p.validate().unwrap();
    }

    #[test]
    fn countdown_switch_overlap_and_tail_are_cut() {
        let parts = [part("", 6000.0, 1920), part("clips/part2", 5000.0, 1920)];
        // The second segment started 4.5 s into the first.
        let p = assemble(&parts, &[Some(4500.0), None], &[], 3000.0, Some(3800.0));
        let cuts: Vec<(f64, f64)> = p.cuts.iter().map(|c| (c.start_ms, c.end_ms)).collect();
        assert_eq!(
            cuts,
            [(0.0, 3000.0), (4500.0, 6000.0), (6000.0 + 3650.0, 11_000.0)]
        );
    }

    #[test]
    fn a_countdown_never_eats_a_whole_short_recording() {
        let p = assemble(&[part("", 2000.0, 1920)], &[None], &[], 3000.0, None);
        assert_eq!(p.cuts[0].end_ms, 1500.0);
    }

    #[test]
    fn markers_land_in_timeline_time() {
        let parts = [part("", 4000.0, 1920), part("clips/part2", 3000.0, 1920)];
        let p = assemble(
            &parts,
            &[None, None],
            &[(0, 1000.0), (1, 500.0), (1, 9999.0)],
            0.0,
            None,
        );
        let at: Vec<f64> = p.markers.iter().map(|m| m.at_ms).collect();
        assert_eq!(at, [1000.0, 4500.0]);
        assert_eq!(p.markers[1].id, "m2");
    }
}
