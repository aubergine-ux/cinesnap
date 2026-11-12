//! Export: decode the recording, composite every output frame, encode.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Context, Result};

use crate::events::EventLog;
use crate::ffmpeg::{self, Encoder, FrameReader, FrameWriter, Quality};
use crate::layout::Layout;
use crate::motion::{compute_motion, Kept};
use crate::project::Project;
use crate::render::{Compositor, CursorState, FrameInput, RippleState, RIPPLE_MS};

#[derive(Debug, thiserror::Error)]
#[error("export cancelled")]
pub struct Cancelled;

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Progress {
    /// `render`, `palette` or `encode`.
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

fn even(v: f64) -> usize {
    (((v / 2.0).round() as usize) * 2).max(2)
}

/// `between(t,a,b)+…` selecting the kept ranges, in seconds.
fn select_expr(kept: &Kept) -> String {
    kept.ranges
        .iter()
        .map(|(s, e)| format!("between(t,{:.4},{:.4})", s / 1000.0, e / 1000.0))
        .collect::<Vec<_>>()
        .join("+")
}

/// Audio filter chain that aligns the mic to source time, removes the cut
/// ranges and pads with silence so the video decides the length.
fn audio_filter(offset_ms: f64, kept: &Kept) -> String {
    let mut chain = Vec::new();
    if offset_ms > 0.5 {
        chain.push(format!("adelay={}:all=1", offset_ms.round() as u64));
    } else if offset_ms < -0.5 {
        chain.push(format!("atrim=start={:.4}", -offset_ms / 1000.0));
        chain.push("asetpts=PTS-STARTPTS".into());
    }
    chain.push(format!("aselect='{}'", select_expr(kept)));
    chain.push("asetpts=N/SR/TB".into());
    chain.push("apad".into());
    chain.join(",")
}

fn part_path(out: &Path) -> PathBuf {
    let ext = out.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
    out.with_extension(format!("part.{ext}"))
}

/// Render `project` (stored in `dir`) to `opts.out`. The format is taken from
/// `project.export.format`.
pub fn export(
    dir: &Path,
    project: &Project,
    opts: &ExportOptions,
    progress: &mut dyn FnMut(Progress),
    cancel: &AtomicBool,
) -> Result<()> {
    let mut project = project.clone();
    project.sanitize();
    let project = &project;
    let gif = project.export.format == "gif";

    let log = EventLog::load(&project.events_path(dir)).unwrap_or_else(|_| {
        EventLog::empty(project.source.video.width, project.source.video.height)
    });
    let tracks = compute_motion(project, &log);
    if tracks.frames == 0 {
        bail!("nothing to export: the whole recording is cut");
    }
    let kept = Kept::new(project.source.video.duration_ms, &project.cuts);
    let layout = Layout::new(project);

    // Decode no more pixels than the deepest zoom can show.
    let crop = project.crop;
    let max_scale = project.zooms.iter().map(|z| z.scale).fold(1.0, f64::max);
    let dec_w = even((layout.frame.width * max_scale).min(crop.width));
    let dec_h = even(dec_w as f64 * crop.height / crop.width);
    let src_fps = project.source.video.fps.max(1.0);

    // Skip a long trimmed start instead of decoding through it.
    let first = kept.ranges.first().map_or(0.0, |r| r.0);
    let seek_frames = if first > 1000.0 {
        (first * src_fps / 1000.0).floor()
    } else {
        0.0
    };
    let seek_ms = seek_frames * 1000.0 / src_fps;

    let filter = format!(
        "crop={}:{}:{}:{},scale={dec_w}:{dec_h}:flags=lanczos,fps={src_fps}",
        crop.width.round() as u32,
        crop.height.round() as u32,
        crop.x.round() as u32,
        crop.y.round() as u32,
    );
    let mut video = FrameReader::open(
        opts.ffmpeg,
        &project.video_path(dir),
        &filter,
        dec_w,
        dec_h,
        seek_ms,
    )?;

    let comp = Compositor::new(project, layout, dec_w, dec_h, dir);

    let mut webcam = match (&project.source.webcam, comp.webcam_side()) {
        (Some(track), Some(side)) => {
            let f = format!(
                "crop='min(iw,ih)':'min(iw,ih)',scale={side}:{side}:flags=bicubic,fps={src_fps}"
            );
            Some((
                FrameReader::open(opts.ffmpeg, &dir.join(&track.file), &f, side, side, 0.0)?,
                track.offset_ms,
            ))
        }
        _ => None,
    };

    let fps = tracks.fps as f64;
    let part = part_path(opts.out);
    let tmp = tempfile::Builder::new()
        .prefix("cinesnap-export")
        .tempdir()
        .context("creating temp dir")?;
    let intermediate = tmp.path().join("frames.mkv");

    let mut writer = if gif {
        FrameWriter::spawn(opts.ffmpeg, layout.width, layout.height, fps, &[], |c| {
            c.args(["-c:v", "ffv1", "-level", "3"]).arg(&intermediate);
        })?
    } else {
        let encoder = opts.encoder.unwrap_or_else(|| Encoder::detect(opts.ffmpeg));
        let quality = Quality::parse(&project.export.quality);
        let mic = project
            .source
            .mic
            .as_ref()
            .map(|m| (dir.join(&m.file), m.offset_ms))
            .filter(|(p, _)| p.is_file());
        FrameWriter::spawn(
            opts.ffmpeg,
            layout.width,
            layout.height,
            fps,
            &encoder.global_args(),
            |c| {
                if let Some((path, offset)) = &mic {
                    c.arg("-i").arg(path);
                    c.args([
                        "-map",
                        "0:v",
                        "-map",
                        "1:a",
                        "-af",
                        &audio_filter(*offset, &kept),
                    ]);
                    c.args(["-c:a", "aac", "-b:a", "160k", "-shortest"]);
                }
                if let Some(f) = encoder.filter() {
                    c.args(["-vf", f]);
                }
                c.args(encoder.codec_args(quality, false));
                c.args(["-movflags", "+faststart"]).arg(&part);
            },
        )?
    };

    let mut out = vec![0u8; comp.frame_bytes()];
    let mut active: Vec<RippleState> = Vec::new();
    let mut next_ripple = 0;
    for i in 0..tracks.frames {
        if cancel.load(Ordering::Relaxed) {
            writer.abort();
            let _ = std::fs::remove_file(&part);
            return Err(Cancelled.into());
        }
        let out_ms = i as f64 * 1000.0 / fps;
        let src_ms = tracks.src_ms[i];

        while next_ripple < tracks.ripples.len() && tracks.ripples[next_ripple].out_ms <= out_ms {
            let r = &tracks.ripples[next_ripple];
            active.push(RippleState {
                age_ms: r.out_ms,
                x: r.x,
                y: r.y,
            }); // age filled in below
            next_ripple += 1;
        }
        active.retain(|r| out_ms - r.age_ms < RIPPLE_MS);
        let ripples: Vec<RippleState> = active
            .iter()
            .map(|r| RippleState {
                age_ms: out_ms - r.age_ms,
                ..*r
            })
            .collect();

        let index = ((src_ms - seek_ms) * src_fps / 1000.0 + 1e-6)
            .floor()
            .max(0.0) as u64;
        let cam_frame = match &mut webcam {
            Some((reader, offset)) if src_ms >= *offset => {
                Some(reader.frame_at(((src_ms - *offset) * src_fps / 1000.0).floor() as u64)?)
            }
            _ => None,
        };
        let src = video.frame_at(index)?;
        let (x, y, alpha, press) = tracks.cursor_at(i);
        comp.render(
            &mut out,
            &FrameInput {
                src,
                cam: tracks.cam_at(i),
                cursor: tracks
                    .draw_cursor
                    .then_some(CursorState { x, y, alpha, press }),
                ripples: &ripples,
                webcam: cam_frame,
            },
        );
        writer.write(&out)?;
        progress(Progress {
            stage: "render",
            frame: i + 1,
            total: tracks.frames,
        });
    }
    drop(video);
    drop(webcam);
    writer.finish()?;

    if gif {
        encode_gif(
            opts.ffmpeg,
            &intermediate,
            tmp.path(),
            &part,
            &project.export.quality,
            progress,
            tracks.frames,
        )?;
    }
    if cancel.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&part);
        return Err(Cancelled.into());
    }
    std::fs::rename(&part, opts.out).with_context(|| format!("writing {}", opts.out.display()))?;
    Ok(())
}

/// Two-pass GIF encode: build a palette from the whole clip, then map every
/// frame to it, only rewriting the rectangle that changed.
fn encode_gif(
    ffmpeg: &Path,
    frames: &Path,
    tmp: &Path,
    out: &Path,
    quality: &str,
    progress: &mut dyn FnMut(Progress),
    total: usize,
) -> Result<()> {
    let palette = tmp.join("palette.png");
    let (colors, dither) = match quality {
        "low" => (96, "bayer:bayer_scale=4"),
        "medium" => (192, "bayer:bayer_scale=5"),
        _ => (256, "sierra2_4a"),
    };
    progress(Progress {
        stage: "palette",
        frame: 0,
        total,
    });
    let mut c = ffmpeg::command(ffmpeg);
    c.args(["-v", "error", "-y", "-i"]).arg(frames);
    c.args([
        "-vf",
        &format!("palettegen=max_colors={colors}:stats_mode=diff"),
        "-update",
        "1",
    ])
    .arg(&palette);
    ffmpeg::run(c, "gif palette")?;

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
        &format!("paletteuse=dither={dither}:diff_mode=rectangle"),
        "-f",
        "gif",
    ])
    .arg(out);
    ffmpeg::run(c, "gif encode")?;
    progress(Progress {
        stage: "encode",
        frame: total,
        total,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Cut;

    #[test]
    fn audio_filter_aligns_and_cuts() {
        let kept = Kept::new(
            10_000.0,
            &[Cut {
                start_ms: 2000.0,
                end_ms: 4000.0,
            }],
        );
        assert_eq!(
            audio_filter(0.0, &kept),
            "aselect='between(t,0.0000,2.0000)+between(t,4.0000,10.0000)',asetpts=N/SR/TB,apad"
        );
        assert!(audio_filter(120.0, &kept).starts_with("adelay=120:all=1,aselect"));
        assert!(audio_filter(-35.0, &kept)
            .starts_with("atrim=start=0.0350,asetpts=PTS-STARTPTS,aselect"));
    }

    #[test]
    fn partial_file_keeps_extension() {
        assert_eq!(
            part_path(Path::new("/a/b/demo.mp4")),
            Path::new("/a/b/demo.part.mp4")
        );
        assert_eq!(part_path(Path::new("x.gif")), Path::new("x.part.gif"));
    }
}
