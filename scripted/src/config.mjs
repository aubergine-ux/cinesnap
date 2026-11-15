// Loading and validating a demo script. Everything downstream works on the
// normalised form returned by `validate`.

import fs from 'node:fs';
import path from 'node:path';
import YAML from 'yaml';
import { parseCombo } from './keys.mjs';

export class ConfigError extends Error {
  constructor(problems, file) {
    const head = file ? `${file}: ` : '';
    super(
      problems.length === 1
        ? `${head}${problems[0]}`
        : `${head}${problems.length} problems\n${problems.map((p) => `  - ${p}`).join('\n')}`,
    );
    this.name = 'ConfigError';
    this.problems = problems;
  }
}

export const DEFAULTS = {
  viewport: { width: 1280, height: 720, deviceScaleFactor: 1 },
  fps: 30,
  timeout: 15000,
};

const STYLE_SECTIONS = ['frame', 'cursor', 'autoZoom', 'export'];
const TOP_KEYS = ['name', 'viewport', 'fps', 'colorScheme', 'baseUrl', 'serve', 'style', 'steps', 'pointer', 'timeout', 'seed'];
const BUTTONS = ['left', 'right', 'middle'];

const isObj = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
const isNum = (v) => typeof v === 'number' && Number.isFinite(v);
const isStr = (v) => typeof v === 'string' && v !== '';
const quote = (list) => list.map((k) => `"${k}"`).join(', ');

// Field checkers return an error string or nothing.
const T = {
  string: (v) => (isStr(v) ? null : 'must be a non-empty string'),
  text: (v) => (typeof v === 'string' ? null : 'must be a string'),
  ms: (v) => (isNum(v) && v >= 0 ? null : 'must be a number of milliseconds (0 or more)'),
  number: (v) => (isNum(v) ? null : 'must be a number'),
  bool: (v) => (typeof v === 'boolean' ? null : 'must be true or false'),
  count: (v) => (Number.isInteger(v) && v >= 1 && v <= 100 ? null : 'must be a whole number from 1 to 100'),
  oneOf: (...opts) => (v) => (opts.includes(v) ? null : `must be one of ${quote(opts)}`),
  point: (v) => (isObj(v) && isNum(v.x) && isNum(v.y) ? null : 'must be { x, y }'),
  combo: (v) => {
    try {
      parseCombo(v);
      return null;
    } catch (e) {
      return e.message;
    }
  },
};

// kind -> { short: field a bare value maps to, fields: { name: [check, required] }, rule }
const STEPS = {
  open: {
    short: 'url',
    fields: {
      url: [T.string, true],
      waitUntil: [T.oneOf('load', 'domcontentloaded', 'networkidle', 'commit')],
      timeout: [T.ms],
    },
  },
  wait: { short: 'ms', fields: { ms: [T.ms, true] } },
  pause: { short: 'ms', fields: { ms: [T.ms, true] }, as: 'wait' },
  click: {
    short: 'selector',
    fields: {
      selector: [T.string, true],
      button: [T.oneOf(...BUTTONS)],
      count: [T.oneOf(1, 2)],
      position: [T.point],
      hold: [T.ms],
      timeout: [T.ms],
    },
  },
  type: {
    short: 'text',
    fields: { text: [T.text, true], into: [T.string], delay: [T.ms], timeout: [T.ms] },
  },
  press: {
    short: 'key',
    fields: { key: [T.combo, true], times: [T.count], delay: [T.ms] },
  },
  hover: { short: 'selector', fields: { selector: [T.string, true], position: [T.point], timeout: [T.ms] } },
  scroll: {
    short: 'y',
    fields: { x: [T.number], y: [T.number], to: [T.string], duration: [T.ms], timeout: [T.ms] },
    rule: (s) => {
      const by = s.x !== undefined || s.y !== undefined;
      if (by && s.to !== undefined) return 'use either "x"/"y" or "to", not both';
      if (!by && s.to === undefined) return 'needs "y" (or "x") in pixels, or "to" with a selector';
    },
  },
  waitFor: {
    short: 'selector',
    fields: {
      selector: [T.string, true],
      state: [T.oneOf('visible', 'hidden', 'attached', 'detached')],
      timeout: [T.ms],
    },
  },
  move: {
    short: 'to',
    fields: { x: [T.number], y: [T.number], to: [T.string], duration: [T.ms], timeout: [T.ms] },
    rule: (s) => {
      const at = s.x !== undefined && s.y !== undefined;
      if (s.to !== undefined && (s.x !== undefined || s.y !== undefined)) return 'use either "x" and "y" or "to", not both';
      if (!at && s.to === undefined) return 'needs both "x" and "y", or "to" with a selector';
    },
  },
  select: {
    fields: { from: [T.string, true], option: [T.string, true], timeout: [T.ms] },
  },
  check: {
    short: 'selector',
    fields: { selector: [T.string, true], checked: [T.bool], timeout: [T.ms] },
  },
  drag: {
    fields: { from: [T.string, true], to: [T.string, true], hold: [T.ms], timeout: [T.ms] },
  },
};

export const STEP_KINDS = Object.keys(STEPS);

function checkFields(value, spec, where, problems) {
  for (const key of Object.keys(value)) {
    if (!spec[key]) problems.push(`${where}: unknown option "${key}" (allowed: ${quote(Object.keys(spec))})`);
  }
  for (const [key, [check, required]] of Object.entries(spec)) {
    if (value[key] === undefined || value[key] === null) {
      if (required) problems.push(`${where}: missing "${key}"`);
      continue;
    }
    const err = check(value[key]);
    if (err) problems.push(`${where}: "${key}" ${err}`);
  }
}

function validateStep(raw, index, problems) {
  const n = index + 1;
  if (!isObj(raw)) {
    const hint = typeof raw === 'string' ? ` (did you mean "- ${raw}: ..."?)` : '';
    problems.push(`step ${n}: must be a mapping with one action, like "- click: button"${hint}`);
    return null;
  }
  const keys = Object.keys(raw);
  if (keys.length !== 1) {
    problems.push(
      keys.length === 0
        ? `step ${n}: is empty`
        : `step ${n}: has ${keys.length} actions (${quote(keys)}); use one per step and put options inside it`,
    );
    return null;
  }
  const kind = keys[0];
  const def = STEPS[kind];
  if (!def) {
    problems.push(`step ${n}: unknown step "${kind}" (known steps: ${STEP_KINDS.join(', ')})`);
    return null;
  }
  const where = `step ${n} (${kind})`;
  let value = raw[kind];
  if (!isObj(value)) {
    if (value === undefined || value === null) {
      problems.push(`${where}: has no value`);
      return null;
    }
    if (!def.short) {
      problems.push(`${where}: must be a mapping with ${quote(Object.keys(def.fields))}`);
      return null;
    }
    value = { [def.short]: value };
  }
  const before = problems.length;
  checkFields(value, def.fields, where, problems);
  if (problems.length === before && def.rule) {
    const err = def.rule(value);
    if (err) problems.push(`${where}: ${err}`);
  }
  return { ...value, kind: def.as ?? kind, index };
}

// Check a parsed script and fill in defaults. `dir` is the directory of the
// script file; relative paths in it resolve against that.
export function validate(raw, { dir = process.cwd(), file } = {}) {
  const problems = [];
  if (!isObj(raw)) throw new ConfigError(['the script must be a mapping with a "steps" list'], file);

  for (const key of Object.keys(raw)) {
    if (!TOP_KEYS.includes(key)) problems.push(`unknown option "${key}" (allowed: ${quote(TOP_KEYS)})`);
  }

  const cfg = { dir, name: 'Demo', fps: DEFAULTS.fps, timeout: DEFAULTS.timeout, seed: 1 };
  if (raw.name != null) {
    if (isStr(raw.name)) cfg.name = raw.name;
    else problems.push('"name" must be a non-empty string');
  }

  cfg.viewport = { ...DEFAULTS.viewport };
  if (raw.viewport != null) {
    if (!isObj(raw.viewport)) problems.push('"viewport" must be { width, height }');
    else {
      for (const [k, v] of Object.entries(raw.viewport)) {
        if (k === 'width' || k === 'height') {
          if (Number.isInteger(v) && v >= 64 && v <= 7680) cfg.viewport[k] = v;
          else problems.push(`viewport: "${k}" must be a whole number from 64 to 7680`);
        } else if (k === 'deviceScaleFactor') {
          if (isNum(v) && v >= 1 && v <= 4) cfg.viewport[k] = v;
          else problems.push('viewport: "deviceScaleFactor" must be a number from 1 to 4');
        } else problems.push(`viewport: unknown option "${k}" (allowed: "width", "height", "deviceScaleFactor")`);
      }
    }
  }

  if (raw.fps != null) {
    if (Number.isInteger(raw.fps) && raw.fps >= 1 && raw.fps <= 60) cfg.fps = raw.fps;
    else problems.push('"fps" must be a whole number from 1 to 60');
  }
  if (raw.timeout != null) {
    if (isNum(raw.timeout) && raw.timeout > 0) cfg.timeout = raw.timeout;
    else problems.push('"timeout" must be a positive number of milliseconds');
  }
  if (raw.seed != null) {
    if (Number.isInteger(raw.seed)) cfg.seed = raw.seed;
    else problems.push('"seed" must be a whole number');
  }
  if (raw.colorScheme != null) {
    if (['light', 'dark', 'no-preference'].includes(raw.colorScheme)) cfg.colorScheme = raw.colorScheme;
    else problems.push('"colorScheme" must be "light", "dark" or "no-preference"');
  }
  if (raw.baseUrl != null) {
    if (isStr(raw.baseUrl) && /^https?:\/\//.test(raw.baseUrl)) cfg.baseUrl = raw.baseUrl;
    else problems.push('"baseUrl" must be an http(s) URL');
  }

  cfg.pointer = { speed: 1 };
  if (raw.pointer != null) {
    if (!isObj(raw.pointer)) problems.push('"pointer" must be a mapping');
    else {
      checkFields(
        raw.pointer,
        { speed: [(v) => (isNum(v) && v >= 0.1 && v <= 10 ? null : 'must be a number from 0.1 to 10')], start: [T.point] },
        'pointer',
        problems,
      );
      Object.assign(cfg.pointer, raw.pointer);
    }
  }

  if (raw.serve != null) {
    const s = raw.serve;
    if (!isObj(s)) problems.push('"serve" must be a mapping');
    else {
      checkFields(
        s,
        {
          command: [T.string],
          url: [(v) => (isStr(v) && /^https?:\/\//.test(v) ? null : 'must be an http(s) URL')],
          dir: [T.string],
          port: [(v) => (Number.isInteger(v) && v >= 0 && v <= 65535 ? null : 'must be a port number')],
          cwd: [T.string],
          timeout: [(v) => (isNum(v) && v > 0 ? null : 'must be a positive number of seconds')],
        },
        'serve',
        problems,
      );
      if (isStr(s.command) && isStr(s.dir)) problems.push('serve: use either "command" or "dir", not both');
      else if (isStr(s.command)) {
        if (!s.url) problems.push('serve: "command" needs a "url" to wait for');
        if (s.port != null) problems.push('serve: "port" only applies to "dir"');
      } else if (isStr(s.dir)) {
        if (s.url != null) problems.push('serve: "url" only applies to "command"');
      } else problems.push('serve: needs "command" (with "url") or "dir"');
      cfg.serve = {
        ...s,
        timeout: s.timeout ?? 60,
        cwd: path.resolve(dir, s.cwd ?? '.'),
        ...(isStr(s.dir) ? { dir: path.resolve(dir, s.dir) } : {}),
      };
    }
  }

  cfg.style = {};
  if (raw.style != null) {
    if (!isObj(raw.style)) problems.push('"style" must be a mapping');
    else {
      for (const [k, v] of Object.entries(raw.style)) {
        if (!STYLE_SECTIONS.includes(k)) problems.push(`style: unknown section "${k}" (allowed: ${quote(STYLE_SECTIONS)})`);
        else if (!isObj(v)) problems.push(`style: "${k}" must be a mapping`);
        else cfg.style[k] = v;
      }
    }
  }

  cfg.steps = [];
  if (!Array.isArray(raw.steps)) problems.push('"steps" must be a list');
  else if (raw.steps.length === 0) problems.push('"steps" is empty');
  else {
    raw.steps.forEach((s, i) => {
      const step = validateStep(s, i, problems);
      if (step) cfg.steps.push(step);
    });
    const first = raw.steps[0];
    if (isObj(first) && STEPS[Object.keys(first)[0]] && Object.keys(first)[0] !== 'open') {
      problems.push('step 1: the first step must be "open" so there is a page to record');
    }
  }

  if (problems.length) throw new ConfigError(problems, file);
  return cfg;
}

export function load(file) {
  let text;
  try {
    text = fs.readFileSync(file, 'utf8');
  } catch (e) {
    throw new Error(`cannot read ${file}: ${e.code === 'ENOENT' ? 'no such file' : e.message}`);
  }
  let raw;
  try {
    // YAML is a superset of JSON, so this reads both.
    raw = YAML.parse(text, { prettyErrors: true });
  } catch (e) {
    throw new ConfigError([e.message.split('\n')[0]], file);
  }
  return validate(raw, { dir: path.dirname(path.resolve(file)), file });
}
