import { describe, expect, it } from "vitest";
import { aspectRatio, clampCentre, coverRect, fitSize, frameRect, gradientLine, mapPoint, outputFps, outputSize, unitScale, viewRect, webcamBox, cornerCentre } from "./layout";
import type { ExportSettings } from "./types";

const crop = { x: 0, y: 0, width: 2560, height: 1440 };
const mp4 = (preset: string): ExportSettings => ({ format: "mp4", preset, fps: 60, quality: "high" });

describe("outputSize", () => {
  it("uses the preset as the shorter side", () => {
    expect(outputSize("16:9", crop, mp4("1080p"))).toEqual({ width: 1920, height: 1080 });
    expect(outputSize("9:16", crop, mp4("1080p"))).toEqual({ width: 1080, height: 1920 });
    expect(outputSize("1:1", crop, mp4("720p"))).toEqual({ width: 720, height: 720 });
    expect(outputSize("4:3", crop, mp4("4k"))).toEqual({ width: 2880, height: 2160 });
  });
  it("follows the crop for the source aspect", () => {
    expect(aspectRatio("source", { x: 0, y: 0, width: 1000, height: 500 })).toBe(2);
    expect(outputSize("source", { x: 0, y: 0, width: 1000, height: 500 }, mp4("720p"))).toEqual({ width: 1440, height: 720 });
  });
  it("sizes GIFs by width and keeps dimensions even", () => {
    expect(outputSize("16:9", crop, { format: "gif", preset: "readme", fps: 60, quality: "high" })).toEqual({ width: 800, height: 450 });
    const s = outputSize("9:16", crop, { format: "gif", preset: "small", fps: 60, quality: "high" });
    expect(s.width).toBe(480);
    expect(s.height % 2).toBe(0);
  });
  it("derives the frame rate", () => {
    expect(outputFps(mp4("1080p"))).toBe(60);
    expect(outputFps({ format: "gif", preset: "readme", fps: 60, quality: "high" })).toBe(15);
    expect(outputFps({ format: "gif", preset: "small", fps: 60, quality: "high" })).toBe(12);
  });
});

describe("frameRect", () => {
  it("centres the recording inside the padding", () => {
    const f = frameRect(1920, 1080, 0.1, crop);
    expect(f.h).toBeCloseTo(1080 - 216);
    expect(f.w).toBeCloseTo((1080 - 216) * (16 / 9));
    expect(f.x).toBeCloseTo((1920 - f.w) / 2);
    expect(f.y).toBeCloseTo(108);
  });
  it("letterboxes a wide recording in a tall output", () => {
    const f = frameRect(1080, 1920, 0.05, crop);
    expect(f.w).toBeCloseTo(1080 - 108);
    expect(f.h).toBeCloseTo((1080 - 108) / (16 / 9));
    expect(f.y + f.h / 2).toBeCloseTo(960);
  });
  it("fills the output with no padding and matching aspect", () => {
    expect(frameRect(1920, 1080, 0, crop)).toEqual({ x: 0, y: 0, w: 1920, h: 1080 });
  });
});

describe("camera", () => {
  it("computes the visible rect", () => {
    expect(viewRect(crop, 1280, 720, 1)).toEqual({ x: 0, y: 0, w: 2560, h: 1440 });
    expect(viewRect(crop, 1000, 500, 2)).toEqual({ x: 360, y: 140, w: 1280, h: 720 });
  });
  it("clamps the centre to the crop", () => {
    expect(clampCentre(crop, 0, 0, 2)).toEqual({ cx: 640, cy: 360 });
    expect(clampCentre(crop, 9999, 9999, 2)).toEqual({ cx: 1920, cy: 1080 });
    expect(clampCentre(crop, 5, 5, 1)).toEqual({ cx: 1280, cy: 720 });
    expect(clampCentre({ x: 100, y: 50, width: 800, height: 600 }, 0, 0, 2)).toEqual({ cx: 300, cy: 200 });
  });
  it("maps source points into the frame", () => {
    const frame = { x: 100, y: 50, w: 1600, h: 900 };
    const view = viewRect(crop, 1000, 500, 2);
    expect(mapPoint(frame, view, 1000, 500)).toEqual({ x: 900, y: 500 });
    expect(mapPoint(frame, view, view.x, view.y)).toEqual({ x: 100, y: 50 });
    expect(mapPoint(frame, view, view.x + view.w, view.y + view.h)).toEqual({ x: 1700, y: 950 });
  });
});

describe("helpers", () => {
  it("fits by aspect", () => {
    expect(fitSize(2, 400, 400)).toEqual({ width: 400, height: 200 });
    expect(fitSize(0.5, 400, 400)).toEqual({ width: 200, height: 400 });
    expect(fitSize(1, 0, 10)).toEqual({ width: 0, height: 0 });
  });
  it("scales units from 1080p", () => {
    expect(unitScale(1920, 1080)).toBe(1);
    expect(unitScale(1080, 1920)).toBe(1);
    expect(unitScale(3840, 2160)).toBe(2);
  });
  it("places the webcam in a corner", () => {
    const at = (corner: "bottom-right" | "top-left") => {
      const c = cornerCentre(corner, 0.2, 16 / 9);
      return webcamBox(1920, 1080, c.x, c.y, 0.2, 1);
    };
    const b = at("bottom-right");
    expect(b.w).toBeCloseTo(216);
    expect(b.x).toBeCloseTo(1920 - 32.4 - 216);
    expect(b.y).toBeCloseTo(1080 - 32.4 - 216);
    const t = at("top-left");
    expect(t.x).toBeCloseTo(32.4);
    expect(t.y).toBeCloseTo(32.4);
    // Vertical output: the margin is still measured on the shorter side.
    const v = cornerCentre("top-left", 0.2, 9 / 16);
    const tall = webcamBox(1080, 1920, v.x, v.y, 0.2, 1);
    expect(tall.x).toBeCloseTo(32.4);
    expect(tall.y).toBeCloseTo(32.4);
    // A fading bubble shrinks around its centre.
    const faded = webcamBox(1920, 1080, 0.5, 0.5, 0.2, 0);
    expect(faded.w).toBeCloseTo(216 * 0.85);
    expect(faded.x + faded.w / 2).toBeCloseTo(960);
  });
  it("resolves export presets", () => {
    const crop = { x: 0, y: 0, width: 1920, height: 1080 };
    const exp = (format: "mp4" | "gif", preset: string) => ({ format, preset, fps: 60, quality: "high" as const });
    expect(outputSize("16:9", crop, exp("gif", "readme"))).toEqual({ width: 800, height: 450 });
    expect(outputFps(exp("gif", "readme"))).toBe(15);
    expect(outputSize("16:9", crop, exp("mp4", "shorts"))).toEqual({ width: 1080, height: 1920 });
    expect(outputFps(exp("mp4", "shorts"))).toBe(30);
    expect(outputSize("16:9", crop, exp("mp4", "youtube"))).toEqual({ width: 2560, height: 1440 });
    expect(outputFps(exp("mp4", "1080p"))).toBe(60);
    // A preset of the other format falls back to that format's default.
    expect(outputSize("16:9", crop, exp("mp4", "readme"))).toEqual({ width: 1920, height: 1080 });
    expect(outputSize("16:9", crop, exp("gif", "4k"))).toEqual({ width: 800, height: 450 });
  });
  it("cover-crops", () => {
    expect(coverRect(1280, 720, 100, 100)).toEqual({ x: 280, y: 0, w: 720, h: 720 });
    expect(coverRect(720, 1280, 100, 100)).toEqual({ x: 0, y: 280, w: 720, h: 720 });
  });
  it("matches CSS gradient geometry", () => {
    const down = gradientLine(100, 50, 180);
    expect(down.x0).toBeCloseTo(50);
    expect(down.y0).toBeCloseTo(0);
    expect(down.y1).toBeCloseTo(50);
    const right = gradientLine(100, 50, 90);
    expect(right.x0).toBeCloseTo(0);
    expect(right.x1).toBeCloseTo(100);
    expect(right.y0).toBeCloseTo(25);
  });
});
