//! Locating and driving the ffmpeg executable.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};

pub fn exe_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

/// Find ffmpeg: `CINESNAP_FFMPEG`, then `data_dir` (where the app downloads
/// its own copy), then `PATH`.
pub fn locate(data_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CINESNAP_FFMPEG").map(PathBuf::from) {
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(p) = data_dir.map(|d| d.join(exe_name())) {
        if p.is_file() {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(exe_name())).find(|p| p.is_file())
}

/// A `Command` for ffmpeg that never pops up a console window on Windows.
pub fn command(ffmpeg: &Path) -> Command {
    let mut c = Command::new(ffmpeg);
    c.arg("-hide_banner").arg("-nostdin");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

pub fn version(ffmpeg: &Path) -> Option<String> {
    let out = command(ffmpeg).arg("-version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().next()?.strip_prefix("ffmpeg version ")?.split_whitespace().next().map(str::to_owned)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoder {
    Nvenc,
    VideoToolbox,
    Qsv,
    Amf,
    Vaapi,
    X264,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Low,
    Medium,
    High,
    /// Visually lossless; used for the recording intermediate.
    Master,
}

impl Quality {
    pub fn parse(s: &str) -> Self {
        match s {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "master" => Self::Master,
            _ => Self::High,
        }
    }
    /// A CRF-like quantiser value (lower is better).
    fn q(self) -> u32 {
        match self {
            Self::Low => 29,
            Self::Medium => 24,
            Self::High => 19,
            Self::Master => 14,
        }
    }
}

impl Encoder {
    pub fn name(self) -> &'static str {
        match self {
            Self::Nvenc => "h264_nvenc",
            Self::VideoToolbox => "h264_videotoolbox",
            Self::Qsv => "h264_qsv",
            Self::Amf => "h264_amf",
            Self::Vaapi => "h264_vaapi",
            Self::X264 => "libx264",
        }
    }

    pub fn is_hardware(self) -> bool {
        self != Self::X264
    }

    fn candidates() -> &'static [Encoder] {
        if cfg!(target_os = "macos") {
            &[Self::VideoToolbox]
        } else if cfg!(windows) {
            &[Self::Nvenc, Self::Qsv, Self::Amf]
        } else {
            &[Self::Nvenc, Self::Vaapi, Self::Qsv]
        }
    }

    /// Arguments that go before the inputs.
    pub fn global_args(self) -> Vec<String> {
        match self {
            Self::Vaapi => vec!["-vaapi_device".into(), vaapi_device()],
            _ => vec![],
        }
    }

    /// The video filter that must end the filter chain, if any.
    pub fn filter(self) -> Option<&'static str> {
        match self {
            Self::Vaapi => Some("format=nv12,hwupload"),
            _ => None,
        }
    }

    /// Codec arguments for the output. `fast` favours encoding speed (live
    /// recording) over compression.
    pub fn codec_args(self, quality: Quality, fast: bool) -> Vec<String> {
        let q = quality.q().to_string();
        let v: Vec<&str> = match self {
            Self::Nvenc => vec![
                "-c:v", "h264_nvenc", "-preset", if fast { "p2" } else { "p5" }, "-rc", "vbr", "-cq", &q, "-b:v", "0",
                "-pix_fmt", "yuv420p",
            ],
            Self::VideoToolbox => {
                // VideoToolbox quality runs 1..100, higher is better.
                let vq = (100 - quality.q() * 2).to_string();
                return ["-c:v", "h264_videotoolbox", "-q:v", &vq, "-pix_fmt", "yuv420p"].map(String::from).to_vec();
            }
            Self::Qsv => vec!["-c:v", "h264_qsv", "-global_quality", &q, "-pix_fmt", "nv12"],
            Self::Amf => vec![
                "-c:v", "h264_amf", "-rc", "cqp", "-qp_i", &q, "-qp_p", &q, "-qp_b", &q, "-pix_fmt", "yuv420p",
            ],
            Self::Vaapi => vec!["-c:v", "h264_vaapi", "-qp", &q],
            Self::X264 => vec![
                "-c:v", "libx264", "-preset", if fast { "ultrafast" } else { "medium" }, "-crf", &q, "-pix_fmt",
                "yuv420p",
            ],
        };
        v.into_iter().map(String::from).collect()
    }

    /// Try a tiny real encode. Being listed by `ffmpeg -encoders` only means
    /// ffmpeg was built with it, not that the GPU and driver work.
    pub fn works(self, ffmpeg: &Path) -> bool {
        let mut c = command(ffmpeg);
        c.args(["-v", "error"]).args(self.global_args());
        c.args(["-f", "lavfi", "-i", "color=black:s=640x360:r=30", "-frames:v", "5"]);
        if let Some(f) = self.filter() {
            c.args(["-vf", f]);
        }
        c.args(self.codec_args(Quality::Medium, true)).args(["-f", "null", "-"]);
        c.stdout(Stdio::null()).stderr(Stdio::null());
        c.status().map(|s| s.success()).unwrap_or(false)
    }

    /// The best working H.264 encoder on this machine.
    pub fn detect(ffmpeg: &Path) -> Encoder {
        if let Ok(forced) = std::env::var("CINESNAP_ENCODER") {
            let all = [Self::Nvenc, Self::VideoToolbox, Self::Qsv, Self::Amf, Self::Vaapi, Self::X264];
            if let Some(e) = all.into_iter().find(|e| e.name() == forced) {
                return e;
            }
        }
        Self::candidates().iter().copied().find(|e| e.works(ffmpeg)).unwrap_or(Self::X264)
    }
}

fn vaapi_device() -> String {
    std::env::var("CINESNAP_VAAPI_DEVICE").unwrap_or_else(|_| "/dev/dri/renderD128".into())
}

/// Collects a child's stderr so failures can be reported with ffmpeg's own
/// message.
#[derive(Clone, Default)]
pub struct ErrorLog(Arc<Mutex<String>>);

impl ErrorLog {
    pub fn attach(child: &mut Child) -> Self {
        let log = ErrorLog::default();
        if let Some(mut err) = child.stderr.take() {
            let sink = log.0.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = err.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut s = sink.lock().unwrap();
                    s.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if s.len() > 16_000 {
                        let cut = s.len() - 8_000;
                        let cut = (cut..s.len()).find(|&i| s.is_char_boundary(i)).unwrap_or(0);
                        s.drain(..cut);
                    }
                }
            });
        }
        log
    }

    pub fn text(&self) -> String {
        self.0.lock().unwrap().trim().to_owned()
    }
}

/// Run to completion, failing with ffmpeg's stderr.
pub fn run(mut c: Command, what: &str) -> Result<()> {
    let out = c.stdout(Stdio::null()).stderr(Stdio::piped()).output().with_context(|| format!("starting ffmpeg ({what})"))?;
    if !out.status.success() {
        bail!("ffmpeg failed ({what}): {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_ms: f64,
}

/// Read size, frame rate and duration of a video file by parsing
/// `ffmpeg -i` (ffprobe is not always shipped next to ffmpeg).
pub fn probe(ffmpeg: &Path, file: &Path) -> Result<MediaInfo> {
    let out = command(ffmpeg).arg("-i").arg(file).output().context("starting ffmpeg")?;
    let text = String::from_utf8_lossy(&out.stderr);
    parse_probe(&text).with_context(|| format!("could not read video info from {}", file.display()))
}

fn parse_probe(text: &str) -> Option<MediaInfo> {
    let dur = text.split("Duration: ").nth(1)?.split(',').next()?.trim();
    let mut parts = dur.split(':');
    let (h, m, s) = (parts.next()?.parse::<f64>().ok()?, parts.next()?.parse::<f64>().ok()?, parts.next()?.parse::<f64>().ok()?);
    let line = text.lines().find(|l| l.contains("Video:"))?;
    let (mut size, mut fps) = (None, None);
    for tok in line.split(',') {
        let tok = tok.trim();
        if size.is_none() {
            // "1920x1080" possibly followed by " [SAR ...]"
            if let Some((w, h)) = tok.split_whitespace().next().and_then(|t| t.split_once('x')) {
                if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                    size = Some((w, h));
                }
            }
        }
        if let Some(v) = tok.strip_suffix(" fps") {
            fps = v.parse::<f64>().ok();
        }
    }
    let (width, height) = size?;
    Some(MediaInfo { width, height, fps: fps?, duration_ms: (h * 3600.0 + m * 60.0 + s) * 1000.0 })
}

/// Sequential raw-RGBA frame reader over an ffmpeg decode process.
pub struct FrameReader {
    child: Child,
    out: ChildStdout,
    errors: ErrorLog,
    frame: Vec<u8>,
    /// Index of the frame currently in `frame`, if any.
    index: Option<u64>,
    eof: bool,
}

impl FrameReader {
    /// `filter` must produce frames of exactly `width × height` at a constant
    /// frame rate. `seek_ms` skips the start of the file.
    pub fn open(ffmpeg: &Path, file: &Path, filter: &str, width: usize, height: usize, seek_ms: f64) -> Result<Self> {
        let mut c = command(ffmpeg);
        c.args(["-v", "error"]);
        if seek_ms > 0.0 {
            c.args(["-ss", &format!("{:.3}", seek_ms / 1000.0)]);
        }
        c.arg("-i").arg(file);
        c.args(["-an", "-vf", filter, "-pix_fmt", "rgba", "-f", "rawvideo", "-"]);
        c.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = c.spawn().context("starting ffmpeg decoder")?;
        let out = child.stdout.take().unwrap();
        let errors = ErrorLog::attach(&mut child);
        Ok(Self { child, out, errors, frame: vec![0; width * height * 4], index: None, eof: false })
    }

    /// The frame with the given index. Indices must not decrease between
    /// calls. Past the end of the stream the last frame is repeated.
    pub fn frame_at(&mut self, index: u64) -> Result<&[u8]> {
        while !self.eof && self.index.is_none_or(|i| i < index) {
            match self.out.read_exact(&mut self.frame) {
                Ok(()) => self.index = Some(self.index.map_or(0, |i| i + 1)),
                Err(_) => {
                    self.eof = true;
                    if self.index.is_none() {
                        bail!("could not decode video: {}", self.errors.text());
                    }
                }
            }
        }
        Ok(&self.frame)
    }
}

impl Drop for FrameReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// An ffmpeg process fed raw RGBA frames on stdin.
pub struct FrameWriter {
    child: Child,
    stdin: Option<ChildStdin>,
    errors: ErrorLog,
}

impl FrameWriter {
    /// `build` receives a command that already has the raw video input set up
    /// (as input 0) and adds further inputs, codec options and the output.
    pub fn spawn(
        ffmpeg: &Path,
        width: u32,
        height: u32,
        fps: f64,
        global: &[String],
        build: impl FnOnce(&mut Command),
    ) -> Result<Self> {
        let mut c = command(ffmpeg);
        c.args(["-v", "error", "-y"]).args(global);
        c.args(["-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{width}x{height}"), "-r", &format!("{fps}"), "-i", "-"]);
        build(&mut c);
        c.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = c.spawn().context("starting ffmpeg encoder")?;
        let stdin = child.stdin.take();
        let errors = ErrorLog::attach(&mut child);
        Ok(Self { child, stdin, errors })
    }

    pub fn write(&mut self, frame: &[u8]) -> Result<()> {
        let stdin = self.stdin.as_mut().context("encoder already finished")?;
        if stdin.write_all(frame).is_err() {
            // The pipe broke: ffmpeg died. Its stderr says why.
            let _ = self.child.wait();
            bail!("encoder stopped: {}", self.errors.text());
        }
        Ok(())
    }

    /// Close the input and wait for ffmpeg to finish writing the file.
    pub fn finish(mut self) -> Result<()> {
        drop(self.stdin.take());
        let status = self.child.wait().context("waiting for ffmpeg")?;
        if !status.success() {
            bail!("encoding failed: {}", self.errors.text());
        }
        Ok(())
    }

    pub fn abort(mut self) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffmpeg_info() {
        let text = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'video.mp4':\n  Duration: 00:01:02.50, start: 0.000000, bitrate: 812 kb/s\n  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(progressive), 2560x1440 [SAR 1:1 DAR 16:9], 800 kb/s, 60 fps, 60 tbr, 15360 tbn (default)\n";
        assert_eq!(parse_probe(text), Some(MediaInfo { width: 2560, height: 1440, fps: 60.0, duration_ms: 62_500.0 }));
        let ntsc = "  Duration: 00:00:01.00, start\n  Stream #0:0: Video: h264, yuv420p, 640x480, 29.97 fps, 30 tbr\n";
        assert_eq!(parse_probe(ntsc).unwrap().fps, 29.97);
        assert_eq!(parse_probe("garbage"), None);
    }

    #[test]
    fn encoder_args_are_well_formed() {
        for e in [Encoder::Nvenc, Encoder::VideoToolbox, Encoder::Qsv, Encoder::Amf, Encoder::Vaapi, Encoder::X264] {
            let args = e.codec_args(Quality::High, false);
            assert_eq!(args[0], "-c:v");
            assert_eq!(args[1], e.name());
            assert_eq!(args.len() % 2, 0, "{args:?}");
        }
        assert_eq!(Quality::parse("nonsense"), Quality::High);
    }
}
