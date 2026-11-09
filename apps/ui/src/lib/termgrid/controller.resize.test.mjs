import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';

const server = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true, include: [] }, server: { middlewareMode: true } });
const { NativeTerminalController } = await server.ssrLoadModule('/src/lib/termgrid/controller.ts');
const { Canvas2DRenderer } = await server.ssrLoadModule('/src/lib/termgrid/canvas2d.ts');
const { WebGLRenderer } = await server.ssrLoadModule('/src/lib/termgrid/webgl.ts');
await server.close();

function fixture() {
  globalThis.window = { devicePixelRatio: 1 };
  const frames = [];
  globalThis.requestAnimationFrame = fn => (frames.push(fn), frames.length);
  const host = { clientWidth: 720, clientHeight: 360 };
  let clears = 0;
  const ctx = { setTransform() {}, fillRect() {}, clearRect() {}, fillText() {} };
  const canvas = { parentElement: host, style: {}, getContext: () => ctx };
  for (const name of ['width', 'height']) {
    let value = 0;
    Object.defineProperty(canvas, name, { get: () => value, set: next => { value = next; clears++; } });
  }
  const calls = [];
  const ctl = new NativeTerminalController({ sessionId: 'resize-only', rpc: { call: async (method, params) => calls.push({ method, params }) }, canvas, cursorBlink: false });
  ctl.renderer = new Canvas2DRenderer(canvas);
  const draw = () => { for (const fn of frames.splice(0)) fn(); };
  return { ctl, host, calls, canvas, draw, clears: () => clears };
}

test('same-size auxiliary panel fits never clear the WebView backing store', () => {
  const f = fixture();
  f.ctl.fit();
  f.draw();
  const before = f.clears();
  for (let i = 0; i < 20; i++) f.ctl.fit();
  assert.equal(f.clears(), before, 'unchanged fit must not reset canvas.width/height');
});

test('WebGL also preserves backing store on unchanged resize', () => {
  const f = fixture();
  // Exercise the real resize method without requiring a GPU context in node.
  const renderer = Object.create(WebGLRenderer.prototype);
  renderer.canvas = f.canvas;
  renderer.gl = { viewport() {} };
  renderer.resize(80, 20, f.ctl.metrics, 1);
  const before = f.clears();
  renderer.resize(80, 20, f.ctl.metrics, 1);
  assert.equal(f.clears(), before);
});

test('panel resize keeps the last received grid visible until the resized full frame arrives', async () => {
  const f = fixture();
  f.ctl.fit();
  f.ctl.grid.resize(f.ctl.cols, f.ctl.rows);
  f.ctl.grid.chars[0] = 'X'.codePointAt(0);
  f.draw();
  const before = f.clears();
  f.host.clientWidth -= 180;
  f.ctl.fit();
  f.draw();
  assert.equal(f.ctl.grid.chars[0], 'X'.codePointAt(0), 'fit must not replace received output with an all-space grid');
  assert.equal(f.clears(), before, 'do not erase old output while awaiting backend resize');
  await flush();
  assert.equal(f.calls.filter(c => c.method === 'terminal.resize').length, 2);
});

test('received resized frame changes backing store only at draw and stale frames do not resubmit resize', () => {
  const f = fixture();
  f.ctl.fit();
  f.ctl.grid.resize(80, 20);
  f.ctl.grid.chars[0] = 88;
  f.draw();
  const before = f.clears();
  f.host.clientWidth = 540;
  f.ctl.fit();
  const count = f.calls.length;
  // A full backend frame with one row containing X.
  const frame = (cols, rows) => {
    const b = new ArrayBuffer(56);
    const d = new DataView(b);
    d.setUint8(0, 1); d.setUint16(2, cols, true); d.setUint16(4, rows, true);
    d.setUint16(32, 1, true); d.setUint16(36, 1, true);
    d.setUint16(38, 1, true); d.setUint8(50, 1); d.setUint32(51, 88, true);
    return b;
  };
  f.ctl.onFrame(frame(80, 20));
  f.ctl.fit();
  assert.equal(f.calls.length, count, 'old full frame must not restart an identical resize');
  const requested = f.calls.at(-1).params;
  f.ctl.onFrame(frame(requested.cols, requested.rows));
  assert.equal(f.clears(), before, 'Channel callback must not clear outside RAF');
  f.draw();
  assert.equal(f.ctl.grid.chars[0], 88);
  assert.equal(f.canvas.width, requested.cols * f.ctl.metrics.deviceWidth);
  assert.equal(f.canvas.height, requested.rows * f.ctl.metrics.deviceHeight);
  assert.ok(f.clears() > before);
});

const flush = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };

test('rapid panel fits serialize engine/PTY resize and coalesce obsolete intermediate sizes', async () => {
  const f = fixture();
  const pending = [];
  f.ctl.rpc = { call: (method, params) => {
    f.calls.push({ method, params });
    return new Promise(resolve => pending.push(resolve));
  } };
  f.ctl.fit();
  const first = f.calls[0].params;
  f.host.clientWidth = 540;
  f.ctl.fit();
  f.host.clientWidth = 450;
  f.ctl.fit();
  assert.equal(f.calls.length, 1, 'SSH resize must finish before another engine resize can start');
  pending.shift()();
  await flush();
  assert.equal(f.calls.length, 2);
  assert.equal(f.calls[1].params.cols, Math.floor(450 / f.ctl.metrics.width));
  assert.notEqual(f.calls[1].params.cols, first.cols);
  pending.shift()();
  await flush();
  for (let i = 0; i < 20; i++) { f.ctl.fit(); f.draw(); }
  assert.equal(f.calls.length, 2, 'settled host must not form a resize feedback loop');
});

test('returning to in-flight geometry drops the obsolete queued resize', async () => {
  const f = fixture();
  let finish;
  f.ctl.rpc = { call: (method, params) => {
    f.calls.push({ method, params });
    return new Promise(resolve => { finish = resolve; });
  } };
  f.ctl.fit();
  f.host.clientWidth = 450; f.ctl.fit();
  f.host.clientWidth = 720; f.ctl.fit();
  finish(); await flush();
  assert.equal(f.calls.length, 1, 'A -> B -> A must not apply stale B after A has completed');
});

test('failed resize is reported and identical geometry is retryable without an automatic loop', async () => {
  const f = fixture();
  const warnings = [];
  const warn = console.warn;
  console.warn = (...args) => warnings.push(args);
  let attempt = 0;
  f.ctl.rpc = { call: async (method, params) => {
    f.calls.push({ method, params });
    if (++attempt === 1) throw new Error('resize transport unavailable');
  } };
  try {
    f.ctl.fit(); await flush();
    assert.equal(f.calls.length, 1, 'failure must not start a retry spin');
    assert.equal(warnings.length, 1, 'failure must not be swallowed');
    f.ctl.fit(); await flush();
    assert.equal(f.calls.length, 2, 'next real fit retries the failed size');
  } finally { console.warn = warn; }
});

test('disposed or native-owned view never dispatches a queued WebView resize', async () => {
  for (const retired of ['disposed', 'native']) {
    const f = fixture(); let finish;
    f.ctl.rpc = { call: (method, params) => {
      f.calls.push({ method, params });
      return new Promise(resolve => { finish = resolve; });
    } };
    f.ctl.fit();
    f.host.clientWidth = 450; f.ctl.fit();
    if (retired === 'disposed') f.ctl.disposed = true;
    else f.ctl.opts.nativeSurface = true;
    finish(); await flush();
    assert.equal(f.calls.length, 1);
  }
});

test('native handoff waits for pending WebView engine/PTY resize completion', async () => {
  const f = fixture(); let finish;
  f.ctl.rpc = { call: () => new Promise(resolve => { finish = resolve; }) };
  f.ctl.fit();
  let drained = false;
  const handoff = f.ctl.quiesceWebviewInput().then(() => { drained = true; });
  await flush(); assert.equal(drained, false);
  finish(); await handoff; assert.equal(drained, true);
});

test('return from native ownership reapplies geometry even when it matches old WebView size', async () => {
  const f = fixture();
  f.ctl.fit(); await flush();
  f.ctl.opts.nativeSurface = true;
  f.ctl.stagingWebview = true;
  f.ctl.observeResize = () => {};
  f.ctl.commitWebviewSurface(); await flush();
  assert.equal(f.calls.filter(c => c.method === 'terminal.resize').length, 2,
    'native owner may have changed engine size since the old WebView acknowledgement');
});

test('stationary fractional-DPR host across 1920 has no canvas-to-fit feedback or width cap', async () => {
  for (const width of [1919, 1920, 1921, 2560]) {
    const f = fixture();
    window.devicePixelRatio = 1.25;
    f.ctl.spec.dpr = 1.25;
    f.ctl.metrics = { width: 7.2, height: 14.4, deviceWidth: 9, deviceHeight: 18, baseline: 11 };
    f.host.clientWidth = width;
    f.ctl.fit(); await flush();
    const requested = f.calls[0].params;
    f.ctl.grid.resize(requested.cols, requested.rows);
    for (let i = 0; i < 20; i++) { f.ctl.fit(); f.draw(); }
    assert.equal(f.calls.length, 1, `stable ${width}px host must not trigger repeated PTY resize`);
    assert.equal(requested.cols, Math.floor(width / 7.2));
    assert.equal(f.canvas.width, requested.cols * 9);
  }
});

test('zero-size hidden WebView host never submits a 2x1 resize', () => {
  const f = fixture();
  f.ctl.fit();
  const before = f.calls.length;
  f.host.clientWidth = 0;
  f.host.clientHeight = 0;
  f.ctl.fit();
  assert.equal(f.calls.length, before);
});
