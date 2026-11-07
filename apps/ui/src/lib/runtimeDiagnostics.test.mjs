import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const url = new URL('../../public/runtime-diagnostics.js', import.meta.url);
function harness(search = '?runtimeDiagnostics=1') {
  const listeners = new Map(); let beat;
  const saved = [];
  const body = { dataset: {}, querySelectorAll: () => [] };
  const document = { body, hidden: false, querySelectorAll: () => [], elementFromPoint: () => null, createElement: () => ({ click() {}, remove() {} }) };
  const context = { document, location: { search }, performance: { now: () => 0 }, Date, JSON, String, Number, Object, Array, RegExp,
    setInterval: fn => { beat = fn; return 1; }, clearInterval() {}, setTimeout: fn => fn(),
    localStorage: { setItem: (...args) => saved.push(args) },
    addEventListener: (type, fn) => {
      const previous = listeners.get(type);
      listeners.set(type, previous ? event => { previous(event); fn(event); } : fn);
    },
  };
  context.window = context;
  vm.runInNewContext(readFileSync(url, 'utf8'), context);
  return { context, listeners, saved, beat: () => beat() };
}
test('default startup installs no probe or listeners', () => {
  const h = harness('');
  assert.equal(h.context.__aerotabRuntimeDiagnostics, undefined);
  assert.equal(h.listeners.size, 0);
  assert.equal(h.saved.length, 0);
});
test('early diagnostic survives missing Svelte mount and never stores content or input keys', () => {
  const h = harness();
  h.listeners.get('error')({ message: 'password=SECRET note=PRIVATE Cannot access before initialization', filename: 'https://host/assets/index-123.js?token=SECRET', lineno: 3, colno: 5 });
  h.listeners.get('unhandledrejection')({ reason: new Error('secret=SECRET effect_update_depth_exceeded') });
  for (let i = 0; i < 200; i++) h.listeners.get('click')({ target: { tagName: 'BUTTON', textContent: 'PRIVATE' }, clientX: 2, clientY: 3 });
  h.beat();
  const report = h.context.__aerotabRuntimeDiagnostics.snapshot();
  const text = JSON.stringify(report);
  assert.ok(!/SECRET|PRIVATE|password|token|https:/.test(text));
  assert.ok(report.events.length <= 64);
  assert.equal(report.counts.click, 200);
  assert.equal(report.phase, 'boot');
  assert.ok(h.saved.length > 0, 'opt-in heartbeat persists a bounded local-only snapshot');
  assert.ok(h.saved.every(([key, value]) => key === 'aerotab.runtimeDiagnostics.v1' && !/SECRET|PRIVATE|password|token|https:/.test(value)));
});
test('diagnostic phase and heartbeat are independent of Svelte state', () => {
  const h = harness();
  h.context.__aerotabRuntimeDiagnostics.mark('mounted'); h.beat();
  assert.equal(h.context.__aerotabRuntimeDiagnostics.snapshot().phase, 'mounted');
  assert.equal(h.context.__aerotabRuntimeDiagnostics.snapshot().beats, 1);
  h.context.__aerotabRuntimeDiagnostics.mark('SECRET arbitrary content');
  assert.equal(h.context.__aerotabRuntimeDiagnostics.snapshot().phase, 'mounted');
});
