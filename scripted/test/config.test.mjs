import assert from 'node:assert/strict';
import test from 'node:test';
import { ConfigError, validate } from '../src/config.mjs';

const ok = (steps, extra = {}) => validate({ steps: [{ open: '/' }, ...steps], ...extra }, { dir: '/tmp/x' });

function problems(raw) {
  try {
    validate(raw, { dir: '/tmp/x' });
  } catch (e) {
    assert.ok(e instanceof ConfigError, e.message);
    return e.problems;
  }
  assert.fail('expected the script to be rejected');
}
const stepProblems = (...steps) => problems({ steps: [{ open: '/' }, ...steps] });

test('defaults', () => {
  const cfg = ok([]);
  assert.deepEqual(cfg.viewport, { width: 1280, height: 720, deviceScaleFactor: 1 });
  assert.equal(cfg.fps, 30);
  assert.equal(cfg.timeout, 15000);
  assert.deepEqual(cfg.style, {});
  assert.deepEqual(cfg.steps, [{ url: '/', kind: 'open', index: 0 }]);
});

test('short and long forms normalise to the same step', () => {
  const cfg = ok([
    { click: 'text=Go' },
    { click: { selector: 'text=Go', button: 'right', count: 2 } },
    { wait: 800 },
    { pause: 500 },
    { type: 'hello' },
    { type: { into: '#email', text: 'a@b.c', delay: 40 } },
    { press: 'Control+K' },
    { hover: '.card >> nth=1' },
    { scroll: 600 },
    { scroll: { to: '#footer' } },
    { waitFor: { selector: '.toast', state: 'hidden', timeout: 1000 } },
    { move: { x: 10, y: 20 } },
    { move: '#logo' },
    { select: { from: 'select', option: 'B' } },
    { check: '#agree' },
    { drag: { from: '#a', to: '#b' } },
  ]);
  const s = cfg.steps;
  assert.deepEqual(s[1], { selector: 'text=Go', kind: 'click', index: 1 });
  assert.equal(s[2].count, 2);
  assert.deepEqual(s[3], { ms: 800, kind: 'wait', index: 3 });
  assert.equal(s[4].kind, 'wait');
  assert.equal(s[5].text, 'hello');
  assert.equal(s[9].y, 600);
  assert.equal(s[13].to, '#logo');
  assert.equal(s.length, 17);
});

test('top-level options', () => {
  const cfg = ok([], {
    name: 'Tour',
    viewport: { width: 800, height: 600, deviceScaleFactor: 2 },
    fps: 60,
    colorScheme: 'dark',
    baseUrl: 'http://localhost:3000',
    serve: { command: 'npm run dev', url: 'http://localhost:3000', timeout: 20 },
    style: { frame: { padding: 0.1 } },
    pointer: { speed: 2, start: { x: 1, y: 2 } },
  });
  assert.equal(cfg.viewport.deviceScaleFactor, 2);
  assert.equal(cfg.serve.timeout, 20);
  assert.equal(cfg.serve.cwd, '/tmp/x');
  assert.equal(cfg.pointer.speed, 2);
  assert.equal(ok([], { serve: { dir: 'site' } }).serve.dir, '/tmp/x/site');
});

test('unknown step, with its position', () => {
  const p = stepProblems({ click: 'a' }, { clik: 'a' });
  assert.equal(p.length, 1);
  assert.match(p[0], /^step 3: unknown step "clik" \(known steps: open, wait, /);
});

test('missing and mistyped fields name the step and the field', () => {
  assert.deepEqual(stepProblems({ type: { into: '#a' } }), ['step 2 (type): missing "text"']);
  assert.deepEqual(stepProblems({ drag: { from: '#a' } }), ['step 2 (drag): missing "to"']);
  assert.match(stepProblems({ wait: 'soon' })[0], /^step 2 \(wait\): "ms" must be a number/);
  assert.match(stepProblems({ click: { selector: 'a', button: 'top' } })[0], /"button" must be one of "left", "right", "middle"/);
  assert.match(stepProblems({ click: { selector: 'a', colour: 1 } })[0], /^step 2 \(click\): unknown option "colour"/);
  assert.match(stepProblems({ waitFor: { selector: 'a', state: 'gone' } })[0], /"state" must be one of/);
  assert.match(stepProblems({ select: 'x' })[0], /^step 2 \(select\): must be a mapping with "from", "option"/);
  assert.match(stepProblems({ click: null })[0], /has no value/);
});

test('steps with alternatives', () => {
  assert.match(stepProblems({ scroll: {} })[0], /needs "y"/);
  assert.match(stepProblems({ scroll: { y: 1, to: 'a' } })[0], /not both/);
  assert.match(stepProblems({ move: { x: 1 } })[0], /needs both "x" and "y"/);
});

test('key combinations are checked up front', () => {
  assert.match(stepProblems({ press: 'Hyper+K' })[0], /^step 2 \(press\): "key" unknown modifier "Hyper"/);
  assert.match(stepProblems({ press: 'Control+' })[0], /not a key combination/);
});

test('malformed steps', () => {
  assert.match(stepProblems('click')[0], /^step 2: must be a mapping.*did you mean "- click: \.\.\."/);
  assert.match(stepProblems({ click: 'a', wait: 5 })[0], /^step 2: has 2 actions/);
  assert.match(stepProblems({})[0], /^step 2: is empty/);
});

test('all problems are reported together', () => {
  const p = problems({ fps: 0, colour: 'red', viewport: { width: 'wide' }, steps: [{ open: '/' }, { nope: 1 }, { click: {} }] });
  assert.equal(p.length, 5);
  assert.match(new ConfigError(p, 'demo.yml').message, /^demo\.yml: 5 problems\n {2}- /);
});

test('top-level mistakes', () => {
  assert.match(problems([])[0], /must be a mapping/);
  assert.match(problems({})[0], /"steps" must be a list/);
  assert.match(problems({ steps: [] })[0], /"steps" is empty/);
  assert.match(problems({ steps: [{ click: 'a' }] })[0], /first step must be "open"/);
  assert.match(problems({ steps: [{ open: '/' }], baseUrl: 'localhost' })[0], /"baseUrl" must be an http/);
  assert.match(problems({ steps: [{ open: '/' }], serve: { command: 'x' } })[0], /needs a "url"/);
  assert.match(problems({ steps: [{ open: '/' }], serve: { command: 'x', url: 'http://a', dir: '.' } })[0], /not both/);
  assert.match(problems({ steps: [{ open: '/' }], style: { zooms: {} } })[0], /unknown section "zooms"/);
  assert.match(problems({ steps: [{ open: '/' }], viewport: { deviceScaleFactor: 9 } })[0], /from 1 to 4/);
});
