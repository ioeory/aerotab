import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const src = readFileSync(new URL('./splitter.ts', import.meta.url), 'utf8');
const m = await import(`data:text/javascript;base64,${Buffer.from(ts.transpileModule(src, { compilerOptions: { target: 99, module: 99 } }).outputText).toString('base64')}`);

function fixture({ captureFails = false } = {}) {
  const listeners = new Map(), targetListeners = new Map();
  const values = [], ends = [], captures = [], releases = [];
  let captured = false;
  globalThis.window = {
    addEventListener: (name, fn) => listeners.set(name, fn),
    removeEventListener: name => listeners.delete(name),
  };
  const target = {
    addEventListener: (name, fn) => targetListeners.set(name, fn),
    removeEventListener: name => targetListeners.delete(name),
    setPointerCapture(id) { captures.push(id); if (captureFails) throw Error('capture unavailable'); captured = true; },
    hasPointerCapture: () => captured,
    releasePointerCapture(id) { releases.push(id); captured = false; targetListeners.get('lostpointercapture')?.({ pointerId: id }); },
  };
  const event = { pointerId: 1, pointerType: 'mouse', button: 0, buttons: 1, clientX: 10, currentTarget: target, preventDefault() {}, stopPropagation() {} };
  const begin = (overrides = {}) => m.pointerResize({ ...event, ...overrides }, { axis: 'x', value: 200, min: 100, max: 400, preview: v => values.push(v), end: v => ends.push(v) });
  const send = (name, overrides = {}) => listeners.get(name)?.({ ...event, ...overrides });
  // A native child HWND consumes its own mouse events unless WebView captured the pointer.
  const overNative = (name, overrides = {}) => { if (captured) send(name, overrides); };
  return { event, listeners, targetListeners, values, ends, captures, releases, begin, send, overNative,
    loseCapture() { captured = false; targetListeners.get('lostpointercapture')?.({ pointerId: 1 }); } };
}

for (const reason of ['pointercancel', 'blur', 'Escape', 'dispose', 'pointerup']) {
  test(`real pointer transaction cleans listeners and ${reason === 'pointerup' ? 'commits' : 'cancels'} on ${reason}`, () => {
    const f = fixture();
    try {
      const cancel = f.begin();
      f.send('pointermove', { clientX: 30 });
      if (reason === 'dispose') cancel();
      else if (reason === 'Escape') f.send('keydown', { key: 'Escape' });
      else f.send(reason);
      assert.equal(f.listeners.size, 0);
      assert.equal(f.targetListeners.size, 0);
      assert.deepEqual(f.ends, reason === 'pointerup' ? [220] : []);
      assert.equal(f.values.at(-1), reason === 'pointerup' ? 220 : 200);
    } finally { delete globalThis.window; }
  });
}

test('native-gpu boundary: held left-button drag captures before moving right and releases on up', () => {
  const f = fixture();
  try {
    f.begin();
    assert.deepEqual(f.captures, [1]);
    f.overNative('pointermove', { clientX: 80 });
    assert.deepEqual(f.values, [270]);
    f.overNative('pointerup', { buttons: 0, clientX: 80 });
    assert.deepEqual(f.ends, [270]);
    assert.deepEqual(f.releases, [1]);
    f.send('pointermove', { clientX: 100, buttons: 0 });
    assert.deepEqual(f.values, [270]);
  } finally { delete globalThis.window; }
});

test('lost capture cancels once, restores width and cannot leave click-to-move resizing active', () => {
  const f = fixture();
  try {
    const cancel = f.begin();
    f.send('pointermove', { clientX: 40 });
    f.loseCapture();
    f.send('pointermove', { clientX: 80, buttons: 0 });
    cancel();
    assert.deepEqual(f.values, [230, 200]);
    assert.deepEqual(f.ends, []);
    assert.equal(f.listeners.size + f.targetListeners.size, 0);
  } finally { delete globalThis.window; }
});

test('missed mouseup ends on buttons=0 without applying unpressed pointer movement', () => {
  const f = fixture();
  try {
    f.begin();
    f.send('pointermove', { clientX: 30 });
    f.send('pointermove', { clientX: 100, buttons: 0 });
    f.send('pointermove', { clientX: 200, buttons: 0 });
    assert.deepEqual(f.values, [220]);
    assert.deepEqual(f.ends, [220]);
    assert.equal(f.listeners.size + f.targetListeners.size, 0);
  } finally { delete globalThis.window; }
});

test('secondary button cannot start resizing and another pointer cannot cancel the active drag', () => {
  const f = fixture();
  try {
    f.begin({ button: 2, buttons: 2 });
    assert.equal(f.listeners.size, 0);
    assert.deepEqual(f.captures, []);
    f.begin();
    f.send('pointercancel', { pointerId: 2 });
    f.send('pointermove', { clientX: 30 });
    f.send('pointerup', { buttons: 0 });
    assert.deepEqual(f.ends, [220]);
  } finally { delete globalThis.window; }
});

test('panelResize compatibility wrapper also captures native-boundary drags and commits once', async () => {
  const wrapper = readFileSync(new URL('./panelResize.ts', import.meta.url), 'utf8').replace("import { pointerResize } from './splitter';", '');
  const js = ts.transpileModule(wrapper, { compilerOptions: { target: 99, module: 99 } }).outputText.replaceAll('export ', '');
  const start = new Function('pointerResize', `${js}; return startHorizontalPanelResize;`)(m.pointerResize);
  const f = fixture();
  try {
    start(f.event, { startWidthPx: 200, minPx: 100, maxPx: 400, growWhenDraggingRight: true, onWidth: v => f.values.push(v), onEnd: () => f.ends.push('end') });
    f.overNative('pointermove', { clientX: 60 });
    f.overNative('pointerup', { buttons: 0 });
    assert.deepEqual(f.values, [250]);
    assert.deepEqual(f.ends, ['end']);
    assert.deepEqual(f.releases, [1]);
  } finally { delete globalThis.window; }
});

test('touch drag does not mistake buttons=0 for a released mouse', () => {
  const f = fixture();
  try {
    f.begin({ pointerType: 'touch' });
    f.send('pointermove', { pointerType: 'touch', buttons: 0, clientX: 40 });
    f.send('pointerup', { pointerType: 'touch', buttons: 0 });
    assert.deepEqual(f.ends, [230]);
  } finally { delete globalThis.window; }
});

test('capture failure aborts without installing a dangling global drag', () => {
  const f = fixture({ captureFails: true });
  try {
    assert.doesNotThrow(() => f.begin());
    f.send('pointermove', { clientX: 100 });
    assert.deepEqual(f.values, []);
    assert.deepEqual(f.ends, []);
    assert.equal(f.listeners.size + f.targetListeners.size, 0);
  } finally { delete globalThis.window; }
});
