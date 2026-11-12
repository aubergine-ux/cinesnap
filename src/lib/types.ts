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
  fps: number;
  durationMs: number;
}

export interface Source {
  video: VideoSource;
  mic?: { file: string; offsetMs: number };
  webcam?: { file: string; offsetMs: number; width: number; height: number };
  events: { file: string };
  platform: string;
  target: "screen" | "window" | "region";
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
  corner: Corner;
  size: number;
  shape: "circle" | "rounded";
}

export type ExportFormat = "mp4" | "gif";
export type Mp4Preset = "4k" | "1440p" | "1080p" | "720p";
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
}

export interface Sources {
  portal: boolean;
  screens: ScreenInfo[];
  windows: { id: string; title: string }[];
  mics: { id: string; name: string }[];
  webcams: { id: string; name: string }[];
}

export interface RecordingOptions {
  target: { kind: "screen" | "window" | "portal"; id?: string };
  region?: Rect | null;
  mic: string | null;
  webcam: string | null;
  fps: number;
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
}

/** Sampled motion, indexed by OUTPUT frame (after cuts). */
export interface MotionTracks {
  fps: number;
  frames: number;
  durationMs: number;
  /** n values: source time of each output frame. */
  srcMs: number[];
  /** 3n values: cx, cy (source px, centre of the visible rect, clamped), scale. */
  cam: number[];
  /** 4n values: x, y (source px), alpha 0..1, press 0..1. */
  cursor: number[];
  ripples: Ripple[];
  events: { clicks: number[]; keys: number[] };
  /** Optional: cursor mode of the recording's event log, when the backend provides it. */
  cursorMode?: CursorMode;
}

export interface ExportProgress {
  frame: number;
  total: number;
  stage: string;
}
