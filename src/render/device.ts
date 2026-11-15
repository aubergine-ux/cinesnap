// Device frames around the recording, drawn from the shared layout. Kept in
// step with `device.rs`.

import type { Box, FrameLayout } from "../lib/layout";
import { CENTER_BASELINE, drawText, measure, type TextStyle } from "./text";

type Ctx = CanvasRenderingContext2D;

export function palette(dark: boolean) {
  return dark
    ? { body: "#1c1d23", bar: "#26272f", field: "#33353f", text: "#d4d6de", base: "#2a2b31", baseTop: "#3d3e45" }
    : { body: "#f2f2f5", bar: "#e4e5ea", field: "#ffffff", text: "#3b3d45", base: "#c6c7cd", baseTop: "#dcdde2" };
}

export const TRAFFIC = ["#ff5f57", "#febc2e", "#28c840"];
export const TITLE_PX = 14;
const K = 0.44771525; // 1 - kappa

/** A rectangle with a radius per corner: top left, top right, bottom right, bottom left. */
export function roundedCornersPath(ctx: Ctx, b: Box, radii: readonly number[]) {
  const lim = Math.min(b.w / 2, b.h / 2);
  const [tl, tr, br, bl] = radii.map((r) => Math.max(0, Math.min(r, lim)));
  const { x, y, w, h } = b;
  ctx.beginPath();
  ctx.moveTo(x + tl, y);
  ctx.lineTo(x + w - tr, y);
  ctx.bezierCurveTo(x + w - tr * K, y, x + w, y + tr * K, x + w, y + tr);
  ctx.lineTo(x + w, y + h - br);
  ctx.bezierCurveTo(x + w, y + h - br * K, x + w - br * K, y + h, x + w - br, y + h);
  ctx.lineTo(x + bl, y + h);
  ctx.bezierCurveTo(x + bl * K, y + h, x, y + h - bl * K, x, y + h - bl);
  ctx.lineTo(x, y + tl);
  ctx.bezierCurveTo(x, y + tl * K, x + tl * K, y, x + tl, y);
  ctx.closePath();
}

function fill(ctx: Ctx, b: Box, radii: readonly number[], color: string, alpha = 1) {
  roundedCornersPath(ctx, b, radii);
  ctx.globalAlpha = alpha;
  ctx.fillStyle = color;
  ctx.fill();
  ctx.globalAlpha = 1;
}

function circle(ctx: Ctx, x: number, y: number, r: number, color: string) {
  ctx.beginPath();
  ctx.arc(x, y, r, 0, Math.PI * 2);
  ctx.fillStyle = color;
  ctx.fill();
}

/** The parts of the device under the recording. */
export function drawDeviceBody(ctx: Ctx, l: FrameLayout, device: { title: string; dark: boolean }, s: number) {
  if (l.device === "none") return;
  const pal = palette(device.dark);
  const { frame: f, body: b } = l;
  const r = l.bodyRadius;
  ctx.save();
  if (l.device === "laptop") {
    fill(ctx, b, [r, r, r, r], pal.body);
    const short = Math.min(f.w + l.insets.left + l.insets.right, f.h + l.insets.top + l.insets.bottom);
    const bezel = f.x - b.x;
    circle(ctx, b.x + b.w / 2, b.y + bezel / 2, 0.18 * Math.min(bezel, 0.03 * short), "#0a0a0c");
    const bx0 = f.x - l.insets.left;
    const bx1 = f.x + f.w + l.insets.right;
    const by0 = b.y + b.h;
    const bh = f.y + f.h + l.insets.bottom - by0;
    fill(ctx, { x: bx0, y: by0, w: bx1 - bx0, h: bh }, [0, 0, 0.6 * bh, 0.6 * bh], pal.base);
    fill(ctx, { x: bx0, y: by0, w: bx1 - bx0, h: 0.28 * bh }, [0, 0, 0, 0], pal.baseTop);
    const nw = 0.14 * b.w;
    fill(ctx, { x: b.x + (b.w - nw) / 2, y: by0, w: nw, h: 0.32 * bh }, [0, 0, 0.3 * bh, 0.3 * bh], pal.body, 0.6);
  } else if (l.device === "phone") {
    fill(ctx, b, [r, r, r, r], pal.body);
  } else {
    fill(ctx, b, [r, r, r, r], pal.bar);
    const cy = b.y + (f.y - b.y) / 2;
    if (l.device === "windows") {
      ctx.strokeStyle = pal.text;
      ctx.lineWidth = 1.2 * s;
      ctx.lineCap = "square";
      const g = 5 * s;
      for (let i = 0; i < 3; i++) {
        const x = b.x + b.w - 46 * (i + 0.5) * s;
        ctx.beginPath();
        if (i === 0) {
          ctx.moveTo(x - g, cy - g);
          ctx.lineTo(x + g, cy + g);
          ctx.moveTo(x + g, cy - g);
          ctx.lineTo(x - g, cy + g);
        } else if (i === 1) {
          ctx.rect(x - g, cy - g, 2 * g, 2 * g);
        } else {
          ctx.moveTo(x - g, cy);
          ctx.lineTo(x + g, cy);
        }
        ctx.stroke();
      }
    } else {
      TRAFFIC.forEach((c, i) => circle(ctx, b.x + (20 + 20 * i) * s, cy, 6 * s, c));
    }
    if (l.device === "browser") {
      const fx = b.x + 90 * s;
      const fw = b.w - 180 * s;
      const fh = 32 * s;
      if (fw > 40 * s) fill(ctx, { x: fx, y: cy - fh / 2, w: fw, h: fh }, [fh / 2, fh / 2, fh / 2, fh / 2], pal.field);
    }
    if (device.title.trim()) {
      const px = TITLE_PX * s;
      const style: TextStyle = { family: "Inter", bold: false, px };
      const x = l.device === "browser" ? b.x + 106 * s : l.device === "windows" ? b.x + 16 * s : b.x + (b.w - measure(ctx, style, device.title)) / 2;
      drawText(ctx, style, device.title, x, cy + CENTER_BASELINE * px, pal.text, 1);
    }
  }
  ctx.restore();
}

/** Parts drawn over the recording: a phone's camera cut-out. */
export function drawDeviceOver(ctx: Ctx, l: FrameLayout) {
  if (l.device !== "phone") return;
  const f = l.frame;
  const m = Math.min(f.w, f.h);
  const w = 0.3 * m;
  const h = 0.085 * m;
  ctx.save();
  fill(ctx, { x: f.x + (f.w - w) / 2, y: f.y + 0.035 * m, w, h }, [h / 2, h / 2, h / 2, h / 2], "#000000");
  ctx.restore();
}

