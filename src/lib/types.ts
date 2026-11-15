// Mirrors docs/recording-format.md (project.json) and the backend command contract.

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface VideoSource {
  file: string;
  width: number;
  height: number;
  /** The highest frame rate; with `vfr`, unchanged frames were dropped. */
  fps: number;
  durationMs: number;
  vfr?: boolean;
  /** "tonemapped" when an HDR screen was converted while recording. */
  hdr?: string;
}

export interface AudioTrack {
  file: string;
  offsetMs: number;
  /** System audio limited to one app: its name. */
  app?: string;
}

export interface Source {
  video: VideoSource;
  mic?: AudioTrack;
  /** What the computer played: everything, or one app. */
  system?: AudioTrack;
  webcam?: { file: string; offsetMs: number; width: number; height: number };
  events: { file: string };
  platform: string;
  target: "screen" | "window" | "region" | "screens";
}

export interface Marker {
  id: string;
  atMs: number;
  label: string;
}

export interface Cut {
  startMs: number;
  endMs: number;
}

export type Easing = "smooth" | "snappy" | "linear";
export type ZoomMode = "follow" | "fixed";

export interface ZoomSegment {
  id: string;
  startMs: number;
  endMs: number;
  scale: number;
  easeInMs: number;
  easeOutMs: number;
  easing: Easing;
  mode: ZoomMode;
  x: number;
  y: number;
  manual: boolean;
}

export interface AutoZoomSettings {
  enabled: boolean;
  scale: number;
  mergeGapMs: number;
  onTyping: boolean;
}

export interface CursorSettings {
  show: boolean;
  size: number;
  smoothing: number;
  ripples: boolean;
  rippleColor: string;
  hideWhenIdleMs: number;
  style: string;
}

export type Background =
  | { type: "solid"; color: string }
  | { type: "gradient"; from: string; to: string; angle: number }
  | { type: "image"; path: string }
  | { type: "wallpaper"; path: string; blur: number };

export type Aspect = "16:9" | "9:16" | "1:1" | "4:3" | "source";

export interface FrameSettings {
  aspect: Aspect;
  padding: number;
  radius: number;
  shadow: number;
  background: Background;
}

export type Corner = "top-left" | "top-right" | "bottom-left" | "bottom-right";

export interface WebcamSettings {
  show: boolean;
  /** Used when x/y are absent. */
  corner: Corner;
  /** Centre of the bubble as fractions of the output. */
  x?: number;
  y?: number;
  size: number;
  shape: "circle" | "rounded" | "square";
}

/** From `atMs` (timeline time) on, the webcam bubble moves to this state. */
export interface WebcamKey {
  id: string;
  atMs: number;
  x: number;
  y: number;
  size: number;
  show: boolean;
}

export type TransitionKind = "cut" | "crossfade" | "zoom";

export interface Transition {
  type: TransitionKind;
  durationMs: number;
}

export interface RecordingClip {
  id: string;
  kind: "recording";
  transition: Transition;
  /** "" for the bundle's own recording, "clips/<id>" for added ones. */
  dir: string;
  source: Source;
  crop: Rect;
}

export interface CardClip {
  id: string;
  kind: "card";
  transition: Transition;
  durationMs: number;
  title: string;
  subtitle: string;
  font?: string;
  titleColor: string;
  subtitleColor: string;
  titleSize: number;
  background?: Background;
  align: "center" | "left";
}

export type Clip = RecordingClip | CardClip;

interface AnnotationBase {
  id: string;
  startMs: number;
  endMs: number;
}

interface Region {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type TextAnnotation = AnnotationBase & { type: "text"; x: number; y: number; text: string; size: number; color: string; background?: string; font?: string; bold: boolean };
export type ArrowAnnotation = AnnotationBase & { type: "arrow"; x1: number; y1: number; x2: number; y2: number; color: string; width: number };
export type BoxAnnotation = AnnotationBase & Region & { type: "box"; color: string; stroke: number; radius: number };
export type HighlightAnnotation = AnnotationBase & Region & { type: "highlight"; color: string; opacity: number };
export type SpotlightAnnotation = AnnotationBase & Region & { type: "spotlight"; dim: number; ellipse: boolean };
export type BlurAnnotation = AnnotationBase & Region & { type: "blur"; strength: number };

/** Drawn over the recording; positions are source pixels of the clip on screen. */
export type Annotation = TextAnnotation | ArrowAnnotation | BoxAnnotation | HighlightAnnotation | SpotlightAnnotation | BlurAnnotation;
export type AnnotationType = Annotation["type"];

export interface Theme {
  font: string;
  accent: string;
  text: string;
  surface: string;
}

export interface KeystrokeSettings {
  show: boolean;
  position: "bottom" | "top" | "bottom-left" | "bottom-right";
  size: number;
  holdMs: number;
}

export interface CaptionSettings {
  show: boolean;
  style: "karaoke" | "pop" | "plain";
  y: number;
  size: number;
  color: string;
  highlight: string;
  background?: string;
  font?: string;
  maxChars: number;
  uppercase: boolean;
}

/** One spoken word, in timeline time. */
export interface Word {
  t0: number;
  t1: number;
  text: string;
}

export interface Transcript {
  language: string;
  words: Word[];
}

export interface VoiceEq {
  enabled: boolean;
  lowCutHz: number;
  bassDb: number;
  presenceDb: number;
  airDb: number;
}

export interface Music {
  /** "builtin:<id>" or a file path. */
  source: string;
  volume: number;
  duck: boolean;
  duckDb: number;
  fadeInMs: number;
  fadeOutMs: number;
}

export interface AudioSettings {
  micVolume: number;
  micMute: boolean;
  systemVolume: number;
  systemMute: boolean;
  denoise: number;
  normalize: boolean;
  eq: VoiceEq;
  music?: Music;
}

export type ExportFormat = "mp4" | "gif";
export type Mp4Preset = "4k" | "1440p" | "1080p" | "720p" | "readme-mp4" | "x" | "youtube" | "shorts";
export type GifPreset = "readme" | "small";
export type Quality = "low" | "medium" | "high";

export interface ExportSettings {
  format: ExportFormat;
  preset: Mp4Preset | GifPreset | string;
  fps: number;
  quality: Quality;
}

export interface Project {
  format: "cinesnap-project";
  version: number;
  createdAt: string;
  app: string;
  source: Source;
  crop: Rect;
  cuts: Cut[];
  zooms: ZoomSegment[];
  autoZoom: AutoZoomSettings;
  cursor: CursorSettings;
  frame: FrameSettings;
  webcam: WebcamSettings;
  export: ExportSettings;
  /** Everything on the timeline, in order. */
  clips: Clip[];
  annotations: Annotation[];
  keystrokes: KeystrokeSettings;
  webcamKeys: WebcamKey[];
  theme: Theme;
  captions: CaptionSettings;
  transcript?: Transcript;
  audio: AudioSettings;
  /** Moments flagged while recording or editing, in timeline time. */
  markers: Marker[];
  // Unknown fields are preserved on save.
  [extra: string]: unknown;
}

// ---- backend contract ------------------------------------------------------

export type Platform = "linux-wayland" | "linux-x11" | "windows" | "macos";
export type CursorMode = "metadata" | "embedded" | "none";
export type ClickMode = "exact" | "inferred" | "none";

export interface Capabilities {
  platform: Platform;
  cursor: CursorMode;
  clicks: ClickMode;
  ffmpeg: { path: string; version: string } | null;
  encoder: string;
  notes: string[];
  /** Base URL of the backend's local media server (Linux); null when asset URLs work. */
  mediaBase?: string | null;
}

export interface ScreenInfo extends Rect {
  id: string;
  name: string;
  /** In HDR mode; converted to SDR while recording. */
  hdr: boolean;
}

export interface Device {
  id: string;
  name: string;
}

export interface Sources {
  portal: boolean;
  screens: ScreenInfo[];
  windows: { id: string; title: string }[];
  mics: Device[];
  webcams: Device[];
  /** Outputs whose sound can be recorded; empty when none can. */
  outputs: Device[];
  /** Apps playing sound that can be recorded on their own. */
  apps: Device[];
}

export type TargetKind = "screen" | "window" | "portal" | "all";

export interface RecordingOptions {
  target: { kind: TargetKind; id?: string };
  region?: Rect | null;
  mic: string | null;
  webcam: string | null;
  /** An output id: record everything it plays. */
  systemAudio: string | null;
  /** An app id: record only its sound. */
  appAudio: string | null;
  appName: string | null;
  fps: number;
  /** Drop frames where nothing changed. */
  vfr: boolean;
  /** Seconds of countdown, cut from the start. */
  countdown: number;
  /** Label shortcuts (Ctrl+K, Enter) for the keystroke overlay. Typed text is never recorded. */
  shortcuts: boolean;
}

export interface RecordingStatus {
  elapsedMs: number;
  paused: boolean;
  countdownMs: number;
  segments: number;
}

export interface Hotkeys {
  toggle: string;
  pause: string;
  marker: string;
}

export interface RecentProject {
  path: string;
  name: string;
  createdAt: string;
  durationMs: number;
}

export interface Ripple {
  outMs: number;
  x: number;
  y: number;
  clip: number;
}

export interface CaptionWord {
  text: string;
  startMs: number;
  endMs: number;
}

/** One caption as it appears on screen, in output time. */
export interface CaptionLine {
  startMs: number;
  endMs: number;
  words: CaptionWord[];
}

/** A blend between two clips, centred on the first frame of the incoming one. */
export interface TransitionMark {
  frame: number;
  half: number;
  kind: TransitionKind;
}

/** Sampled motion, indexed by OUTPUT frame (after cuts). */
export interface MotionTracks {
  fps: number;
  frames: number;
  durationMs: number;
  /** n values: timeline time of each output frame. */
  srcMs: number[];
  /** n values: index of the clip each output frame belongs to. */
  clip: number[];
  /** Timeline time at which every clip starts. */
  clipStarts: number[];
  /** 3n values: cx, cy (source px, centre of the visible rect, clamped), scale. */
  cam: number[];
  /** 4n values: x, y (source px), alpha 0..1, press 0..1. */
  cursor: number[];
  /** 4n values: x, y (fractions of the output), size (fraction of the shorter side), alpha. */
  webcam: number[];
  ripples: Ripple[];
  events: { clicks: number[]; keys: number[] };
  keys: { outMs: number; combo: string }[];
  captions: CaptionLine[];
  transitions: TransitionMark[];
  /** Per clip: whether the cursor should be redrawn. */
  cursorDrawn: boolean[];
  cursorMode?: CursorMode;
}

export interface ExportPreset {
  id: string;
  label: string;
  hint: string;
  format: ExportFormat;
  size: { short: number } | { width: number };
  fps: number | null;
  aspect: Aspect | null;
  maxBytes: number | null;
  maxSeconds: number | null;
}

export interface MusicTrack {
  id: string;
  name: string;
  mood: string;
  bpm: number;
  durationMs: number;
}

export interface SpeechModel {
  id: string;
  label: string;
  hint: string;
  megabytes: number;
  englishOnly: boolean;
  downloaded: boolean;
}

export interface Speech {
  available: boolean;
  models: SpeechModel[];
}

/** A cut offered for review before anything changes. */
export interface Suggestion {
  kind: "filler" | "silence";
  startMs: number;
  endMs: number;
  label: string;
}

/** A recording copied into the project, with its edits in the clip's own time. */
export interface Imported {
  clip: RecordingClip;
  cuts: Cut[];
  zooms: ZoomSegment[];
}

export interface Shared {
  id: string;
  url: string;
  embedUrl: string;
  embedHtml: string;
  mediaUrl: string;
}

export interface JobProgress {
  job: string;
  stage: string;
  fraction: number;
}

/** The look of a video, reusable across projects. */
export interface StylePreset {
  name: string;
  frame: Omit<FrameSettings, "aspect">;
  cursor: CursorSettings;
  theme: Theme;
  keystrokes: KeystrokeSettings;
  captions: CaptionSettings;
  webcam: Pick<WebcamSettings, "shape" | "size">;
}

export interface AppSettings {
  shareServer?: string;
  shareToken?: string;
  speechModel?: string;
  hotkeys?: Partial<Hotkeys>;
  /** Keep the recording bar on screen where it cannot be kept out of the video. */
  keepBarVisible?: boolean;
  /** The last choices on the home screen. */
  record?: Partial<RecordingOptions>;
}

export interface ExportProgress {
  frame: number;
  total: number;
  stage: string;
}
