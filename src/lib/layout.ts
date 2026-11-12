// Layout math shared by the preview renderer. The export compositor follows the same rules.

import type { Aspect, Corner, ExportPreset, ExportSettings, Rect } from "./types";

export interface Size {
  width: number;
  height: number;
}

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

const MB = 1024 * 1024;
const plain = (id: string, label: string, short: number): ExportPreset => ({ id, label, hint: "", format: "mp4", size: { short }, fps: null, aspect: null, maxBytes: null, maxSeconds: null });

/** Mirrors `presets.rs` in the core. */
export const PRESETS: ExportPreset[] = [
  { id: "readme", label: "GitHub README", hint: "GIF, 800 px wide, under GitHub's 10 MB image limit", format: "gif", size: { width: 800 }, fps: 15, aspect: null, maxBytes: 10 * MB, maxSeconds: null },
  { id: "readme-mp4", label: "GitHub video", hint: "MP4, 720p, under GitHub's 10 MB upload limit", format: "mp4", size: { short: 720 }, fps: 30, aspect: null, maxBytes: 10 * MB, maxSeconds: null },
  { id: "x", label: "X / Twitter", hint: "MP4, 1080p, 30 fps, at most 2:20 and 512 MB", format: "mp4", size: { short: 1080 }, fps: 30, aspect: null, maxBytes: 512 * MB, maxSeconds: 140 },
  { id: "youtube", label: "YouTube", hint: "MP4, 1440p, 60 fps (YouTube keeps more detail above 1080p)", format: "mp4", size: { short: 1440 }, fps: 60, aspect: null, maxBytes: null, maxSeconds: null },
  { id: "shorts", label: "TikTok / Shorts", hint: "MP4, vertical 1080×1920, 30 fps, at most 3 minutes", format: "mp4", size: { short: 1080 }, fps: 30, aspect: "9:16", maxBytes: null, maxSeconds: 180 },
  plain("4k", "4K", 2160),
  plain("1440p", "1440p", 1440),
  plain("1080p", "1080p", 1080),
  plain("720p", "720p", 720),
  { id: "small", label: "Small GIF", hint: "GIF, 480 px wide", format: "gif", size: { width: 480 }, fps: 12, aspect: null, maxBytes: null, maxSeconds: null },
];

/** The preset for the export settings, or the default for the format. */
export function resolvePreset(exp: ExportSettings): ExportPreset {
  const found = PRESETS.find((p) => p.id === exp.preset && p.format === exp.format);
  return found ?? PRESETS.find((p) => p.id === (exp.format === "gif" ? "readme" : "1080p"))!;
}

/** Width / height of the output for an aspect preset. */
export function aspectRatio(aspect: Aspect, crop: Rect): number {
  switch (aspect) {
    case "16:9":
      return 16 / 9;
    case "9:16":
      return 9 / 16;
    case "1:1":
      return 1;
    case "4:3":
      return 4 / 3;
    default:
      return crop.height > 0 ? crop.width / crop.height : 16 / 9;
  }
}

const even = (v: number) => Math.max(2, Math.round(v / 2) * 2);

/** Aspect the export will actually use: some presets force their own. */
export function effectiveAspect(aspect: Aspect, exp: ExportSettings): Aspect {
  return resolvePreset(exp).aspect ?? aspect;
}

/** Output pixel size for the chosen aspect and export preset. */
export function outputSize(aspect: Aspect, crop: Rect, exp: ExportSettings): Size {
  const preset = resolvePreset(exp);
  const a = aspectRatio(preset.aspect ?? aspect, crop);
  if ("width" in preset.size) return { width: even(preset.size.width), height: even(preset.size.width / a) };
  const short = preset.size.short;
  return a >= 1 ? { width: even(short * a), height: even(short) } : { width: even(short), height: even(short / a) };
}

/** Frame rate the export (and therefore the motion tracks) will use. */
export function outputFps(exp: ExportSettings): number {
  return Math.max(1, resolvePreset(exp).fps ?? exp.fps);
}

/** Largest size with the given aspect ratio that fits inside maxW x maxH. */
export function fitSize(aspect: number, maxW: number, maxH: number): Size {
  if (maxW <= 0 || maxH <= 0) return { width: 0, height: 0 };
  return maxW / maxH > aspect ? { width: maxH * aspect, height: maxH } : { width: maxW, height: maxW / aspect };
}

/** Resolution-independent unit: 1 at 1080p. */
export function unitScale(W: number, H: number): number {
  return Math.min(W, H) / 1080;
}

/** Where the recording sits inside the output. */
export function frameRect(W: number, H: number, padding: number, crop: Rect): Box {
  const pad = padding * Math.min(W, H);
  const a = crop.height > 0 ? crop.width / crop.height : 16 / 9;
  const { width, height } = fitSize(a, Math.max(1, W - 2 * pad), Math.max(1, H - 2 * pad));
  return { x: (W - width) / 2, y: (H - height) / 2, w: width, h: height };
}

/** Visible part of the source (source px) for a camera centred at (cx, cy). */
export function viewRect(crop: Rect, cx: number, cy: number, scale: number): Box {
  const sc = Math.max(1, scale);
  const w = crop.width / sc;
  const h = crop.height / sc;
  return { x: cx - w / 2, y: cy - h / 2, w, h };
}

/** Clamp a camera centre so the view stays inside the crop. */
export function clampCentre(crop: Rect, cx: number, cy: number, scale: number): { cx: number; cy: number } {
  const sc = Math.max(1, scale);
  const hw = crop.width / sc / 2;
  const hh = crop.height / sc / 2;
  return {
    cx: Math.min(crop.x + crop.width - hw, Math.max(crop.x + hw, cx)),
    cy: Math.min(crop.y + crop.height - hh, Math.max(crop.y + hh, cy)),
  };
}

/** Map a source-pixel position to output pixels. */
export function mapPoint(frame: Box, view: Box, x: number, y: number): { x: number; y: number } {
  return {
    x: frame.x + ((x - view.x) / view.w) * frame.w,
    y: frame.y + ((y - view.y) / view.h) * frame.h,
  };
}

export const WEBCAM_MARGIN = 0.03;

/** Centre of a cornered webcam bubble, as fractions of the output. */
export function cornerCentre(corner: Corner, size: number, ratio: number): { x: number; y: number } {
  const kx = ratio >= 1 ? 1 / ratio : 1;
  const ky = ratio >= 1 ? 1 : ratio;
  const off = WEBCAM_MARGIN + size / 2;
  return {
    x: corner.endsWith("left") ? off * kx : 1 - off * kx,
    y: corner.startsWith("top") ? off * ky : 1 - off * ky,
  };
}

/** Box of the webcam bubble for a state from the motion tracks. */
export function webcamBox(W: number, H: number, x: number, y: number, size: number, alpha: number): Box {
  // The bubble shrinks a little as it fades.
  const side = size * Math.min(W, H) * (0.85 + 0.15 * alpha);
  return { x: x * W - side / 2, y: y * H - side / 2, w: side, h: side };
}

/** Source rectangle that cover-fills a destination of the given aspect. */
export function coverRect(srcW: number, srcH: number, dstW: number, dstH: number): Box {
  if (srcW <= 0 || srcH <= 0 || dstW <= 0 || dstH <= 0) return { x: 0, y: 0, w: srcW, h: srcH };
  const a = dstW / dstH;
  if (srcW / srcH > a) {
    const w = srcH * a;
    return { x: (srcW - w) / 2, y: 0, w, h: srcH };
  }
  const h = srcW / a;
  return { x: 0, y: (srcH - h) / 2, w: srcW, h };
}

/** Start and end points of a CSS-style linear gradient. */
export function gradientLine(W: number, H: number, angleDeg: number): { x0: number; y0: number; x1: number; y1: number } {
  const a = (angleDeg * Math.PI) / 180;
  const dx = Math.sin(a);
  const dy = -Math.cos(a);
  const len = Math.abs(W * dx) + Math.abs(H * dy);
  return {
    x0: W / 2 - (dx * len) / 2,
    y0: H / 2 - (dy * len) / 2,
    x1: W / 2 + (dx * len) / 2,
    y1: H / 2 + (dy * len) / 2,
  };
}
