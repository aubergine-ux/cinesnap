//! Command-line front end for the Cinesnap core: render projects without the
//! GUI, regenerate zooms, and create a synthetic demo recording.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result};
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
        /// 4k, 1440p, 1080p, 720p, readme (GIF, 800 px wide) or small (GIF, 480 px)
        #[arg(long)]
        preset: Option<String>,
        #[arg(long)]
        fps: Option<u32>,
        /// low, medium or high
        #[arg(long)]
        quality: Option<String>,
        /// 16:9, 9:16, 1:1, 4:3 or source
        #[arg(long)]
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
            let ffmpeg = find_ffmpeg()?;
            let mut project = Project::load(&dir)?;
            let gif = out
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("gif"));
            project.export.format = if gif { "gif" } else { "mp4" }.into();
            if gif {
                project.export.preset = "readme".into();
                project.export.fps = 15;
            }
            if let Some(p) = preset {
                project.export.preset = p;
            }
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
