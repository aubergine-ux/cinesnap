//! ffmpeg-driven capture processes (screen, microphone, webcam).

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use cinesnap_core::ffmpeg::{self, Encoder, Quality};

/// A running ffmpeg capture. The filter graph must contain `showinfo` (or
/// `ashowinfo`): its first log line tells us when the first frame arrived,
/// which is what every track is synchronised against.
pub struct Grab {
    child: Child,
    first_frame: Arc<OnceLock<Instant>>,
    log: Arc<Mutex<String>>,
}

impl Grab {
    pub fn spawn(mut cmd: Command) -> Result<Self> {
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().context("starting ffmpeg")?;
        let first_frame = Arc::new(OnceLock::new());
        let log = Arc::new(Mutex::new(String::new()));
        let stderr = child.stderr.take().expect("stderr is piped");
        let (first, sink) = (first_frame.clone(), log.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).split(b'\n').map_while(|l| l.ok()) {
                let line = String::from_utf8_lossy(&line);
                // "[Parsed_showinfo_1 @ 0x…] n: 0 pts: … pts_time:0 …"
                if line.contains("showinfo_") || line.contains("showinfo @") {
                    if line.contains("pts_time") {
                        let _ = first.set(Instant::now());
                    }
                    continue;
                }
                let mut s = sink.lock().unwrap();
                if s.len() > 12_000 {
                    let cut = (s.len() - 6_000..s.len())
                        .find(|&i| s.is_char_boundary(i))
                        .unwrap_or(0);
                    s.drain(..cut);
                }
                s.push_str(line.trim_end());
                s.push('\n');
            }
        });
        Ok(Self {
            child,
            first_frame,
            log,
        })
    }

    pub fn first_frame(&self) -> Option<Instant> {
        self.first_frame.get().copied()
    }

    /// The last lines ffmpeg printed, for error messages.
    fn error_tail(&self) -> String {
        let s = self.log.lock().unwrap();
        let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
        lines[lines.len().saturating_sub(4)..].join("\n")
    }

    /// Block until the first frame was captured, or fail with ffmpeg's error.
    pub fn wait_first_frame(&mut self, timeout: Duration) -> Result<Instant> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(t) = self.first_frame() {
                return Ok(t);
            }
            if let Ok(Some(_)) = self.child.try_wait() {
                // Give the reader thread a moment to collect the message.
                std::thread::sleep(Duration::from_millis(50));
                bail!("{}", self.error_tail());
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                bail!("capture did not start in time\n{}", self.error_tail());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Ask ffmpeg to finish the file and wait for it.
    pub fn stop(mut self) -> Result<()> {
        if let Some(mut stdin) = self.child.stdin.take() {
            let _ = stdin.write_all(b"q\n");
            let _ = stdin.flush();
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match self.child.try_wait()? {
                Some(_) => return Ok(()),
                None if Instant::now() > deadline => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    bail!("ffmpeg did not finish writing the recording");
                }
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }

    pub fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A screen (or the part of the desktop) to capture, in desktop pixels.
#[derive(Clone, Debug)]
pub struct ScreenArea {
    /// Position in the OS's screen list (used on Windows and macOS).
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    pub index: usize,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Which grabber to use on Windows; `Dda` is faster, `Gdi` works everywhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WinGrab {
    Dda,
    Gdi,
}

/// Add the platform's screen input to `cmd`; returns filters that must come
/// first in the filter chain.
#[allow(unused_variables)]
fn screen_input(cmd: &mut Command, area: &ScreenArea, fps: u32, win: WinGrab) -> Vec<String> {
    let fps_s = fps.to_string();
    #[cfg(target_os = "linux")]
    {
        let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
        cmd.args(["-f", "x11grab", "-draw_mouse", "0", "-framerate", &fps_s]);
        cmd.args(["-video_size", &format!("{}x{}", area.width, area.height)]);
        cmd.args(["-i", &format!("{display}+{},{}", area.x, area.y)]);
        vec![]
    }
    #[cfg(target_os = "windows")]
    {
        if win == WinGrab::Dda {
            let src = format!(
                "ddagrab=output_idx={}:draw_mouse=0:framerate={fps}",
                area.index
            );
            cmd.args(["-f", "lavfi", "-i", &src]);
            vec!["hwdownload".into(), "format=bgra".into()]
        } else {
            cmd.args(["-f", "gdigrab", "-draw_mouse", "0", "-framerate", &fps_s]);
            cmd.args([
                "-offset_x",
                &area.x.to_string(),
                "-offset_y",
                &area.y.to_string(),
            ]);
            cmd.args([
                "-video_size",
                &format!("{}x{}", area.width, area.height),
                "-i",
                "desktop",
            ]);
            vec![]
        }
    }
    #[cfg(target_os = "macos")]
    {
        cmd.args([
            "-f",
            "avfoundation",
            "-capture_cursor",
            "0",
            "-framerate",
            &fps_s,
        ]);
        cmd.args(["-i", &format!("Capture screen {}:none", area.index)]);
        vec![]
    }
}

/// Record a screen to `out` as a constant-frame-rate H.264 intermediate.
pub fn screen(
    ffmpeg: &Path,
    encoder: Encoder,
    area: &ScreenArea,
    fps: u32,
    out: &Path,
    win: WinGrab,
) -> Result<Grab> {
    let mut cmd = ffmpeg::command_interactive(ffmpeg);
    cmd.args(["-nostats", "-loglevel", "info", "-y"])
        .args(encoder.global_args());
    let mut filters = screen_input(&mut cmd, area, fps, win);
    filters.push("crop=trunc(iw/2)*2:trunc(ih/2)*2".into());
    filters.push("showinfo=checksum=0".into());
    filters.extend(encoder.filter().map(String::from));
    cmd.args(["-vf", &filters.join(",")]);
    cmd.args(encoder.codec_args(Quality::Master, true));
    // One keyframe per second keeps scrubbing in the editor responsive.
    cmd.args([
        "-fps_mode",
        "cfr",
        "-r",
        &fps.to_string(),
        "-g",
        &fps.to_string(),
    ])
    .arg(out);
    Grab::spawn(cmd)
}

pub fn microphone(ffmpeg: &Path, device: &str, out: &Path) -> Result<Grab> {
    let mut cmd = ffmpeg::command_interactive(ffmpeg);
    cmd.args(["-nostats", "-loglevel", "info", "-y"]);
    #[cfg(target_os = "linux")]
    cmd.args(["-f", "pulse", "-i", device]);
    #[cfg(target_os = "macos")]
    cmd.args(["-f", "avfoundation", "-i", &format!(":{device}")]);
    #[cfg(target_os = "windows")]
    cmd.args(["-f", "dshow", "-i", &format!("audio={device}")]);
    cmd.args(["-af", "ashowinfo", "-c:a", "pcm_s16le", "-ar", "48000"])
        .arg(out);
    Grab::spawn(cmd)
}

pub fn webcam(ffmpeg: &Path, device: &str, out: &Path) -> Result<Grab> {
    let mut cmd = ffmpeg::command_interactive(ffmpeg);
    cmd.args(["-nostats", "-loglevel", "info", "-y"]);
    #[cfg(target_os = "linux")]
    cmd.args(["-f", "v4l2", "-i", device]);
    #[cfg(target_os = "macos")]
    cmd.args([
        "-f",
        "avfoundation",
        "-framerate",
        "30",
        "-i",
        &format!("{device}:none"),
    ]);
    #[cfg(target_os = "windows")]
    cmd.args(["-f", "dshow", "-i", &format!("video={device}")]);
    cmd.args(["-vf", "showinfo=checksum=0,scale=-2:'min(720,ih)'"]);
    cmd.args([
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-crf",
        "20",
        "-pix_fmt",
        "yuv420p",
    ]);
    cmd.args(["-fps_mode", "cfr", "-r", "30"]).arg(out);
    Grab::spawn(cmd)
}
