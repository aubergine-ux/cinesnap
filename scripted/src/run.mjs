// One recording from start to finish: serve the app, drive the browser,
// write the bundle, optionally render it.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { chromium } from 'playwright';
import { autozoom, checkDir, cinesnapVersion, prepareDir, writeBundle } from './bundle.mjs';
import { Capture, encode, pngSize } from './capture.mjs';
import { load } from './config.mjs';
import { describe, Player } from './player.mjs';
import { serveCommand, serveDir } from './serve.mjs';
import { findCinesnap, findFfmpeg, run } from './tools.mjs';

const pkg = JSON.parse(fs.readFileSync(new URL('../package.json', import.meta.url), 'utf8'));

async function launch(cfg, headed) {
  const dsf = cfg.viewport.deviceScaleFactor;
  const options = {
    headless: !headed,
    // Signals are handled in record(), which also has other things to stop.
    handleSIGINT: false,
    handleSIGTERM: false,
    handleSIGHUP: false,
    // Without this the screencast comes out at 1x whatever the page's
    // devicePixelRatio says.
    args: dsf !== 1 ? [`--force-device-scale-factor=${dsf}`] : [],
  };
  const custom = process.env.CINESNAP_SCRIPT_CHROMIUM;
  if (custom) options.executablePath = custom;
  try {
    return await chromium.launch(options);
  } catch (e) {
    const msg = String(e.message);
    if (!custom && /Executable doesn't exist|playwright install/i.test(msg)) {
      throw new Error(
        'Chromium for Playwright is not installed. Run `npx playwright install chromium`, ' +
          'or point CINESNAP_SCRIPT_CHROMIUM at a Chromium or Chrome binary.',
      );
    }
    throw new Error(`could not start the browser: ${msg.split('\n')[0]}`);
  }
}

// Record `opts.script`. Resolves with the process exit code.
export async function record(opts) {
  const log = opts.log ?? ((m) => console.error(m));
  const cfg = load(opts.script);
  if (opts.timeout !== undefined) cfg.timeout = opts.timeout;

  const ffmpeg = findFfmpeg(opts.ffmpeg);
  const cinesnap = findCinesnap(opts.cinesnap);
  if (opts.render && !cinesnap) {
    throw new Error('--render needs the Cinesnap CLI: pass --cinesnap, set CINESNAP_BIN, or put cinesnap-cli on PATH');
  }
  // The CLI looks for ffmpeg on its own; make sure it finds the same one.
  const env = { CINESNAP_FFMPEG: ffmpeg };

  const cleanups = [];
  let cleaning = null;
  const cleanup = () =>
    (cleaning ??= (async () => {
      for (const fn of cleanups.reverse()) await Promise.resolve().then(fn).catch(() => {});
    })());
  const onSignal = (sig) => {
    log(`\n${sig}, cleaning up`);
    cleanup().finally(() => process.exit(sig === 'SIGINT' ? 130 : 143));
  };
  process.on('SIGINT', onSignal);
  process.on('SIGTERM', onSignal);
  cleanups.push(() => {
    process.off('SIGINT', onSignal);
    process.off('SIGTERM', onSignal);
  });

  try {
    const work = fs.mkdtempSync(path.join(os.tmpdir(), 'cinesnap-script-'));
    let keepWork = false;
    cleanups.push(() => keepWork || fs.rmSync(work, { recursive: true, force: true }));

    // Rendering without -o records into the temp dir and throws it away.
    const temporary = !opts.out && !!opts.render;
    const base = path.basename(opts.script).replace(/\.(ya?ml|json)$/i, '') || 'demo';
    const bundle = path.resolve(opts.out ?? (temporary ? path.join(work, `${base}.cinesnap`) : `${base}.cinesnap`));
    checkDir(bundle);

    if (cfg.serve) {
      const server = cfg.serve.dir
        ? await serveDir(cfg.serve.dir, cfg.serve.port)
        : await serveCommand(cfg.serve, log);
      cleanups.push(server.stop);
      cfg.baseUrl ??= server.url;
      log(`serving ${server.url}`);
    }

    const browser = await launch(cfg, opts.headed);
    cleanups.push(() => browser.close());
    const { width, height, deviceScaleFactor } = cfg.viewport;
    const context = await browser.newContext({
      viewport: { width, height },
      deviceScaleFactor,
      colorScheme: cfg.colorScheme,
    });
    const page = await context.newPage();
    page.setDefaultTimeout(cfg.timeout);
    page.setDefaultNavigationTimeout(Math.max(cfg.timeout, 30000));

    const player = new Player(page, cfg);
    const total = cfg.steps.length;
    const step = async (s) => {
      log(`[${s.index + 1}/${total}] ${describe(s).replace(/^step \d+ \((.*)\)$/, '$1')}`);
      await player.run(s);
    };

    // The first page loads before the camera rolls, so the video opens on
    // the app rather than on a blank tab.
    await step(cfg.steps[0]);

    const frameDir = path.join(work, 'frames');
    fs.mkdirSync(frameDir);
    const cdp = await context.newCDPSession(page);
    const capture = new Capture(cdp, frameDir, {
      width: Math.round(width * deviceScaleFactor),
      height: Math.round(height * deviceScaleFactor),
    });
    await capture.start();
    await capture.writes[0];
    const frame = pngSize(fs.readFileSync(capture.frames[0].file));
    player.begin(() => capture.now(), frame.width / width);

    for (const s of cfg.steps.slice(1)) await step(s);

    const durationMs = await capture.stop();
    await browser.close();

    log(`encoding ${capture.frames.length} captured frames`);
    prepareDir(bundle);
    const video = await encode(capture, durationMs, cfg.fps, ffmpeg, path.join(bundle, 'video.mp4'));
    const app = (cinesnap && (await cinesnapVersion(cinesnap))) ?? pkg.version;
    writeBundle(bundle, {
      events: player.events.filter((e) => e.t <= video.durationMs),
      video: { ...video, fps: cfg.fps },
      style: cfg.style,
      app,
    });

    if (cinesnap) {
      log(`autozoom: ${await autozoom(cinesnap, bundle, env)}`);
    } else {
      log('warning: the Cinesnap CLI was not found, so no zoom segments were generated');
    }
    const secs = (video.durationMs / 1000).toFixed(1);
    if (!temporary) log(`wrote ${bundle} (${video.width}x${video.height}, ${secs} s)`);

    if (!opts.render) return 0;
    const out = path.resolve(opts.render);
    fs.mkdirSync(path.dirname(out), { recursive: true });
    const args = ['render', bundle, '-o', out];
    if (opts.preset) args.push('--preset', opts.preset);
    const r = await run(cinesnap, args, { env, inherit: true });
    if (temporary && (opts.keep || r.code !== 0)) {
      keepWork = true;
      log(`kept the recording at ${bundle}`);
    }
    return r.code;
  } finally {
    await cleanup();
  }
}
