// Timeline geometry: time <-> pixels, ruler ticks, zooming.

export function pxToMs(px: number, pxPerMs: number): number {
  return pxPerMs > 0 ? px / pxPerMs : 0;
}

const STEPS = [100, 200, 500, 1000, 2000, 5000, 10000, 15000, 30000, 60000, 120000, 300000, 600000];

/** Smallest "nice" tick interval whose labels are at least `minPx` apart. */
export function tickStep(pxPerMs: number, minPx = 70): number {
  for (const s of STEPS) if (s * pxPerMs >= minPx) return s;
  return STEPS[STEPS.length - 1];
}

export function ticks(durationMs: number, pxPerMs: number, minPx = 70): number[] {
  const step = tickStep(pxPerMs, minPx);
  const out: number[] = [];
  for (let t = 0; t <= durationMs + 0.5; t += step) out.push(t);
  return out;
}

export function clampZoom(pxPerMs: number, durationMs: number, viewportPx: number): number {
  const fit = durationMs > 0 ? viewportPx / durationMs : 1;
  return Math.max(fit, Math.min(Math.max(fit, 2), pxPerMs));
}

/**
 * Zoom the timeline around an anchor so the time under the pointer stays put.
 * Returns the new scale and the new scroll offset.
 */
export function zoomAround(
  pxPerMs: number,
  scrollPx: number,
  anchorPx: number,
  factor: number,
  durationMs: number,
  viewportPx: number,
): { pxPerMs: number; scrollPx: number } {
  const anchorMs = pxToMs(scrollPx + anchorPx, pxPerMs);
  const next = clampZoom(pxPerMs * factor, durationMs, viewportPx);
  const maxScroll = Math.max(0, durationMs * next - viewportPx);
  return { pxPerMs: next, scrollPx: Math.max(0, Math.min(maxScroll, anchorMs * next - anchorPx)) };
}

export function formatTime(ms: number, withFraction = true): string {
  const total = Math.max(0, ms);
  const m = Math.floor(total / 60000);
  const s = Math.floor((total % 60000) / 1000);
  const base = `${m}:${String(s).padStart(2, "0")}`;
  if (!withFraction) return base;
  return `${base}.${String(Math.floor((total % 1000) / 100))}`;
}
