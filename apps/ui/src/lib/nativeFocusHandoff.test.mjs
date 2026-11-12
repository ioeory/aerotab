import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  expectNativeFocusHandoff,
  completeNativeFocusHandoff,
  isNativeFocusHandoffPending,
  nativeFocusHandoffPendingCount,
  waitForNativeOsFocusHandoff,
  NATIVE_OS_FOCUS_HANDOFF_EVENT,
} from './nativeFocusHandoff.ts';

function withDocument(html, run) {
  const prev = globalThis.document;
  const prevWindow = globalThis.window;
  const listeners = new Map();
  const surfaces = [];
  const re = /data-terminal-view-backend="native-surface"/g;
  let m;
  while ((m = re.exec(html))) surfaces.push({});
  const document = {
    querySelector: (sel) => {
      if (sel === '[data-terminal-view-backend="native-surface"]') {
        return surfaces[0] ?? null;
      }
      return null;
    },
    querySelectorAll: (sel) => {
      if (sel === '[data-terminal-view-backend="native-surface"]') return surfaces;
      return [];
    },
    addEventListener: (type, fn) => {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type).add(fn);
    },
    removeEventListener: (type, fn) => listeners.get(type)?.delete(fn),
    dispatchEvent: (event) => {
      for (const fn of listeners.get(event.type) ?? []) fn(event);
      return true;
    },
  };
  globalThis.document = document;
  globalThis.window = {
    setTimeout: (fn, ms) => setTimeout(fn, ms),
    clearTimeout: (id) => clearTimeout(id),
  };
  const restore = () => {
    if (prev === undefined) delete globalThis.document; else globalThis.document = prev;
    if (prevWindow === undefined) delete globalThis.window; else globalThis.window = prevWindow;
  };
  try {
    const result = run(document);
    return result && typeof result.then === 'function'
      ? result.finally(restore)
      : (restore(), result);
  } catch (error) {
    restore();
    throw error;
  }
}

test('without a native surface, expect completes immediately', () => {
  withDocument('<div data-terminal-view-backend="webview"></div>', () => {
    expectNativeFocusHandoff();
    assert.equal(isNativeFocusHandoffPending(), false);
  });
});

test('with a native surface, wait resolves only after complete', async () => {
  await withDocument('<div data-terminal-view-backend="native-surface"></div>', async () => {
    expectNativeFocusHandoff();
    assert.equal(isNativeFocusHandoffPending(), true);
    let done = false;
    const waiting = waitForNativeOsFocusHandoff(500).then(() => { done = true; });
    await Promise.resolve();
    assert.equal(done, false, 'must not resolve before SetFocus(parent) handoff');
    completeNativeFocusHandoff();
    await waiting;
    assert.equal(done, true);
    assert.equal(isNativeFocusHandoffPending(), false);
  });
});

test('multiple native surfaces must all report hide before handoff completes', async () => {
  await withDocument(
    '<div data-terminal-view-backend="native-surface"></div><div data-terminal-view-backend="native-surface"></div>',
    async () => {
      expectNativeFocusHandoff();
      assert.equal(nativeFocusHandoffPendingCount(), 2);
      let done = false;
      const waiting = waitForNativeOsFocusHandoff(500).then(() => { done = true; });
      completeNativeFocusHandoff();
      await Promise.resolve();
      assert.equal(done, false, 'first surface hide alone must not release modal focus');
      assert.equal(isNativeFocusHandoffPending(), true);
      assert.equal(nativeFocusHandoffPendingCount(), 1);
      completeNativeFocusHandoff();
      await waiting;
      assert.equal(done, true);
      assert.equal(isNativeFocusHandoffPending(), false);
    },
  );
});

test('complete is idempotent and dispatches the handoff event once per expect', () => {
  withDocument('<div data-terminal-view-backend="native-surface"></div>', (document) => {
    let events = 0;
    document.addEventListener(NATIVE_OS_FOCUS_HANDOFF_EVENT, () => { events += 1; });
    expectNativeFocusHandoff();
    completeNativeFocusHandoff();
    completeNativeFocusHandoff();
    assert.equal(events, 1);
  });
});

test('App expects handoff when overlay opens; modal focus forces desktop webview focus', () => {
  const app = readFileSync(new URL('../App.svelte', import.meta.url), 'utf8');
  const view = readFileSync(new URL('../components/NativeTerminalView.svelte', import.meta.url), 'utf8');
  const modal = readFileSync(new URL('./modalFocus.ts', import.meta.url), 'utf8');
  assert.match(app, /expectNativeFocusHandoff/);
  assert.match(view, /completeNativeFocusHandoff/);
  assert.match(modal, /waitForNativeOsFocusHandoff/);
  assert.match(modal, /focusWebviewDesktop/);
});
