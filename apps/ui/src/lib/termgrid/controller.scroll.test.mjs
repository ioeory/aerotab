import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { createServer } from 'vite';
const server = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true, include: [] }, server: { middlewareMode: true } });
const { NativeTerminalController } = await server.ssrLoadModule('/src/lib/termgrid/controller.ts');
await server.close();

test('DOM wheel up scrolls toward history and down toward live screen', async () => {
  const calls = [];
  const ctl = new NativeTerminalController({ sessionId: 's', rpc: { call: async (...args) => calls.push(args) }, canvas: {}, cursorBlink: false });
  await ctl.onWheel({ deltaY: -120, preventDefault() {} });
  await ctl.onWheel({ deltaY: 120, preventDefault() {} });
  assert.deepEqual(calls, [
    ['terminal.scroll', { id: 's', cmd: { delta: 3 } }],
    ['terminal.scroll', { id: 's', cmd: { delta: -3 } }],
  ]);
});

test('one paste event handled in window capture and target writes only once', async () => {
  const writes = [];
  const ctl = new NativeTerminalController({ sessionId: 's', rpc: { call: async () => {} }, canvas: { focus() {} }, cursorBlink: false, writeInput: async text => writes.push(text) });
  // v0.3.5+ paste/key handlers require the controller to own focus; the
  // window-capture + target delivery path is what this test exercises.
  globalThis.requestAnimationFrame = globalThis.requestAnimationFrame ?? (() => 0);
  ctl.focus();
  const event = { defaultPrevented: false, clipboardData: { getData: () => 'one paste' }, preventDefault() { this.defaultPrevented = true; }, stopPropagation() {} };
  // The window capture listener and canvas/IME listener receive the same event.
  ctl.shouldHandleGlobalInput = () => true;
  ctl.boundWinPaste(event);
  ctl.boundPaste(event);
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(writes, ['one paste']);
  const nextEvent = { ...event, defaultPrevented: false };
  ctl.boundWinPaste(nextEvent);
  ctl.boundPaste(nextEvent);
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(writes, ['one paste', 'one paste'], 'distinct paste events must both work');
});

test('switching away during IME composition cannot leave the current terminal permanently rejecting keys', async () => {
  const writes = [], windowListeners = new Map(), inputListeners = new Map();
  // Node has no DOM; supply the minimal surface the controller touches:
  // fontMetrics (constructor), focus() (activeElement), bindInput (listeners).
  const fakeCtx = { font: '', measureText: () => ({ width: 8 }), };
  const fakeCanvas = { width: 64, height: 64, getContext: () => fakeCtx };
  const savedWindow = globalThis.window, savedDocument = globalThis.document;
  globalThis.window = { addEventListener: (name, fn) => windowListeners.set(name, fn), removeEventListener: (name) => windowListeners.delete(name), clearTimeout() {} };
  globalThis.document = { activeElement: null, createElement: () => fakeCanvas };
  globalThis.cancelAnimationFrame = () => {};
  try {
    const input = { value: 'unfinished', focus() {}, addEventListener: (name, fn) => inputListeners.set(name, fn), removeEventListener: name => inputListeners.delete(name) };
    const canvas = { addEventListener() {}, removeEventListener() {}, focus() {} };
    const ctl = new NativeTerminalController({ sessionId: 's', rpc: { call: async () => {} }, canvas, input, cursorBlink: false, writeInput: async text => writes.push(text) });
    ctl.bindInput();
    // The pane is active when composition starts; blur only interrupts IME.
    ctl.focus();
    inputListeners.get('compositionstart')();
    assert.equal(ctl.composing, true);
    // WebView2 may omit compositionend when Alt+Tab deactivates the window.
    windowListeners.get('blur')?.();
    // v0.3.5+ onKey guards on `focused` (stale-IME cross-pane protection).
    // Deactivation cleared it; reactivation must restore ordinary keys.
    ctl.focus();
    await ctl.onKey({ key: 'a', defaultPrevented: false, preventDefault() {}, stopPropagation() {} });
    assert.deepEqual(writes, ['a'], 'returning from another app must not suppress ordinary keys');
    assert.equal(input.value, '', 'interrupted preedit must not be replayed later');
    await ctl.dispose();
    assert.equal(windowListeners.has('blur'), false);
    assert.equal(inputListeners.has('blur'), false);
  } finally {
    globalThis.window = savedWindow;
    globalThis.document = savedDocument;
  }
});

test('terminal input blur also cancels stale composition without requiring a tab switch', async () => {
  const writes = [], listeners = new Map();
  const fakeCtx = { font: '', measureText: () => ({ width: 8 }) };
  const fakeCanvas = { width: 64, height: 64, getContext: () => fakeCtx };
  const savedWindow = globalThis.window, savedDocument = globalThis.document;
  globalThis.window = { addEventListener() {}, removeEventListener() {}, clearTimeout() {} };
  globalThis.document = { activeElement: null, createElement: () => fakeCanvas };
  try {
    const ctl = new NativeTerminalController({ sessionId: 's', rpc: { call: async () => {} }, canvas: { addEventListener() {}, removeEventListener() {}, focus() {} }, input: { value: 'preedit', focus() {}, addEventListener: (name, fn) => listeners.set(name, fn), removeEventListener() {} }, cursorBlink: false, writeInput: async text => writes.push(text) });
    ctl.bindInput(); ctl.focus(); listeners.get('compositionstart')(); listeners.get('blur')?.();
    // blur cleared `focused`; a same-tab refocus must accept keys again.
    ctl.focus();
    await ctl.onKey({ key: 'b', defaultPrevented: false, preventDefault() {}, stopPropagation() {} });
    assert.deepEqual(writes, ['b']);
    await ctl.dispose();
  } finally {
    globalThis.window = savedWindow;
    globalThis.document = savedDocument;
  }
});

const pane = readFileSync(new URL('../../components/TerminalPane.svelte', import.meta.url), 'utf8');
const source = pane.slice(pane.indexOf('  async function sendSessionInputAsync('), pane.indexOf('  function sendSessionInput('));
const js = ts.transpileModule(source + '\nreturn sendSessionInputAsync;', { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText;
for (const useNative of [true, false]) {
  test(`Enter returns to live input before writing (native=${useNative})`, async () => {
    const calls = [];
    const fn = new Function('useNative', 'session', 'rpc', 'sendSessionBytes', 'encoder', 'terminalInputBytes', js)(
      useNative, { id: 's' }, { call: async (...args) => calls.push(args) },
      async bytes => calls.push(['write', [...bytes]]), new TextEncoder(), async x => x,
    );
    await fn('\r');
    assert.deepEqual(calls, [...(useNative ? [['terminal.scroll', { id: 's', cmd: 'bottom' }]] : []), ['write', [13]]]);
    calls.length = 0;
    await fn('\x1b[<65;1;1M');
    assert.equal(calls.length, 1, 'mouse reports must not jump to bottom');
  });
}
