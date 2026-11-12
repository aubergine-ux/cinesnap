import { describe, expect, it } from "vitest";
import { addCut, cutAt, keptRanges, moveZoom, newZoomRange, normalizeCuts, outToSrc, outputDuration, resizeZoom, setTrimEnd, setTrimStart, srcToOut, trimBounds } from "./time";
import type { ZoomSegment } from "./types";

const D = 10000;
const z = (id: string, startMs: number, endMs: number): ZoomSegment => ({ id, startMs, endMs, scale: 2, easeInMs: 600, easeOutMs: 700, easing: "smooth", mode: "follow", x: 0, y: 0, manual: false });

describe("cuts", () => {
  it("normalises: sorts, clamps, merges, drops empties", () => {
    expect(
      normalizeCuts(
        [
          { startMs: 5000, endMs: 6000 },
          { startMs: -100, endMs: 500 },
          { startMs: 5500, endMs: 7000 },
          { startMs: 3000, endMs: 3000 },
          { startMs: 9000, endMs: 20000 },
          { startMs: 2000, endMs: 1500 },
        ],
        D,
      ),
    ).toEqual([
      { startMs: 0, endMs: 500 },
      { startMs: 1500, endMs: 2000 },
      { startMs: 5000, endMs: 7000 },
      { startMs: 9000, endMs: D },
    ]);
  });
  it("adds a cut and merges with neighbours", () => {
    expect(addCut([{ startMs: 1000, endMs: 2000 }], { startMs: 2000, endMs: 3000 }, D)).toEqual([{ startMs: 1000, endMs: 3000 }]);
  });
  it("computes kept ranges and output duration", () => {
    const cuts = [
      { startMs: 0, endMs: 1000 },
      { startMs: 4000, endMs: 6000 },
    ];
    expect(keptRanges(cuts, D)).toEqual([
      { startMs: 1000, endMs: 4000 },
      { startMs: 6000, endMs: D },
    ]);
    expect(outputDuration(cuts, D)).toBe(7000);
    expect(keptRanges([], D)).toEqual([{ startMs: 0, endMs: D }]);
    expect(outputDuration([{ startMs: 0, endMs: D }], D)).toBe(0);
  });
  it("finds the cut under a time", () => {
    const cuts = [{ startMs: 4000, endMs: 6000 }];
    expect(cutAt(cuts, 5000)).toBe(0);
    expect(cutAt(cuts, 6000)).toBe(-1);
  });
});

describe("time mapping", () => {
  const cuts = [
    { startMs: 0, endMs: 1000 },
    { startMs: 4000, endMs: 6000 },
  ];
  it("maps source to output", () => {
    expect(srcToOut(500, cuts, D)).toBe(0);
    expect(srcToOut(1000, cuts, D)).toBe(0);
    expect(srcToOut(2500, cuts, D)).toBe(1500);
    expect(srcToOut(5000, cuts, D)).toBe(3000);
    expect(srcToOut(7000, cuts, D)).toBe(4000);
    expect(srcToOut(D, cuts, D)).toBe(7000);
  });
  it("maps output to source", () => {
    expect(outToSrc(0, cuts, D)).toBe(1000);
    expect(outToSrc(1500, cuts, D)).toBe(2500);
    expect(outToSrc(3000, cuts, D)).toBe(6000);
    expect(outToSrc(4000, cuts, D)).toBe(7000);
    expect(outToSrc(99999, cuts, D)).toBe(D);
  });
  it("round-trips outside cuts", () => {
    for (const t of [1000, 1234, 3999, 6000, 8765]) expect(outToSrc(srcToOut(t, cuts, D), cuts, D)).toBeCloseTo(t);
  });
  it("is the identity without cuts", () => {
    expect(srcToOut(4321, [], D)).toBe(4321);
    expect(outToSrc(4321, [], D)).toBe(4321);
  });
});

describe("trim", () => {
  it("reads trim bounds", () => {
    expect(trimBounds([], D)).toEqual({ startMs: 0, endMs: D });
    expect(trimBounds([{ startMs: 0, endMs: 800 }, { startMs: 9000, endMs: D }], D)).toEqual({ startMs: 800, endMs: 9000 });
    expect(trimBounds([{ startMs: 3000, endMs: 4000 }], D)).toEqual({ startMs: 0, endMs: D });
  });
  it("moves the start trim without touching other cuts", () => {
    const cuts = [{ startMs: 0, endMs: 800 }, { startMs: 3000, endMs: 4000 }];
    expect(setTrimStart(cuts, 1500, D)).toEqual([{ startMs: 0, endMs: 1500 }, { startMs: 3000, endMs: 4000 }]);
    expect(setTrimStart(cuts, 0, D)).toEqual([{ startMs: 3000, endMs: 4000 }]);
  });
  it("moves the end trim and keeps at least 100 ms", () => {
    expect(setTrimEnd([], 9000, D)).toEqual([{ startMs: 9000, endMs: D }]);
    expect(setTrimEnd([{ startMs: 9000, endMs: D }], D, D)).toEqual([]);
    expect(setTrimEnd([{ startMs: 0, endMs: 5000 }], 100, D)).toEqual([{ startMs: 0, endMs: 5000 }, { startMs: 5100, endMs: D }]);
  });
});

describe("zoom segments", () => {
  const zooms = [z("a", 1000, 3000), z("b", 4000, 6000), z("c", 8000, 9000)];
  it("moves without overlapping neighbours", () => {
    expect(moveZoom(zooms, "b", 2000, D)).toEqual({ startMs: 3000, endMs: 5000 });
    expect(moveZoom(zooms, "b", 7500, D)).toEqual({ startMs: 6000, endMs: 8000 });
    expect(moveZoom(zooms, "a", -500, D)).toEqual({ startMs: 0, endMs: 2000 });
    expect(moveZoom(zooms, "c", 99999, D)).toEqual({ startMs: 9000, endMs: D });
  });
  it("resizes with a minimum length", () => {
    expect(resizeZoom(zooms, "b", "start", 2000, D)).toEqual({ startMs: 3000, endMs: 6000 });
    expect(resizeZoom(zooms, "b", "start", 5900, D)).toEqual({ startMs: 5700, endMs: 6000 });
    expect(resizeZoom(zooms, "b", "end", 9000, D)).toEqual({ startMs: 4000, endMs: 8000 });
    expect(resizeZoom(zooms, "b", "end", 0, D)).toEqual({ startMs: 4000, endMs: 4300 });
  });
  it("finds room for a new segment", () => {
    expect(newZoomRange(zooms, 7000, D)).toEqual({ startMs: 6000, endMs: 8000 });
    expect(newZoomRange(zooms, 500, D)).toEqual({ startMs: 0, endMs: 1000 });
    expect(newZoomRange(zooms, 5000, D)).toBeNull();
    expect(newZoomRange([], 5000, D)).toEqual({ startMs: 3750, endMs: 6250 });
  });
});
