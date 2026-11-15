import { describe, expect, it } from "vitest";
import type { MotionTracks } from "../lib/types";
import { blurSamples, trailAlpha } from "./renderer";

/** Motion tracks of `n` frames in one clip with camera x and cursor x per frame. */
function tracks(camX: (i: number) => number, curX: (i: number) => number, n = 10): MotionTracks {
  return {
    fps: 60,
    frames: n,
    durationMs: (n * 1000) / 60,
    srcMs: Array.from({ length: n }, (_, i) => (i * 1000) / 60),
    clip: new Array(n).fill(0),
    clipStarts: [0],
    cam: Array.from({ length: n }, (_, i) => [camX(i), 540, 2]).flat(),
    cursor: Array.from({ length: n }, (_, i) => [curX(i), 500, 1, 0]).flat(),
    webcam: new Array(n * 4).fill(0),
    ripples: [],
    events: { clicks: [], keys: [] },
    keys: [],
    captions: [],
    transitions: [],
    cursorDrawn: [true],
    cursorMode: "metadata",
  } as unknown as MotionTracks;
}

// The same cases as `blur_samples` in motion/mod.rs.
describe("motion blur samples", () => {
  it("leave still frames alone", () => {
    const b = blurSamples(tracks(() => 960, () => 400), 5, 0.5, 1920, 1920);
    expect(b.cams).toEqual([]);
    expect(b.trail).toEqual([]);
  });

  it("take more samples the further the picture moves", () => {
    // 40 source px a frame at 2× zoom on a 1:1 frame: 80 output px, half
    // of it inside a 180° shutter.
    const b = blurSamples(tracks((i) => 900 + 40 * i, () => 400), 5, 0.5, 1920, 1920);
    expect(b.cams).toHaveLength(12);
    expect(b.cams[0][0]).toBeCloseTo(900 + 40 * (4.75 + 0.5 / 24), 6);
    const slow = blurSamples(tracks((i) => 900 + 6 * i, () => 400), 5, 0.5, 1920, 1920);
    expect(slow.cams).toHaveLength(3);
    expect(blurSamples(tracks((i) => 900 + 2 * i, () => 400), 5, 0.5, 1920, 1920).cams).toEqual([]);
  });

  it("follow a fast cursor and never cross into another clip", () => {
    const t = tracks(() => 960, (i) => 100 + 30 * i);
    expect(blurSamples(t, 5, 0.5, 1920, 1920).trail.length).toBeGreaterThan(2);
    t.clip = [0, 0, 0, 0, 0, 1, 1, 1, 1, 1];
    const edge = blurSamples(t, 5, 1, 1920, 1920);
    expect(edge.trail.every(([x]) => x >= 100 + 30 * 5)).toBe(true);
  });

  it("fade cursor copies so they add up to about one", () => {
    expect(trailAlpha(1)).toBe(1);
    expect(trailAlpha(3)).toBeCloseTo(0.5, 9);
  });
});
