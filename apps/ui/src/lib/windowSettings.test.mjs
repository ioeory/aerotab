import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';
import { applyWindowSettings, getWindowSettings } from './windowSettings.ts';
const properties = new Map();
globalThis.document = { documentElement: { style: { setProperty: (key, value) => properties.set(key, value) } }, body: { dataset: {} }, dispatchEvent() {} };
globalThis.CustomEvent = class {};

test('missing persisted window settings restore complete defaults after preview', () => {
  applyWindowSettings({ opacity: 45, spaciness: 1.4, tabsLocation: 'left', sidebarVisible: false, frameStyle: 'frameless', focusFollowsMouse: true });
  applyWindowSettings({}, { replace: true });
  assert.equal(properties.get('--ui-opacity'), '1');
  assert.equal(properties.get('--ui-bg-opacity-percent'), '100%');
  assert.equal(properties.get('--ui-spaciness'), '1');
  assert.equal(document.body.dataset.translucent, 'false');
  assert.equal(document.body.dataset.tabsLocation, 'top');
  assert.equal(document.body.dataset.sidebarVisible, 'true');
  assert.equal(document.body.dataset.frameStyle, 'native');
  assert.equal(document.body.dataset.focusFollowsMouse, 'false');
  assert.equal(getWindowSettings().opacity, 100);
});
test('window section teardown restores last applied snapshot even when a reload is unavailable', async () => {
  const source = await readFile(new URL('../components/settings/sections/WindowSection.svelte', import.meta.url), 'utf8');
  const start = source.indexOf('  onDestroy(() => {');
  const end = source.indexOf('\n  });', start) + 6;
  const snapshot = { opacity: 85, tabsLocation: 'bottom', sidebarVisible: false };
  const js = ts.transpileModule(source.slice(start, end), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  let destroy;
  new Function('onDestroy', 'applyWindowSettings', 'appliedWindow', 'settingsCoord', js)(fn => { destroy = fn; }, applyWindowSettings, snapshot, { unregisterSaver() {} });
  applyWindowSettings({ opacity: 40, tabsLocation: 'left', sidebarVisible: true });
  destroy();
  assert.equal(properties.get('--ui-opacity'), '0.85');
  assert.equal(document.body.dataset.tabsLocation, 'bottom');
  assert.equal(document.body.dataset.sidebarVisible, 'false');
});

test('partial persisted settings reset omitted fields while incremental callers retain their semantics', () => {
  applyWindowSettings({ opacity: 50, tabsLocation: 'left' });
  applyWindowSettings({ sidebarVisible: false });
  assert.equal(getWindowSettings().opacity, 50);
  applyWindowSettings({ sidebarVisible: false }, { replace: true });
  assert.equal(getWindowSettings().opacity, 100);
  assert.equal(getWindowSettings().sidebarVisible, false);
  assert.equal(document.body.dataset.tabsLocation, 'top');
});
