//! Command-line front end: render projects without the GUI, regenerate
//! zooms, and create a synthetic demo recording. Shared by the standalone
//! `cinesnap-cli` binary and the desktop app (`cinesnap render …`).

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use crate::export::{export, ExportOptions};
use crate::ffmpeg::{self, Encoder};
use crate::motion::{auto_zoom, load_logs, Kept};
use crate::project::Project;
use crate::project::TransitionKind;
use crate::synth::{generate, SynthOptions};
use crate::transcript::{caption_lines, srt, vtt};
use crate::{clips, presets, share, transcribe};

/// The subcommands [`run`] understands.
pub const COMMANDS: [&str; 9] = [
    "render",
    "autozoom",
    "demo",
    "doctor",
    "add",
    "transcribe",
    "captions",
    "share",
    "music",
];

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
        /// Output file; the extension (.mp4 / .gif) picks the format.
        /// Defaults to the project's name in the current folder
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// readme (GIF under 10 MB), readme-mp4, x, youtube, shorts, 4k,
        /// 1440p, 1080p, 720p, or small (GIF, 480 px)
        #[arg(long)]
        preset: Option<String>,
        /// Frames per second, for presets that do not set their own
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
    /// Append another recording to a project's timeline
    Add {
        project: PathBuf,
        recording: PathBuf,
        /// How the new clip comes in
        #[arg(long, default_value = "crossfade", value_parser = ["cut", "crossfade", "zoom"])]
        transition: String,
    },
    /// Transcribe the microphone track with a local speech model
    Transcribe {
        project: PathBuf,
        /// tiny.en, base.en, small.en, base, small, or the path of a ggml model file
        #[arg(long, default_value = "base.en")]
        model: String,
        /// Spoken language as a code such as en or de; detected when omitted
        #[arg(long)]
        language: Option<String>,
    },
    /// Write the captions of a transcribed project as .srt or .vtt
    Captions {
        project: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Upload a rendered video to a Cinesnap share server
    Share {
        file: PathBuf,
        #[arg(long, env = "CINESNAP_SHARE_SERVER")]
        server: String,
        #[arg(long, env = "CINESNAP_SHARE_TOKEN", hide_env_values = true)]
        token: String,
        #[arg(long)]
        title: Option<String>,
    },
    /// List the built-in music tracks
    Music,
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

/// Decide the output file, format and preset from what was given.
fn output_plan(
    project_dir: &Path,
    out: Option<PathBuf>,
    preset: Option<&str>,
    saved: (&str, &str),
) -> Result<(PathBuf, bool, String)> {
    let chosen = match preset {
        Some(id) => Some(presets::find(id).with_context(|| {
            format!(
                "unknown preset `{id}` (expected one of: {})",
                presets::PRESETS.map(|p| p.id).join(", ")
            )
        })?),
        None => None,
    };
    let gif = match (&out, chosen) {
        (Some(out), _) => is_gif(out)?,
        (None, Some(p)) => p.format == "gif",
        (None, None) => saved.0 == "gif",
    };
    let format = if gif { "gif" } else { "mp4" };
    if let Some(p) = chosen.filter(|p| p.format != format) {
        bail!(
            "the {} preset makes a .{} file, not a .{format}",
            p.id,
            p.format
        );
    }
    // The preset saved in the project belongs to the format it was last
    // exported in, which may not be this one.
    let preset = chosen.map_or_else(|| presets::resolve(saved.1, format).id, |p| p.id);
    let out = out.unwrap_or_else(|| {
        let stem = project_dir
            .file_stem()
            .map_or("video".into(), |s| s.to_string_lossy().into_owned());
        PathBuf::from(format!("{stem}.{format}"))
    });
    Ok((out, gif, preset.to_owned()))
}

/// Where the desktop app keeps its data (downloaded ffmpeg, speech models).
pub fn data_dir() -> Option<PathBuf> {
    const ID: &str = "dev.cinesnap.app";
    if let Some(dir) = std::env::var_os("CINESNAP_DATA") {
        return Some(dir.into());
    }
    let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
    if cfg!(windows) {
        var("APPDATA").map(|d| d.join(ID))
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library/Application Support").join(ID))
    } else {
        var("XDG_DATA_HOME")
            .or_else(|| var("HOME").map(|h| h.join(".local/share")))
            .map(|d| d.join(ID))
    }
}

fn percent(label: &str) -> impl FnMut(f64) + '_ {
    let mut last = usize::MAX;
    move |f: f64| {
        let pct = (f * 100.0) as usize;
        if pct != last {
            last = pct;
            eprint!("\r{label:<12} {pct:>3}%");
            let _ = std::io::stderr().flush();
        }
    }
}

fn find_ffmpeg() -> Result<PathBuf> {
    ffmpeg::locate(data_dir().as_deref())
        .context("ffmpeg not found; install it or set CINESNAP_FFMPEG")
}

/// Run the command line in `args` (program name first). Prints usage and
/// exits on a malformed command line, like any clap program.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    match Cli::parse_from(args).command {
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
            let saved = project.export.clone();
            let (out, gif, preset) =
                output_plan(&dir, out, preset.as_deref(), (&saved.format, &saved.preset))?;
            project.export.format = if gif { "gif" } else { "mp4" }.into();
            project.export.preset = preset;
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
            project.zooms = auto_zoom(&project, &load_logs(&dir, &project));
            project.save(&dir)?;
            println!("{} zoom segments", project.zooms.len());
        }
        Cmd::Add {
            project: dir,
            recording,
            transition,
        } => {
            let mut project = Project::load(&dir)?;
            let mut imported = clips::import_recording(&dir, &project, &recording)?;
            imported.clip.transition.kind = match transition.as_str() {
                "cut" => TransitionKind::Cut,
                "zoom" => TransitionKind::Zoom,
                _ => TransitionKind::Crossfade,
            };
            clips::append(&mut project, imported);
            project.save(&dir)?;
            println!(
                "{} clips, {:.1} s",
                project.clips.len(),
                project.duration_ms() / 1000.0
            );
        }
        Cmd::Transcribe {
            project: dir,
            model,
            language,
        } => {
            let ffmpeg = find_ffmpeg()?;
            let mut project = Project::load(&dir)?;
            let cancel = AtomicBool::new(false);
            let model = if transcribe::model(&model).is_some() {
                let models = data_dir()
                    .context("no data folder; pass the path of a model file instead")?
                    .join("models");
                transcribe::download_model(&models, &model, &mut percent("downloading"), &cancel)?
            } else {
                PathBuf::from(model)
            };
            let transcript = transcribe::transcribe(
                &dir,
                &project,
                &ffmpeg,
                &model,
                language.as_deref(),
                &mut percent("transcribing"),
                &cancel,
            )?;
            eprintln!(
                "\r{} words ({})        ",
                transcript.words.len(),
                transcript.language
            );
            project.transcript = Some(transcript);
            project.save(&dir)?;
        }
        Cmd::Captions { project: dir, out } => {
            let project = Project::load(&dir)?;
            let transcript = project
                .transcript
                .as_ref()
                .context("the project has no transcript yet; run `cinesnap transcribe` first")?;
            let kept = Kept::new(project.duration_ms(), &project.cuts);
            let lines = caption_lines(&transcript.words, &kept, project.captions.max_chars);
            let text = match out.extension().and_then(|e| e.to_str()) {
                Some(e) if e.eq_ignore_ascii_case("srt") => srt(&lines),
                Some(e) if e.eq_ignore_ascii_case("vtt") => vtt(&lines),
                _ => bail!("{}: the output must end in .srt or .vtt", out.display()),
            };
            std::fs::write(&out, text).with_context(|| format!("writing {}", out.display()))?;
            eprintln!("wrote {} ({} captions)", out.display(), lines.len());
        }
        Cmd::Share {
            file,
            server,
            token,
            title,
        } => {
            let title = title.unwrap_or_else(|| {
                file.file_stem()
                    .map_or("Untitled".into(), |s| s.to_string_lossy().into_owned())
            });
            let cancel = AtomicBool::new(false);
            let shared = share::upload(
                &server,
                &token,
                &file,
                &title,
                &mut percent("uploading"),
                &cancel,
            )?;
            eprintln!();
            println!("{}", shared.url);
            eprintln!("embed: {}", shared.embed_html);
        }
        Cmd::Music => {
            for t in crate::music::tracks() {
                println!(
                    "builtin:{:<14} {:<14} {} ({} bpm, {} s)",
                    t.id,
                    t.name,
                    t.mood,
                    t.bpm,
                    t.duration_ms / 1000
                );
            }
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
    fn output_plans() {
        let dir = Path::new("/x/My demo.cinesnap");
        let plan = |out: Option<&str>, preset: Option<&str>, saved: (&str, &str)| {
            output_plan(dir, out.map(PathBuf::from), preset, saved)
                .map(|(o, gif, p)| (o.to_string_lossy().into_owned(), gif, p))
        };
        assert!(is_gif(Path::new("a.GIF")).unwrap());
        assert!(is_gif(Path::new("a.webm")).is_err());
        // The preset alone names the format and the file.
        assert_eq!(
            plan(None, Some("readme"), ("mp4", "1080p")).unwrap(),
            ("My demo.gif".into(), true, "readme".into())
        );
        assert_eq!(
            plan(None, Some("tiktok"), ("gif", "readme")).unwrap(),
            ("My demo.mp4".into(), false, "shorts".into())
        );
        // The extension decides; a saved preset of the other format is dropped.
        assert_eq!(
            plan(Some("o.mp4"), None, ("gif", "readme")).unwrap(),
            ("o.mp4".into(), false, "1080p".into())
        );
        assert_eq!(
            plan(None, None, ("mp4", "720p")).unwrap(),
            ("My demo.mp4".into(), false, "720p".into())
        );
        assert!(plan(Some("o.mp4"), Some("readme"), ("mp4", "1080p")).is_err());
        assert!(plan(Some("o.gif"), Some("720p"), ("mp4", "1080p")).is_err());
        assert!(plan(None, Some("banana"), ("mp4", "1080p")).is_err());
        assert!(plan(Some("noext"), None, ("mp4", "1080p")).is_err());
    }
}
