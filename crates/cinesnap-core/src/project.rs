//! The `.cinesnap` project bundle: manifest and edit state.
//!
//! See `docs/recording-format.md` for the on-disk description.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const PROJECT_FILE: &str = "project.json";
pub const EVENTS_FILE: &str = "events.jsonl";
pub const VIDEO_FILE: &str = "video.mp4";
pub const MIC_FILE: &str = "mic.wav";
pub const WEBCAM_FILE: &str = "webcam.mp4";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub format: String,
    pub version: u32,
    pub created_at: String,
    pub app: String,
    pub source: Source,
    pub crop: Rect,
    #[serde(default)]
    pub cuts: Vec<Cut>,
    #[serde(default)]
    pub zooms: Vec<Zoom>,
    #[serde(default)]
    pub auto_zoom: AutoZoom,
    #[serde(default)]
    pub cursor: CursorStyle,
    #[serde(default)]
    pub frame: Frame,
    #[serde(default)]
    pub webcam: WebcamStyle,
    #[serde(default)]
    pub export: ExportSettings,
    /// Fields written by newer versions; preserved on save.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub video: VideoTrack,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mic: Option<AudioTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webcam: Option<WebcamTrack>,
    pub events: FileRef,
    pub platform: String,
    pub target: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VideoTrack {
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_ms: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrack {
    pub file: String,
    #[serde(default)]
    pub offset_ms: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WebcamTrack {
    pub file: String,
    #[serde(default)]
    pub offset_ms: f64,
    pub width: u32,
    pub height: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileRef {
    pub file: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Cut {
    pub start_ms: f64,
    pub end_ms: f64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Easing {
    #[default]
    Smooth,
    Snappy,
    Linear,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ZoomMode {
    #[default]
    Follow,
    Fixed,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Zoom {
    pub id: String,
    pub start_ms: f64,
    pub end_ms: f64,
    pub scale: f64,
    #[serde(default = "default_ease_in")]
    pub ease_in_ms: f64,
    #[serde(default = "default_ease_out")]
    pub ease_out_ms: f64,
    #[serde(default)]
    pub easing: Easing,
    #[serde(default)]
    pub mode: ZoomMode,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub manual: bool,
}

pub fn default_ease_in() -> f64 {
    600.0
}
pub fn default_ease_out() -> f64 {
    700.0
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoZoom {
    pub enabled: bool,
    pub scale: f64,
    pub merge_gap_ms: f64,
    pub on_typing: bool,
}

impl Default for AutoZoom {
    fn default() -> Self {
        Self { enabled: true, scale: 2.0, merge_gap_ms: 2500.0, on_typing: true }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct CursorStyle {
    pub show: bool,
    pub size: f64,
    pub smoothing: f64,
    pub ripples: bool,
    pub ripple_color: String,
    pub hide_when_idle_ms: f64,
    pub style: String,
}

impl Default for CursorStyle {
    fn default() -> Self {
        Self {
            show: true,
            size: 1.6,
            smoothing: 0.6,
            ripples: true,
            ripple_color: "#ffffff".into(),
            hide_when_idle_ms: 0.0,
            style: "arrow".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Background {
    Solid {
        color: String,
    },
    Gradient {
        from: String,
        to: String,
        #[serde(default = "default_angle")]
        angle: f64,
    },
    Image {
        path: String,
    },
    Wallpaper {
        path: String,
        #[serde(default = "default_blur")]
        blur: f64,
    },
}

fn default_angle() -> f64 {
    135.0
}
fn default_blur() -> f64 {
    40.0
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Frame {
    pub aspect: String,
    pub padding: f64,
    pub radius: f64,
    pub shadow: f64,
    pub background: Background,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            aspect: "16:9".into(),
            padding: 0.06,
            radius: 14.0,
            shadow: 0.55,
            background: Background::Gradient {
                from: "#6d5dfc".into(),
                to: "#e44d8a".into(),
                angle: 135.0,
            },
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct WebcamStyle {
    pub show: bool,
    pub corner: String,
    pub size: f64,
    pub shape: String,
}

impl Default for WebcamStyle {
    fn default() -> Self {
        Self { show: true, corner: "bottom-right".into(), size: 0.18, shape: "circle".into() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportSettings {
    pub format: String,
    pub preset: String,
    pub fps: u32,
    pub quality: String,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self { format: "mp4".into(), preset: "1080p".into(), fps: 60, quality: "high".into() }
    }
}

impl Project {
    /// A fresh project for a just-finished recording, with default edit state.
    pub fn new(source: Source, created_at: String) -> Self {
        let crop = Rect {
            x: 0.0,
            y: 0.0,
            width: source.video.width as f64,
            height: source.video.height as f64,
        };
        Self {
            format: "cinesnap-project".into(),
            version: 1,
            created_at,
            app: env!("CARGO_PKG_VERSION").into(),
            source,
            crop,
            cuts: vec![],
            zooms: vec![],
            auto_zoom: AutoZoom::default(),
            cursor: CursorStyle::default(),
            frame: Frame::default(),
            webcam: WebcamStyle::default(),
            export: ExportSettings::default(),
            extra: Map::new(),
        }
    }

    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(PROJECT_FILE);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mut project: Project =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        anyhow::ensure!(
            project.format == "cinesnap-project",
            "{} is not a Cinesnap project",
            path.display()
        );
        project.sanitize();
        Ok(project)
    }

    /// Atomic save: write to a temp file next to the target, then rename.
    pub fn save(&self, dir: &Path) -> Result<()> {
        let path = dir.join(PROJECT_FILE);
        let tmp = dir.join(format!("{PROJECT_FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// Clamp edit state into a valid range so downstream math never sees
    /// nonsense (hand-edited files, older versions, UI bugs).
    pub fn sanitize(&mut self) {
        let (w, h) = (self.source.video.width as f64, self.source.video.height as f64);
        let c = &mut self.crop;
        c.x = c.x.clamp(0.0, (w - 2.0).max(0.0));
        c.y = c.y.clamp(0.0, (h - 2.0).max(0.0));
        c.width = c.width.clamp(2.0, w - c.x);
        c.height = c.height.clamp(2.0, h - c.y);
        let dur = self.source.video.duration_ms;
        for z in &mut self.zooms {
            z.start_ms = z.start_ms.clamp(0.0, dur);
            z.end_ms = z.end_ms.clamp(z.start_ms, dur);
            z.scale = z.scale.clamp(1.0, 8.0);
            z.ease_in_ms = z.ease_in_ms.max(0.0);
            z.ease_out_ms = z.ease_out_ms.max(0.0);
        }
        self.zooms.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
        self.frame.padding = self.frame.padding.clamp(0.0, 0.4);
        self.frame.shadow = self.frame.shadow.clamp(0.0, 1.0);
        self.frame.radius = self.frame.radius.clamp(0.0, 200.0);
        self.cursor.size = self.cursor.size.clamp(0.2, 6.0);
        self.cursor.smoothing = self.cursor.smoothing.clamp(0.0, 1.0);
        self.export.fps = self.export.fps.clamp(1, 120);
    }

    pub fn video_path(&self, dir: &Path) -> PathBuf {
        dir.join(&self.source.video.file)
    }
    pub fn events_path(&self, dir: &Path) -> PathBuf {
        dir.join(&self.source.events.file)
    }
}

/// Parse `#rgb`, `#rrggbb` or `#rrggbbaa`. Unparseable input yields opaque black.
pub fn parse_color(s: &str) -> [u8; 4] {
    let h = s.trim().trim_start_matches('#');
    let hex = |i: usize, n: usize| u8::from_str_radix(h.get(i..i + n)?, 16).ok();
    let parsed = match h.len() {
        3 => (|| Some([hex(0, 1)? * 17, hex(1, 1)? * 17, hex(2, 1)? * 17, 255]))(),
        6 => (|| Some([hex(0, 2)?, hex(2, 2)?, hex(4, 2)?, 255]))(),
        8 => (|| Some([hex(0, 2)?, hex(2, 2)?, hex(4, 2)?, hex(6, 2)?]))(),
        _ => None,
    };
    parsed.unwrap_or([0, 0, 0, 255])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#fff"), [255, 255, 255, 255]);
        assert_eq!(parse_color("#6d5dfc"), [0x6d, 0x5d, 0xfc, 255]);
        assert_eq!(parse_color("#11223380"), [0x11, 0x22, 0x33, 0x80]);
        assert_eq!(parse_color("nope"), [0, 0, 0, 255]);
    }

    #[test]
    fn unknown_fields_survive_roundtrip() {
        let json = r#"{"format":"cinesnap-project","version":1,"createdAt":"x","app":"0",
            "source":{"video":{"file":"video.mp4","width":100,"height":50,"fps":30,"durationMs":1000},
            "events":{"file":"events.jsonl"},"platform":"test","target":"screen"},
            "crop":{"x":0,"y":0,"width":100,"height":50},"futureThing":{"a":1}}"#;
        let p: Project = serde_json::from_str(json).unwrap();
        assert!(p.auto_zoom.enabled);
        let out = serde_json::to_value(&p).unwrap();
        assert_eq!(out["futureThing"]["a"], 1);
        assert_eq!(out["frame"]["background"]["type"], "gradient");
    }
}
