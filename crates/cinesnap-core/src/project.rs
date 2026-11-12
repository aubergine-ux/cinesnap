//! The `.cinesnap` project bundle: manifest and edit state.
//!
//! See `docs/recording-format.md` for the on-disk description.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const PROJECT_FILE: &str = "project.json";
/// The manifest as it was before the last save.
pub const BACKUP_FILE: &str = "project.json.bak";
pub const PROJECT_VERSION: u32 = 2;
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
    /// Everything on the timeline, in order. Empty in version 1 files, where
    /// the bundle's own recording is the only clip; `load` fills it in.
    #[serde(default)]
    pub clips: Vec<Clip>,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    #[serde(default)]
    pub keystrokes: KeystrokeStyle,
    #[serde(default)]
    pub webcam_keys: Vec<WebcamKey>,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub captions: CaptionStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<Transcript>,
    #[serde(default)]
    pub audio: AudioSettings,
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
        Self {
            enabled: true,
            scale: 2.0,
            merge_gap_ms: 2500.0,
            on_typing: true,
        }
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
    /// Used when `x`/`y` are absent (projects from before free placement).
    pub corner: String,
    /// Centre of the bubble as a fraction of the output, 0..1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// Side as a fraction of the shorter output side.
    pub size: f64,
    /// `circle`, `rounded` or `square`.
    pub shape: String,
}

impl Default for WebcamStyle {
    fn default() -> Self {
        Self {
            show: true,
            corner: "bottom-right".into(),
            x: None,
            y: None,
            size: 0.18,
            shape: "circle".into(),
        }
    }
}

/// Distance of a cornered webcam bubble from the output edge, as a fraction
/// of the shorter side.
pub const WEBCAM_MARGIN: f64 = 0.03;

impl WebcamStyle {
    /// Centre of the bubble (fractions of output width and height) for an
    /// output of the given aspect ratio.
    pub fn center(&self, ratio: f64) -> (f64, f64) {
        if let (Some(x), Some(y)) = (self.x, self.y) {
            return (x, y);
        }
        // Fractions of the shorter side, converted per axis.
        let (kx, ky) = if ratio >= 1.0 {
            (1.0 / ratio, 1.0)
        } else {
            (1.0, ratio)
        };
        let off = WEBCAM_MARGIN + self.size / 2.0;
        let x = if self.corner.ends_with("left") {
            off * kx
        } else {
            1.0 - off * kx
        };
        let y = if self.corner.starts_with("top") {
            off * ky
        } else {
            1.0 - off * ky
        };
        (x, y)
    }
}

/// From `at_ms` (timeline time) on, the webcam bubble moves to this state.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebcamKey {
    pub id: String,
    pub at_ms: f64,
    pub x: f64,
    pub y: f64,
    pub size: f64,
    #[serde(default = "yes")]
    pub show: bool,
}

fn yes() -> bool {
    true
}

// ---- clips -----------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TransitionKind {
    #[default]
    Cut,
    Crossfade,
    Zoom,
}

/// How a clip comes in from the one before it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Transition {
    #[serde(rename = "type")]
    pub kind: TransitionKind,
    pub duration_ms: f64,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            kind: TransitionKind::Cut,
            duration_ms: 500.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    #[serde(default)]
    pub transition: Transition,
    #[serde(flatten)]
    pub body: ClipBody,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ClipBody {
    Recording(RecordingClip),
    Card(Card),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RecordingClip {
    /// Where the clip's files live inside the bundle: empty for the bundle's
    /// own recording, `clips/<id>` for recordings added later.
    #[serde(default)]
    pub dir: String,
    pub source: Source,
    pub crop: Rect,
}

/// A title, section or outro card.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Card {
    pub duration_ms: f64,
    pub title: String,
    pub subtitle: String,
    /// `None` uses the theme font.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    pub title_color: String,
    pub subtitle_color: String,
    /// Title height in px at 1080p.
    pub title_size: f64,
    /// `None` uses the project background.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<Background>,
    /// `center` or `left`.
    pub align: String,
}

impl Default for Card {
    fn default() -> Self {
        Self {
            duration_ms: 3000.0,
            title: "Title".into(),
            subtitle: String::new(),
            font: None,
            title_color: "#ffffff".into(),
            subtitle_color: "#ffffffb3".into(),
            title_size: 96.0,
            background: None,
            align: "center".into(),
        }
    }
}

impl Clip {
    pub fn duration_ms(&self) -> f64 {
        match &self.body {
            ClipBody::Recording(r) => r.source.video.duration_ms,
            ClipBody::Card(c) => c.duration_ms,
        }
    }
    pub fn recording(&self) -> Option<&RecordingClip> {
        match &self.body {
            ClipBody::Recording(r) => Some(r),
            ClipBody::Card(_) => None,
        }
    }
    pub fn card(&self) -> Option<&Card> {
        match &self.body {
            ClipBody::Card(c) => Some(c),
            ClipBody::Recording(_) => None,
        }
    }
}

/// Where a clip sits on the timeline (before cuts).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub index: usize,
    pub start_ms: f64,
    pub duration_ms: f64,
}

impl Span {
    pub fn end_ms(&self) -> f64 {
        self.start_ms + self.duration_ms
    }
}

// ---- annotations -----------------------------------------------------------

/// Something drawn over the recording between two timeline times. Positions
/// are in source pixels of the clip on screen, so annotations stay attached
/// to the content when the camera zooms.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    pub id: String,
    pub start_ms: f64,
    pub end_ms: f64,
    #[serde(flatten)]
    pub shape: Shape,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Shape {
    /// A text callout; `x`, `y` is its top-left corner.
    Text {
        x: f64,
        y: f64,
        text: String,
        /// Text height in px at 1080p.
        size: f64,
        color: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        background: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        font: Option<String>,
        #[serde(default)]
        bold: bool,
    },
    /// Points from `x1,y1` to `x2,y2`.
    Arrow {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        color: String,
        /// Line width in px at 1080p.
        width: f64,
    },
    Box {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        color: String,
        stroke: f64,
        #[serde(default)]
        radius: f64,
    },
    /// A translucent marker over a region.
    Highlight {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        color: String,
        opacity: f64,
    },
    /// Dims everything except the region.
    Spotlight {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        dim: f64,
        #[serde(default)]
        ellipse: bool,
    },
    /// Hides a region.
    Blur {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        /// Blur radius in source pixels.
        strength: f64,
    },
}

/// Fade at both ends of an annotation.
pub const ANNOTATION_FADE_MS: f64 = 180.0;

impl Annotation {
    /// Opacity at a timeline time: 0 outside the range, fading at the ends.
    /// Blur regions do not fade; half-hidden text is still readable.
    pub fn alpha(&self, t: f64) -> f64 {
        if t < self.start_ms || t >= self.end_ms {
            return 0.0;
        }
        if matches!(self.shape, Shape::Blur { .. }) {
            return 1.0;
        }
        let fade = ANNOTATION_FADE_MS
            .min((self.end_ms - self.start_ms) / 2.0)
            .max(1.0);
        ((t - self.start_ms) / fade)
            .min((self.end_ms - t) / fade)
            .clamp(0.0, 1.0)
    }
}

// ---- look ------------------------------------------------------------------

/// Fonts and colours shared by cards, callouts, captions and the keystroke
/// overlay.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Theme {
    pub font: String,
    pub accent: String,
    pub text: String,
    pub surface: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            font: "Inter".into(),
            accent: "#6d5dfc".into(),
            text: "#ffffff".into(),
            surface: "#14151af0".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct KeystrokeStyle {
    pub show: bool,
    /// `bottom`, `top`, `bottom-left` or `bottom-right`.
    pub position: String,
    pub size: f64,
    /// How long a shortcut stays on screen.
    pub hold_ms: f64,
}

impl Default for KeystrokeStyle {
    fn default() -> Self {
        Self {
            show: true,
            position: "bottom".into(),
            size: 1.0,
            hold_ms: 1500.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CaptionStyle {
    pub show: bool,
    /// `karaoke` (spoken word lights up), `pop` (words appear as spoken),
    /// `plain`.
    pub style: String,
    /// Vertical centre as a fraction of the output height.
    pub y: f64,
    pub size: f64,
    pub color: String,
    /// Colour of the word being spoken (`karaoke`).
    pub highlight: String,
    /// Absent means no background (unlike the other fields, which fall
    /// back to their defaults).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Longest caption line, in characters.
    pub max_chars: usize,
    pub uppercase: bool,
}

impl Default for CaptionStyle {
    fn default() -> Self {
        Self {
            show: true,
            style: "karaoke".into(),
            y: 0.82,
            size: 1.0,
            color: "#ffffff".into(),
            highlight: "#ffd84a".into(),
            background: Some("#000000a6".into()),
            font: None,
            max_chars: 36,
            uppercase: false,
        }
    }
}

/// One spoken word, in timeline time.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Word {
    pub t0: f64,
    pub t1: f64,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Transcript {
    pub language: String,
    pub words: Vec<Word>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct VoiceEq {
    pub enabled: bool,
    pub low_cut_hz: f64,
    pub bass_db: f64,
    pub presence_db: f64,
    pub air_db: f64,
}

impl Default for VoiceEq {
    fn default() -> Self {
        Self {
            enabled: false,
            low_cut_hz: 80.0,
            bass_db: 0.0,
            presence_db: 3.0,
            air_db: 1.5,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Music {
    /// `builtin:<id>` or the path of an audio file.
    pub source: String,
    pub volume: f64,
    /// Lower the music while someone is speaking.
    pub duck: bool,
    /// How far, in dB (negative).
    pub duck_db: f64,
    pub fade_in_ms: f64,
    pub fade_out_ms: f64,
}

impl Default for Music {
    fn default() -> Self {
        Self {
            source: String::new(),
            volume: 0.35,
            duck: true,
            duck_db: -14.0,
            fade_in_ms: 800.0,
            fade_out_ms: 2500.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioSettings {
    pub mic_volume: f64,
    /// Noise reduction strength, 0 (off) to 1.
    pub denoise: f64,
    /// Bring speech to a consistent loudness (-16 LUFS).
    pub normalize: bool,
    pub eq: VoiceEq,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub music: Option<Music>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            mic_volume: 1.0,
            denoise: 0.0,
            normalize: false,
            eq: VoiceEq::default(),
            music: None,
        }
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
        Self {
            format: "mp4".into(),
            preset: "1080p".into(),
            fps: 60,
            quality: "high".into(),
        }
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
        let mut project = Self {
            format: "cinesnap-project".into(),
            version: PROJECT_VERSION,
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
            clips: vec![],
            annotations: vec![],
            keystrokes: KeystrokeStyle::default(),
            webcam_keys: vec![],
            theme: Theme::default(),
            captions: CaptionStyle::default(),
            transcript: None,
            audio: AudioSettings::default(),
            extra: Map::new(),
        };
        project.ensure_clips();
        project
    }

    /// Version 1 manifests have no clip list: the bundle's own recording is
    /// the whole timeline.
    pub fn ensure_clips(&mut self) {
        if !self.clips.iter().any(|c| c.recording().is_some()) {
            self.clips.insert(
                0,
                Clip {
                    id: "main".into(),
                    transition: Transition::default(),
                    body: ClipBody::Recording(RecordingClip {
                        dir: String::new(),
                        source: self.source.clone(),
                        crop: self.crop,
                    }),
                },
            );
        }
    }

    /// The first recording on the timeline.
    pub fn main_mut(&mut self) -> &mut RecordingClip {
        self.ensure_clips();
        self.clips
            .iter_mut()
            .find_map(|c| match &mut c.body {
                ClipBody::Recording(r) => Some(r),
                ClipBody::Card(_) => None,
            })
            .expect("a project has at least one recording")
    }

    /// Clips with their position on the timeline.
    pub fn spans(&self) -> Vec<Span> {
        let mut start = 0.0;
        self.clips
            .iter()
            .enumerate()
            .map(|(index, c)| {
                let duration_ms = c.duration_ms().max(0.0);
                let span = Span {
                    index,
                    start_ms: start,
                    duration_ms,
                };
                start += duration_ms;
                span
            })
            .collect()
    }

    /// Length of the timeline before cuts.
    pub fn duration_ms(&self) -> f64 {
        self.clips.iter().map(|c| c.duration_ms().max(0.0)).sum()
    }

    /// The clip showing at a timeline time (the last one at the very end).
    pub fn span_at(&self, t: f64) -> Option<Span> {
        let spans = self.spans();
        spans
            .iter()
            .find(|s| t < s.end_ms())
            .or(spans.last())
            .copied()
    }

    /// A single-recording view of one clip, in the clip's own time: its
    /// source and crop, with the cuts and zooms that fall inside it shifted
    /// to start at zero. The per-recording motion code works on these.
    pub fn local(&self, span: Span) -> Option<Project> {
        let rec = self.clips.get(span.index)?.recording()?;
        let (a, b) = (span.start_ms, span.end_ms());
        let mut p = Project::new(rec.source.clone(), self.created_at.clone());
        p.crop = rec.crop;
        p.cuts = self
            .cuts
            .iter()
            .filter(|c| c.end_ms > a && c.start_ms < b)
            .map(|c| Cut {
                start_ms: (c.start_ms - a).max(0.0),
                end_ms: (c.end_ms - a).min(span.duration_ms),
            })
            .collect();
        p.zooms = self
            .zooms
            .iter()
            .filter(|z| z.start_ms >= a - 0.5 && z.start_ms < b)
            .map(|z| Zoom {
                start_ms: (z.start_ms - a).max(0.0),
                end_ms: (z.end_ms - a).min(span.duration_ms),
                ..z.clone()
            })
            .collect();
        p.auto_zoom = self.auto_zoom.clone();
        p.cursor = self.cursor.clone();
        p.frame = self.frame.clone();
        p.export = self.export.clone();
        Some(p)
    }

    /// Directory holding a recording clip's files.
    pub fn clip_dir(dir: &Path, clip: &RecordingClip) -> PathBuf {
        if clip.dir.is_empty() {
            dir.to_owned()
        } else {
            dir.join(&clip.dir)
        }
    }

    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(PROJECT_FILE);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mut project: Project = match serde_json::from_str(&text) {
            Ok(p) => p,
            // A crash while saving cannot damage the manifest (it is replaced
            // atomically), but a failing disk can. The previous save is kept
            // next to it.
            Err(e) => {
                let backup = dir.join(BACKUP_FILE);
                match std::fs::read_to_string(&backup)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                {
                    Some(p) => p,
                    None => return Err(e).with_context(|| format!("parsing {}", path.display())),
                }
            }
        };
        project.ensure_clips();
        project.version = project.version.max(PROJECT_VERSION);
        anyhow::ensure!(
            project.format == "cinesnap-project",
            "{} is not a Cinesnap project",
            path.display()
        );
        project
            .validate()
            .with_context(|| format!("{} is damaged", path.display()))?;
        project.sanitize();
        Ok(project)
    }

    /// Reject manifests no amount of clamping can make usable: a video with
    /// no size or length, or tracks that point outside the bundle.
    pub fn validate(&self) -> Result<()> {
        Self::validate_source(&self.source)?;
        let mut ids = std::collections::HashSet::new();
        for clip in &self.clips {
            anyhow::ensure!(ids.insert(&clip.id), "two clips share the id {:?}", clip.id);
            match &clip.body {
                ClipBody::Recording(r) => {
                    anyhow::ensure!(
                        is_clip_dir(&r.dir),
                        "{:?} is not a folder inside the recording",
                        r.dir
                    );
                    Self::validate_source(&r.source)?;
                }
                ClipBody::Card(c) => anyhow::ensure!(
                    c.duration_ms.is_finite() && c.duration_ms > 0.0,
                    "a card has no duration"
                ),
            }
        }
        Ok(())
    }

    fn validate_source(src: &Source) -> Result<()> {
        let v = &src.video;
        anyhow::ensure!(
            v.width >= 2 && v.height >= 2,
            "the video size {}x{} is not usable",
            v.width,
            v.height
        );
        anyhow::ensure!(
            v.duration_ms.is_finite() && v.duration_ms > 0.0,
            "the video has no duration"
        );
        anyhow::ensure!(
            v.fps.is_finite() && v.fps > 0.0,
            "the video has no frame rate"
        );
        let files = [Some(&v.file), Some(&src.events.file)]
            .into_iter()
            .chain([
                src.mic.as_ref().map(|m| &m.file),
                src.webcam.as_ref().map(|w| &w.file),
            ])
            .flatten();
        for file in files {
            anyhow::ensure!(
                is_bundle_file(file),
                "{file:?} is not a file inside the recording"
            );
        }
        Ok(())
    }

    /// Atomic save: write to a temp file next to the target, flush it to
    /// disk, then rename. The manifest it replaces is kept as a backup, so
    /// neither a crash nor a power cut can lose more than the last edit.
    pub fn save(&self, dir: &Path) -> Result<()> {
        use std::io::Write;
        let path = dir.join(PROJECT_FILE);
        let tmp = dir.join(format!("{PROJECT_FILE}.tmp"));
        let mut file =
            std::fs::File::create(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
        file.write_all(&serde_json::to_vec_pretty(self)?)
            .and_then(|_| file.sync_all())
            .with_context(|| format!("writing {}", tmp.display()))?;
        drop(file);
        if path.is_file() {
            // Best effort: a missing backup must not stop a save.
            let _ = std::fs::copy(&path, dir.join(BACKUP_FILE));
        }
        std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// Clamp edit state into a valid range so downstream math never sees
    /// nonsense (hand-edited files, older versions, UI bugs).
    pub fn sanitize(&mut self) {
        // `clamp` panics when min > max, so keep every bound well-formed
        // even for a manifest that `validate` would reject.
        fn clamp_crop(c: &mut Rect, video: &VideoTrack) {
            let (w, h) = (video.width.max(2) as f64, video.height.max(2) as f64);
            c.x = c.x.clamp(0.0, w - 2.0);
            c.y = c.y.clamp(0.0, h - 2.0);
            c.width = c.width.clamp(2.0, w - c.x);
            c.height = c.height.clamp(2.0, h - c.y);
        }
        self.ensure_clips();
        clamp_crop(&mut self.crop, &self.source.video);
        for clip in &mut self.clips {
            clip.transition.duration_ms = clip.transition.duration_ms.clamp(100.0, 3000.0);
            match &mut clip.body {
                ClipBody::Recording(r) => clamp_crop(&mut r.crop, &r.source.video),
                ClipBody::Card(c) => {
                    c.duration_ms = if c.duration_ms.is_finite() {
                        c.duration_ms.clamp(300.0, 60_000.0)
                    } else {
                        3000.0
                    };
                    c.title_size = c.title_size.clamp(16.0, 400.0);
                }
            }
        }
        // Older builds read the top-level crop; keep it in step with the
        // bundle's own recording.
        if let Some(r) = self
            .clips
            .iter()
            .filter_map(Clip::recording)
            .find(|r| r.dir.is_empty())
        {
            self.crop = r.crop;
        }
        let dur = self.duration_ms();
        for a in &mut self.annotations {
            a.start_ms = a.start_ms.clamp(0.0, dur);
            a.end_ms = a.end_ms.clamp(a.start_ms, dur);
        }
        self.annotations
            .sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
        for k in &mut self.webcam_keys {
            k.at_ms = k.at_ms.clamp(0.0, dur);
            k.x = k.x.clamp(0.0, 1.0);
            k.y = k.y.clamp(0.0, 1.0);
            k.size = k.size.clamp(0.04, 1.0);
        }
        self.webcam_keys.sort_by(|a, b| a.at_ms.total_cmp(&b.at_ms));
        self.webcam.size = self.webcam.size.clamp(0.04, 1.0);
        self.keystrokes.size = self.keystrokes.size.clamp(0.3, 4.0);
        self.keystrokes.hold_ms = self.keystrokes.hold_ms.clamp(200.0, 10_000.0);
        self.captions.size = self.captions.size.clamp(0.3, 4.0);
        self.captions.y = self.captions.y.clamp(0.05, 0.95);
        self.captions.max_chars = self.captions.max_chars.clamp(8, 80);
        if let Some(t) = &mut self.transcript {
            t.words.retain(|w| w.t0.is_finite() && w.t1.is_finite());
            for w in &mut t.words {
                w.t0 = w.t0.clamp(0.0, dur);
                w.t1 = w.t1.clamp(w.t0, dur);
            }
            t.words.sort_by(|a, b| a.t0.total_cmp(&b.t0));
        }
        let a = &mut self.audio;
        a.mic_volume = a.mic_volume.clamp(0.0, 4.0);
        a.denoise = a.denoise.clamp(0.0, 1.0);
        a.eq.low_cut_hz = a.eq.low_cut_hz.clamp(20.0, 400.0);
        a.eq.bass_db = a.eq.bass_db.clamp(-12.0, 12.0);
        a.eq.presence_db = a.eq.presence_db.clamp(-12.0, 12.0);
        a.eq.air_db = a.eq.air_db.clamp(-12.0, 12.0);
        if let Some(m) = &mut a.music {
            m.volume = m.volume.clamp(0.0, 2.0);
            m.duck_db = m.duck_db.clamp(-40.0, 0.0);
            m.fade_in_ms = m.fade_in_ms.clamp(0.0, 20_000.0);
            m.fade_out_ms = m.fade_out_ms.clamp(0.0, 20_000.0);
        }
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

/// Empty (the bundle itself) or `clips/<name>`.
fn is_clip_dir(dir: &str) -> bool {
    dir.is_empty()
        || dir
            .strip_prefix("clips/")
            .is_some_and(|name| is_bundle_file(name) && name != "..")
}

/// A plain file name: no directories, no `..`, not absolute.
fn is_bundle_file(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!(parts.next(), Some(std::path::Component::Normal(_)))
        && parts.next().is_none()
        && !name.contains(['/', '\\'])
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

    fn manifest(video: &str, events: &str) -> Project {
        let json = format!(
            r#"{{"format":"cinesnap-project","version":1,"createdAt":"x","app":"0",
            "source":{{"video":{video},"events":{{"file":"{events}"}},"platform":"test","target":"screen"}},
            "crop":{{"x":0,"y":0,"width":100,"height":50}}}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    const VIDEO: &str =
        r#"{"file":"video.mp4","width":100,"height":50,"fps":30,"durationMs":1000}"#;

    #[test]
    fn damaged_manifests_are_rejected() {
        assert!(manifest(VIDEO, "events.jsonl").validate().is_ok());
        for video in [
            r#"{"file":"video.mp4","width":0,"height":0,"fps":30,"durationMs":1000}"#,
            r#"{"file":"video.mp4","width":100,"height":50,"fps":30,"durationMs":-5}"#,
            r#"{"file":"video.mp4","width":100,"height":50,"fps":0,"durationMs":1000}"#,
            r#"{"file":"/etc/hostname","width":100,"height":50,"fps":30,"durationMs":1000}"#,
            r#"{"file":"a/video.mp4","width":100,"height":50,"fps":30,"durationMs":1000}"#,
        ] {
            assert!(
                manifest(video, "events.jsonl").validate().is_err(),
                "{video}"
            );
        }
        for events in ["../events.jsonl", "..", "C:\\\\x.jsonl", ""] {
            assert!(manifest(VIDEO, events).validate().is_err(), "{events}");
        }
    }

    #[test]
    fn sanitize_never_panics_on_nonsense() {
        let mut p = manifest(
            r#"{"file":"video.mp4","width":0,"height":1,"fps":0,"durationMs":-5}"#,
            "events.jsonl",
        );
        p.crop = Rect {
            x: -50.0,
            y: 1e9,
            width: 1e12,
            height: 0.0,
        };
        p.zooms = serde_json::from_str(
            r#"[{"id":"a","startMs":5000,"endMs":100,"scale":-3,"easeInMs":-1,"easeOutMs":1e12,"x":-1e9,"y":1e9}]"#,
        )
        .unwrap();
        p.sanitize();
        assert_eq!(
            p.crop,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 2.0
            }
        );
        assert_eq!((p.zooms[0].start_ms, p.zooms[0].end_ms), (0.0, 0.0));
        assert_eq!(p.zooms[0].scale, 1.0);
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
