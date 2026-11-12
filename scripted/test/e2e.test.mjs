// Records a real script against a page served from this file, then checks
// the bundle and, when the Cinesnap CLI is around, renders it.

import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { onPath } from '../src/tools.mjs';

const BIN = fileURLToPath(new URL('../bin/cinesnap-script.mjs', import.meta.url));
const SECRET = 'hunter2-typed-text';

// The page goes white while the mouse button is held on the button, which
// makes the click visible in the video for the sync check.
const PAGE = `<!doctype html><meta charset="utf-8"><title>e2e</title>
<body style="margin:0;background:#000;color:#888;font:16px sans-serif">
<button id="go" style="position:absolute;left:500px;top:260px;width:160px;height:60px">Go</button>
<input id="name" style="position:absolute;left:100px;top:100px;width:200px;height:30px">
<div id="out" style="position:absolute;left:100px;top:400px"></div>
<div style="height:2000px"></div>
<script>
  const go = document.getElementById('go');
  go.addEventListener('mousedown', () => (document.body.style.background = '#fff'));
  addEventListener('mouseup', () => (document.body.style.background = '#000'));
  addEventListener('keydown', (e) => {
    if (e.ctrlKey && e.key === 'k') { e.preventDefault(); document.getElementById('out').textContent = 'palette'; }
  });
</script>`;

function findChromium() {
  if (process.env.CINESNAP_SCRIPT_CHROMIUM) return process.env.CINESNAP_SCRIPT_CHROMIUM;
  if (fs.existsSync(chromium.executablePath())) return null; // Playwright's own
  const system = ['/usr/bin/chromium', '/usr/bin/chromium-browser', '/usr/bin/google-chrome'].find((p) => fs.existsSync(p));
  return system ?? false;
}

function findCinesnap() {
  const names = ['cinesnap-cli', 'cinesnap'];
  const dirs = [process.env.CARGO_TARGET_DIR, fileURLToPath(new URL('../../target', import.meta.url))].filter(Boolean);
  const built = dirs.flatMap((d) => ['release', 'debug'].map((p) => path.join(d, p, 'cinesnap-cli')));
  return process.env.CINESNAP_BIN || names.map(onPath).find(Boolean) || built.find((p) => fs.existsSync(p)) || null;
}

function runScript(args, env) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [BIN, ...args], { env: { ...process.env, ...env } });
    let out = '';
    child.stdout.on('data', (d) => (out += d));
    child.stderr.on('data', (d) => (out += d));
    child.on('close', (code) => resolve({ code, out }));
  });
}

test('check accepts a good script and explains a bad one', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cinesnap-e2e-'));
  try {
    fs.writeFileSync(path.join(dir, 'good.yml'), 'steps:\n  - open: /\n  - click: "#go"\n');
    fs.writeFileSync(path.join(dir, 'bad.yml'), 'steps:\n  - open: /\n  - clik: "#go"\n  - type: { into: "#a" }\n');
    fs.writeFileSync(path.join(dir, 'good.json'), JSON.stringify({ steps: [{ open: '/' }] }));
    const good = await runScript(['check', path.join(dir, 'good.yml')]);
    assert.equal(good.code, 0, good.out);
    assert.match(good.out, /ok \(2 steps\)/);
    assert.equal((await runScript(['check', path.join(dir, 'good.json')])).code, 0);
    const bad = await runScript(['check', path.join(dir, 'bad.yml')]);
    assert.equal(bad.code, 1);
    assert.match(bad.out, /step 2: unknown step "clik"/);
    assert.match(bad.out, /step 3 \(type\): missing "text"/);
    assert.equal((await runScript(['run'])).code, 2);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('records a bundle that Cinesnap can render', async (t) => {
  const ffmpeg = process.env.CINESNAP_FFMPEG || onPath('ffmpeg');
  if (!ffmpeg) return t.skip('ffmpeg is not installed');
  const browser = findChromium();
  if (browser === false) {
    return t.skip('no Chromium: run `npx playwright install chromium` or set CINESNAP_SCRIPT_CHROMIUM');
  }
  const cinesnap = findCinesnap();

  const server = http.createServer((req, res) => {
    res.writeHead(200, { 'content-type': 'text/html' }).end(PAGE);
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cinesnap-e2e-'));
  t.after(() => {
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
  });

  const fps = 30;
  const script = {
    name: 'e2e',
    viewport: { width: 800, height: 450 },
    fps,
    baseUrl: `http://127.0.0.1:${server.address().port}`,
    style: { frame: { padding: 0.1 }, cursor: { size: 2 } },
    steps: [
      { open: '/' },
      { wait: 300 },
      { click: { selector: '#go', hold: 300 } },
      { wait: 300 },
      { type: { into: '#name', text: SECRET, delay: 15 } },
      { press: 'Control+K' },
      { waitFor: 'text=palette' },
      { scroll: { y: 300 } },
      { move: { x: 700, y: 60 } },
      { wait: 300 },
    ],
  };
  const file = path.join(dir, 'demo.yml');
  fs.writeFileSync(file, JSON.stringify(script));
  const bundle = path.join(dir, 'demo.cinesnap');
  const env = { ...(browser ? { CINESNAP_SCRIPT_CHROMIUM: browser } : {}), ...(cinesnap ? { CINESNAP_BIN: cinesnap } : {}) };

  const run = await runScript(['run', file, '-o', bundle], env);
  assert.equal(run.code, 0, run.out);

  // project.json
  const project = JSON.parse(fs.readFileSync(path.join(bundle, 'project.json'), 'utf8'));
  assert.equal(project.format, 'cinesnap-project');
  // The CLI may have migrated the manifest to a newer version while adding zooms.
  assert.ok(project.version >= 1);
  assert.match(project.createdAt, /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/);
  assert.equal(project.source.platform, 'script');
  assert.equal(project.source.target, 'window');
  const v = project.source.video;
  assert.deepEqual([v.file, v.width, v.height, v.fps], ['video.mp4', 800, 450, fps]);
  assert.deepEqual(project.crop, { x: 0, y: 0, width: 800, height: 450 });
  assert.equal(project.frame.padding, 0.1);
  assert.equal(project.cursor.size, 2);
  assert.ok(v.durationMs > 3000 && v.durationMs < 20000, `duration ${v.durationMs}`);
  if (cinesnap) assert.ok(project.zooms.length >= 1, 'autozoom found the click');

  // events.jsonl
  const text = fs.readFileSync(path.join(bundle, 'events.jsonl'), 'utf8');
  assert.ok(!text.includes(SECRET), 'typed text must not be logged');
  const [header, ...events] = text.trimEnd().split('\n').map((l) => JSON.parse(l));
  assert.deepEqual(header, { format: 'cinesnap-events', version: 1, width: 800, height: 450, clicks: 'exact', cursor: 'metadata' });
  for (let i = 1; i < events.length; i++) assert.ok(events[i].t >= events[i - 1].t, 'times never go back');
  assert.ok(events[0].t >= 0 && events.at(-1).t <= v.durationMs);
  const of = (k) => events.filter((e) => e.k === k);
  const [down] = of('down');
  const [up] = of('up');
  assert.equal(of('down').length, 2);
  assert.deepEqual([down.b, down.x, down.y], ['left', 580, 290], 'the click is in the middle of the button');
  assert.ok(up.t - down.t >= 290 && up.t - down.t < 450);
  const keys = of('key');
  assert.equal(keys.length, SECRET.length + 1);
  assert.deepEqual(keys.filter((e) => e.combo).map((e) => e.combo), ['Ctrl+K']);
  assert.ok(keys.every((e) => Object.keys(e).every((f) => ['t', 'k', 'combo'].includes(f))));
  assert.ok(of('scroll').reduce((a, e) => a + e.dy, 0) === -3);
  assert.ok(of('move').length > 60);
  const last = of('move').at(-1);
  assert.deepEqual([last.x, last.y], [700, 60]);

  // video.mp4: size, frame count, and when the click shows up in it.
  const probe = spawnSync(ffmpeg, ['-hide_banner', '-i', path.join(bundle, 'video.mp4')], { encoding: 'utf8' }).stderr;
  assert.match(probe, /Video: h264.*yuv420p.*800x450.* 30 fps/s);
  const luma = spawnSync(
    ffmpeg,
    ['-v', 'error', '-i', path.join(bundle, 'video.mp4'), '-vf', 'scale=1:1,format=gray', '-f', 'rawvideo', '-'],
    { maxBuffer: 1 << 26 },
  ).stdout;
  assert.equal(luma.length, Math.round((v.durationMs * fps) / 1000), 'duration matches the frame count');
  const lit = [...luma].map((b) => b > 100);
  const on = lit.indexOf(true);
  const off = lit.indexOf(false, on);
  assert.ok(on > 0, 'the click is visible in the video');
  const frame = 1000 / fps;
  const lagDown = on * frame - down.t;
  const lagUp = off * frame - up.t;
  t.diagnostic(`click shows ${lagDown.toFixed(1)} ms after the logged press, release ${lagUp.toFixed(1)} ms`);
  // Normally within a frame; the margin is for loaded CI machines.
  for (const lag of [lagDown, lagUp]) assert.ok(lag > -frame && lag < 3 * frame, `out of sync by ${lag.toFixed(1)} ms`);

  if (!cinesnap) {
    t.diagnostic('SKIPPED the render check: no cinesnap-cli (set CINESNAP_BIN to include it)');
    return;
  }
  const gif = path.join(dir, 'out.gif');
  const render = await runScript(['run', file, '--render', gif, '--preset', 'readme'], env);
  assert.equal(render.code, 0, render.out);
  const head = Buffer.alloc(6);
  const fd = fs.openSync(gif, 'r');
  fs.readSync(fd, head, 0, 6, 0);
  fs.closeSync(fd);
  assert.match(head.toString('latin1'), /^GIF8[79]a$/);
  assert.ok(fs.statSync(gif).size > 10000);
});
