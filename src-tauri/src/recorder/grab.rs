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

/// A capture that is dropped without `stop` (an error or a panic further up)
/// must not leave ffmpeg recording in the background.
impl Drop for Grab {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// A screen to capture, in video pixels. Which fields are used depends on
/// the platform's grabber.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct ScreenArea {
    /// The grabber's number for the screen: the Desktop Duplication output
    /// on Windows, the avfoundation screen on macOS.
    pub index: usize,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// The screen is in HDR mode (Windows): captured in 10 bit, converted.
    pub hdr: bool,
    /// Where SDR content sits on it in HDR mode.
    pub sdr_white_nits: f64,
}

/// The rectangle around all `areas`: `(x, y, width, height)`.
pub fn bounds(areas: &[ScreenArea]) -> (i32, i32, u32, u32) {
    let x0 = areas.iter().map(|a| a.x).min().unwrap_or(0);
    let y0 = areas.iter().map(|a| a.y).min().unwrap_or(0);
    let x1 = areas
        .iter()
        .map(|a| a.x + a.width as i32)
        .max()
        .unwrap_or(2);
    let y1 = areas
        .iter()
        .map(|a| a.y + a.height as i32)
        .max()
        .unwrap_or(2);
    (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
}

/// Which grabber to use on Windows; `Dda` is faster, `Gdi` works everywhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WinGrab {
    Dda,
    Gdi,
}

/// One grabber: its ffmpeg input arguments, and the filters its frames go
/// through before anything else. `at` is where it goes in the picture.
#[allow(dead_code)]
struct Input {
    args: Vec<String>,
    filters: Vec<String>,
    at: (i32, i32),
}

/// Where the HDR conversion's lookup tables are kept. ffmpeg runs there so
/// filter arguments can name them without escaping a Windows path.
pub fn lut_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("cinesnap-luts")
}

fn owned(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[allow(unused_variables)]
fn inputs(areas: &[ScreenArea], fps: u32, win: WinGrab) -> Result<Vec<Input>> {
    let fps_s = fps.to_string();
    let (bx, by, bw, bh) = bounds(areas);
    #[cfg(target_os = "linux")]
    {
        // X11 is one big framebuffer: grab the rectangle around the screens.
        let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
        Ok(vec![Input {
            args: owned(&[
                "-f",
                "x11grab",
                "-draw_mouse",
                "0",
                "-framerate",
                &fps_s,
                "-video_size",
                &format!("{bw}x{bh}"),
                "-i",
                &format!("{display}+{bx},{by}"),
            ]),
            filters: vec![],
            at: (0, 0),
        }])
    }
    #[cfg(target_os = "windows")]
    {
        if win == WinGrab::Gdi {
            return Ok(vec![Input {
                args: owned(&[
                    "-f",
                    "gdigrab",
                    "-draw_mouse",
                    "0",
                    "-framerate",
                    &fps_s,
                    "-offset_x",
                    &bx.to_string(),
                    "-offset_y",
                    &by.to_string(),
                    "-video_size",
                    &format!("{bw}x{bh}"),
                    "-i",
                    "desktop",
                ]),
                filters: vec![],
                at: (0, 0),
            }]);
        }
        areas
            .iter()
            .map(|a| -> Result<Input> {
                let fmt = if a.hdr { ":output_fmt=x2bgr10" } else { "" };
                let src = format!(
                    "ddagrab=output_idx={}:draw_mouse=0:framerate={fps}{fmt}",
                    a.index
                );
                let mut filters = owned(&["hwdownload"]);
                if a.hdr {
                    filters.push("format=x2bgr10le".into());
                    filters.extend(cinesnap_core::tonemap::filters(
                        &lut_dir(),
                        a.sdr_white_nits,
                    )?);
                } else {
                    filters.push("format=bgra".into());
                }
                Ok(Input {
                    args: owned(&["-f", "lavfi", "-i", &src]),
                    filters,
                    at: (a.x - bx, a.y - by),
                })
            })
            .collect()
    }
    #[cfg(target_os = "macos")]
    {
        Ok(areas
            .iter()
            .map(|a| Input {
                args: owned(&[
                    "-f",
                    "avfoundation",
                    "-capture_cursor",
                    "0",
                    "-framerate",
                    &fps_s,
                    "-i",
                    &format!("Capture screen {}:none", a.index),
                ]),
                filters: vec![],
                at: (a.x - bx, a.y - by),
            })
            .collect())
    }
}

/// The filter graph for `inputs` (each a filter chain, possibly empty),
/// laid out at `at` and followed by `post`. One input is a plain `-vf`
/// chain; several are stacked with `xstack` into the output `[v]`.
fn graph(inputs: &[(Vec<String>, (i32, i32))], post: &[String]) -> (bool, String) {
    let chain = |f: &[String]| {
        if f.is_empty() {
            "null".to_owned()
        } else {
            f.join(",")
        }
    };
    if let [(filters, _)] = inputs {
        let all: Vec<String> = filters.iter().chain(post).cloned().collect();
        return (false, chain(&all));
    }
    let mut g = String::new();
    for (i, (filters, _)) in inputs.iter().enumerate() {
        g += &format!("[{i}:v]{}[s{i}];", chain(filters));
    }
    for i in 0..inputs.len() {
        g += &format!("[s{i}]");
    }
    let layout: Vec<String> = inputs
        .iter()
        .map(|(_, (x, y))| format!("{x}_{y}"))
        .collect();
    g += &format!(
        "xstack=inputs={}:layout={}:fill=black",
        inputs.len(),
        layout.join("|")
    );
    if !post.is_empty() {
        g += ",";
        g += &post.join(",");
    }
    g += "[v]";
    (true, g)
}

/// Record screens to `out` as an H.264 intermediate, at a constant frame
/// rate or, with `vfr`, dropping frames where nothing changed.
pub fn screen(
    ffmpeg: &Path,
    encoder: Encoder,
    areas: &[ScreenArea],
    fps: u32,
    vfr: bool,
    out: &Path,
    win: WinGrab,
) -> Result<Grab> {
    let mut cmd = ffmpeg::command_interactive(ffmpeg);
    cmd.args(["-nostats", "-loglevel", "info", "-y"])
        .args(encoder.global_args());
    let inputs = inputs(areas, fps, win)?;
    if areas.iter().any(|a| a.hdr) {
        cmd.current_dir(lut_dir());
    }
    for i in &inputs {
        cmd.args(&i.args);
    }
    let mut post = vec![
        "crop=trunc(iw/2)*2:trunc(ih/2)*2".to_string(),
        "showinfo=checksum=0".into(),
    ];
    if vfr {
        post.extend(encoder.decimate(fps));
    }
    post.extend(encoder.filter().map(String::from));
    let chains: Vec<_> = inputs.into_iter().map(|i| (i.filters, i.at)).collect();
    let (complex, filters) = graph(&chains, &post);
    if complex {
        cmd.args(["-filter_complex", &filters, "-map", "[v]"]);
    } else {
        cmd.args(["-vf", &filters]);
    }
    cmd.args(encoder.codec_args(Quality::Master, true));
    cmd.args(timing(fps, vfr)).arg(out);
    Grab::spawn(cmd)
}

/// Output frame timing. Keyframes once a second either way, so scrubbing
/// in the editor stays responsive.
pub fn timing(fps: u32, vfr: bool) -> Vec<String> {
    let fps = fps.to_string();
    if vfr {
        owned(&[
            "-fps_mode",
            "vfr",
            "-force_key_frames",
            "expr:gte(t,n_forced)",
        ])
    } else {
        owned(&["-fps_mode", "cfr", "-r", &fps, "-g", &fps])
    }
}

/// What the computer plays, from a PulseAudio/PipeWire monitor source.
#[cfg(target_os = "linux")]
pub fn system_audio(ffmpeg: &Path, source: &str, out: &Path) -> Result<Grab> {
    let mut cmd = ffmpeg::command_interactive(ffmpeg);
    cmd.args(["-nostats", "-loglevel", "info", "-y"]);
    cmd.args(["-f", "pulse", "-i", source]);
    cmd.args([
        "-af",
        "ashowinfo",
        "-c:a",
        "pcm_s16le",
        "-ar",
        "48000",
        "-ac",
        "2",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        owned(s)
    }

    #[test]
    fn one_screen_is_a_plain_chain() {
        let (complex, g) = graph(&[(vec![], (0, 0))], &v(&["crop=2:2", "showinfo"]));
        assert!(!complex);
        assert_eq!(g, "crop=2:2,showinfo");
        let (_, g) = graph(&[(vec![], (0, 0))], &[]);
        assert_eq!(g, "null");
    }

    #[test]
    fn several_screens_are_stacked_where_they_are() {
        let inputs = [
            (v(&["hwdownload", "format=bgra"]), (0, 120)),
            (vec![], (2560, 0)),
        ];
        let (complex, g) = graph(&inputs, &v(&["showinfo"]));
        assert!(complex);
        assert_eq!(
            g,
            "[0:v]hwdownload,format=bgra[s0];[1:v]null[s1];[s0][s1]\
             xstack=inputs=2:layout=0_120|2560_0:fill=black,showinfo[v]"
        );
    }

    #[test]
    fn bounds_cover_every_screen() {
        let a = |x, y, width, height| ScreenArea {
            index: 0,
            x,
            y,
            width,
            height,
            hdr: false,
            sdr_white_nits: 203.0,
        };
        assert_eq!(bounds(&[a(0, 0, 1920, 1080)]), (0, 0, 1920, 1080));
        assert_eq!(
            bounds(&[a(0, 200, 1920, 1080), a(-2560, 0, 2560, 1440)]),
            (-2560, 0, 4480, 1440)
        );
    }

    #[test]
    fn vfr_timing_forces_a_keyframe_every_second() {
        assert_eq!(
            timing(60, false),
            v(&["-fps_mode", "cfr", "-r", "60", "-g", "60"])
        );
        assert_eq!(
            timing(60, true),
            v(&[
                "-fps_mode",
                "vfr",
                "-force_key_frames",
                "expr:gte(t,n_forced)"
            ])
        );
    }
}
