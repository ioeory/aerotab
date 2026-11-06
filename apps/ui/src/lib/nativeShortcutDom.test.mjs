import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { HotkeyManager } from './hotkeys.ts';

// Execute the actual Rust-generated JS template, not a second bridge implementation.
async function script() {
  const source = await readFile(new URL('../../../../src-tauri/src/terminal/native_host/shortcut.rs', import.meta.url), 'utf8');
  const raw = source.match(/format!\("(\(\(\)=>.*)"\)/)[1];
  return JSON.parse(`"${raw}"`).replaceAll('{{', '{').replaceAll('}}', '}')
    .replaceAll('{x}', '320').replaceAll('{y}', '240').replaceAll('{id}', 's')
    .replaceAll('{init}', JSON.stringify({ key: 's', ctrlKey: true, altKey: true, metaKey: false, shiftKey: false, bubbles: true, cancelable: true }));
}

test('actual native DOM script invokes configurable dispatcher once and consumes before PTY fallback', async () => {
  const js = await script(); const manager = new HotkeyManager(); let actions = 0;
  manager.registerHandler('toggle-sidebar', () => actions++);
  const element = { dataset: { nativeInputActive: 'true' }, dispatchEvent: ev => manager.dispatch(ev) };
  class KeyboardEvent { constructor(_, init) { Object.assign(this, init); this.defaultPrevented = false; } preventDefault() { this.defaultPrevented = true; } }
  const document = { hidden: false, body: { dataset: {} }, elementFromPoint: () => ({ closest: () => element }) };
  const run = () => vm.runInNewContext(js, { document, KeyboardEvent });
  assert.equal(run().consumed, true); assert.equal(actions, 1);
  manager.setBindings('toggle-sidebar', ['Meta+F12']);
  assert.equal(run().consumed, false); assert.equal(actions, 1);
  document.body.dataset.nativeViewOccluded = 'true'; assert.equal(run().blocked, true);
  delete document.body.dataset.nativeViewOccluded;
  element.dataset.nativeInputActive = 'false'; assert.equal(run().blocked, true);
  element.dataset.nativeInputActive = 'true'; document.hidden = true; assert.equal(run().blocked, true);
  document.hidden = false; document.elementFromPoint = () => ({ closest: () => null }); assert.equal(run().blocked, true);
  assert.equal(actions, 1);
});
