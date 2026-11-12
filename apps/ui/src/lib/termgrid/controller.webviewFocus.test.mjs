import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';
import { compileModule } from 'svelte/compiler';
import { createServer } from 'vite';

// Run with --conditions=browser: these are real client effects and tick(), not
// a synchronous rewrite of Svelte's scheduler or source-pattern assertions.
const server = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true, include: [] }, server: { middlewareMode: true } });
const { NativeTerminalController } = await server.ssrLoadModule('/src/lib/termgrid/controller.ts');
await server.close();
const view = await readFile(new URL('../../components/NativeTerminalView.svelte', import.meta.url), 'utf8');
const pane = await readFile(new URL('../../components/TerminalPane.svelte', import.meta.url), 'utf8');
const grid = await readFile(new URL('../../components/PaneGrid.svelte', import.meta.url), 'utf8');
const fit = await readFile(new URL('../terminalFit.ts', import.meta.url), 'utf8');
const section = (source, start, end) => source.slice(source.indexOf(start), source.indexOf(end, source.indexOf(start)));
const client = new URL('../../../node_modules/svelte/src/index-client.js', import.meta.url).href;
const internal = import.meta.resolve('svelte/internal/client');
let serial = 0;

class ElementTarget extends EventTarget {
  value = '';
  focus() {
    const previous = document.activeElement;
    if (previous === this) return;
    document.activeElement = this;
    previous?.dispatchEvent(new Event('blur'));
    this.dispatchEvent(new Event('focus'));
  }
  blur() {
    if (document.activeElement !== this) return;
    document.activeElement = document.body;
    this.dispatchEvent(new Event('blur'));
  }
  getBoundingClientRect() { return { left: 0, top: 0 }; }
  setPointerCapture() {}
  removeEventListener(...args) { super.removeEventListener(...args); }
}
class Textarea extends ElementTarget {}

async function fixture() {
  const frames = [], timers = [], writes = [], selections = [];
  globalThis.Element = ElementTarget;
  globalThis.HTMLElement = ElementTarget;
  globalThis.HTMLInputElement = class extends ElementTarget {};
  globalThis.HTMLTextAreaElement = Textarea;
  globalThis.document = {
    activeElement: null, body: new ElementTarget(), documentElement: { dataset: {} }, hidden: false,
    createElement: () => ({ getContext: () => null }),
  };
  document.activeElement = document.body;
  globalThis.window = Object.assign(new EventTarget(), {
    devicePixelRatio: 1, setTimeout: fn => { timers.push(fn); return timers.length; },
    clearTimeout() {},
  });
  globalThis.requestAnimationFrame = fn => { frames.push(fn); return frames.length; };
  globalThis.cancelAnimationFrame = () => {};
  const canvas = new ElementTarget(), input = new Textarea(), shell = new ElementTarget();
  const ctl = new NativeTerminalController({
    sessionId: 'target', canvas, input, cursorBlink: false,
    rpc: { call: async (method, params) => { selections.push([method, params]); } },
    writeInput: async data => { writes.push(data); },
  });
  // Only rendering/transport boundaries are replaced. Focus, blur, input
  // listeners, selection and key/IME encoding execute the production controller.
  ctl.schedulePaint = () => {};
  ctl.bindInput();
  globalThis.__webviewFocusFixture = { ctl, frames };
  const source = `
import { tick, untrack, flushSync } from ${JSON.stringify(client)};
import { effect_root } from ${JSON.stringify(internal)};
const controller = globalThis.__webviewFocusFixture.ctl;
let selected = $state(false), active = $derived(selected), visible = $state(true);
let occluded = $state(false), globalOccluded = false, disposed = false;
let surfaceError = '', ackRecovering = false, rollbackPending = false;
let backend = 'webview', committedWebview = false, committedNative = false;
let hostAdapter = null, surfaceReady = false, requestSurfaceFocus = null;
const fail = error => { throw error; };
${section(view, '  function canTakeFocus(', '  /** Commit this request')}
${section(view, '  export function focus(', '  /** Await OS view detach')}
${section(fit, 'const FIT_DELAY_MS', '/** Refit all listed')}
const nativeView = { focus }, useNative = true, searchOpen = false, term = null, host = null;
const tabs = { clearActivity() {}, focusPane() { selected = true; } };
const tab = { id: 'tab' }, session = { id: 'target' };
const nativeActivationRouter = { invalidate() {} };
const dispatchFocusPane = () => { void focus({ force: true }); };
const safeFit = () => {}, focusTerminalAuto = () => {};
${section(grid, '  function focusPane(', '  function closePane(')}
${section(pane, '  let wasActive = false;', '  async function teardownXtermInstance()').replace('  $effect(() => {', '  const stopActivation = effect_root(() => { $effect(() => {').replace('\n  });', '\n  }); });')}
const stopBlur = effect_root(() => {
${section(view, '  $effect(() => {\n    if (!active || !visible', '  $effect(() => {\n    void theme;')}
});
flushSync();
export { tick, flushSync };
export const activate = () => focusPane('target');
export const setActive = value => { selected = value; };
export const setVisible = value => { visible = value; };
export const setOccluded = value => { occluded = value; };
export const explicitFocus = focus;
export const stop = () => { stopActivation(); stopBlur(); };
`;
  const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
  const compiled = compileModule(js, { filename: 'webview-focus.svelte.js', generate: 'client' }).js.code
    .replaceAll("'svelte/internal/client'", JSON.stringify(internal));
  const runtime = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}#${++serial}`);
  async function settle() {
    await runtime.tick();
    for (let i = 0; i < 5 && (frames.length || timers.length); i++) {
      frames.splice(0).forEach(fn => fn());
      timers.splice(0).forEach(fn => fn());
      await runtime.tick();
    }
  }
  function pointer(target, { flushBeforeDefault = false, button = 0 } = {}) {
    const ev = new Event('pointerdown', { cancelable: true });
    Object.assign(ev, { button, pointerId: 1, clientX: 10, clientY: 10, detail: 1 });
    target.dispatchEvent(ev);
    // Chromium default action follows JS listeners and may precede Svelte's
    // queued prop flush. Inactive terminal CSS hit-tests the pane shell instead.
    runtime.activate();
    if (flushBeforeDefault) runtime.flushSync();
    if (!ev.defaultPrevented) target.focus();
    return ev;
  }
  async function type() {
    const key = new Event('keydown', { cancelable: true });
    Object.assign(key, { key: 'a' });
    document.activeElement.dispatchEvent(key);
    if (document.activeElement === input) {
      input.dispatchEvent(new Event('compositionstart'));
      const end = new Event('compositionend'); Object.assign(end, { data: '中文' }); input.dispatchEvent(end);
    }
    await new Promise(resolve => setImmediate(resolve));
  }
  return { runtime, ctl, canvas, input, shell, writes, selections, settle, pointer, type,
    close() { runtime.stop(); ctl.blur(); } };
}

test('active WebView canvas pointer default must not replace the IME focus selected by production controller', async () => {
  const f = await fixture();
  try {
    f.runtime.setActive(true); await f.settle();
    assert.equal(document.activeElement, f.input);
    f.pointer(f.canvas);
    // No animation-frame rescue: an already-active pane may receive keys as
    // soon as pointerdown completes, including before any Svelte effect runs.
    assert.equal(document.activeElement, f.input, 'pointer default focus displaced the IME textarea');
    await f.type(); assert.deepEqual(f.writes, ['a', '中文']);
    assert.ok(f.selections.some(([method]) => method === 'terminal.selectionStart'));
  } finally { f.close(); }
});

for (const flushBeforeDefault of [false, true]) {
  test(`inactive pane shell focus is repaired when Svelte flush runs ${flushBeforeDefault ? 'before' : 'after'} pointer default`, async () => {
    const f = await fixture();
    try {
      f.pointer(f.shell, { flushBeforeDefault }); assert.equal(document.activeElement, f.shell);
      await f.settle(); assert.equal(document.activeElement, f.input);
      await f.type(); assert.deepEqual(f.writes, ['a', '中文']);
    } finally { f.close(); }
  });
}

test('secondary canvas click retains the existing context-menu default behavior', async () => {
  const f = await fixture();
  try {
    f.runtime.setActive(true); await f.settle();
    assert.equal(f.pointer(f.canvas, { button: 2 }).defaultPrevented, false);
    assert.ok(!f.selections.some(([method]) => method === 'terminal.selectionStart'));
  } finally { f.close(); }
});

test('retained WebView tab hide/show regains keyboard and IME without another click', async () => {
  const f = await fixture();
  try {
    f.runtime.setActive(true); await f.settle();
    f.input.dispatchEvent(new Event('compositionstart'));
    f.runtime.setActive(false); f.runtime.setVisible(false); f.runtime.flushSync();
    document.body.focus();
    f.runtime.setVisible(true); f.runtime.setActive(true); await f.settle();
    assert.equal(document.activeElement, f.input);
    await f.type(); assert.deepEqual(f.writes, ['a', '中文']);
  } finally { f.close(); }
});

test('stale activation fit callbacks and queued focus cannot steal focus after another pane activates', async () => {
  const f = await fixture();
  try {
    f.runtime.setActive(true); f.runtime.flushSync();
    const pending = f.runtime.explicitFocus({ force: true });
    f.runtime.setActive(false); f.runtime.flushSync();
    const other = new Textarea(); other.focus();
    await pending; await f.settle();
    assert.equal(document.activeElement, other);
    assert.equal(f.ctl.shouldHandleGlobalInput(other), false);
    assert.deepEqual(f.writes, []);
  } finally { f.close(); }
});

test('occluded WebView does not focus through a modal even with explicit activation', async () => {
  const f = await fixture();
  try {
    f.runtime.setOccluded(true); f.runtime.setActive(true); f.runtime.flushSync();
    const modal = new Textarea(); modal.focus();
    await f.runtime.explicitFocus({ force: true }); await f.settle();
    assert.equal(document.activeElement, modal);
  } finally { f.close(); }
});

// Same-tab pane switch: outline moves (active=false) but WebView2 may leave the old
// IME as document.activeElement. Keys must not keep writing the previous session.
test('deactivating a WebView pane releases IME and ignores keys on the old session', async () => {
  const f = await fixture();
  try {
    f.runtime.setActive(true); await f.settle();
    assert.equal(document.activeElement, f.input);
    f.runtime.setActive(false); f.runtime.flushSync();
    assert.notEqual(document.activeElement, f.input, 'deactivate must blur the IME textarea');
    f.input.focus();
    await f.type();
    assert.deepEqual(f.writes, [], 'old pane must not accept keys after deactivate');
  } finally { f.close(); }
});

// becameActive already has active=true; waiting an extra tick lets mousedown default
// reclaim the pane shell before IME.focus() runs (WebView2 click-to-activate).
test('already-active explicit focus moves IME before any deferred tick', async () => {
  const f = await fixture();
  try {
    f.runtime.setActive(true); f.runtime.flushSync();
    f.shell.focus();
    assert.equal(document.activeElement, f.shell);
    const pending = f.runtime.explicitFocus({ force: true });
    assert.equal(document.activeElement, f.input, 'active pane focus must not wait for tick');
    await pending;
    await f.type();
    assert.deepEqual(f.writes, ['a', '中文']);
  } finally { f.close(); }
});
