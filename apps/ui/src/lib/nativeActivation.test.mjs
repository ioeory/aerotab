import test from 'node:test';
import assert from 'node:assert/strict';
import { createNativeActivationRouter } from './nativeActivation.ts';
import { createNativeInputQueue } from './nativeInputBridge.ts';

test('two input owners route keys only to their session, independent of latest attachment and disposal', async () => {
  const writes = { a: [], b: [] };
  const queues = Object.fromEntries(['a', 'b'].map(id => [id, createNativeInputQueue({
    sessionId: id, generation: () => `g${id}`,
    write: async bytes => { writes[id].push(new TextDecoder().decode(bytes)); },
    ack: async () => {}, fail: () => assert.fail('unexpected queue failure'),
  })]));
  const send = (id, seq, text) => {
    const event = { sessionId: id, generation: `g${id}`, seq, data: btoa(text) };
    for (const queue of Object.values(queues)) queue.accept(event);
  };
  send('a', 1, 'left'); send('b', 1, 'right'); send('a', 2, 'again');
  await queues.a.drainThrough(2); await queues.b.drainThrough(1);
  assert.deepEqual(writes, { a: ['left', 'again'], b: ['right'] });
  queues.b.dispose(); send('b', 2, 'retired'); send('a', 3, 'survives');
  await queues.a.drainThrough(3);
  assert.deepEqual(writes, { a: ['left', 'again', 'survives'], b: ['right'] });
  queues.a.dispose();
});

test('two native panes reactivate independently; stale, hidden and retired owners cannot take focus', async () => {
  let selected = 'b';
  let osFocus = 'b';
  const live = new Map([['a', 'ga'], ['b', 'gb']]);
  const visible = new Set(['a', 'b']);
  const router = createNativeActivationRouter();
  const activate = (id, seq, generation = live.get(id), state) => router.accept({ sessionId: id, generation, seq }, {
    sessionId: id, generation: () => live.get(id) ?? null,
    eligible: () => visible.has(id),
    state: state ?? (async () => ({ generation: live.get(id), focused: osFocus === id })),
    activate: () => { selected = id; },
  });
  osFocus = 'a'; await activate('a', 1); assert.equal(selected, 'a');
  osFocus = 'b'; await activate('b', 2); assert.equal(selected, 'b');
  osFocus = 'a'; await activate('a', 3); assert.equal(selected, 'a');
  assert.deepEqual([...live.keys()], ['a', 'b']);
  let finish;
  const late = activate('a', 4, 'ga', () => new Promise(r => finish = r));
  osFocus = 'b'; await activate('b', 5); finish({ generation: 'ga', focused: true }); await late;
  assert.equal(selected, 'b');
  visible.delete('a'); osFocus = 'a'; await activate('a', 6); assert.equal(selected, 'b');
  visible.add('a'); live.set('a', 'ga2'); await activate('a', 7, 'ga'); assert.equal(selected, 'b');
  // Reverse to WebView or close B must leave A independently usable.
  live.delete('b'); osFocus = 'a'; await activate('a', 8); assert.equal(selected, 'a');
  await activate('b', 9, 'gb'); assert.equal(selected, 'a');
  // A late native notification after a WebView click must not reclaim selection.
  selected = 'webview'; osFocus = 'webview'; await activate('a', 10); assert.equal(selected, 'webview');
  const interrupted = activate('a', 11, 'ga2', () => new Promise(r => finish = r));
  router.invalidate(); finish({ generation: 'ga2', focused: true }); await interrupted;
  assert.equal(selected, 'webview', 'DOM pane activation invalidates already queried native focus');
  const closing = activate('a', 12, 'ga2', () => new Promise(r => finish = r));
  live.delete('a'); finish({ generation: 'ga2', focused: true }); await closing;
  assert.equal(selected, 'webview');
});
