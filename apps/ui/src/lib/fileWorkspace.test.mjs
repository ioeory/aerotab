import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
async function load(name) {
  let source = '';
  try { source = readFileSync(new URL(`./${name}.ts`, import.meta.url), 'utf8'); } catch {}
  const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
  return import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
}
const connections = await load('sftpConnections');
const workspace = await load('fileWorkspace');
const queue = await load('transferQueue');
const tick = () => new Promise(resolve => setImmediate(resolve));
test('browser release does not close a session retained by a queued task', async () => {
  const closed = [];
  const manager = new connections.SftpConnections(async () => 'session', async id => closed.push(id));
  const browser = manager.acquire('profile', {});
  assert.equal(await browser.ready, 'session');
  const task = manager.retainSession('session');
  browser.release();
  assert.deepEqual(closed, []);
  task.release(); await tick();
  assert.deepEqual(closed, ['session']);
});
test('late connections are reclaimed after browser closes and cannot be retained', async () => {
  let resolve; const closed = [];
  const manager = new connections.SftpConnections(() => new Promise(r => { resolve = r; }), async id => closed.push(id));
  const lease = manager.acquire('p', {});
  lease.release(); resolve('late');
  await assert.rejects(lease.ready, /released/); await tick();
  assert.deepEqual(closed, ['late']);
  assert.throws(() => manager.retainSession('late'), /unavailable/);
});
test('each side permits independent same-host tabs and serializes no runtime secrets', () => {
  const model = new workspace.FileWorkspace();
  const a = model.add('left', 'p', '/one'); const b = model.add('left', 'p', '/two');
  model.add('right', 'p', '/three');
  model.activate('left', a.id); model.close('left', b.id);
  assert.equal(model.active('left').path, '/one');
  a.sessionId = 'DO_NOT_SAVE'; a.credentials = 'DO_NOT_SAVE';
  const raw = JSON.stringify(model.serialize()); assert.ok(!raw.includes('DO_NOT_SAVE'));
  const restored = new workspace.FileWorkspace(model.serialize());
  assert.equal(restored.active('right').path, '/three');
});
test('queue holds leases while queued and cancel waits for executor settlement before retry', async () => {
  let finish; let releases = 0;
  const q = new queue.TransferQueue(async (_task, context) => { await new Promise(r => { finish = r; }); context.checkCanceled(); }, () => ({ release() { releases++; } }));
  const id = q.enqueue({ name: 'f', source: { kind: 'remote', sessionId: 's', path: '/f' }, destination: { kind: 'local', path: '/f' }, entryKind: 'File', size: 4 });
  await tick(); q.cancel(id);
  assert.equal(q.tasks[0].status, 'canceling'); assert.equal(q.retry(id), false); assert.equal(releases, 0);
  finish(); await q.idle();
  assert.equal(q.tasks[0].status, 'canceled'); assert.equal(releases, 1);
  assert.ok(!JSON.stringify(q.serialize()).includes('sessionId'));
});
test('chunk copier accepts short reads, uses actual offsets and rejects premature EOF', async () => {
  const writes = []; let progress = 0;
  const result = await queue.copyChunks(5, async (offset) => new Uint8Array(offset === 0 ? 2 : 3), async (offset, bytes) => writes.push([offset, bytes.length]), value => { progress = value; });
  assert.equal(result, 5); assert.equal(progress, 5); assert.deepEqual(writes, [[0, 2], [2, 3]]);
  await assert.rejects(queue.copyChunks(5, async () => new Uint8Array(), async () => {}, () => {}), /EOF/);
  await queue.copyChunks(0, async () => { throw Error('must not read'); }, async (offset, bytes) => writes.push([offset, bytes.length]), () => {});
  assert.deepEqual(writes.at(-1), [0, 0]);
});
test('chunk copier preserves offsets beyond four GiB without bitwise coercion', async () => {
  const total = 2 ** 32 + 5; const offsets = [];
  await queue.copyChunks(total, async (offset, len) => { offsets.push(offset); return { byteLength: len }; }, async () => {}, () => {}, () => {}, 2 ** 31);
  assert.deepEqual(offsets, [0, 2 ** 31, 2 ** 32]);
});
