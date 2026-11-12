// Canvas 2D preview compositor. Drawing rules are shared with the export
// compositor (`render.rs`): change them in both places or the preview stops
// being WYSIWYG.

import { coverRect, frameRect, gradientLine, mapPoint, unitScale, viewRect, webcamBox, type Box } from "../lib/layout";
import type { Annotation, Background, CaptionLine, CardClip, MotionTracks, Project, RecordingClip, TransitionKind, TransitionMark } from "../lib/types";
import { CENTER_BASELINE, FIRST_BASELINE, LINE_HEIGHT, drawText, measure, type TextStyle } from "./text";

/** What the renderer needs for one clip. */
export interface ClipMedia {
  /** The recording (video element, or a canvas in demo mode). */
  source: CanvasImageSource | null;
  webcam: HTMLVideoElement | null;
}

export interface RenderAssets {
  /** Media per clip index; cards have none. */
  clips: (ClipMedia | undefined)[];
  /** A loaded background image for a path, if it is ready. */
  backdrop: (path: string) => HTMLImageElement | null;
}

// Arrow in unit coordinates (height = 1), tip at the origin.
const ARROW: [number, number][] = [
  [0, 0],
  [0, 0.8],
  [0.19, 0.63],
  [0.33, 0.95],
  [0.45, 0.9],
  [0.31, 0.59],
  [0.57, 0.59],
];

export const RIPPLE_MS = 500;
export const ANNOTATION_FADE_MS = 180;
export const KEY_FADE_IN_MS = 90;
export const KEY_FADE_OUT_MS = 260;
export const CARD_INTRO_MS = 500;

type Ctx = CanvasRenderingContext2D;

const clamp01 = (v: number) => Math.max(0, Math.min(1, v));
const smoothstep = (t: number) => {
  const x = clamp01(t);
  return x * x * (3 - 2 * x);
};

export function roundedRectPath(ctx: Ctx, b: Box, radius: number) {
  const r = Math.max(0, Math.min(radius, b.w / 2, b.h / 2));
  ctx.beginPath();
  ctx.moveTo(b.x + r, b.y);
  ctx.arcTo(b.x + b.w, b.y, b.x + b.w, b.y + b.h, r);
  ctx.arcTo(b.x + b.w, b.y + b.h, b.x, b.y + b.h, r);
  ctx.arcTo(b.x, b.y + b.h, b.x, b.y, r);
  ctx.arcTo(b.x, b.y, b.x + b.w, b.y, r);
  ctx.closePath();
}

function fillRounded(ctx: Ctx, b: Box, radius: number, color: string, alpha = 1) {
  ctx.save();
  ctx.globalAlpha *= clamp01(alpha);
  ctx.fillStyle = color;
  roundedRectPath(ctx, b, radius);
  ctx.fill();
  ctx.restore();
}

/** Opacity of an annotation at a timeline time. Blur regions never fade. */
export function annotationAlpha(a: Annotation, t: number): number {
  if (t < a.startMs || t >= a.endMs) return 0;
  if (a.type === "blur") return 1;
  const fade = Math.max(1, Math.min(ANNOTATION_FADE_MS, (a.endMs - a.startMs) / 2));
  return clamp01(Math.min((t - a.startMs) / fade, (a.endMs - t) / fade));
}

/** The shortcut on screen at an output time, with its opacity. */
export function keyAt(keys: MotionTracks["keys"], holdMs: number, outMs: number): { combo: string; alpha: number } | null {
  let n = 0;
  while (n < keys.length && keys[n].outMs <= outMs) n++;
  if (n === 0) return null;
  const key = keys[n - 1];
  const age = outMs - key.outMs;
  if (age >= holdMs) return null;
  const alpha = clamp01(Math.min(age / KEY_FADE_IN_MS, (holdMs - age) / KEY_FADE_OUT_MS));
  // A shortcut that replaces another one shows immediately.
  const replaced = n >= 2 && key.outMs - keys[n - 2].outMs < holdMs;
  return { combo: key.combo, alpha: replaced && age < KEY_FADE_IN_MS ? 1 : alpha };
}

export function captionAt(lines: CaptionLine[], outMs: number): CaptionLine | null {
  let found: CaptionLine | null = null;
  for (const l of lines) {
    if (l.startMs > outMs) break;
    found = l;
  }
  return found && outMs < found.endMs ? found : null;
}

/** The transition an output frame is part of, with its progress 0..1. */
export function transitionAt(motion: MotionTracks, frame: number): { mark: TransitionMark; p: number } | null {
  for (const mark of motion.transitions) {
    const start = mark.frame - mark.half;
    if (frame >= start && frame < mark.frame + mark.half) return { mark, p: (frame - start + 0.5) / (2 * mark.half) };
  }
  return null;
}

function drawBackground(ctx: Ctx, W: number, H: number, s: number, bg: Background, image: HTMLImageElement | null) {
  ctx.fillStyle = "#0b0b0f";
  ctx.fillRect(0, 0, W, H);
  if (bg.type === "solid") {
    ctx.fillStyle = bg.color;
    ctx.fillRect(0, 0, W, H);
  } else if (bg.type === "gradient") {
    const l = gradientLine(W, H, bg.angle);
    const g = ctx.createLinearGradient(l.x0, l.y0, l.x1, l.y1);
    g.addColorStop(0, bg.from);
    g.addColorStop(1, bg.to);
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, W, H);
  } else if (image && image.naturalWidth > 0) {
    const src = coverRect(image.naturalWidth, image.naturalHeight, W, H);
    const blur = bg.type === "wallpaper" ? Math.max(0, bg.blur * s) : 0;
    if (blur < 0.5) {
      ctx.drawImage(image, src.x, src.y, src.w, src.h, 0, 0, W, H);
    } else if (hasFilter(ctx)) {
      ctx.save();
      ctx.filter = `blur(${blur}px)`;
      // Overscan so the blur does not pull transparency in from the edges.
      const o = blur * 2.5;
      ctx.drawImage(image, src.x, src.y, src.w, src.h, -o, -o, W + 2 * o, H + 2 * o);
      ctx.restore();
    } else {
      // No canvas filters: approximate the blur by resampling through a tiny buffer.
      const k = Math.max(2, blur / 2);
      const tw = Math.max(2, Math.round(W / k));
      const th = Math.max(2, Math.round(H / k));
      const tmp = document.createElement("canvas");
      tmp.width = tw;
      tmp.height = th;
      const t = tmp.getContext("2d")!;
      t.imageSmoothingQuality = "high";
      t.drawImage(image, src.x, src.y, src.w, src.h, 0, 0, tw, th);
      ctx.imageSmoothingQuality = "high";
      ctx.drawImage(tmp, 0, 0, tw, th, 0, 0, W, H);
    }
  }
}

function hasFilter(ctx: Ctx): boolean {
  return "filter" in ctx && typeof ctx.filter === "string";
}

function backdropPath(bg: Background): string {
  return bg.type === "image" || bg.type === "wallpaper" ? bg.path : "";
}

/** Composites output frames; caches static layers between frames. */
export class Renderer {
  private layers = new Map<string, HTMLCanvasElement>();
  private scratch: HTMLCanvasElement | null = null;
  private bufA: HTMLCanvasElement | null = null;
  private bufB: HTMLCanvasElement | null = null;

  /** Background, optionally with the shadow of a frame baked in. */
  private staticLayer(W: number, H: number, s: number, bg: Background, shadow: number, frame: Box | null, radius: number, assets: RenderAssets): HTMLCanvasElement {
    const path = backdropPath(bg);
    const image = path ? assets.backdrop(path) : null;
    const key = JSON.stringify([W, H, bg, frame ? [shadow, radius, frame] : null, image !== null]);
    const cached = this.layers.get(key);
    if (cached) return cached;
    // Sizes and settings change rarely; a handful of layers covers a project.
    if (this.layers.size > 12) this.layers.clear();
    const layer = document.createElement("canvas");
    layer.width = W;
    layer.height = H;
    const ctx = layer.getContext("2d")!;
    drawBackground(ctx, W, H, s, bg, image);
    if (frame && shadow > 0) {
      ctx.save();
      ctx.shadowColor = `rgba(0,0,0,${Math.min(1, shadow * 0.6)})`;
      ctx.shadowBlur = 40 * s;
      ctx.shadowOffsetY = 16 * s;
      ctx.fillStyle = "#000";
      roundedRectPath(ctx, frame, radius);
      ctx.fill();
      ctx.restore();
    }
    this.layers.set(key, layer);
    return layer;
  }

  private buffer(which: "bufA" | "bufB" | "scratch", W: number, H: number): HTMLCanvasElement {
    const c = this[which] ?? (this[which] = document.createElement("canvas"));
    if (c.width !== W || c.height !== H) {
      c.width = W;
      c.height = H;
    }
    return c;
  }

  /** Draw an output frame, blending two clips when it is inside a transition. */
  draw(ctx: Ctx, W: number, H: number, project: Project, motion: MotionTracks | null, frameIndex: number, assets: RenderAssets) {
    const tr = motion ? transitionAt(motion, frameIndex) : null;
    if (!motion || !tr) {
      this.drawFrame(ctx, W, H, project, motion, frameIndex, assets);
      return;
    }
    const { mark, p } = tr;
    const a = this.buffer("bufA", W, H);
    const b = this.buffer("bufB", W, H);
    // The clip that is not playing is shown frozen: the incoming one on its
    // first frame, the outgoing one on its last.
    const incomingLive = frameIndex >= mark.frame;
    this.drawFrame(a.getContext("2d")!, W, H, project, motion, incomingLive ? mark.frame - 1 : frameIndex, assets);
    this.drawFrame(b.getContext("2d")!, W, H, project, motion, incomingLive ? frameIndex : mark.frame, assets);
    blendTransition(ctx, W, H, mark.kind, p, a, b);
  }

  /** Composite one output frame of one clip. */
  drawFrame(ctx: Ctx, W: number, H: number, project: Project, motion: MotionTracks | null, frameIndex: number, assets: RenderAssets) {
    const s = unitScale(W, H);
    const n = motion?.frames ?? 0;
    const i = n > 0 ? Math.max(0, Math.min(n - 1, frameIndex)) : -1;
    const clipIndex = i >= 0 ? motion!.clip[i] : project.clips.findIndex((c) => c.kind === "recording");
    const clip = project.clips[clipIndex];
    const outMs = i >= 0 ? (i * 1000) / motion!.fps : 0;
    const t = i >= 0 ? motion!.srcMs[i] : 0;

    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.globalAlpha = 1;
    ctx.globalCompositeOperation = "source-over";

    if (clip?.kind === "card") {
      ctx.drawImage(this.staticLayer(W, H, s, clip.background ?? project.frame.background, 0, null, 0, assets), 0, 0);
      const local = t - (motion?.clipStarts[clipIndex] ?? 0);
      drawCard(ctx, W, H, s, project, clip, local);
    } else if (clip) {
      this.drawRecording(ctx, W, H, s, project, clip, clipIndex, motion, i, t, outMs, assets);
    } else {
      ctx.drawImage(this.staticLayer(W, H, s, project.frame.background, 0, null, 0, assets), 0, 0);
    }

    if (!motion || i < 0) return;
    const cam = assets.clips[clipIndex]?.webcam;
    const alpha = motion.webcam[i * 4 + 3];
    if (cam && cam.videoWidth > 0 && alpha > 0.004) {
      const b = webcamBox(W, H, motion.webcam[i * 4], motion.webcam[i * 4 + 1], motion.webcam[i * 4 + 2], alpha);
      const src = coverRect(cam.videoWidth, cam.videoHeight, b.w, b.h);
      const shape = project.webcam.shape;
      ctx.save();
      ctx.globalAlpha = clamp01(alpha);
      roundedRectPath(ctx, b, shape === "circle" ? b.w / 2 : shape === "square" ? b.w * 0.04 : b.w * 0.18);
      ctx.clip();
      try {
        ctx.drawImage(cam, src.x, src.y, src.w, src.h, b.x, b.y, b.w, b.h);
      } catch {
        // Webcam frame not ready yet.
      }
      ctx.restore();
    }

    if (project.keystrokes.show) {
      const key = keyAt(motion.keys, project.keystrokes.holdMs, outMs);
      if (key) drawKeys(ctx, W, H, s, project, key.combo, key.alpha);
    }
    if (project.captions.show) {
      const line = captionAt(motion.captions, outMs);
      if (line) drawCaption(ctx, W, H, s, project, line, outMs);
    }
  }

  private drawRecording(ctx: Ctx, W: number, H: number, s: number, project: Project, clip: RecordingClip, clipIndex: number, motion: MotionTracks | null, i: number, t: number, outMs: number, assets: RenderAssets) {
    const crop = clip.crop;
    const frame = frameRect(W, H, project.frame.padding, crop);
    const radius = project.frame.radius * s;
    ctx.drawImage(this.staticLayer(W, H, s, project.frame.background, project.frame.shadow, frame, radius, assets), 0, 0);

    const cx = i >= 0 ? motion!.cam[i * 3] : crop.x + crop.width / 2;
    const cy = i >= 0 ? motion!.cam[i * 3 + 1] : crop.y + crop.height / 2;
    const scale = i >= 0 ? Math.max(1, motion!.cam[i * 3 + 2]) : 1;
    const view = viewRect(crop, cx, cy, scale);
    const unit = s * Math.sqrt(scale);

    ctx.save();
    roundedRectPath(ctx, frame, radius);
    ctx.clip();
    ctx.fillStyle = "#000";
    ctx.fillRect(frame.x, frame.y, frame.w, frame.h);
    const source = assets.clips[clipIndex]?.source;
    if (source) {
      ctx.imageSmoothingEnabled = true;
      ctx.imageSmoothingQuality = "high";
      try {
        ctx.drawImage(source, view.x, view.y, view.w, view.h, frame.x, frame.y, frame.w, frame.h);
      } catch {
        // Video not ready yet.
      }
    }

    for (const a of project.annotations) {
      const alpha = annotationAlpha(a, t);
      if (alpha > 0) this.drawAnnotation(ctx, W, H, project, a, alpha, frame, view, unit);
    }

    const drawCursor = motion?.cursorDrawn[clipIndex] ?? false;
    if (motion && i >= 0 && drawCursor) {
      if (project.cursor.ripples) {
        for (const r of motion.ripples) {
          const age = outMs - r.outMs;
          if (r.clip !== clipIndex || age < 0 || age >= RIPPLE_MS) continue;
          const p = age / RIPPLE_MS;
          const c = mapPoint(frame, view, r.x, r.y);
          ctx.beginPath();
          ctx.arc(c.x, c.y, (8 + 36 * (1 - Math.pow(1 - p, 3))) * unit, 0, Math.PI * 2);
          ctx.globalAlpha = 0.55 * (1 - p);
          ctx.lineWidth = 3 * s;
          ctx.strokeStyle = project.cursor.rippleColor;
          ctx.stroke();
        }
        ctx.globalAlpha = 1;
      }
      const alpha = motion.cursor[i * 4 + 2];
      if (project.cursor.show && alpha > 0.003) {
        const press = motion.cursor[i * 4 + 3];
        const tip = mapPoint(frame, view, motion.cursor[i * 4], motion.cursor[i * 4 + 1]);
        const h = 26 * project.cursor.size * unit * (1 - 0.15 * press);
        ctx.beginPath();
        ARROW.forEach(([x, y], k) => (k ? ctx.lineTo(tip.x + x * h, tip.y + y * h) : ctx.moveTo(tip.x + x * h, tip.y + y * h)));
        ctx.closePath();
        ctx.globalAlpha = alpha;
        ctx.lineJoin = "round";
        ctx.lineWidth = 0.07 * h;
        ctx.strokeStyle = "#fff";
        ctx.shadowColor = "rgba(0,0,0,0.35)";
        ctx.shadowBlur = 0.22 * h;
        ctx.shadowOffsetY = 0.06 * h;
        ctx.stroke();
        ctx.shadowColor = "transparent";
        ctx.shadowBlur = 0;
        ctx.shadowOffsetY = 0;
        ctx.fillStyle = "#000";
        ctx.fill();
        ctx.globalAlpha = 1;
      }
    }
    ctx.restore();
  }

  /** Called with the context already clipped to the frame. */
  private drawAnnotation(ctx: Ctx, W: number, H: number, project: Project, a: Annotation, alpha: number, frame: Box, view: Box, unit: number) {
    // Output pixels per source pixel.
    const k = frame.w / view.w;
    const rect = (r: { x: number; y: number; width: number; height: number }): Box => {
      const o = mapPoint(frame, view, r.x, r.y);
      return { x: o.x, y: o.y, w: r.width * k, h: r.height * k };
    };
    ctx.save();
    switch (a.type) {
      case "blur":
        this.blurRegion(ctx, W, H, rect(a), frame, a.strength * k);
        break;
      case "spotlight": {
        const b = rect(a);
        ctx.beginPath();
        ctx.rect(frame.x, frame.y, frame.w, frame.h);
        if (a.ellipse) {
          ctx.moveTo(b.x + b.w, b.y + b.h / 2);
          ctx.ellipse(b.x + b.w / 2, b.y + b.h / 2, b.w / 2, b.h / 2, 0, 0, Math.PI * 2);
        } else {
          const r = Math.max(0, Math.min(10 * unit, b.w / 2, b.h / 2));
          ctx.moveTo(b.x + r, b.y);
          ctx.arcTo(b.x + b.w, b.y, b.x + b.w, b.y + b.h, r);
          ctx.arcTo(b.x + b.w, b.y + b.h, b.x, b.y + b.h, r);
          ctx.arcTo(b.x, b.y + b.h, b.x, b.y, r);
          ctx.arcTo(b.x, b.y, b.x + b.w, b.y, r);
          ctx.closePath();
        }
        ctx.fillStyle = `rgba(0,0,0,${clamp01(a.dim * alpha)})`;
        ctx.fill("evenodd");
        break;
      }
      case "highlight":
        ctx.globalCompositeOperation = "multiply";
        fillRounded(ctx, rect(a), 6 * unit, a.color, a.opacity * alpha);
        break;
      case "box":
        ctx.globalAlpha = alpha;
        ctx.strokeStyle = a.color;
        ctx.lineWidth = a.stroke * unit;
        ctx.lineJoin = "round";
        roundedRectPath(ctx, rect(a), a.radius * k);
        ctx.stroke();
        break;
      case "arrow": {
        const p1 = mapPoint(frame, view, a.x1, a.y1);
        const p2 = mapPoint(frame, view, a.x2, a.y2);
        const len = Math.hypot(p2.x - p1.x, p2.y - p1.y);
        if (len < 1) break;
        const w = a.width * unit;
        const dx = (p2.x - p1.x) / len;
        const dy = (p2.y - p1.y) / len;
        const head = Math.min(4.5 * w, 0.6 * len);
        const hx = p2.x - dx * head;
        const hy = p2.y - dy * head;
        const half = 0.55 * head;
        ctx.globalAlpha = alpha;
        ctx.strokeStyle = a.color;
        ctx.fillStyle = a.color;
        ctx.lineWidth = w;
        ctx.lineCap = "round";
        ctx.beginPath();
        ctx.moveTo(p1.x, p1.y);
        ctx.lineTo(p2.x - dx * head * 0.8, p2.y - dy * head * 0.8);
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(p2.x, p2.y);
        ctx.lineTo(hx - dy * half, hy + dx * half);
        ctx.lineTo(hx + dy * half, hy - dx * half);
        ctx.closePath();
        ctx.fill();
        break;
      }
      case "text": {
        const px = a.size * unit;
        const style: TextStyle = { family: a.font ?? project.theme.font, bold: a.bold, px };
        const lines = a.text.split("\n");
        const o = mapPoint(frame, view, a.x, a.y);
        const padX = a.background ? 0.45 * px : 0;
        const padY = a.background ? 0.3 * px : 0;
        if (a.background) {
          const widest = Math.max(0, ...lines.map((l) => measure(ctx, style, l)));
          fillRounded(ctx, { x: o.x, y: o.y, w: widest + 2 * padX, h: lines.length * LINE_HEIGHT * px + 2 * padY }, 0.3 * px, a.background, alpha);
        }
        lines.forEach((line, n) => drawText(ctx, style, line, o.x + padX, o.y + padY + (FIRST_BASELINE + n * LINE_HEIGHT) * px, a.color, alpha));
        break;
      }
    }
    ctx.restore();
  }

  /** Blur a rectangle of what has been drawn so far so it cannot be read. */
  private blurRegion(ctx: Ctx, W: number, H: number, r: Box, frame: Box, sigmaIn: number) {
    const bound = (v: number, lo: number, hi: number, max: number) => Math.max(0, Math.min(max, Math.round(Math.max(lo, Math.min(hi, v)))));
    const x0 = bound(r.x, frame.x, frame.x + frame.w, W);
    const x1 = bound(r.x + r.w, frame.x, frame.x + frame.w, W);
    const y0 = bound(r.y, frame.y, frame.y + frame.h, H);
    const y1 = bound(r.y + r.h, frame.y, frame.y + frame.h, H);
    if (x1 - x0 < 2 || y1 - y0 < 2) return;
    const sigma = Math.max(1.5, sigmaIn);
    // Blur a slightly larger area so the edges are as soft as the middle.
    const m = Math.ceil(2 * sigma);
    const ex0 = bound(x0 - m, frame.x, frame.x + frame.w, W);
    const ex1 = bound(x1 + m, frame.x, frame.x + frame.w, W);
    const ey0 = bound(y0 - m, frame.y, frame.y + frame.h, H);
    const ey1 = bound(y1 + m, frame.y, frame.y + frame.h, H);
    const ew = ex1 - ex0;
    const eh = ey1 - ey0;

    // Shrink the area into a small buffer and stretch it back: an opaque,
    // unreadable base that works without canvas filters.
    const f = Math.max(2, sigma);
    const tw = Math.max(1, Math.round(ew / f));
    const th = Math.max(1, Math.round(eh / f));
    const tmp = this.buffer("scratch", Math.max(tw, 1), Math.max(th, 1));
    const t = tmp.getContext("2d")!;
    t.imageSmoothingEnabled = true;
    t.imageSmoothingQuality = "high";
    t.drawImage(ctx.canvas, ex0, ey0, ew, eh, 0, 0, tw, th);
    ctx.save();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.beginPath();
    ctx.rect(x0, y0, x1 - x0, y1 - y0);
    ctx.clip();
    ctx.imageSmoothingEnabled = true;
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(tmp, 0, 0, tw, th, ex0, ey0, ew, eh);
    if (hasFilter(ctx)) {
      // Smooth the blockiness of the base.
      ctx.filter = `blur(${sigma / 2}px)`;
      ctx.drawImage(tmp, 0, 0, tw, th, ex0, ey0, ew, eh);
    }
    ctx.restore();
  }
}

/** Title and subtitle of a card, fading up into place. */
function drawCard(ctx: Ctx, W: number, H: number, s: number, project: Project, card: CardClip, localMs: number) {
  const family = card.font ?? project.theme.font;
  const left = card.align === "left";
  const limit = left ? 0.8 * W : 0.84 * W;
  const fit = (lines: string[], px: number, bold: boolean) => {
    const widest = Math.max(0, ...lines.map((l) => measure(ctx, { family, bold, px }, l)));
    return widest > limit ? (px * limit) / widest : px;
  };
  const titles = card.title.split("\n").filter(Boolean);
  const subs = card.subtitle.split("\n").filter(Boolean);
  const tpx = fit(titles, card.titleSize * s, true);
  const spx = fit(subs, 0.42 * card.titleSize * s, false);
  const gap = titles.length && subs.length ? 0.35 * tpx : 0;
  const block = titles.length * LINE_HEIGHT * tpx + gap + subs.length * LINE_HEIGHT * spx;
  let y = (H - block) / 2;
  const draw = (lines: string[], px: number, bold: boolean, color: string, delay: number) => {
    const p = smoothstep((localMs - delay) / CARD_INTRO_MS);
    const style = { family, bold, px };
    for (const line of lines) {
      const x = left ? 0.1 * W : (W - measure(ctx, style, line)) / 2;
      drawText(ctx, style, line, x, y + FIRST_BASELINE * px + (1 - p) * 24 * s, color, p);
      y += LINE_HEIGHT * px;
    }
  };
  draw(titles, tpx, true, card.titleColor, 0);
  y += gap;
  draw(subs, spx, false, card.subtitleColor, 150);
}

/** The pressed shortcut as a row of keycaps. */
function drawKeys(ctx: Ctx, W: number, H: number, s: number, project: Project, combo: string, alpha: number) {
  const keys = combo.split("+").filter(Boolean);
  if (!keys.length || alpha <= 0) return;
  const cap = 54 * s * project.keystrokes.size;
  const style: TextStyle = { family: project.theme.font, bold: true, px: 0.46 * cap };
  const gap = 0.16 * cap;
  const pad = 0.2 * cap;
  const widths = keys.map((k) => Math.max(cap, measure(ctx, style, k) + 0.7 * cap));
  const total = widths.reduce((a, b) => a + b, 0) + gap * (keys.length - 1) + 2 * pad;
  const boxH = cap + 2 * pad;
  const margin = 0.045 * Math.min(W, H);
  const pos = project.keystrokes.position;
  const x = pos === "bottom-left" ? margin : pos === "bottom-right" ? W - margin - total : (W - total) / 2;
  const y = pos === "top" ? margin : H - margin - boxH;
  fillRounded(ctx, { x, y, w: total, h: boxH }, 0.34 * cap, project.theme.surface, alpha);
  let kx = x + pad;
  keys.forEach((key, n) => {
    const kw = widths[n];
    fillRounded(ctx, { x: kx, y: y + pad, w: kw, h: cap }, 0.22 * cap, "rgba(255,255,255,0.141)", alpha);
    drawText(ctx, style, key, kx + (kw - measure(ctx, style, key)) / 2, y + pad + cap / 2 + CENTER_BASELINE * style.px, project.theme.text, alpha);
    kx += kw + gap;
  });
}

function drawCaption(ctx: Ctx, W: number, H: number, s: number, project: Project, line: CaptionLine, outMs: number) {
  const c = project.captions;
  const words = line.words.map((w) => (c.uppercase ? w.text.toUpperCase() : w.text));
  const family = c.font ?? project.theme.font;
  let px = 44 * s * c.size;
  const width = (size: number) => {
    const st = { family, bold: true, px: size };
    return words.reduce((sum, w) => sum + measure(ctx, st, w), 0) + measure(ctx, st, " ") * Math.max(0, words.length - 1);
  };
  // Shrink lines that would not fit.
  const natural = width(px);
  if (natural > 0.9 * W) px *= (0.9 * W) / natural;
  const style: TextStyle = { family, bold: true, px };
  const space = measure(ctx, style, " ");
  const total = width(px);
  const padX = 0.5 * px;
  const padY = 0.28 * px;
  const cy = c.y * H;
  const x0 = (W - total) / 2;
  if (c.background) {
    const bh = LINE_HEIGHT * px + 2 * padY;
    fillRounded(ctx, { x: x0 - padX, y: cy - bh / 2, w: total + 2 * padX, h: bh }, 0.3 * px, c.background);
  }
  const baseline = cy + CENTER_BASELINE * px;
  // The word being spoken: the last one that has started.
  let current = -1;
  line.words.forEach((w, n) => {
    if (w.startMs <= outMs) current = n;
  });
  let x = x0;
  words.forEach((word, n) => {
    const karaoke = c.style === "karaoke" && current === n;
    const alpha = c.style === "pop" ? clamp01((outMs - line.words[n].startMs) / 90) : 1;
    drawText(ctx, style, word, x, baseline, karaoke ? c.highlight : c.color, alpha);
    x += measure(ctx, style, word) + space;
  });
}

/** Blend two finished frames. `p` runs 0..1 from `a` (outgoing) to `b` (incoming). */
export function blendTransition(ctx: Ctx, W: number, H: number, kind: TransitionKind, p: number, a: CanvasImageSource, b: CanvasImageSource) {
  const e = smoothstep(p);
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.globalAlpha = 1;
  if (kind === "cut") {
    ctx.drawImage(p < 0.5 ? a : b, 0, 0);
    return;
  }
  if (kind === "crossfade") {
    ctx.drawImage(a, 0, 0);
    ctx.globalAlpha = e;
    ctx.drawImage(b, 0, 0);
    ctx.globalAlpha = 1;
    return;
  }
  // Push through the outgoing frame while the incoming one grows out of its centre.
  const scaled = (img: CanvasImageSource, scale: number) => ctx.drawImage(img, (W - W * scale) / 2, (H - H * scale) / 2, W * scale, H * scale);
  ctx.imageSmoothingEnabled = true;
  scaled(a, 1 + 1.2 * e);
  ctx.globalAlpha = Math.min(1, e * 1.6);
  scaled(b, 0.4 + 0.6 * e);
  ctx.globalAlpha = 1;
}

/** What is visible of a recording clip at an output frame, for editing overlays. */
export function frameGeometry(W: number, H: number, project: Project, motion: MotionTracks | null, frameIndex: number): { clipIndex: number; frame: Box; view: Box } | null {
  const n = motion?.frames ?? 0;
  const i = n > 0 ? Math.max(0, Math.min(n - 1, frameIndex)) : -1;
  const clipIndex = i >= 0 ? motion!.clip[i] : project.clips.findIndex((c) => c.kind === "recording");
  const clip = project.clips[clipIndex];
  if (!clip || clip.kind !== "recording") return null;
  const frame = frameRect(W, H, project.frame.padding, clip.crop);
  const view = i >= 0 ? viewRect(clip.crop, motion!.cam[i * 3], motion!.cam[i * 3 + 1], Math.max(1, motion!.cam[i * 3 + 2])) : viewRect(clip.crop, clip.crop.x + clip.crop.width / 2, clip.crop.y + clip.crop.height / 2, 1);
  return { clipIndex, frame, view };
}
