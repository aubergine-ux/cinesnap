// Canvas 2D preview compositor. Drawing rules are shared with the export
// compositor: change them in both places or the preview stops being WYSIWYG.

import { coverRect, frameRect, gradientLine, mapPoint, unitScale, viewRect, webcamRect, type Box } from "../lib/layout";
import type { Background, MotionTracks, Project } from "../lib/types";

export interface RenderAssets {
  /** The recording (video element, or a canvas in demo mode). */
  source: CanvasImageSource | null;
  /** Background image / wallpaper, when loaded. */
  backdrop: HTMLImageElement | null;
  webcam: HTMLVideoElement | null;
  /** False when the cursor is baked into the video and must not be redrawn. */
  drawCursor: boolean;
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

export function roundedRectPath(ctx: CanvasRenderingContext2D, b: Box, radius: number) {
  const r = Math.max(0, Math.min(radius, b.w / 2, b.h / 2));
  ctx.beginPath();
  ctx.moveTo(b.x + r, b.y);
  ctx.arcTo(b.x + b.w, b.y, b.x + b.w, b.y + b.h, r);
  ctx.arcTo(b.x + b.w, b.y + b.h, b.x, b.y + b.h, r);
  ctx.arcTo(b.x, b.y + b.h, b.x, b.y, r);
  ctx.arcTo(b.x, b.y, b.x + b.w, b.y, r);
  ctx.closePath();
}

function drawBackground(ctx: CanvasRenderingContext2D, W: number, H: number, s: number, bg: Background, image: HTMLImageElement | null) {
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
    } else if ("filter" in ctx && typeof ctx.filter === "string") {
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

/** Caches the static layer (background + frame shadow) between frames. */
export class Renderer {
  private layer: HTMLCanvasElement | null = null;
  private layerKey = "";

  private staticLayer(W: number, H: number, project: Project, frame: Box, radius: number, s: number, backdrop: HTMLImageElement | null) {
    const f = project.frame;
    const key = JSON.stringify([W, H, f.background, f.shadow, radius, frame, backdrop?.src ?? null, backdrop?.complete ?? false]);
    if (this.layer && key === this.layerKey) return this.layer;
    const layer = this.layer ?? document.createElement("canvas");
    layer.width = W;
    layer.height = H;
    const ctx = layer.getContext("2d")!;
    drawBackground(ctx, W, H, s, f.background, backdrop);
    if (f.shadow > 0) {
      ctx.save();
      ctx.shadowColor = `rgba(0,0,0,${Math.min(1, f.shadow * 0.6)})`;
      ctx.shadowBlur = 40 * s;
      ctx.shadowOffsetY = 16 * s;
      ctx.fillStyle = "#000";
      roundedRectPath(ctx, frame, radius);
      ctx.fill();
      ctx.restore();
    }
    this.layer = layer;
    this.layerKey = key;
    return layer;
  }

  draw(ctx: CanvasRenderingContext2D, W: number, H: number, project: Project, motion: MotionTracks | null, frameIndex: number, assets: RenderAssets) {
    const s = unitScale(W, H);
    const crop = project.crop;
    const frame = frameRect(W, H, project.frame.padding, crop);
    const radius = project.frame.radius * s;

    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.globalAlpha = 1;
    ctx.drawImage(this.staticLayer(W, H, project, frame, radius, s, assets.backdrop), 0, 0);

    const n = motion?.frames ?? 0;
    const i = n > 0 ? Math.max(0, Math.min(n - 1, frameIndex)) : -1;
    const cx = i >= 0 ? motion!.cam[i * 3] : crop.x + crop.width / 2;
    const cy = i >= 0 ? motion!.cam[i * 3 + 1] : crop.y + crop.height / 2;
    const scale = i >= 0 ? Math.max(1, motion!.cam[i * 3 + 2]) : 1;
    const view = viewRect(crop, cx, cy, scale);
    const zoomUnit = s * Math.sqrt(scale);

    ctx.save();
    roundedRectPath(ctx, frame, radius);
    ctx.clip();
    ctx.fillStyle = "#000";
    ctx.fillRect(frame.x, frame.y, frame.w, frame.h);
    if (assets.source) {
      ctx.imageSmoothingEnabled = true;
      ctx.imageSmoothingQuality = "high";
      try {
        ctx.drawImage(assets.source, view.x, view.y, view.w, view.h, frame.x, frame.y, frame.w, frame.h);
      } catch {
        // Video not ready yet.
      }
    }

    if (motion && i >= 0) {
      const outMs = (i * 1000) / motion.fps;

      if (project.cursor.ripples && assets.drawCursor) {
        for (const r of motion.ripples) {
          const age = outMs - r.outMs;
          if (age < 0 || age >= RIPPLE_MS) continue;
          const p = age / RIPPLE_MS;
          const c = mapPoint(frame, view, r.x, r.y);
          ctx.beginPath();
          ctx.arc(c.x, c.y, (8 + 36 * (1 - Math.pow(1 - p, 3))) * zoomUnit, 0, Math.PI * 2);
          ctx.globalAlpha = 0.55 * (1 - p);
          ctx.lineWidth = 3 * s;
          ctx.strokeStyle = project.cursor.rippleColor;
          ctx.stroke();
        }
        ctx.globalAlpha = 1;
      }

      if (project.cursor.show && assets.drawCursor) {
        const alpha = motion.cursor[i * 4 + 2];
        if (alpha > 0.003) {
          const press = motion.cursor[i * 4 + 3];
          const tip = mapPoint(frame, view, motion.cursor[i * 4], motion.cursor[i * 4 + 1]);
          const h = 26 * project.cursor.size * zoomUnit * (1 - 0.15 * press);
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
    }
    ctx.restore();

    const cam = assets.webcam;
    if (project.webcam.show && cam && cam.videoWidth > 0) {
      const b = webcamRect(W, H, project.webcam.size, project.webcam.corner);
      const src = coverRect(cam.videoWidth, cam.videoHeight, b.w, b.h);
      ctx.save();
      roundedRectPath(ctx, b, project.webcam.shape === "circle" ? b.w / 2 : b.w * 0.18);
      ctx.clip();
      try {
        ctx.drawImage(cam, src.x, src.y, src.w, src.h, b.x, b.y, b.w, b.h);
      } catch {
        // Webcam frame not ready yet.
      }
      ctx.restore();
    }
  }
}
