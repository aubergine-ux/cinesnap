import assert from 'node:assert/strict';
import test from 'node:test';
import { cursorPath, ease, rng, scrollSteps, travelMs } from '../src/path.mjs';
import { pickFrames } from '../src/capture.mjs';
import { deepMerge, eventsJsonl, manifest } from '../src/bundle.mjs';

test('a path ends exactly on its target and takes the expected time', () => {
  const from = { x: 100, y: 100 };
  const to = { x: 900, y: 500 };
  const p = cursorPath(from, to, { random: rng(1) });
  const total = travelMs(Math.hypot(800, 400));
  assert.deepEqual({ x: p.at(-1).x, y: p.at(-1).y }, to);
  assert.ok(Math.abs(p.at(-1).t - total) < 1e-6);
  // About 60 samples a second, evenly spaced.
  assert.equal(p.length, Math.round((total / 1000) * 60));
  const dt = p[1].t - p[0].t;
  assert.ok(dt > 15 && dt < 18.5, `dt ${dt}`);
});

test('a path is smooth: slow at both ends, fastest in the middle, no jumps', () => {
  const p = cursorPath({ x: 0, y: 0 }, { x: 1000, y: 0 }, { random: rng(7) });
  const pts = [{ x: 0, y: 0 }, ...p];
  const step = pts.slice(1).map((s, i) => Math.hypot(s.x - pts[i].x, s.y - pts[i].y));
  const peak = Math.max(...step);
  assert.ok(step[0] < peak * 0.1, 'starts slowly');
  assert.ok(step.at(-1) < peak * 0.1, 'ends slowly');
  const mid = step.indexOf(peak) / step.length;
  assert.ok(mid > 0.35 && mid < 0.65, `peak at ${mid}`);
  for (let i = 1; i < step.length; i++) assert.ok(Math.abs(step[i] - step[i - 1]) < peak * 0.2);
});

test('a path bows away from the straight line, but only a little', () => {
  const p = cursorPath({ x: 0, y: 0 }, { x: 1000, y: 0 }, { random: rng(3) });
  const off = Math.max(...p.map((s) => Math.abs(s.y)));
  assert.ok(off >= 25 && off <= 85, `bow ${off}`);
});

test('longer moves take longer, sub-linearly; speed scales time', () => {
  assert.equal(travelMs(0), 0);
  assert.ok(travelMs(100) < travelMs(400));
  assert.ok(travelMs(1200) < travelMs(100) * 3);
  assert.ok(Math.abs(travelMs(500, 2) - travelMs(500) / 2) < 1e-9);
});

test('paths are reproducible for a seed, and trivial moves are empty', () => {
  const a = cursorPath({ x: 0, y: 0 }, { x: 300, y: 200 }, { random: rng(42) });
  const b = cursorPath({ x: 0, y: 0 }, { x: 300, y: 200 }, { random: rng(42) });
  assert.deepEqual(a, b);
  assert.deepEqual(cursorPath({ x: 5, y: 5 }, { x: 5.2, y: 5 }), []);
  assert.equal(cursorPath({ x: 0, y: 0 }, { x: 100, y: 0 }, { duration: 1000 }).length, 60);
});

test('easing runs from 0 to 1', () => {
  assert.equal(ease(0), 0);
  assert.equal(ease(1), 1);
  assert.equal(ease(0.5), 0.5);
});

test('scroll steps add up to the requested distance', () => {
  for (const amount of [600, -250, 1, 0]) {
    const steps = scrollSteps(amount, 700);
    assert.equal(steps.reduce((a, s) => a + s.d, 0), amount);
    assert.ok(steps.every((s) => Number.isInteger(s.d)));
  }
});

test('output frames take the newest capture by the middle of their interval', () => {
  // Captures at 0, 40 and 95 ms; 30 fps output over 150 ms is 5 frames.
  assert.deepEqual(pickFrames([0, 40, 95], 150, 30), [0, 1, 1, 2, 2]);
  assert.deepEqual(pickFrames([0], 1000, 10).length, 10);
  assert.deepEqual(pickFrames([0], 0, 30), [0]);
});

test('events.jsonl: header first, sorted, rounded, combos kept', () => {
  const text = eventsJsonl(
    [
      { t: 20.26, k: 'move', x: 10.04, y: 20.06 },
      { t: 5, k: 'key', combo: 'Ctrl+K' },
      { t: 20.26, k: 'down', b: 'left', x: 10, y: 20 },
    ],
    { width: 640, height: 360 },
  );
  const lines = text.trimEnd().split('\n').map((l) => JSON.parse(l));
  assert.deepEqual(lines[0], { format: 'cinesnap-events', version: 1, width: 640, height: 360, clicks: 'exact', cursor: 'metadata' });
  assert.deepEqual(lines[1], { t: 5, k: 'key', combo: 'Ctrl+K' });
  assert.deepEqual(lines[2], { t: 20.3, k: 'move', x: 10, y: 20.1 });
  assert.equal(lines[3].k, 'down');
  assert.ok(text.endsWith('\n'));
});

test('manifest: style overrides merge deeply over the base', () => {
  const m = manifest({
    video: { width: 640, height: 360, fps: 30, durationMs: 1000 },
    style: { frame: { padding: 0.1, background: { type: 'solid', color: '#111' } } },
    app: '0.1.0',
    createdAt: new Date('2026-01-02T03:04:05.678Z'),
  });
  assert.equal(m.format, 'cinesnap-project');
  assert.equal(m.createdAt, '2026-01-02T03:04:05Z');
  assert.deepEqual(m.crop, { x: 0, y: 0, width: 640, height: 360 });
  assert.equal(m.source.platform, 'script');
  assert.equal(m.frame.background.color, '#111');
  assert.deepEqual(deepMerge({ a: { b: 1, c: 2 }, d: [1] }, { a: { c: 3 }, d: [2] }), { a: { b: 1, c: 3 }, d: [2] });
});
