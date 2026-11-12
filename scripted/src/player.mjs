// Plays the steps of a script in the page: moves the mouse along hand-like
// paths, clicks, types and scrolls, and logs each input with its time.

import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { parseCombo } from './keys.mjs';
import { cursorPath, rng, scrollSteps } from './path.mjs';

const sleep = (ms) => new Promise((r) => setTimeout(r, Math.max(0, ms)));
const WHEEL_NOTCH = 100; // px of scrolling that count as one wheel step

// Where an `open` target points: absolute URLs as they are, paths against
// `baseUrl`, and without one, files next to the script.
export function resolveUrl(target, cfg) {
  if (/^[a-z][a-z0-9+.-]*:/i.test(target)) return target;
  if (cfg.baseUrl) return new URL(target, cfg.baseUrl).href;
  const [, file, rest = ''] = /^([^?#]*)(.*)$/.exec(target);
  const abs = path.resolve(cfg.dir, file);
  if (!fs.existsSync(abs)) {
    throw new Error(`"${target}" is not a URL and no file ${abs} exists (set "baseUrl" to open paths on a server)`);
  }
  return pathToFileURL(abs).href + rest;
}

export function describe(step) {
  const subject = step.selector ?? step.into ?? step.from ?? step.to ?? step.url ?? step.key;
  return `step ${step.index + 1} (${step.kind}${subject !== undefined ? ` ${JSON.stringify(subject)}` : ''})`;
}

export class Player {
  constructor(page, cfg) {
    this.page = page;
    this.cfg = cfg;
    this.random = rng(cfg.seed);
    this.events = [];
    this.clock = null; // () => ms since the first video frame; set once capture runs
    this.scale = 1; // video pixels per CSS pixel
    const { width, height } = cfg.viewport;
    this.pos = cfg.pointer.start ? { ...cfg.pointer.start } : { x: width * 0.5, y: height * 0.62 };
  }

  log(k, extra = {}) {
    if (this.clock) this.events.push({ t: this.clock(), k, ...extra });
  }

  // Current pointer position in video pixels.
  at() {
    return { x: this.pos.x * this.scale, y: this.pos.y * this.scale };
  }

  async place() {
    await this.page.mouse.move(this.pos.x, this.pos.y);
  }

  // Recording has started: from here on inputs are logged.
  begin(clock, scale) {
    this.clock = clock;
    this.scale = scale;
    this.events.push({ t: 0, k: 'move', ...this.at() });
  }

  async moveTo(to, duration) {
    const samples = cursorPath(this.pos, to, { speed: this.cfg.pointer.speed, duration, random: this.random });
    const start = Date.now();
    for (const s of samples) {
      await sleep(start + s.t - Date.now());
      this.pos = { x: s.x, y: s.y };
      this.log('move', this.at());
      await this.page.mouse.move(s.x, s.y);
    }
  }

  async scrollBy(dx, dy, duration) {
    const dist = Math.hypot(dx, dy);
    if (dist < 1) return;
    const total = duration ?? Math.min(1400, 350 + dist * 0.6);
    const xs = scrollSteps(Math.round(dx), total);
    const ys = scrollSteps(Math.round(dy), total);
    const start = Date.now();
    let ax = 0;
    let ay = 0;
    for (let i = 0; i < ys.length; i++) {
      await sleep(start + ys[i].t - Date.now());
      if (xs[i].d === 0 && ys[i].d === 0) continue;
      await this.page.mouse.wheel(xs[i].d, ys[i].d);
      // Wheel steps, with the usual sign: scrolling down the page is negative.
      ax += xs[i].d / WHEEL_NOTCH;
      ay -= ys[i].d / WHEEL_NOTCH;
      const last = i === ys.length - 1;
      const sx = last ? Math.round(ax * 100) / 100 : Math.trunc(ax);
      const sy = last ? Math.round(ay * 100) / 100 : Math.trunc(ay);
      if (sx !== 0 || sy !== 0) {
        this.log('scroll', { dx: sx, dy: sy, ...this.at() });
        ax -= sx;
        ay -= sy;
      }
    }
    await sleep(120); // let the page finish moving
  }

  // Wait until an element stops moving (panels sliding in, layout shifts).
  async settled(loc, timeout = this.cfg.timeout) {
    const deadline = Date.now() + timeout;
    let prev = null;
    for (;;) {
      const box = await loc.boundingBox();
      if (box && prev && box.x === prev.x && box.y === prev.y && box.width === prev.width && box.height === prev.height) return box;
      if (Date.now() > deadline) return box;
      prev = box;
      await sleep(50);
    }
  }

  // Bring an element into the viewport with a visible scroll.
  async reveal(loc, timeout) {
    const { width, height } = this.cfg.viewport;
    for (let pass = 0; pass < 2; pass++) {
      const box = await this.settled(loc, timeout);
      if (!box) return;
      const cx = box.x + box.width / 2;
      const cy = box.y + box.height / 2;
      const margin = 24;
      const offX = cx < margin || cx > width - margin;
      const offY = cy < margin || cy > height - margin;
      if (!offX && !offY) return;
      await this.scrollBy(offX ? cx - width / 2 : 0, offY ? cy - height / 2 : 0);
    }
    // Nested scrollers the wheel did not reach.
    await loc.scrollIntoViewIfNeeded();
  }

  // The point to aim at for an element, once it is ready to be used.
  async target(selector, { timeout, position, action = 'hover' } = {}) {
    const loc = this.page.locator(selector);
    await loc.waitFor({ state: 'visible', timeout });
    await this.reveal(loc, timeout);
    // Dry run: waits until the element is stable and not covered, without
    // touching the mouse.
    await loc.hover({ trial: true, timeout, position });
    if (action === 'click') {
      const deadline = Date.now() + (timeout ?? this.cfg.timeout);
      while (!(await loc.isEnabled())) {
        if (Date.now() > deadline) throw new Error(`${selector} is disabled`);
        await sleep(50);
      }
    }
    const box = await loc.boundingBox();
    if (!box) throw new Error(`${selector} is not visible`);
    return {
      loc,
      point: {
        x: box.x + (position ? position.x : box.width / 2),
        y: box.y + (position ? position.y : box.height / 2),
      },
    };
  }

  async down(button = 'left', clickCount = 1) {
    this.log('down', { b: button, ...this.at() });
    await this.page.mouse.down({ button, clickCount });
  }

  async up(button = 'left', clickCount = 1) {
    this.log('up', { b: button, ...this.at() });
    await this.page.mouse.up({ button, clickCount });
  }

  async clickHere({ button = 'left', count = 1, hold = 90 } = {}) {
    await sleep(70 + this.random() * 50); // the hand settles before pressing
    for (let n = 1; n <= count; n++) {
      if (n > 1) await sleep(110);
      await this.down(button, n);
      await sleep(hold);
      await this.up(button, n);
    }
  }

  async run(step) {
    try {
      await this[`_${step.kind}`](step);
    } catch (e) {
      const first = String(e?.message ?? e).split('\n').find((l) => l.trim()) ?? 'failed';
      const err = new Error(`${describe(step)}: ${first.trim()}`);
      err.cause = e;
      throw err;
    }
  }

  async _open(step) {
    await this.page.goto(resolveUrl(step.url, this.cfg), {
      waitUntil: step.waitUntil ?? 'load',
      timeout: step.timeout,
    });
    // The new page has not seen the pointer yet.
    await this.place();
  }

  async _wait(step) {
    await sleep(step.ms);
  }

  async _click(step) {
    const { point } = await this.target(step.selector, { ...step, action: 'click' });
    await this.moveTo(point);
    await this.clickHere(step);
  }

  async _hover(step) {
    const { point } = await this.target(step.selector, step);
    await this.moveTo(point);
  }

  async _move(step) {
    if (step.to !== undefined) {
      const { point } = await this.target(step.to, step);
      await this.moveTo(point, step.duration);
    } else {
      await this.moveTo({ x: step.x, y: step.y }, step.duration);
    }
  }

  async _type(step) {
    if (step.into !== undefined) {
      const { point } = await this.target(step.into, { ...step, action: 'click' });
      await this.moveTo(point);
      await this.clickHere();
      await sleep(180);
    }
    const delay = step.delay ?? 60;
    for (const ch of step.text) {
      // Only the fact that a key went down is logged, never which one.
      this.log('key');
      if (ch === '\n') await this.page.keyboard.press('Enter');
      else if (ch === '\t') await this.page.keyboard.press('Tab');
      else await this.page.keyboard.type(ch);
      await sleep(delay * (0.7 + 0.6 * this.random()));
    }
  }

  async _press(step) {
    const combo = parseCombo(step.key);
    for (let i = 0; i < (step.times ?? 1); i++) {
      if (i > 0) await sleep(step.delay ?? 120);
      this.log('key', { combo: combo.display });
      await this.page.keyboard.press(combo.keys);
    }
  }

  async _scroll(step) {
    if (step.to !== undefined) {
      const loc = this.page.locator(step.to);
      await loc.waitFor({ state: 'attached', timeout: step.timeout });
      const box = await loc.boundingBox();
      if (!box) throw new Error(`${step.to} has no box to scroll to`);
      const { width, height } = this.cfg.viewport;
      // Centre it, or put its top near the top when it is taller than the view.
      const dy = box.height > height * 0.8 ? box.y - 40 : box.y + box.height / 2 - height / 2;
      const cx = box.x + box.width / 2;
      const dx = cx < 0 || cx > width ? cx - width / 2 : 0;
      await this.scrollBy(dx, dy, step.duration);
    } else {
      await this.scrollBy(step.x ?? 0, step.y ?? 0, step.duration);
    }
  }

  async _waitFor(step) {
    await this.page.locator(step.selector).waitFor({ state: step.state ?? 'visible', timeout: step.timeout });
  }

  async _select(step) {
    const { loc, point } = await this.target(step.from, { ...step, action: 'click' });
    await this.moveTo(point);
    // The native menu is not part of the page, so the pick is made directly
    // and shown as a click on the control.
    await sleep(90);
    this.log('down', { b: 'left', ...this.at() });
    await loc.selectOption(step.option, { timeout: step.timeout });
    await sleep(90);
    this.log('up', { b: 'left', ...this.at() });
  }

  async _check(step) {
    const { loc, point } = await this.target(step.selector, { ...step, action: 'click' });
    await this.moveTo(point);
    if ((await loc.isChecked()) !== (step.checked ?? true)) await this.clickHere();
  }

  async _drag(step) {
    const from = await this.target(step.from, step);
    await this.moveTo(from.point);
    await sleep(100);
    await this.down();
    await sleep(step.hold ?? 150);
    const to = this.page.locator(step.to);
    await to.waitFor({ state: 'visible', timeout: step.timeout });
    const box = await to.boundingBox();
    if (!box) throw new Error(`${step.to} is not visible`);
    const dest = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
    const dist = Math.hypot(dest.x - this.pos.x, dest.y - this.pos.y);
    // Dragging is slower than pointing.
    await this.moveTo(dest, 400 + dist * 1.2);
    await sleep(120);
    await this.up();
  }
}
