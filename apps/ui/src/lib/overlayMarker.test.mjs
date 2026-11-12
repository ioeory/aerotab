import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { syncAerotabOverlayMarker, isAerotabOverlayOpen } from './overlayMarker.ts';

test('overlay marker sets and clears documentElement.dataset.aerotabOverlayOpen', () => {
  const dataset = {};
  const prev = globalThis.document;
  globalThis.document = { documentElement: { dataset } };
  try {
    assert.equal(isAerotabOverlayOpen(), false);
    syncAerotabOverlayMarker(true);
    assert.equal(dataset.aerotabOverlayOpen, '');
    assert.equal(isAerotabOverlayOpen(), true);
    syncAerotabOverlayMarker(false);
    assert.equal('aerotabOverlayOpen' in dataset, false);
    assert.equal(isAerotabOverlayOpen(), false);
  } finally {
    if (prev === undefined) delete globalThis.document;
    else globalThis.document = prev;
  }
});

test('App syncs aerotabOverlayOpen whenever native view occlusion is evaluated', () => {
  const app = readFileSync(new URL('../App.svelte', import.meta.url), 'utf8');
  assert.match(app, /syncAerotabOverlayMarker/);
  assert.match(app, /from ['"]\.\/lib\/overlayMarker['"]/);
  // Closing a modal must clear the marker in the same tick, not only via MutationObserver.
  assert.match(app, /delete document\.body\.dataset\.modalOverlay;[\s\S]*?syncAerotabOverlayMarker\(false\)/);
});

test('NativeTerminalView canFocusSurface rejects aerotabOverlayOpen like aerotabResizing', () => {
  const view = readFileSync(new URL('../components/NativeTerminalView.svelte', import.meta.url), 'utf8');
  const body = view.match(/function canTakeFocus\(force = false\) \{[\s\S]*?\n  \}/)?.[0] ?? '';
  assert.match(body, /aerotabResizing/);
  assert.match(body, /aerotabOverlayOpen/);
  assert.match(view, /function canFocusSurface\(\) \{\s*return canTakeFocus\(false\);/);
});
