// Writing the .cinesnap bundle: see docs/recording-format.md.

import fs from 'node:fs';
import path from 'node:path';
import { run } from './tools.mjs';

const round = (n, places) => Math.round(n * 10 ** places) / 10 ** places;

// events.jsonl text: the header, then events in time order.
export function eventsJsonl(events, { width, height }) {
  const lines = [JSON.stringify({ format: 'cinesnap-events', version: 1, width, height, clicks: 'exact', cursor: 'metadata' })];
  // Stable sort; the log is nearly ordered already.
  const sorted = events.map((e, i) => [e, i]).sort((a, b) => a[0].t - b[0].t || a[1] - b[1]);
  for (const [e] of sorted) {
    const out = { t: round(e.t, 1), k: e.k };
    for (const [key, v] of Object.entries(e)) {
      if (key === 't' || key === 'k') continue;
      out[key] = key === 'x' || key === 'y' ? round(v, 1) : v;
    }
    lines.push(JSON.stringify(out));
  }
  return lines.join('\n') + '\n';
}

export function deepMerge(base, over) {
  const out = { ...base };
  for (const [k, v] of Object.entries(over)) {
    const both = v && typeof v === 'object' && !Array.isArray(v) && out[k] && typeof out[k] === 'object' && !Array.isArray(out[k]);
    out[k] = both ? deepMerge(out[k], v) : v;
  }
  return out;
}

// A fresh manifest. Edit state that is left out takes the app's defaults.
export function manifest({ video, style = {}, app, createdAt = new Date() }) {
  return deepMerge(
    {
      format: 'cinesnap-project',
      version: 1,
      createdAt: createdAt.toISOString().replace(/\.\d+Z$/, 'Z'),
      app,
      source: {
        video: { file: 'video.mp4', width: video.width, height: video.height, fps: video.fps, durationMs: video.durationMs },
        events: { file: 'events.jsonl' },
        platform: 'script',
        target: 'window',
      },
      crop: { x: 0, y: 0, width: video.width, height: video.height },
      cuts: [],
      zooms: [],
    },
    style,
  );
}

// Refuse to write a bundle over something that is not one.
export function checkDir(dir) {
  if (!fs.existsSync(dir)) return;
  if (!fs.statSync(dir).isDirectory()) throw new Error(`${dir} exists and is not a directory`);
  const entries = fs.readdirSync(dir);
  if (entries.length && !entries.includes('project.json')) {
    throw new Error(`${dir} exists and is not a Cinesnap recording; refusing to overwrite it`);
  }
}

// Create `dir`, or clear the previous recording out of it.
export function prepareDir(dir) {
  checkDir(dir);
  fs.mkdirSync(dir, { recursive: true });
  for (const f of ['project.json', 'events.jsonl', 'video.mp4']) fs.rmSync(path.join(dir, f), { force: true });
}

export function writeBundle(dir, { events, video, style, app }) {
  fs.writeFileSync(path.join(dir, 'events.jsonl'), eventsJsonl(events, video));
  fs.writeFileSync(path.join(dir, 'project.json'), JSON.stringify(manifest({ video, style, app }), null, 2) + '\n');
}

// "0.1.0" from `cinesnap-cli --version`, or null.
export async function cinesnapVersion(bin) {
  const r = await run(bin, ['--version']).catch(() => null);
  return r && r.code === 0 ? (/(\d+\.\d+\.\d+\S*)/.exec(r.stdout)?.[1] ?? null) : null;
}

// Generate zoom segments. This is also the check that the app can load what
// we wrote, style overrides included.
export async function autozoom(bin, dir, env) {
  const r = await run(bin, ['autozoom', dir], { env });
  if (r.code !== 0) {
    throw new Error(`${path.basename(bin)} could not load the recording\n${(r.stderr || r.stdout).trim()}`);
  }
  return r.stdout.trim();
}
