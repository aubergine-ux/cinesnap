// Cursor paths: eased, slightly curved, sampled at a fixed rate.

export const SAMPLE_HZ = 60;

// Small seeded generator so a script records the same path every time.
export function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// Minimum-jerk profile: starts and stops with zero speed and acceleration.
export function ease(u) {
  return u * u * u * (u * (6 * u - 15) + 10);
}

// How long a hand takes to travel `distance` px (Fitts-like: long moves are
// faster per pixel than short ones).
export function travelMs(distance, speed = 1) {
  if (distance < 1) return 0;
  return (220 + 150 * Math.log2(1 + distance / 50)) / speed;
}

// Samples from `from` to `to` as [{ t, x, y }], t in ms from the start of the
// move. The first sample is one tick in; the last one is exactly `to`.
export function cursorPath(from, to, { speed = 1, duration, random = Math.random } = {}) {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const dist = Math.hypot(dx, dy);
  if (dist < 1) return [];
  const total = duration ?? travelMs(dist, speed);
  const n = Math.max(1, Math.round((total / 1000) * SAMPLE_HZ));
  // Bow the path sideways by a few percent of its length, either way.
  const bow = (0.03 + 0.05 * random()) * (random() < 0.5 ? -1 : 1);
  const out = [];
  for (let i = 1; i <= n; i++) {
    const u = i / n;
    const e = ease(u);
    const arc = Math.sin(Math.PI * e) * bow;
    out.push({
      t: (total * i) / n,
      x: i === n ? to.x : from.x + dx * e - dy * arc,
      y: i === n ? to.y : from.y + dy * e + dx * arc,
    });
  }
  return out;
}

// Per-tick deltas for a smooth scroll of `amount` px; they sum to `amount`.
export function scrollSteps(amount, duration) {
  const n = Math.max(1, Math.round((duration / 1000) * SAMPLE_HZ));
  const out = [];
  let done = 0;
  for (let i = 1; i <= n; i++) {
    const want = i === n ? amount : Math.round(amount * ease(i / n));
    out.push({ t: (duration * i) / n, d: want - done });
    done = want;
  }
  return out;
}
