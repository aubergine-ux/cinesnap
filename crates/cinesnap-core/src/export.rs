//! Export: decode the recordings, composite every output frame, encode.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Context, Result};

use crate::audio;
use crate::ffmpeg::{self, Encoder, FrameReader, FrameWriter, Quality};
use crate::motion::{compute_motion, load_logs, MotionTracks};
use crate::presets::{self, ExportPreset};
use crate::project::{Project, Span};
use crate::render::{
    blend_transition, caption_at, key_at, Compositor, CursorState, FrameInput, RippleState,
    WebcamFrame, RIPPLE_MS,
};

#[derive(Debug, thiserror::Error)]
#[error("export cancelled")]
pub struct Cancelled;

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Progress {
    /// `audio`, `render`, `palette` or `encode`.
    pub stage: &'static str,
    pub frame: usize,
    pub total: usize,
}

pub struct ExportOptions<'a> {
    pub ffmpeg: &'a Path,
    pub out: &'a Path,
    /// `None` probes for the best encoder.
    pub encoder: Option<Encoder>,
}

/// Presets with a size limit this small are rendered once to a lossless
/// intermediate and then encoded as often as it takes to fit.
const FIT_LIMIT: u64 = 32 * 1024 * 1024;

fn part_path(out: &Path) -> PathBuf {
    let ext = out.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
    out.with_extension(format!("part.{ext}"))
}

/// Render `project` (stored in `dir`) to `opts.out`. The format is taken from
/// `project.export.format`. Nothing is left behind when the export fails or
/// is cancelled.
pub fn export(
    dir: &Path,
    project: &Project,
    opts: &ExportOptions,
    progress: &mut dyn FnMut(Progress),
    cancel: &AtomicBool,
) -> Result<()> {
    let part = part_path(opts.out);
    let result = export_to(dir, project, opts, &part, progress, cancel);
    if result.is_err() {
        ffmpeg::remove_file_patiently(&part);
    }
    result
}

/// Decoders for one recording clip.
struct ClipReader {
    video: FrameReader,
    seek_ms: f64,
    fps: f64,
    /// Reader, track offset, and how far into the track it was opened.
    webcam: Option<(FrameReader, f64, f64)>,
}

/// Renders output frames by index. Frames must be requested in order per
/// clip, which both the plain walk and transitions do.
struct Frames<'a> {
    dir: &'a Path,
    ffmpeg: &'a Path,
    project: &'a Project,
    spans: Vec<Span>,
    tracks: &'a MotionTracks,
    comp: &'a Compositor,
    readers: HashMap<usize, ClipReader>,
}

impl Frames<'_> {
    fn reader(&mut self, clip: usize, local_ms: f64) -> Result<Option<&mut ClipReader>> {
        let Some(rec) = self.project.clips[clip].recording() else {
            return Ok(None);
        };
        if !self.readers.contains_key(&clip) {
            let dir = Project::clip_dir(self.dir, rec);
            let (w, h) = self.comp.decode_size(clip).context("clip has no layout")?;
            let fps = rec.source.video.fps.max(1.0);
            // Skip a long trimmed start instead of decoding through it.
            let seek = |ms: f64| {
                if ms > 1000.0 {
                    (ms * fps / 1000.0).floor() * 1000.0 / fps
                } else {
                    0.0
                }
            };
            let seek_ms = seek(local_ms);
            let crop = rec.crop;
            let filter = format!(
                "crop={}:{}:{}:{},scale={w}:{h}:flags=lanczos,fps={fps}",
                crop.width.round() as u32,
                crop.height.round() as u32,
                crop.x.round() as u32,
                crop.y.round() as u32,
            );
            let video = FrameReader::open(
                self.ffmpeg,
                &dir.join(&rec.source.video.file),
                &filter,
                w,
                h,
                seek_ms,
            )?;
            let webcam = match (&rec.source.webcam, self.comp.webcam_side()) {
                (Some(track), Some(side)) => {
                    let f = format!(
                        "crop='min(iw,ih)':'min(iw,ih)',scale={side}:{side}:flags=bicubic,fps={fps}"
                    );
                    let skip = seek(local_ms - track.offset_ms);
                    Some((
                        FrameReader::open(
                            self.ffmpeg,
                            &dir.join(&track.file),
                            &f,
                            side,
                            side,
                            skip,
                        )?,
                        track.offset_ms,
                        skip,
                    ))
                }
                _ => None,
            };
            self.readers.insert(
                clip,
                ClipReader {
                    video,
                    seek_ms,
                    fps,
                    webcam,
                },
            );
        }
        Ok(self.readers.get_mut(&clip))
    }

    /// Composite output frame `i` into `out`.
    fn render(&mut self, i: usize, out: &mut [u8]) -> Result<()> {
        let tracks = self.tracks;
        let clip = tracks.clip[i] as usize;
        let out_ms = i as f64 * 1000.0 / tracks.fps as f64;
        let t_ms = tracks.src_ms[i];
        let local_ms = t_ms - self.spans[clip].start_ms;

        let ripples: Vec<RippleState> = tracks
            .ripples
            .iter()
            .filter(|r| r.clip as usize == clip)
            .filter(|r| (0.0..RIPPLE_MS).contains(&(out_ms - r.out_ms)))
            .map(|r| RippleState {
                age_ms: out_ms - r.out_ms,
                x: r.x,
                y: r.y,
            })
            .collect();
        let (x, y, alpha, press) = tracks.cursor_at(i);
        let (wx, wy, wsize, walpha) = tracks.webcam_at(i);
        let key = key_at(&tracks.keys, self.project.keystrokes.hold_ms, out_ms);
        let caption = caption_at(&tracks.captions, out_ms);
        let cursor = tracks.cursor_drawn[clip].then_some(CursorState { x, y, alpha, press });
        let comp = self.comp;

        let mut input = FrameInput {
            clip,
            t_ms,
            local_ms,
            out_ms,
            src: &[],
            cam: tracks.cam_at(i),
            cursor,
            ripples: &ripples,
            webcam: None,
            key,
            caption,
        };
        let Some(reader) = self.reader(clip, local_ms)? else {
            comp.render(out, &input);
            return Ok(());
        };
        let fps = reader.fps;
        let index = ((local_ms - reader.seek_ms) * fps / 1000.0 + 1e-6)
            .floor()
            .max(0.0) as u64;
        input.webcam = match &mut reader.webcam {
            Some((cam, offset, skip)) if walpha > 0.0 && local_ms >= *offset => {
                let at = ((local_ms - *offset - *skip) * fps / 1000.0)
                    .floor()
                    .max(0.0) as u64;
                Some(WebcamFrame {
                    pixels: cam.frame_at(at)?,
                    x: wx,
                    y: wy,
                    size: wsize,
                    alpha: walpha,
                })
            }
            _ => None,
        };
        input.src = reader.video.frame_at(index)?;
        comp.render(out, &input);
        Ok(())
    }

    /// Close decoders of clips that are finished.
    fn release_before(&mut self, clip: usize) {
        self.readers.retain(|&c, _| c >= clip);
    }
}

fn export_to(
    dir: &Path,
    project: &Project,
    opts: &ExportOptions,
    part: &Path,
    progress: &mut dyn FnMut(Progress),
    cancel: &AtomicBool,
) -> Result<()> {
    project.validate()?;
    if let Some(parent) = opts.out.parent().filter(|p| !p.as_os_str().is_empty()) {
        if !parent.is_dir() {
            bail!("the folder {} does not exist", parent.display());
        }
    }
    let mut project = project.clone();
    project.sanitize();
    let project = &project;
    let gif = project.export.format == "gif";
    let preset = presets::resolve(&project.export.preset, &project.export.format);

    let logs = load_logs(dir, project);
    let tracks = compute_motion(project, &logs);
    if tracks.frames == 0 {
        bail!("nothing to export: the whole recording is cut");
    }
    let comp = Compositor::new(project, dir);
    let fps = tracks.fps as f64;
    let tmp = tempfile::Builder::new()
        .prefix("cinesnap-export")
        .tempdir()
        .context("creating temp dir")?;

    let mix = tmp.path().join("mix.wav");
    let has_audio = !gif
        && audio::build_mix(
            dir,
            project,
            opts.ffmpeg,
            &mix,
            &mut |f| {
                progress(Progress {
                    stage: "audio",
                    frame: (f * 100.0) as usize,
                    total: 100,
                })
            },
            cancel,
        )?;
    let mix = has_audio.then_some(mix.as_path());

    // Small size limits need several encodes of the same frames.
    let fit = preset.max_bytes.filter(|&m| m <= FIT_LIMIT);
    let intermediate = tmp.path().join("frames.mkv");
    let quality = Quality::parse(&project.export.quality);
    let (width, height) = (comp.width(), comp.height());
    let mut writer = if gif || fit.is_some() {
        FrameWriter::spawn(opts.ffmpeg, width, height, fps, &[], |c| {
            c.args(["-c:v", "ffv1", "-level", "3"]).arg(&intermediate);
        })?
    } else {
        let encoder = opts.encoder.unwrap_or_else(|| Encoder::detect(opts.ffmpeg));
        FrameWriter::spawn(
            opts.ffmpeg,
            width,
            height,
            fps,
            &encoder.global_args(),
            |c| {
                if let Some(mix) = mix {
                    c.arg("-i").arg(mix);
                    c.args(["-map", "0:v", "-map", "1:a"]);
                    c.args(["-c:a", "aac", "-b:a", "160k", "-shortest"]);
                }
                if let Some(f) = encoder.filter() {
                    c.args(["-vf", f]);
                }
                c.args(encoder.codec_args(quality, false));
                c.args(["-movflags", "+faststart"]).arg(part);
            },
        )?
    };

    let mut frames = Frames {
        dir,
        ffmpeg: opts.ffmpeg,
        project,
        spans: project.spans(),
        tracks: &tracks,
        comp: &comp,
        readers: HashMap::new(),
    };
    let size = comp.frame_bytes();
    let mut cur = vec![0u8; size];
    // During a transition: the incoming clip's first frame, then the
    // outgoing clip's last one.
    let mut frozen = vec![0u8; size];
    let mut frozen_for = usize::MAX;
    let mut out = vec![0u8; size];
    let has_transitions = !tracks.transitions.is_empty();
    for i in 0..tracks.frames {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        if i > 0 && tracks.clip[i] != tracks.clip[i - 1] {
            frames.release_before(tracks.clip[i] as usize);
        }
        frames.render(i, &mut cur)?;
        let frame = match tracks.transition_at(i) {
            Some((t, p)) if i < t.frame => {
                if frozen_for != t.frame {
                    frames.render(t.frame, &mut frozen)?;
                    frozen_for = t.frame;
                }
                blend_transition(t.kind, p, &cur, &frozen, &mut out, width as usize);
                &out
            }
            Some((t, p)) => {
                blend_transition(t.kind, p, &frozen, &cur, &mut out, width as usize);
                &out
            }
            None => &cur,
        };
        writer.write(frame)?;
        // The last frame of a clip that blends into the next one.
        if has_transitions && tracks.transitions.iter().any(|t| t.frame == i + 1) {
            frozen.copy_from_slice(&cur);
        }
        progress(Progress {
            stage: "render",
            frame: i + 1,
            total: tracks.frames,
        });
    }
    drop(frames);
    writer.finish()?;

    let seconds = tracks.frames as f64 / fps;
    if gif {
        encode_gif(
            opts.ffmpeg,
            &intermediate,
            tmp.path(),
            part,
            &project.export.quality,
            preset,
            progress,
            tracks.frames,
            cancel,
        )?;
    } else if let Some(limit) = fit {
        encode_to_fit(
            opts.ffmpeg,
            &intermediate,
            mix,
            part,
            quality,
            limit,
            seconds,
            progress,
            tracks.frames,
            cancel,
        )?;
    } else if let Some(limit) = preset.max_bytes {
        let size = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        if size > limit {
            bail!(
                "the video is {} MB, over the {} MB limit of the {} preset; \
                 shorten it or lower the quality",
                size / (1024 * 1024),
                limit / (1024 * 1024),
                preset.label
            );
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Cancelled.into());
    }
    std::fs::rename(part, opts.out).with_context(|| format!("writing {}", opts.out.display()))?;
    Ok(())
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Encode the rendered frames as H.264 that fits in `limit` bytes: first at
/// the chosen quality, then at whatever bitrate the limit allows.
#[allow(clippy::too_many_arguments)]
fn encode_to_fit(
    ffmpeg: &Path,
    frames: &Path,
    mix: Option<&Path>,
    out: &Path,
    quality: Quality,
    limit: u64,
    seconds: f64,
    progress: &mut dyn FnMut(Progress),
    total: usize,
    cancel: &AtomicBool,
) -> Result<()> {
    const AUDIO_KBPS: f64 = 96.0;
    let encode = |video: &[String]| -> Result<u64> {
        let mut c = ffmpeg::command(ffmpeg);
        c.args(["-v", "error", "-y", "-i"]).arg(frames);
        if let Some(mix) = mix {
            c.arg("-i").arg(mix);
            c.args(["-map", "0:v", "-map", "1:a"]);
            c.args([
                "-c:a",
                "aac",
                "-b:a",
                &format!("{AUDIO_KBPS}k"),
                "-shortest",
            ]);
        }
        c.args(video);
        c.args(["-movflags", "+faststart", "-f", "mp4"]).arg(out);
        audio::run_cancellable(c, "encode", cancel)?;
        Ok(file_size(out))
    };
    progress(Progress {
        stage: "encode",
        frame: 0,
        total,
    });
    if encode(&Encoder::X264.codec_args(quality, false))? <= limit {
        return Ok(());
    }
    // Aim a little under the limit; container overhead is not predictable.
    let audio_bits = if mix.is_some() {
        AUDIO_KBPS * 1000.0 * seconds
    } else {
        0.0
    };
    let mut kbps = (limit as f64 * 8.0 * 0.93 - audio_bits) / seconds / 1000.0;
    for _ in 0..4 {
        if kbps < 120.0 {
            break;
        }
        let rate = |k: f64| format!("{}k", k.round() as u64);
        let args: Vec<String> = [
            "-c:v",
            "libx264",
            "-preset",
            "slow",
            "-pix_fmt",
            "yuv420p",
            "-b:v",
            &rate(kbps),
            "-maxrate",
            &rate(kbps * 1.4),
            "-bufsize",
            &rate(kbps * 2.0),
        ]
        .map(String::from)
        .to_vec();
        if encode(&args)? <= limit {
            return Ok(());
        }
        kbps *= 0.85;
    }
    bail!(
        "a video of {:.0} s cannot be made to fit in {} MB; shorten it or pick another preset",
        seconds,
        limit / (1024 * 1024)
    )
}

/// Two-pass GIF encode: build a palette from the whole clip, then map every
/// frame to it, only rewriting the rectangle that changed. When the preset
/// has a size limit, colours, size and frame rate are reduced until the file
/// fits.
#[allow(clippy::too_many_arguments)]
fn encode_gif(
    ffmpeg: &Path,
    frames: &Path,
    tmp: &Path,
    out: &Path,
    quality: &str,
    preset: &ExportPreset,
    progress: &mut dyn FnMut(Progress),
    total: usize,
    cancel: &AtomicBool,
) -> Result<()> {
    let palette = tmp.join("palette.png");
    let (colors, dither) = match quality {
        "low" => (96, "bayer:bayer_scale=4"),
        "medium" => (192, "bayer:bayer_scale=5"),
        _ => (256, "sierra2_4a"),
    };
    // (scale, keep every nth frame, colours, dither)
    let mut attempts = vec![(1.0, 1, colors, dither)];
    if preset.max_bytes.is_some() {
        attempts.extend([
            (1.0, 1, 128.min(colors), "bayer:bayer_scale=4"),
            (0.8, 1, 96, "bayer:bayer_scale=4"),
            (0.8, 2, 64, "bayer:bayer_scale=3"),
            (0.6, 2, 48, "bayer:bayer_scale=3"),
            (0.45, 2, 32, "bayer:bayer_scale=2"),
        ]);
    }
    for (scale, step, colors, dither) in attempts {
        let mut pre = String::new();
        if step > 1 {
            pre += &format!("framestep={step},");
        }
        if scale < 1.0 {
            pre += &format!("scale=trunc(iw*{scale}/2)*2:-2:flags=lanczos,");
        }
        progress(Progress {
            stage: "palette",
            frame: 0,
            total,
        });
        let mut c = ffmpeg::command(ffmpeg);
        c.args(["-v", "error", "-y", "-i"]).arg(frames);
        c.args([
            "-vf",
            &format!("{pre}palettegen=max_colors={colors}:stats_mode=diff"),
            "-update",
            "1",
        ])
        .arg(&palette);
        audio::run_cancellable(c, "gif palette", cancel)?;

        progress(Progress {
            stage: "encode",
            frame: 0,
            total,
        });
        let mut c = ffmpeg::command(ffmpeg);
        c.args(["-v", "error", "-y", "-i"])
            .arg(frames)
            .arg("-i")
            .arg(&palette);
        c.args([
            "-lavfi",
            &format!("{pre}paletteuse=dither={dither}:diff_mode=rectangle"),
            "-f",
            "gif",
        ])
        .arg(out);
        audio::run_cancellable(c, "gif encode", cancel)?;
        if preset.max_bytes.is_none_or(|limit| file_size(out) <= limit) {
            progress(Progress {
                stage: "encode",
                frame: total,
                total,
            });
            return Ok(());
        }
    }
    bail!(
        "the GIF does not fit in {} MB even at its smallest; shorten the video or export MP4",
        preset.max_bytes.unwrap_or(0) / (1024 * 1024)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_file_keeps_extension() {
        assert_eq!(
            part_path(Path::new("/a/b/demo.mp4")),
            Path::new("/a/b/demo.part.mp4")
        );
        assert_eq!(part_path(Path::new("x.gif")), Path::new("x.part.gif"));
    }
}
