// Layout math shared by the preview renderer. The export compositor follows the same rules.

import type { Aspect, Corner, ExportSettings, Rect } from "./types";

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

export const MP4_PRESETS: Record<string, number> = { "4k": 2160, "1440p": 1440, "1080p": 1080, "720p": 720 };
export const GIF_PRESETS: Record<string, { width: number; fps: number }> = {
  readme: { width: 800, fps: 15 },
  small: { width: 480, fps: 12 },
};

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

/** Output pixel size for the chosen aspect and export preset. */
export function outputSize(aspect: Aspect, crop: Rect, exp: ExportSettings): Size {
  const a = aspectRatio(aspect, crop);
  if (exp.format === "gif") {
    const width = (GIF_PRESETS[exp.preset] ?? GIF_PRESETS.readme).width;
    return { width: even(width), height: even(width / a) };
  }
  const short = MP4_PRESETS[exp.preset] ?? 1080;
  return a >= 1 ? { width: even(short * a), height: even(short) } : { width: even(short), height: even(short / a) };
}

/** Frame rate the export (and therefore the motion tracks) will use. */
export function outputFps(exp: ExportSettings): number {
  if (exp.format === "gif") return (GIF_PRESETS[exp.preset] ?? GIF_PRESETS.readme).fps;
  return exp.fps;
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

/** Square box of the webcam overlay. */
export function webcamRect(W: number, H: number, size: number, corner: Corner): Box {
  const m = Math.min(W, H);
  const side = size * m;
  const margin = 0.03 * m;
  const x = corner.endsWith("left") ? margin : W - margin - side;
  const y = corner.startsWith("top") ? margin : H - margin - side;
  return { x, y, w: side, h: side };
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
