import { describe, expect, it } from "vitest";
import { clampZoom, formatTime, pxToMs, tickStep, ticks, zoomAround } from "./timeline";

describe("timeline geometry", () => {
  it("converts between time and pixels", () => {
    expect(pxToMs(200, 0.1)).toBe(2000);
    expect(pxToMs(200, 0)).toBe(0);
  });
  it("picks readable tick steps", () => {
    expect(tickStep(0.1)).toBe(1000);
    expect(tickStep(0.01)).toBe(10000);
    expect(tickStep(1)).toBe(100);
    expect(ticks(3000, 0.1)).toEqual([0, 1000, 2000, 3000]);
  });
  it("never zooms out past fit", () => {
    expect(clampZoom(0.001, 10000, 1000)).toBe(0.1);
    expect(clampZoom(0.5, 10000, 1000)).toBe(0.5);
    expect(clampZoom(50, 10000, 1000)).toBe(2);
  });
  it("keeps the anchor time under the pointer when zooming", () => {
    const before = { pxPerMs: 0.1, scrollPx: 0 };
    const anchorPx = 400;
    const after = zoomAround(before.pxPerMs, before.scrollPx, anchorPx, 2, 10000, 1000);
    expect(after.pxPerMs).toBe(0.2);
    expect(pxToMs(after.scrollPx + anchorPx, after.pxPerMs)).toBeCloseTo(pxToMs(anchorPx, before.pxPerMs));
  });
  it("clamps scroll when zooming back out", () => {
    expect(zoomAround(0.2, 1000, 900, 0.5, 10000, 1000)).toEqual({ pxPerMs: 0.1, scrollPx: 0 });
  });
  it("formats time", () => {
    expect(formatTime(0)).toBe("0:00.0");
    expect(formatTime(65432)).toBe("1:05.4");
    expect(formatTime(65432, false)).toBe("1:05");
    expect(formatTime(-5)).toBe("0:00.0");
  });
});
