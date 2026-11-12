import { describe, expect, it } from "vitest";
import { BUILTIN_STYLES, annotationBox, appendImported, applyStyle, extractStyle, insertClip, isWordCut, moveAnnotation, moveClip, newAnnotation, newCard, outputRatio, removeClip, restoreWords, spanAt, spans, totalDuration, webcamStateAt, wordsCut } from "./edit";
import { demoProject } from "./mock";
import type { Project, RecordingClip } from "./types";

function twoClips(): Project {
  const p = demoProject();
  p.zooms = [];
  const first = p.clips[0] as RecordingClip;
  first.source.video.durationMs = 10_000;
  const second: RecordingClip = structuredClone(first);
  second.id = "second";
  second.dir = "clips/second";
  second.source.video.durationMs = 6000;
  appendImported(p, { clip: second, cuts: [{ startMs: 5000, endMs: 6000 }], zooms: [{ id: "z", startMs: 1000, endMs: 3000, scale: 2, easeInMs: 600, easeOutMs: 700, easing: "smooth", mode: "follow", x: 1, y: 1, manual: true }] });
  return p;
}

describe("clip timeline", () => {
  it("lays clips end to end", () => {
    const p = twoClips();
    expect(spans(p).map((s) => [s.startMs, s.endMs])).toEqual([
      [0, 10_000],
      [10_000, 16_000],
    ]);
    expect(totalDuration(p)).toBe(16_000);
    expect(spanAt(p, 9999)?.index).toBe(0);
    expect(spanAt(p, 10_000)?.index).toBe(1);
    expect(spanAt(p, 99_999)?.index).toBe(1);
    // Imported edits arrive shifted to where the clip landed.
    expect(p.cuts).toEqual([{ startMs: 15_000, endMs: 16_000 }]);
    expect(p.zooms[0]).toMatchObject({ id: "second-z", startMs: 11_000, endMs: 13_000 });
  });

  it("inserting a card moves later edits along", () => {
    const p = twoClips();
    p.annotations.push(newAnnotation(p, "box", 2000, { x: 0, y: 0, w: 1920, h: 1080 }));
    p.transcript = { language: "en", words: [{ t0: 500, t1: 800, text: "hi" }] };
    insertClip(p, 0, newCard(p, "Title"));
    expect(p.clips[0].kind).toBe("card");
    expect(p.cuts[0].startMs).toBe(18_000);
    expect(p.annotations[0].startMs).toBe(5000);
    expect(p.transcript.words[0].t0).toBe(3500);
    // At the end nothing moves, not even the cut that ends exactly there.
    insertClip(p, 99, newCard(p, "Bye"));
    expect(p.cuts).toEqual([{ startMs: 18_000, endMs: 19_000 }]);
    expect(totalDuration(p)).toBe(22_000);
    // A cut running across the insertion point is split around the new clip.
    p.cuts = [{ startMs: 12_000, endMs: 14_000 }];
    insertClip(p, 2, newCard(p, "Middle"));
    expect(p.cuts).toEqual([
      { startMs: 12_000, endMs: 13_000 },
      { startMs: 16_000, endMs: 17_000 },
    ]);
  });

  it("removing a clip takes its edits with it", () => {
    const p = twoClips();
    p.cuts.push({ startMs: 9000, endMs: 11_000 });
    p.webcamKeys.push({ id: "k", atMs: 12_000, x: 0.5, y: 0.5, size: 0.2, show: true });
    expect(removeClip(p, 0)).toBe(true);
    expect(p.clips.map((c) => c.id)).toEqual(["second"]);
    // The straddling cut keeps only its part in the surviving clip.
    expect(p.cuts).toEqual([
      { startMs: 0, endMs: 1000 },
      { startMs: 5000, endMs: 6000 },
    ]);
    expect(p.zooms[0].startMs).toBe(1000);
    expect(p.webcamKeys[0].atMs).toBe(2000);
    // The last recording stays.
    expect(removeClip(p, 0)).toBe(false);
    expect(removeClip(p, 7)).toBe(false);
    insertClip(p, 0, newCard(p, "T"));
    expect(removeClip(p, 0)).toBe(true);
  });

  it("reordering carries edits with their clip", () => {
    const p = twoClips();
    p.cuts.push({ startMs: 9000, endMs: 11_000 });
    p.annotations.push({ id: "a", type: "blur", startMs: 500, endMs: 1500, x: 0, y: 0, width: 10, height: 10, strength: 5 });
    moveClip(p, 1, 0);
    expect(p.clips.map((c) => c.id)).toEqual(["second", "main"]);
    expect(p.zooms[0]).toMatchObject({ startMs: 1000, endMs: 3000 });
    expect(p.annotations[0]).toMatchObject({ startMs: 6500, endMs: 7500 });
    // The cut across the join is now two cuts, one at the end of each clip... of which
    // the second's start joins its own trailing cut.
    expect(p.cuts).toEqual([
      { startMs: 0, endMs: 1000 },
      { startMs: 5000, endMs: 6000 },
      { startMs: 15_000, endMs: 16_000 },
    ]);
    const snapshot = JSON.stringify(p);
    moveClip(p, 0, 0);
    moveClip(p, 5, 0);
    expect(JSON.stringify(p)).toBe(snapshot);
    moveClip(p, 0, 1);
    expect(p.clips.map((c) => c.id)).toEqual(["main", "second"]);
    expect(p.zooms[0].startMs).toBe(11_000);
  });
});

describe("annotations", () => {
  it("new ones land in view and inside the clip", () => {
    const p = twoClips();
    const view = { x: 100, y: 50, w: 800, h: 400 };
    for (const type of ["text", "arrow", "box", "highlight", "spotlight", "blur"] as const) {
      const a = newAnnotation(p, type, 9000, view);
      expect(a.type).toBe(type);
      expect(a.endMs).toBeLessThanOrEqual(10_000);
      expect(a.endMs - a.startMs).toBeGreaterThanOrEqual(300);
      const box = annotationBox(a);
      if (box) {
        expect(box.x).toBeGreaterThanOrEqual(view.x);
        expect(box.x + box.w).toBeLessThanOrEqual(view.x + view.w);
      }
      const moved = moveAnnotation(a, 10, -5);
      if (moved.type === "arrow" && a.type === "arrow") expect([moved.x1, moved.y2]).toEqual([a.x1 + 10, a.y2 - 5]);
      else if (moved.type !== "arrow" && a.type !== "arrow") expect([moved.x, moved.y]).toEqual([a.x + 10, a.y - 5]);
    }
  });
});

describe("transcript cuts", () => {
  const words = [
    { t0: 0, t1: 400, text: "So" },
    { t0: 600, t1: 900, text: "um" },
    { t0: 1000, t1: 1400, text: "hello" },
    { t0: 3000, t1: 3300, text: "there" },
  ];
  it("cuts words with half the surrounding pause", () => {
    expect(wordsCut(words, 1, 1)).toEqual({ startMs: 500, endMs: 950 });
    // Long pauses are only nibbled.
    expect(wordsCut(words, 2, 2)).toEqual({ startMs: 950, endMs: 1520 });
    expect(wordsCut(words, 0, 3)).toEqual({ startMs: 0, endMs: 3300 });
    const cuts = [wordsCut(words, 1, 1)];
    expect(words.map((w) => isWordCut(w, cuts))).toEqual([false, true, false, false]);
  });
  it("restores words without touching other cuts", () => {
    const cuts = [
      { startMs: 400, endMs: 1500 },
      { startMs: 5000, endMs: 6000 },
    ];
    const out = restoreWords(cuts, words, 2, 2, 10_000);
    expect(out).toEqual([
      { startMs: 400, endMs: 950 },
      { startMs: 5000, endMs: 6000 },
    ]);
    expect(isWordCut(words[1], out)).toBe(true);
    expect(isWordCut(words[2], out)).toBe(false);
  });
});

describe("styles", () => {
  it("round-trip the look and leave the content alone", () => {
    const p = twoClips();
    p.frame.aspect = "9:16";
    p.captions.show = false;
    const before = extractStyle(p, "mine");
    for (const style of BUILTIN_STYLES) {
      applyStyle(p, style);
      expect(p.theme.font).toBe(style.theme.font);
      expect(p.frame.background).toEqual(style.frame.background);
    }
    expect(p.frame.aspect).toBe("9:16");
    expect(p.captions.show).toBe(false);
    // The last style has no caption background; the one before it must not linger.
    expect(p.captions.background).toBeUndefined();
    expect(p.clips).toHaveLength(2);
    applyStyle(p, before);
    expect(extractStyle(p, "mine")).toEqual(before);
    // A preset is a copy: editing the project later does not change it.
    p.theme.accent = "#000000";
    expect(before.theme.accent).not.toBe("#000000");
  });
});

describe("webcam", () => {
  it("new keyframes start from the current state", () => {
    const p = twoClips();
    const ratio = outputRatio(p);
    expect(ratio).toBeCloseTo(16 / 9);
    const base = webcamStateAt(p, 1000);
    expect(base.x).toBeGreaterThan(0.9);
    expect(base.y).toBeGreaterThan(0.8);
    p.webcamKeys.push({ id: "k", atMs: 2000, x: 0.1, y: 0.2, size: 0.3, show: false });
    expect(webcamStateAt(p, 1000)).toEqual(base);
    expect(webcamStateAt(p, 2500)).toEqual({ x: 0.1, y: 0.2, size: 0.3, show: false });
  });
});
