import { beforeEach, describe, expect, it, vi } from "vitest";
import { frameForSource, useStore } from "./store";

// Outside the desktop shell the store talks to the in-browser demo backend.
const DEMO = "mock://demo.cinesnap";
const s = () => useStore.getState();
const zoomCount = () => s().project!.zooms.length;

async function open() {
  await s().openProject(DEMO);
  await vi.waitFor(() => expect(s().motion).not.toBeNull());
}

describe("editor store", () => {
  beforeEach(async () => {
    s().closeProject();
    await open();
  });

  it("opens the demo project with automatic zooms and motion tracks", () => {
    expect(s().screen).toBe("editor");
    expect(zoomCount()).toBeGreaterThan(0);
    expect(s().project!.zooms.every((z) => !z.manual)).toBe(true);
    expect(s().motion!.frames).toBeGreaterThan(0);
    expect(s().past).toHaveLength(0);
  });

  it("adds, edits and deletes a zoom, each as an undo step", () => {
    const before = zoomCount();
    const duration = s().project!.source.video.durationMs;
    s().addZoomAt(duration - 1500);
    expect(zoomCount()).toBe(before + 1);
    const sel = s().selection;
    expect(sel?.kind).toBe("zoom");
    const id = sel?.kind === "zoom" ? sel.id : "";

    s().editZoom(id, { scale: 3 });
    const edited = s().project!.zooms.find((z) => z.id === id)!;
    expect(edited.scale).toBe(3);
    expect(edited.manual).toBe(true);

    s().deleteSelection();
    expect(zoomCount()).toBe(before);
    expect(s().selection).toBeNull();

    s().undo();
    expect(zoomCount()).toBe(before + 1);
    s().undo();
    expect(s().project!.zooms.find((z) => z.id === id)!.scale).not.toBe(3);
    s().undo();
    expect(zoomCount()).toBe(before);
    expect(s().past).toHaveLength(0);
    s().redo();
    s().redo();
    s().redo();
    expect(zoomCount()).toBe(before);
    expect(s().future).toHaveLength(0);
  });

  it("does not add a zoom on top of an existing one", () => {
    const z = s().project!.zooms[0];
    const before = zoomCount();
    s().addZoomAt((z.startMs + z.endMs) / 2);
    expect(zoomCount()).toBe(before);
  });

  it("folds a drag into a single undo step", () => {
    const z = s().project!.zooms[0];
    for (const scale of [2.1, 2.2, 2.3, 2.4]) s().editZoom(z.id, { scale }, "scale");
    expect(s().past).toHaveLength(1);
    s().undo();
    expect(s().project!.zooms[0].scale).toBe(z.scale);
  });

  it("a new edit clears the redo stack", () => {
    const z = s().project!.zooms[0];
    s().editZoom(z.id, { scale: 3 });
    s().undo();
    expect(s().future).toHaveLength(1);
    s().editZoom(z.id, { scale: 1.5 });
    expect(s().future).toHaveLength(0);
  });

  it("marks and cuts with S, shortening the output", async () => {
    const framesBefore = s().motion!.frames;
    s().seekSource(6000);
    s().markOrCut();
    expect(s().cutMark).not.toBeNull();
    s().seekSource(9000);
    s().markOrCut();
    expect(s().cutMark).toBeNull();
    const [cut] = s().project!.cuts;
    expect(cut.startMs).toBeCloseTo(6000, -2);
    expect(cut.endMs).toBeCloseTo(9000, -2);
    await vi.waitFor(() => expect(s().motion!.frames).toBeLessThan(framesBefore));
    // The playhead never rests inside a removed section.
    const at = s().sourceTime();
    expect(at < cut.startMs || at >= cut.endMs - 1).toBe(true);

    s().select({ kind: "cut", startMs: cut.startMs });
    s().deleteSelection();
    expect(s().project!.cuts).toHaveLength(0);
  });

  it("ignores a cut too short to matter", () => {
    s().seekSource(6000);
    s().markOrCut();
    s().markOrCut();
    expect(s().project!.cuts).toHaveLength(0);
    expect(s().past).toHaveLength(0);
  });

  it("clamps the playhead to the clip", () => {
    const last = s().motion!.frames - 1;
    s().setFrame(-50);
    expect(s().frame).toBe(0);
    s().setFrame(1e9);
    expect(s().frame).toBe(last);
    s().setPlaying(true);
    expect(s().frame).toBe(0);
  });

  it("regenerating zooms keeps the ones edited by hand", async () => {
    const z = s().project!.zooms[0];
    s().editZoom(z.id, { scale: 3.5 });
    s().update((p) => void (p.autoZoom.scale = 1.5));
    await s().regenerateZooms();
    const zooms = s().project!.zooms;
    expect(zooms.find((x) => x.id === z.id)?.scale).toBe(3.5);
    expect(zooms.filter((x) => !x.manual).every((x) => x.scale === 1.5)).toBe(true);
  });

  it("closing saves and resets the editor", async () => {
    const z = s().project!.zooms[0];
    s().editZoom(z.id, { scale: 3.25 });
    s().closeProject();
    expect(s().project).toBeNull();
    expect(s().screen).toBe("home");
    await open();
    expect(s().project!.zooms.find((x) => x.id === z.id)?.scale).toBe(3.25);
    expect(s().past).toHaveLength(0);
  });
});

describe("frameForSource", () => {
  const motion = { frames: 5, srcMs: [0, 100, 200, 800, 900] } as never;
  it("finds the first frame at or after a source time", () => {
    expect(frameForSource(motion, -10)).toBe(0);
    expect(frameForSource(motion, 100)).toBe(1);
    expect(frameForSource(motion, 101)).toBe(2);
    expect(frameForSource(motion, 500)).toBe(3);
    expect(frameForSource(motion, 5000)).toBe(4);
  });
});
