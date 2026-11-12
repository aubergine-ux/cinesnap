// Key combos: what the script says, what Playwright wants, and the display
// string that goes into the event log.

const MODIFIERS = {
  control: 'Control',
  ctrl: 'Control',
  alt: 'Alt',
  option: 'Alt',
  shift: 'Shift',
  meta: 'Meta',
  cmd: 'Meta',
  command: 'Meta',
  super: 'Meta',
  win: 'Meta',
  controlormeta: 'ControlOrMeta',
  mod: 'ControlOrMeta',
};

// lower-case alias -> [Playwright key, display name]
const NAMED = {
  enter: ['Enter', 'Enter'],
  return: ['Enter', 'Enter'],
  escape: ['Escape', 'Esc'],
  esc: ['Escape', 'Esc'],
  tab: ['Tab', 'Tab'],
  space: ['Space', 'Space'],
  ' ': ['Space', 'Space'],
  backspace: ['Backspace', 'Backspace'],
  delete: ['Delete', 'Delete'],
  del: ['Delete', 'Delete'],
  arrowup: ['ArrowUp', 'Up'],
  up: ['ArrowUp', 'Up'],
  arrowdown: ['ArrowDown', 'Down'],
  down: ['ArrowDown', 'Down'],
  arrowleft: ['ArrowLeft', 'Left'],
  left: ['ArrowLeft', 'Left'],
  arrowright: ['ArrowRight', 'Right'],
  right: ['ArrowRight', 'Right'],
  home: ['Home', 'Home'],
  end: ['End', 'End'],
  pageup: ['PageUp', 'PageUp'],
  pagedown: ['PageDown', 'PageDown'],
  insert: ['Insert', 'Insert'],
  plus: ['+', '+'],
};

const ORDER = ['Control', 'Alt', 'Shift', 'Meta'];
const SHORT = { Control: 'Ctrl', Alt: 'Alt', Shift: 'Shift', Meta: 'Meta' };

// "ctrl+k" -> { keys: "Control+K", display: "Ctrl+K" }. Throws on anything
// that is not zero or more modifiers followed by one key.
export function parseCombo(input, platform = process.platform) {
  if (typeof input !== 'string' || input === '') throw new Error('expected a key such as "Enter" or "Control+K"');
  // A trailing "+" is the plus key itself ("Control++").
  let parts;
  let last;
  if (input === '+' || input.endsWith('++')) {
    last = '+';
    parts = input.length > 1 ? input.slice(0, -2).split('+') : [];
  } else {
    parts = input.split('+');
    last = parts.pop();
  }
  if (last === '' || parts.some((p) => p === '')) throw new Error(`"${input}" is not a key combination`);

  const mods = new Set();
  for (const p of parts) {
    let m = MODIFIERS[p.toLowerCase()];
    if (!m) throw new Error(`unknown modifier "${p}" in "${input}" (use Control, Alt, Shift or Meta)`);
    if (m === 'ControlOrMeta') m = platform === 'darwin' ? 'Meta' : 'Control';
    mods.add(m);
  }

  let key;
  let display;
  const named = NAMED[last.toLowerCase()];
  if (MODIFIERS[last.toLowerCase()]) {
    throw new Error(`"${input}" ends in a modifier; add the key to press`);
  } else if (named) {
    [key, display] = named;
  } else if (/^f([1-9]|1[0-2])$/i.test(last)) {
    key = display = last.toUpperCase();
  } else if (/^Key[A-Z]$/.test(last)) {
    key = last;
    display = last.slice(3);
  } else if (/^Digit[0-9]$/.test(last)) {
    key = last;
    display = last.slice(5);
  } else if ([...last].length === 1) {
    // With modifiers held, press the plain key so Shift is not applied twice.
    key = mods.size && /[a-z]/i.test(last) ? last.toLowerCase() : last;
    display = last.toUpperCase();
  } else {
    throw new Error(`unknown key "${last}" in "${input}"`);
  }

  const held = ORDER.filter((m) => mods.has(m));
  return {
    keys: [...held, key].join('+'),
    display: [...held.map((m) => SHORT[m]), display].join('+'),
  };
}
