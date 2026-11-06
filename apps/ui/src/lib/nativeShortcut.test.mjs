import test from 'node:test';
import assert from 'node:assert/strict';
import { HotkeyManager } from './hotkeys.ts';

test('native dispatcher honors customized bindings, guards and exact modifiers', () => {
  const manager = new HotkeyManager(); let count = 0;
  manager.registerHandler('toggle-sidebar', () => count++);
  manager.setBindings('toggle-sidebar', ['Meta+Shift+F12', 'Ctrl+Alt+S']);
  const event = (key, mods = {}) => ({ key, ctrlKey: false, altKey: false, metaKey: false, shiftKey: false, isComposing: false, getModifierState: () => false, preventDefault() {}, ...mods });
  assert.equal(manager.dispatch(event('F12', { metaKey: true, shiftKey: true })), true);
  assert.equal(manager.dispatch(event('s', { ctrlKey: true, altKey: true })), true);
  assert.equal(count, 2);
  assert.equal(manager.dispatch(event('s', { ctrlKey: true, altKey: true, getModifierState: name => name === 'AltGraph' })), false);
  assert.equal(manager.dispatch(event('s', { ctrlKey: true, altKey: true, isComposing: true })), false);
  manager.registerGuard('toggle-sidebar', () => false);
  assert.equal(manager.dispatch(event('F12', { metaKey: true, shiftKey: true })), false);
  assert.equal(count, 2);
});

test('platform hosts route through configurable DOM resolver rather than swallow defaults', async () => {
  const { readFile } = await import('node:fs/promises');
  for (const name of ['windows', 'linux', 'macos']) {
    const source = await readFile(new URL(`../../../../src-tauri/src/terminal/native_host/${name}.rs`, import.meta.url), 'utf8');
    assert.match(source, /shortcut_script/, name);
    assert.doesNotMatch(source, /input::reserved_shortcut\(/, name);
  }
});
