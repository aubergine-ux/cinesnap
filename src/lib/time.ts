// Cut handling and the mapping between source time and output time.

import type { Cut, ZoomSegment } from "./types";

const EPS = 1;

/** Sort, clamp to [0, duration], merge overlapping/adjacent cuts, drop empty ones. */
export function normalizeCuts(cuts: Cut[], durationMs: number): Cut[] {
  const sorted = cuts
    .map((c) => ({
      startMs: Math.max(0, Math.min(durationMs, Math.min(c.startMs, c.endMs))),
      endMs: Math.max(0, Math.min(durationMs, Math.max(c.startMs, c.endMs))),
    }))
    .filter((c) => c.endMs - c.startMs >= EPS)
    .sort((a, b) => a.startMs - b.startMs);
  const out: Cut[] = [];
  for (const c of sorted) {
    const last = out[out.length - 1];
    if (last && c.startMs <= last.endMs + EPS) last.endMs = Math.max(last.endMs, c.endMs);
    else out.push({ ...c });
  }
  return out;
}

export function addCut(cuts: Cut[], cut: Cut, durationMs: number): Cut[] {
  return normalizeCuts([...cuts, cut], durationMs);
}

/** Ranges of source time that survive the cuts. */
export function keptRanges(cuts: Cut[], durationMs: number): Cut[] {
  const out: Cut[] = [];
  let t = 0;
  for (const c of normalizeCuts(cuts, durationMs)) {
    if (c.startMs - t >= EPS) out.push({ startMs: t, endMs: c.startMs });
    t = c.endMs;
  }
  if (durationMs - t >= EPS) out.push({ startMs: t, endMs: durationMs });
  return out;
}

export function outputDuration(cuts: Cut[], durationMs: number): number {
  return keptRanges(cuts, durationMs).reduce((s, r) => s + (r.endMs - r.startMs), 0);
}

/** Index of the cut containing `ms`, or -1. */
export function cutAt(cuts: Cut[], ms: number): number {
  return cuts.findIndex((c) => ms >= c.startMs && ms < c.endMs);
}

/** Source time -> output time. Times inside a cut map to the point where the cut was made. */
export function srcToOut(ms: number, cuts: Cut[], durationMs: number): number {
  let removed = 0;
  for (const c of normalizeCuts(cuts, durationMs)) {
    if (ms <= c.startMs) break;
    removed += Math.min(ms, c.endMs) - c.startMs;
  }
  return Math.max(0, Math.min(ms, durationMs) - removed);
}

/** Output time -> source time. */
export function outToSrc(ms: number, cuts: Cut[], durationMs: number): number {
  const kept = keptRanges(cuts, durationMs);
  if (!kept.length) return 0;
  let acc = 0;
  for (const r of kept) {
    const len = r.endMs - r.startMs;
    if (ms < acc + len) return r.startMs + Math.max(0, ms - acc);
    acc += len;
  }
  return kept[kept.length - 1].endMs;
}

/** Leading / trailing cuts (the trim handles). */
export function trimBounds(cuts: Cut[], durationMs: number): { startMs: number; endMs: number } {
  const n = normalizeCuts(cuts, durationMs);
  const first = n[0];
  const last = n[n.length - 1];
  return {
    startMs: first && first.startMs < EPS ? first.endMs : 0,
    endMs: last && last.endMs > durationMs - EPS ? last.startMs : durationMs,
  };
}

/** Replace the leading cut so the recording starts at `ms`. */
export function setTrimStart(cuts: Cut[], ms: number, durationMs: number): Cut[] {
  const { endMs } = trimBounds(cuts, durationMs);
  const t = Math.max(0, Math.min(ms, endMs - 100));
  const rest = normalizeCuts(cuts, durationMs).filter((c) => c.startMs >= EPS);
  return normalizeCuts(t > 0 ? [...rest, { startMs: 0, endMs: t }] : rest, durationMs);
}

/** Replace the trailing cut so the recording ends at `ms`. */
export function setTrimEnd(cuts: Cut[], ms: number, durationMs: number): Cut[] {
  const { startMs } = trimBounds(cuts, durationMs);
  const t = Math.min(durationMs, Math.max(ms, startMs + 100));
  const rest = normalizeCuts(cuts, durationMs).filter((c) => c.endMs < durationMs - EPS);
  return normalizeCuts(t < durationMs ? [...rest, { startMs: t, endMs: durationMs }] : rest, durationMs);
}

// ---- zoom segments ---------------------------------------------------------

export const MIN_ZOOM_MS = 300;

/** Free range around a segment (or around a point when `id` is not in the list). */
export function zoomBounds(zooms: ZoomSegment[], id: string | null, atMs: number, durationMs: number): { min: number; max: number } {
  let min = 0;
  let max = durationMs;
  const self = id ? zooms.find((z) => z.id === id) : undefined;
  const ref = self ? (self.startMs + self.endMs) / 2 : atMs;
  for (const z of zooms) {
    if (z.id === id) continue;
    if (z.endMs <= ref) min = Math.max(min, z.endMs);
    else if (z.startMs >= ref) max = Math.min(max, z.startMs);
    else return { min: ref, max: ref }; // point is inside another segment
  }
  return { min, max };
}

/** Move a segment by `deltaMs` without overlapping its neighbours. */
export function moveZoom(zooms: ZoomSegment[], id: string, startMs: number, durationMs: number): { startMs: number; endMs: number } | null {
  const z = zooms.find((s) => s.id === id);
  if (!z) return null;
  const len = z.endMs - z.startMs;
  const { min, max } = zoomBounds(zooms, id, 0, durationMs);
  const s = Math.max(min, Math.min(max - len, startMs));
  return { startMs: s, endMs: s + len };
}

/** Resize one edge of a segment, keeping a minimum length and no overlap. */
export function resizeZoom(
  zooms: ZoomSegment[],
  id: string,
  edge: "start" | "end",
  ms: number,
  durationMs: number,
): { startMs: number; endMs: number } | null {
  const z = zooms.find((s) => s.id === id);
  if (!z) return null;
  const { min, max } = zoomBounds(zooms, id, 0, durationMs);
  if (edge === "start") return { startMs: Math.max(min, Math.min(z.endMs - MIN_ZOOM_MS, ms)), endMs: z.endMs };
  return { startMs: z.startMs, endMs: Math.min(max, Math.max(z.startMs + MIN_ZOOM_MS, ms)) };
}

/** Range for a new segment centred near `atMs`, or null when there is no room. */
export function newZoomRange(zooms: ZoomSegment[], atMs: number, durationMs: number, lengthMs = 2500): { startMs: number; endMs: number } | null {
  const { min, max } = zoomBounds(zooms, null, atMs, durationMs);
  if (max - min < MIN_ZOOM_MS) return null;
  const len = Math.min(lengthMs, max - min);
  const s = Math.max(min, Math.min(max - len, atMs - len / 2));
  return { startMs: s, endMs: s + len };
}
