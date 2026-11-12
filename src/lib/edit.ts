// Pure editing operations on a project: the clip timeline, annotations,
// transcript cuts and reusable styles. Everything here is undoable simply
// because it only ever changes the project object.

import { aspectRatio, cornerCentre, effectiveAspect, type Box } from "./layout";
import { normalizeCuts } from "./time";
import type { Annotation, AnnotationType, CardClip, Clip, Cut, Imported, Project, Rect, RecordingClip, StylePreset, WebcamKey, Word } from "./types";

export interface Span {
  index: number;
  startMs: number;
  durationMs: number;
  endMs: number;
}

export function clipDuration(c: Clip): number {
  return Math.max(0, c.kind === "card" ? c.durationMs : c.source.video.durationMs);
}

/** Clips with their position on the timeline (before cuts). */
export function spans(p: Project): Span[] {
  let at = 0;
  return p.clips.map((c, index) => {
    const durationMs = clipDuration(c);
    const span = { index, startMs: at, durationMs, endMs: at + durationMs };
    at += durationMs;
    return span;
  });
}

export function totalDuration(p: Project): number {
  return p.clips.reduce((s, c) => s + clipDuration(c), 0);
}

/** The clip showing at a timeline time (the last one at the very end). */
export function spanAt(p: Project, ms: number): Span | undefined {
  const all = spans(p);
  return all.find((s) => ms < s.endMs) ?? all[all.length - 1];
}

export function firstRecording(p: Project): RecordingClip | undefined {
  return p.clips.find((c): c is RecordingClip => c.kind === "recording");
}

/** Crop of the first recording: decides the output shape for the "source" aspect. */
export function mainCrop(p: Project): Rect {
  return firstRecording(p)?.crop ?? p.crop;
}

/** Width / height of the output. */
export function outputRatio(p: Project): number {
  return aspectRatio(effectiveAspect(p.frame.aspect, p.export), mainCrop(p));
}

export function newId(prefix: string): string {
  return `${prefix}${Date.now().toString(36)}${Math.floor(Math.random() * 46656).toString(36)}`;
}

/**
 * Move everything at or after `atMs` by `deltaMs`. Something that merely
 * ends at `atMs` stays where it is; a cut running across `atMs` is split so
 * it does not swallow whatever is being inserted there.
 */
export function shiftFrom(p: Project, atMs: number, deltaMs: number): void {
  const EPS = 0.5;
  const moved = (t: number) => (t >= atMs - EPS ? t + deltaMs : t);
  const cuts: Cut[] = [];
  for (const c of p.cuts) {
    if (c.endMs <= atMs + EPS) cuts.push(c);
    else if (c.startMs >= atMs - EPS) cuts.push({ startMs: c.startMs + deltaMs, endMs: c.endMs + deltaMs });
    else cuts.push({ startMs: c.startMs, endMs: atMs }, { startMs: atMs + deltaMs, endMs: c.endMs + deltaMs });
  }
  p.cuts = cuts.filter((c) => c.endMs - c.startMs >= 1);
  // Zooms and annotations belong to the clip they start in.
  for (const list of [p.zooms, p.annotations]) {
    for (const it of list) {
      if (it.startMs >= atMs - EPS) [it.startMs, it.endMs] = [it.startMs + deltaMs, it.endMs + deltaMs];
      else it.endMs = Math.min(it.endMs, atMs);
    }
  }
  for (const k of p.webcamKeys) k.atMs = moved(k.atMs);
  if (p.transcript) for (const w of p.transcript.words) if (w.t0 >= atMs - EPS) [w.t0, w.t1] = [w.t0 + deltaMs, w.t1 + deltaMs];
}

/** Put a clip on the timeline before the clip at `index`. */
export function insertClip(p: Project, index: number, clip: Clip): void {
  const i = Math.max(0, Math.min(p.clips.length, index));
  const at = spans(p)[i]?.startMs ?? totalDuration(p);
  shiftFrom(p, at, clipDuration(clip));
  p.clips.splice(i, 0, clip);
}

/** Append an imported recording, bringing its cuts and zooms along. */
export function appendImported(p: Project, imported: Imported): void {
  const start = totalDuration(p);
  p.clips.push(imported.clip);
  p.cuts.push(...imported.cuts.map((c) => ({ startMs: c.startMs + start, endMs: c.endMs + start })));
  p.zooms.push(...imported.zooms.map((z) => ({ ...z, id: `${imported.clip.id}-${z.id}`, startMs: z.startMs + start, endMs: z.endMs + start })));
  p.cuts = normalizeCuts(p.cuts, totalDuration(p));
}

/** Keep the parts of a range that lie outside [a, b). */
function outside<T extends { startMs: number; endMs: number }>(items: T[], a: number, b: number): T[] {
  const out: T[] = [];
  for (const it of items) {
    if (it.endMs <= a || it.startMs >= b) out.push(it);
    else {
      if (it.startMs < a) out.push({ ...it, endMs: a });
      if (it.endMs > b) out.push({ ...it, startMs: b });
    }
  }
  return out;
}

/**
 * Take a clip off the timeline along with everything attached to it.
 * The last recording cannot be removed; returns false then.
 */
export function removeClip(p: Project, index: number): boolean {
  const clip = p.clips[index];
  if (!clip) return false;
  if (clip.kind === "recording" && p.clips.filter((c) => c.kind === "recording").length <= 1) return false;
  const { startMs: a, endMs: b } = spans(p)[index];
  p.cuts = outside(p.cuts, a, b);
  p.zooms = p.zooms.filter((z) => z.startMs < a || z.startMs >= b);
  p.annotations = p.annotations.filter((x) => x.startMs < a || x.startMs >= b);
  p.webcamKeys = p.webcamKeys.filter((k) => k.atMs < a || k.atMs >= b);
  if (p.transcript) p.transcript.words = p.transcript.words.filter((w) => w.t0 < a || w.t0 >= b);
  p.clips.splice(index, 1);
  shiftFrom(p, b, a - b);
  p.cuts = normalizeCuts(p.cuts, totalDuration(p));
  return true;
}

/** Move a clip to another position; its edits travel with it. */
export function moveClip(p: Project, from: number, to: number): void {
  const n = p.clips.length;
  if (from < 0 || from >= n) return;
  const target = Math.max(0, Math.min(n - 1, to));
  if (target === from) return;
  const before = spans(p);
  const order = p.clips.map((_, i) => i);
  order.splice(target, 0, ...order.splice(from, 1));
  // New start of every old clip.
  const starts = new Array<number>(n);
  let at = 0;
  for (const old of order) {
    starts[old] = at;
    at += before[old].durationMs;
  }
  const owner = (t: number) => before.find((s) => t < s.endMs) ?? before[n - 1];
  const point = (t: number) => {
    const s = owner(t);
    return starts[s.index] + (t - s.startMs);
  };
  // A range stays with the clip it starts in and never leaves it.
  const range = <T extends { startMs: number; endMs: number }>(it: T): T => {
    const s = owner(it.startMs);
    const start = starts[s.index] + (it.startMs - s.startMs);
    return { ...it, startMs: start, endMs: start + Math.min(it.endMs, s.endMs) - it.startMs };
  };
  // Cuts that span several clips are split at clip edges first.
  const pieces: Cut[] = [];
  for (const c of p.cuts) for (const s of before) if (c.endMs > s.startMs && c.startMs < s.endMs) pieces.push({ startMs: Math.max(c.startMs, s.startMs), endMs: Math.min(c.endMs, s.endMs) });
  p.cuts = normalizeCuts(pieces.map(range), totalDuration(p));
  p.zooms = p.zooms.map(range).sort((a, b) => a.startMs - b.startMs);
  p.annotations = p.annotations.map(range).sort((a, b) => a.startMs - b.startMs);
  p.webcamKeys = p.webcamKeys.map((k) => ({ ...k, atMs: point(k.atMs) })).sort((a, b) => a.atMs - b.atMs);
  if (p.transcript) {
    p.transcript.words = p.transcript.words
      .map((w) => {
        const r = range({ startMs: w.t0, endMs: w.t1 });
        return { ...w, t0: r.startMs, t1: r.endMs };
      })
      .sort((a, b) => a.t0 - b.t0);
  }
  p.clips = order.map((i) => p.clips[i]);
}

export function newCard(p: Project, title: string, subtitle = ""): CardClip {
  return {
    id: newId("card"),
    kind: "card",
    transition: { type: "crossfade", durationMs: 500 },
    durationMs: 3000,
    title,
    subtitle,
    titleColor: p.theme.text,
    subtitleColor: `${p.theme.text.slice(0, 7)}b3`,
    titleSize: 96,
    align: "center",
  };
}

/** A new annotation at `atMs`, placed in the middle of what is visible (`view`, source px). */
export function newAnnotation(p: Project, type: AnnotationType, atMs: number, view: Box): Annotation {
  const total = totalDuration(p);
  const span = spanAt(p, atMs);
  const end = Math.min(atMs + 3000, span?.endMs ?? total, total);
  const base = { id: newId("a"), startMs: Math.max(0, Math.min(atMs, end - 300)), endMs: end };
  const w = view.w * 0.3;
  const h = view.h * 0.22;
  const region = { x: view.x + (view.w - w) / 2, y: view.y + (view.h - h) / 2, width: w, height: h };
  const accent = p.theme.accent;
  switch (type) {
    case "text":
      return { ...base, type, x: view.x + view.w * 0.36, y: view.y + view.h * 0.42, text: "Text", size: 44, color: p.theme.text, background: p.theme.surface, bold: true };
    case "arrow":
      return { ...base, type, x1: view.x + view.w * 0.35, y1: view.y + view.h * 0.6, x2: view.x + view.w * 0.55, y2: view.y + view.h * 0.45, color: accent, width: 8 };
    case "box":
      return { ...base, type, ...region, color: accent, stroke: 6, radius: 8 };
    case "highlight":
      return { ...base, type, ...region, height: h * 0.4, color: "#ffe14a", opacity: 0.7 };
    case "spotlight":
      return { ...base, type, ...region, dim: 0.65, ellipse: false };
    case "blur":
      return { ...base, type, ...region, strength: 14 };
  }
}

/** Bounding box of an annotation in source pixels (text has no intrinsic size here). */
export function annotationBox(a: Annotation): Box | null {
  if (a.type === "text") return null;
  if (a.type === "arrow") return { x: Math.min(a.x1, a.x2), y: Math.min(a.y1, a.y2), w: Math.abs(a.x2 - a.x1), h: Math.abs(a.y2 - a.y1) };
  return { x: a.x, y: a.y, w: a.width, h: a.height };
}

/** Move an annotation by a source-pixel delta. */
export function moveAnnotation<T extends Annotation>(a: T, dx: number, dy: number): T {
  if (a.type === "arrow") return { ...a, x1: a.x1 + dx, y1: a.y1 + dy, x2: a.x2 + dx, y2: a.y2 + dy };
  return { ...a, x: a.x + dx, y: a.y + dy };
}

// ---- webcam ----------------------------------------------------------------

/** State of the webcam bubble a new keyframe at `atMs` starts from. */
export function webcamStateAt(p: Project, atMs: number): Omit<WebcamKey, "id" | "atMs"> {
  const prev = [...p.webcamKeys].filter((k) => k.atMs <= atMs).pop();
  if (prev) return { x: prev.x, y: prev.y, size: prev.size, show: prev.show };
  const c = p.webcam.x !== undefined && p.webcam.y !== undefined ? { x: p.webcam.x, y: p.webcam.y } : cornerCentre(p.webcam.corner, p.webcam.size, outputRatio(p));
  return { ...c, size: p.webcam.size, show: true };
}

// ---- transcript ------------------------------------------------------------

/** A word counts as cut when its middle is inside a cut. */
export function isWordCut(w: Word, cuts: Cut[]): boolean {
  const mid = (w.t0 + w.t1) / 2;
  return cuts.some((c) => mid >= c.startMs && mid < c.endMs);
}

/**
 * The cut that removes words i..=j: the words plus half of the pause on
 * either side (at most 120 ms), so the sentence closes up naturally.
 */
export function wordsCut(words: Word[], i: number, j: number): Cut {
  const PAD = 120;
  const first = words[i];
  const last = words[j];
  const before = i > 0 ? Math.min(words[i - 1].t1, first.t0) : first.t0;
  const after = j + 1 < words.length ? Math.max(words[j + 1].t0, last.t1) : last.t1;
  return { startMs: first.t0 - Math.min(PAD, (first.t0 - before) / 2), endMs: last.t1 + Math.min(PAD, (after - last.t1) / 2) };
}

/** Bring words i..=j back: remove every cut piece that overlaps them. */
export function restoreWords(cuts: Cut[], words: Word[], i: number, j: number, durationMs: number): Cut[] {
  const { startMs: a, endMs: b } = wordsCut(words, i, j);
  return normalizeCuts(outside(cuts, a, b), durationMs);
}

// ---- styles ----------------------------------------------------------------

/** The look of a project as a reusable preset. */
export function extractStyle(p: Project, name: string): StylePreset {
  const { aspect: _aspect, ...frame } = p.frame;
  return structuredClone({
    name,
    frame,
    cursor: p.cursor,
    theme: p.theme,
    keystrokes: p.keystrokes,
    captions: p.captions,
    webcam: { shape: p.webcam.shape, size: p.webcam.size },
  });
}

/** Apply a preset: everything about the look, nothing about the content. */
export function applyStyle(p: Project, style: StylePreset): void {
  const s = structuredClone(style);
  p.frame = { ...s.frame, aspect: p.frame.aspect };
  p.cursor = s.cursor;
  p.theme = s.theme;
  p.keystrokes = s.keystrokes;
  // Whether captions show is content, not style.
  p.captions = { ...s.captions, show: p.captions.show };
  p.webcam = { ...p.webcam, ...s.webcam };
}

export const BUILTIN_STYLES: StylePreset[] = [
  {
    name: "Cinesnap",
    frame: { padding: 0.06, radius: 14, shadow: 0.55, background: { type: "gradient", from: "#6d5dfc", to: "#e44d8a", angle: 135 } },
    cursor: { show: true, size: 1.6, smoothing: 0.6, ripples: true, rippleColor: "#ffffff", hideWhenIdleMs: 0, style: "arrow" },
    theme: { font: "Inter", accent: "#6d5dfc", text: "#ffffff", surface: "#14151af0" },
    keystrokes: { show: true, position: "bottom", size: 1, holdMs: 1500 },
    captions: { show: true, style: "karaoke", y: 0.82, size: 1, color: "#ffffff", highlight: "#ffd84a", background: "#000000a6", maxChars: 36, uppercase: false },
    webcam: { shape: "circle", size: 0.18 },
  },
  {
    name: "Midnight",
    frame: { padding: 0.05, radius: 10, shadow: 0.7, background: { type: "gradient", from: "#1e293b", to: "#0f172a", angle: 180 } },
    cursor: { show: true, size: 1.5, smoothing: 0.65, ripples: true, rippleColor: "#38bdf8", hideWhenIdleMs: 0, style: "arrow" },
    theme: { font: "IBM Plex Mono", accent: "#38bdf8", text: "#e2e8f0", surface: "#0b1220f0" },
    keystrokes: { show: true, position: "bottom", size: 1, holdMs: 1500 },
    captions: { show: true, style: "plain", y: 0.82, size: 0.9, color: "#e2e8f0", highlight: "#38bdf8", background: "#0b1220d0", maxChars: 40, uppercase: false },
    webcam: { shape: "rounded", size: 0.18 },
  },
  {
    name: "Paper",
    frame: { padding: 0.08, radius: 8, shadow: 0.3, background: { type: "solid", color: "#f3efe6" } },
    cursor: { show: true, size: 1.6, smoothing: 0.6, ripples: true, rippleColor: "#c2410c", hideWhenIdleMs: 0, style: "arrow" },
    theme: { font: "PT Serif", accent: "#c2410c", text: "#1c1917", surface: "#fffaf0f5" },
    keystrokes: { show: true, position: "bottom", size: 1, holdMs: 1500 },
    captions: { show: true, style: "karaoke", y: 0.82, size: 1, color: "#1c1917", highlight: "#c2410c", background: "#fffaf0e6", maxChars: 36, uppercase: false },
    webcam: { shape: "rounded", size: 0.18 },
  },
  {
    name: "Pop",
    frame: { padding: 0.07, radius: 22, shadow: 0.5, background: { type: "gradient", from: "#f97316", to: "#ec4899", angle: 135 } },
    cursor: { show: true, size: 1.9, smoothing: 0.55, ripples: true, rippleColor: "#fde047", hideWhenIdleMs: 0, style: "arrow" },
    theme: { font: "Poppins", accent: "#ec4899", text: "#ffffff", surface: "#1a1020f0" },
    keystrokes: { show: true, position: "bottom", size: 1.15, holdMs: 1500 },
    captions: { show: true, style: "pop", y: 0.8, size: 1.2, color: "#ffffff", highlight: "#fde047", maxChars: 24, uppercase: true },
    webcam: { shape: "circle", size: 0.2 },
  },
];
