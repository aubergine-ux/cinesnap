import assert from 'node:assert/strict';
import test from 'node:test';
import { parseCombo } from '../src/keys.mjs';

const display = (s, platform = 'linux') => parseCombo(s, platform).display;
const keys = (s, platform = 'linux') => parseCombo(s, platform).keys;

test('display strings', () => {
  assert.equal(display('Control+K'), 'Ctrl+K');
  assert.equal(display('control+k'), 'Ctrl+K');
  assert.equal(display('k'), 'K');
  assert.equal(display('Enter'), 'Enter');
  assert.equal(display('Escape'), 'Esc');
  assert.equal(display('ArrowUp'), 'Up');
  assert.equal(display('Shift+ArrowLeft'), 'Shift+Left');
  assert.equal(display('PageDown'), 'PageDown');
  assert.equal(display('f5'), 'F5');
  assert.equal(display('Space'), 'Space');
  assert.equal(display(' '), 'Space');
  assert.equal(display('Control+/'), 'Ctrl+/');
  assert.equal(display('Control++'), 'Ctrl++');
  assert.equal(display('+'), '+');
  assert.equal(display('Alt+Digit1'), 'Alt+1');
  assert.equal(display('Control+KeyS'), 'Ctrl+S');
});

test('modifiers come out in a fixed order', () => {
  assert.equal(display('Meta+Shift+Alt+Control+P'), 'Ctrl+Alt+Shift+Meta+P');
  assert.equal(display('Shift+Control+Shift+P'), 'Ctrl+Shift+P');
  assert.equal(display('Cmd+Option+Esc'), 'Alt+Meta+Esc');
});

test('ControlOrMeta follows the platform', () => {
  assert.equal(display('ControlOrMeta+S', 'linux'), 'Ctrl+S');
  assert.equal(display('ControlOrMeta+S', 'darwin'), 'Meta+S');
  assert.equal(keys('Mod+S', 'win32'), 'Control+s');
});

test('what Playwright is asked to press', () => {
  assert.equal(keys('Ctrl+K'), 'Control+k');
  assert.equal(keys('K'), 'K');
  assert.equal(keys('Esc'), 'Escape');
  assert.equal(keys('Up'), 'ArrowUp');
  assert.equal(keys('Shift+Tab'), 'Shift+Tab');
  assert.equal(keys('Control++'), 'Control++');
});

test('rejects nonsense', () => {
  for (const bad of ['', 'Control+', 'Control', 'Hyper+K', 'Control+Bogus', '+K', 'Control++K', 'F13']) {
    assert.throws(() => parseCombo(bad), Error, bad);
  }
  assert.throws(() => parseCombo(5));
});
