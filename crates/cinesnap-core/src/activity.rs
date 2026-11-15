//! Where things change on screen.
//!
//! A recording is decoded once at a low resolution and ten times a second;
//! each step records how much of the picture changed since the step before
//! and where. The cursor is not in the video, so what is left is the
//! content: text appearing where someone types, a terminal scrolling, a
//! dialog opening. Auto-zoom uses it to find the text cursor, reframing to
//! follow the action when the mouse is still. Cached per recording.

use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::export::Cancelled;
use crate::ffmpeg;
use crate::project::{ClipBody, Project, RecordingClip};

pub const STEP_MS: f64 = 100.0;
/// Width of the analysis grid; the height follows the recording's shape.
pub const GRID_W: u32 = 240;
/// A cell counts as changed when its brightness moves by more than this.
const THRESHOLD: u8 = 10;
const VERSION: u32 = 1;

/// One step: the share of the picture that changed and the box around it,
/// in source pixels of the clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Change {
    pub amount: f64,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Change {
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Activity {
    pub version: u32,
    pub step_ms: f64,
    /// Per step: changed share, then the changed box `x, y, w, h`.
    pub amount: Vec<f32>,
    pub boxes: Vec<[f32; 4]>,
}

impl Activity {
    pub fn len(&self) -> usize {
        self.amount.len()
    }

    pub fn is_empty(&self) -> bool {
        self.amount.is_empty()
    }

    /// What changed in the step ending at `t_ms`, if anything did.
    pub fn at(&self, t_ms: f64) -> Option<Change> {
        let i = (t_ms / self.step_ms).floor();
        if i < 0.0 {
            return None;
        }
        let i = i as usize;
        let amount = *self.amount.get(i)? as f64;
        (amount > 0.0).then(|| {
            let b = self.boxes[i];
            Change {
                amount,
                x: b[0] as f64,
                y: b[1] as f64,
                w: b[2] as f64,
                h: b[3] as f64,
            }
        })
    }

    /// Where small, local changes happened between `t0` and `t1`: the text
    /// cursor while someone types. Changes bigger than `max_share` of the
    /// picture (scrolling, a page loading) say nothing about it.
    pub fn local_change(&self, t0: f64, t1: f64, max_share: f64) -> Option<(f64, f64)> {
        let mut points: Vec<(f64, f64)> = Vec::new();
        let mut t = t0;
        while t <= t1 {
            if let Some(c) = self.at(t).filter(|c| c.amount <= max_share) {
                points.push(c.center());
            }
            t += self.step_ms;
        }
        if points.is_empty() {
            return None;
        }
        // The median ignores the odd far-away blink (a clock, a spinner).
        let mid = points.len() / 2;
        let mut xs: Vec<f64> = points.iter().map(|p| p.0).collect();
        let mut ys: Vec<f64> = points.iter().map(|p| p.1).collect();
        xs.sort_by(f64::total_cmp);
        ys.sort_by(f64::total_cmp);
        Some((xs[mid], ys[mid]))
    }
}

/// Analyse grey frames of `gw × gh` cells taken every `STEP_MS`, from a
/// source of `src_w × src_h` pixels.
pub fn analyze_frames<'a>(
    frames: impl IntoIterator<Item = &'a [u8]>,
    gw: usize,
    gh: usize,
    src_w: f64,
    src_h: f64,
) -> Activity {
    let (kx, ky) = (src_w / gw as f64, src_h / gh as f64);
    let mut a = Activity {
        version: VERSION,
        step_ms: STEP_MS,
        ..Default::default()
    };
    let mut prev: Option<&[u8]> = None;
    for frame in frames {
        let mut count = 0usize;
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        if let Some(p) = prev {
            for (i, (&a, &b)) in frame.iter().zip(p).enumerate() {
                if a.abs_diff(b) > THRESHOLD {
                    let (x, y) = (i % gw, i / gw);
                    count += 1;
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        if count == 0 {
            a.amount.push(0.0);
            a.boxes.push([0.0; 4]);
        } else {
            a.amount.push((count as f64 / (gw * gh) as f64) as f32);
            a.boxes.push([
                (x0 as f64 * kx) as f32,
                (y0 as f64 * ky) as f32,
                ((x1 - x0 + 1) as f64 * kx) as f32,
                ((y1 - y0 + 1) as f64 * ky) as f32,
            ]);
        }
        prev = Some(frame);
    }
    a
}

/// Decode `video` at the analysis resolution and analyse it.
pub fn analyze(
    ffmpeg_path: &Path,
    video: &Path,
    src_w: u32,
    src_h: u32,
    duration_ms: f64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<Activity> {
    let gw = GRID_W as usize;
    let gh = ((GRID_W as f64 * src_h as f64 / src_w.max(1) as f64).round() as usize).max(2);
    let fps = 1000.0 / STEP_MS;
    let mut c = ffmpeg::command(ffmpeg_path);
    c.args(["-v", "error", "-threads", "0", "-i"]).arg(video);
    c.args([
        "-an",
        "-vf",
        &format!("fps={fps},scale={gw}:{gh}:flags=area,format=gray"),
        "-f",
        "rawvideo",
        "-",
    ]);
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c
        .spawn()
        .context("starting ffmpeg to analyse the recording")?;
    let mut out = child.stdout.take().unwrap();
    let errors = ffmpeg::ErrorLog::attach(&mut child);
    let total = (duration_ms / STEP_MS).ceil().max(1.0);
    let mut frames: Vec<Vec<u8>> = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Cancelled.into());
        }
        let mut f = vec![0u8; gw * gh];
        if out.read_exact(&mut f).is_err() {
            break;
        }
        frames.push(f);
        if frames.len().is_multiple_of(50) {
            progress((frames.len() as f64 / total).min(0.99));
        }
    }
    let status = child.wait()?;
    if !status.success() && frames.is_empty() {
        bail!("could not analyse {}: {}", video.display(), errors.text());
    }
    progress(1.0);
    Ok(analyze_frames(
        frames.iter().map(Vec::as_slice),
        gw,
        gh,
        src_w as f64,
        src_h as f64,
    ))
}

/// Where a clip's analysis is cached: named after the video it describes,
/// so replacing the video never reuses a stale one.
pub fn cache_path(dir: &Path, clip_id: &str, rec: &RecordingClip) -> PathBuf {
    let video = Project::clip_dir(dir, rec).join(&rec.source.video.file);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Ok(meta) = std::fs::metadata(&video) {
        meta.len().hash(&mut h);
        if let Ok(t) = meta.modified() {
            t.hash(&mut h);
        }
    }
    VERSION.hash(&mut h);
    crate::audio::cache_dir(dir).join(format!("activity-{clip_id}-{:016x}.json", h.finish()))
}

/// The cached analysis of every clip (`None` for cards and for recordings
/// not analysed yet).
pub fn load(dir: &Path, project: &Project) -> Vec<Option<Activity>> {
    project
        .clips
        .iter()
        .map(|c| match &c.body {
            ClipBody::Recording(r) => std::fs::read(cache_path(dir, &c.id, r))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok()),
            ClipBody::Card(_) => None,
        })
        .collect()
}

/// Whether some recording still has to be analysed.
pub fn missing(dir: &Path, project: &Project) -> bool {
    project.clips.iter().any(|c| match &c.body {
        ClipBody::Recording(r) => !cache_path(dir, &c.id, r).is_file(),
        ClipBody::Card(_) => false,
    })
}

/// Analyse the recordings that are not cached yet; returns every clip's.
pub fn ensure(
    dir: &Path,
    project: &Project,
    ffmpeg_path: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<Vec<Option<Activity>>> {
    let todo: Vec<_> = project
        .clips
        .iter()
        .filter_map(|c| c.recording().map(|r| (c, r)))
        .filter(|(c, r)| !cache_path(dir, &c.id, r).is_file())
        .collect();
    for (n, (clip, rec)) in todo.iter().enumerate() {
        let video = Project::clip_dir(dir, rec).join(&rec.source.video.file);
        let v = &rec.source.video;
        let base = n as f64 / todo.len() as f64;
        let mut sub = |f: f64| progress(base + f / todo.len() as f64);
        let activity = analyze(
            ffmpeg_path,
            &video,
            v.width,
            v.height,
            v.duration_ms,
            cancel,
            &mut sub,
        )?;
        let path = cache_path(dir, &clip.id, rec);
        std::fs::create_dir_all(path.parent().unwrap())?;
        let part = path.with_extension("part");
        std::fs::write(&part, serde_json::to_vec(&activity)?)?;
        std::fs::rename(part, &path)?;
        // Analyses of the clip's earlier videos.
        let prefix = format!("activity-{}-", clip.id);
        if let Ok(entries) = std::fs::read_dir(path.parent().unwrap()) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with(&prefix) && e.path() != path {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
    progress(1.0);
    Ok(load(dir, project))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GW: usize = 24;
    const GH: usize = 12;

    fn blank() -> Vec<u8> {
        vec![30; GW * GH]
    }

    /// `n` frames, where frame `i` gets the changes `f(i)` set to white.
    fn frames(n: usize, f: impl Fn(usize) -> Vec<(usize, usize)>) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut cur = blank();
        for i in 0..n {
            for (x, y) in f(i) {
                cur[y * GW + x] = if cur[y * GW + x] == 30 { 230 } else { 30 };
            }
            out.push(cur.clone());
        }
        out
    }

    fn run(fr: &[Vec<u8>]) -> Activity {
        // A 2400 × 1200 source: one cell is 100 px.
        analyze_frames(fr.iter().map(Vec::as_slice), GW, GH, 2400.0, 1200.0)
    }

    #[test]
    fn a_still_screen_has_no_activity() {
        let a = run(&frames(10, |_| vec![]));
        assert_eq!(a.len(), 10);
        assert!(a.amount.iter().all(|&v| v == 0.0));
        assert_eq!(a.at(500.0), None);
    }

    #[test]
    fn typing_shows_up_as_a_small_moving_change() {
        // One character a step, left to right along row 5, from column 4.
        let a = run(&frames(
            8,
            |i| if i == 0 { vec![] } else { vec![(3 + i, 5)] },
        ));
        let c = a.at(300.0).unwrap();
        assert!((c.amount - 1.0 / (GW * GH) as f64).abs() < 1e-6);
        assert_eq!((c.x, c.y, c.w, c.h), (600.0, 500.0, 100.0, 100.0));
        let (x, y) = a.local_change(100.0, 799.0, 0.02).unwrap();
        // Columns 4..=10 were typed; the median is column 7.
        assert_eq!((x, y), (750.0, 550.0));
    }

    #[test]
    fn large_changes_do_not_count_as_the_text_cursor() {
        // Typing at one place, then the whole screen changes (a page load).
        let a = run(&frames(6, |i| match i {
            1 | 2 => vec![(2, 2)],
            4 => (0..GW).flat_map(|x| (0..GH).map(move |y| (x, y))).collect(),
            _ => vec![],
        }));
        let big = a.at(400.0).unwrap();
        assert_eq!(big.amount, 1.0);
        assert_eq!((big.w, big.h), (2400.0, 1200.0));
        assert_eq!(a.local_change(0.0, 599.0, 0.05), Some((250.0, 250.0)));
        assert_eq!(a.local_change(400.0, 499.0, 0.05), None);
    }

    #[test]
    fn serialises_compactly_and_back() {
        let a = run(&frames(5, |i| vec![(i, i)]));
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<Activity>(&json).unwrap(), a);
    }

    #[test]
    fn analyses_a_real_video() {
        let Some(ff) = crate::testutil::ffmpeg() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("v.mp4");
        // Two still seconds, then a moving box in the top-left quarter.
        let status = ffmpeg::command(&ff)
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .arg(
                "color=c=gray:s=640x360:r=30:d=4[bg];color=c=white:s=60x40:r=30:d=4[box];\
                 [bg][box]overlay=x='if(gt(t,2),20+40*t,20)':y=40",
            )
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&video)
            .status()
            .unwrap();
        assert!(status.success());
        let a = analyze(
            &ff,
            &video,
            640,
            360,
            4000.0,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert!((38..=42).contains(&a.len()), "{} steps", a.len());
        assert!(a.at(1000.0).is_none(), "still at first");
        let c = a.at(3000.0).expect("the box moves");
        assert!(c.x < 320.0 && c.y < 180.0, "{c:?}");
        assert!(c.amount < 0.1);
    }
}
