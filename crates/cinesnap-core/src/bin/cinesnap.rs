//! Command-line front end for the Cinesnap core: render projects without the
//! GUI, regenerate zooms, and create a synthetic demo recording.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use cinesnap_core::events::EventLog;
use cinesnap_core::export::{export, ExportOptions};
use cinesnap_core::ffmpeg::{self, Encoder};
use cinesnap_core::motion::auto_zoom;
use cinesnap_core::project::Project;
use cinesnap_core::synth::{generate, SynthOptions};

#[derive(Parser)]
#[command(
    name = "cinesnap",
    version,
    about = "Render Cinesnap recordings from the command line"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Render a .cinesnap project to MP4 or GIF
    Render {
        project: PathBuf,
        /// Output file; the extension (.mp4 / .gif) picks the format
        #[arg(short, long)]
        out: PathBuf,
        /// MP4: 4k, 1440p, 1080p or 720p. GIF: readme (800 px wide) or small (480 px)
        #[arg(long)]
        preset: Option<String>,
        /// Frames per second (MP4 only; GIF presets set their own)
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=120))]
        fps: Option<u32>,
        #[arg(long, value_parser = ["low", "medium", "high"])]
        quality: Option<String>,
        /// 16:9, 9:16, 1:1, 4:3, any other W:H, or source
        #[arg(long, value_parser = parse_aspect)]
        aspect: Option<String>,
        /// Use the software encoder even if a hardware one is available
        #[arg(long)]
        software: bool,
    },
    /// Regenerate automatic zoom segments and save the project
    Autozoom { project: PathBuf },
    /// Create a synthetic demo recording
    Demo {
        dir: PathBuf,
        #[arg(long, default_value_t = 1920)]
        width: u32,
        #[arg(long, default_value_t = 60)]
        fps: u32,
        /// Bake the raw cursor into the video (a plain, unpolished recording)
        #[arg(long)]
        baked_cursor: bool,
    },
    /// Show which ffmpeg and encoder would be used
    Doctor,
}

fn parse_aspect(s: &str) -> Result<String, String> {
    let ratio = |(w, h): (&str, &str)| Some((w.parse::<f64>().ok()?, h.parse::<f64>().ok()?));
    match s.split_once(':').and_then(ratio) {
        _ if s == "source" => Ok(s.into()),
        Some((w, h)) if w > 0.0 && h > 0.0 && (0.2..=5.0).contains(&(w / h)) => Ok(s.into()),
        _ => Err("expected W:H (for example 16:9) or `source`".into()),
    }
}

/// The output format, from the file extension.
fn is_gif(out: &Path) -> Result<bool> {
    match out.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case("gif") => Ok(true),
        Some(e) if e.eq_ignore_ascii_case("mp4") => Ok(false),
        _ => bail!("{}: the output must end in .mp4 or .gif", out.display()),
    }
}

fn check_preset(preset: &str, gif: bool) -> Result<()> {
    let known: &[&str] = if gif {
        &["readme", "small"]
    } else {
        &["4k", "1440p", "1080p", "720p"]
    };
    if !known.contains(&preset) {
        bail!(
            "unknown {} preset `{preset}` (expected one of: {})",
            if gif { "GIF" } else { "MP4" },
            known.join(", ")
        );
    }
    Ok(())
}

fn find_ffmpeg() -> Result<PathBuf> {
    ffmpeg::locate(None).context("ffmpeg not found; install it or set CINESNAP_FFMPEG")
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Cmd::Render {
            project: dir,
            out,
            preset,
            fps,
            quality,
            aspect,
            software,
        } => {
            let gif = is_gif(&out)?;
            if let Some(p) = &preset {
                check_preset(p, gif)?;
            }
            let ffmpeg = find_ffmpeg()?;
            let mut project = Project::load(&dir)?;
            project.export.format = if gif { "gif" } else { "mp4" }.into();
            // The preset saved in the project belongs to the format it was
            // last exported in, which may not be this one.
            project.export.preset = preset.unwrap_or_else(|| match &*project.export.preset {
                p if check_preset(p, gif).is_ok() => p.into(),
                _ if gif => "readme".into(),
                _ => "1080p".into(),
            });
            if let Some(f) = fps {
                project.export.fps = f;
            }
            if let Some(q) = quality {
                project.export.quality = q;
            }
            if let Some(a) = aspect {
                project.frame.aspect = a;
            }
            let opts = ExportOptions {
                ffmpeg: &ffmpeg,
                out: &out,
                encoder: software.then_some(Encoder::X264),
            };
            let cancel = AtomicBool::new(false);
            let mut last = usize::MAX;
            export(
                &dir,
                &project,
                &opts,
                &mut |p| {
                    let pct = p.frame * 100 / p.total.max(1);
                    if pct != last {
                        last = pct;
                        eprint!("\r{:<8} {pct:>3}%", p.stage);
                        let _ = std::io::stderr().flush();
                    }
                },
                &cancel,
            )?;
            eprintln!("\rwrote {}", out.display());
        }
        Cmd::Autozoom { project: dir } => {
            let mut project = Project::load(&dir)?;
            let log = EventLog::load(&project.events_path(&dir))?;
            project.zooms = auto_zoom(&project, &log);
            project.save(&dir)?;
            println!("{} zoom segments", project.zooms.len());
        }
        Cmd::Demo {
            dir,
            width,
            fps,
            baked_cursor,
        } => {
            let ffmpeg = find_ffmpeg()?;
            let height = (width as f64 * 9.0 / 16.0 / 2.0).round() as u32 * 2;
            let p = generate(
                &dir,
                &ffmpeg,
                &SynthOptions {
                    width,
                    height,
                    fps,
                    baked_cursor,
                },
            )?;
            println!("{} ({} zoom segments)", dir.display(), p.zooms.len());
        }
        Cmd::Doctor => {
            let ffmpeg = find_ffmpeg()?;
            println!(
                "ffmpeg   {} ({})",
                ffmpeg.display(),
                ffmpeg::version(&ffmpeg).unwrap_or_default()
            );
            println!("encoder  {}", Encoder::detect(&ffmpeg).name());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_argument() {
        for ok in ["16:9", "9:16", "1:1", "4:3", "21:9", "source"] {
            assert!(parse_aspect(ok).is_ok(), "{ok}");
        }
        for bad in [
            "", "banana", "16x9", "0:9", "16:0", "-16:9", "100:1", "nan:1",
        ] {
            assert!(parse_aspect(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn output_format_and_presets() {
        assert!(is_gif(Path::new("a.GIF")).unwrap());
        assert!(!is_gif(Path::new("a.mp4")).unwrap());
        assert!(is_gif(Path::new("a.webm")).is_err());
        assert!(is_gif(Path::new("noext")).is_err());
        assert!(check_preset("readme", true).is_ok());
        assert!(check_preset("readme", false).is_err());
        assert!(check_preset("720p", false).is_ok());
        assert!(check_preset("720p", true).is_err());
    }
}
