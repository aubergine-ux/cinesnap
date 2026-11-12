// The project file is written by this frontend and read by the Rust core.
// This builds a project that uses every editor feature and pins it as a
// fixture; the core has a test that loads the same file.
//
// After changing the project format on purpose: UPDATE_FIXTURE=1 npm test

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { BUILTIN_STYLES, appendImported, applyStyle, insertClip, newAnnotation, newCard } from "./edit";
import { demoProject } from "./mock";
import type { AnnotationType, Project, RecordingClip } from "./types";

const FIXTURE = fileURLToPath(new URL("../../crates/cinesnap-core/tests/fixtures/editor-project.json", import.meta.url));

function everything(): Project {
  const p = demoProject();
  p.createdAt = "2026-01-01T00:00:00.000Z";
  const main = p.clips[0] as RecordingClip;
  main.source.webcam = { file: "webcam.mp4", offsetMs: 120, width: 1280, height: 720 };
  const second: RecordingClip = { ...structuredClone(main), id: "clip1", dir: "clips/clip1", transition: { type: "zoom", durationMs: 600 } };
  second.source.video = { file: "video.mp4", width: 1280, height: 720, fps: 30, durationMs: 8000 };
  second.crop = { x: 0, y: 0, width: 1280, height: 720 };
  appendImported(p, { clip: second, cuts: [{ startMs: 7000, endMs: 8000 }], zooms: [] });
  const view = { x: 0, y: 0, w: 1920, h: 1080 };
  (["text", "arrow", "box", "highlight", "spotlight", "blur"] as AnnotationType[]).forEach((type, i) => {
    p.annotations.push({ ...newAnnotation(p, type, 2000 + i * 1000, view), id: `a${i}` });
  });
  insertClip(p, 0, { ...newCard(p, "Cinesnap", "Finish the whole video here"), id: "title", transition: { type: "cut", durationMs: 500 } });
  insertClip(p, 99, { ...newCard(p, "Thanks for watching"), id: "outro", align: "left", font: "PT Serif", background: { type: "solid", color: "#101116" } });
  p.webcam = { show: true, corner: "top-left", x: 0.12, y: 0.2, size: 0.2, shape: "square" };
  p.webcamKeys = [
    { id: "w1", atMs: 6000, x: 0.8, y: 0.8, size: 0.25, show: true },
    { id: "w2", atMs: 12000, x: 0.8, y: 0.8, size: 0.25, show: false },
  ];
  p.transcript = {
    language: "en",
    words: [
      { t0: 3500, t1: 3900, text: "Hello" },
      { t0: 4000, t1: 4300, text: "um" },
      { t0: 4400, t1: 5000, text: "world." },
    ],
  };
  p.audio = { micVolume: 1.2, denoise: 0.5, normalize: true, eq: { enabled: true, lowCutHz: 90, bassDb: -1, presenceDb: 4, airDb: 2 }, music: { source: "builtin:open-sky", volume: 0.3, duck: true, duckDb: -12, fadeInMs: 500, fadeOutMs: 2000 } };
  applyStyle(p, BUILTIN_STYLES[3]);
  p.export = { format: "mp4", preset: "shorts", fps: 60, quality: "medium" };
  return p;
}

describe("project file contract", () => {
  it("matches the fixture the core is tested against", () => {
    const json = `${JSON.stringify(everything(), null, 2)}\n`;
    if (process.env.UPDATE_FIXTURE || !existsSync(FIXTURE)) writeFileSync(FIXTURE, json);
    expect(json).toBe(readFileSync(FIXTURE, "utf8"));
  });
});
