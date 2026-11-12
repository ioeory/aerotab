import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = ts.transpileModule(readFileSync(new URL('./monitoring.ts', import.meta.url), 'utf8'), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const { MonitoringService } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const flush = () => new Promise(resolve => setImmediate(resolve));
const profile = host => ({ id: 'a', kind: 'ssh', name: host, ssh: { host, port: 22, user: 'u', jump_via: [] } });
// Synthetic counters and clocks only; not a real SSH integration test.
function harness() {
  let wall = 10000, mono = 1000, nextTimer = 0; const callbacks = new Map(), calls = [];
  const service = new MonitoringService({ call: () => new Promise(resolve => calls.push(resolve)) }, { now: () => wall, monotonicNow: () => mono, setTimer: fn => { callbacks.set(++nextTimer, fn); return nextTimer; }, clearTimer: id => callbacks.delete(id) });
  service.setProfiles([profile('host-one')]);
  return { service, calls, tick() { wall -= 5000; mono += 1000; const pending = [...callbacks.values()]; callbacks.clear(); pending.forEach(fn => fn()); } };
}
test('a changed reported hostname cannot join another host history or counters', async () => {
  const h = harness(); const stop = h.service.subscribe('a'); h.calls[0]({ ...stats(100), hostname: 'one' }); await flush();
  h.tick(); h.calls[1]({ ...stats(200), hostname: 'two' }); await flush();
  assert.equal(h.service.snapshot('a').history.length, 1);
  assert.equal(h.service.snapshot('a').latest.network_rates[0].rx_bytes_per_second, null); stop();
});
const stats = count => ({ network_interfaces: [{ name: 'eth0', rx_bytes: count, tx_bytes: count }] });
test('counter deltas and probe durations use monotonic time despite wall clock adjustment', async () => {
  const h = harness(); const stop = h.service.subscribe('a'); h.calls[0](stats(100)); await flush();
  h.tick(); h.calls[1](stats(200)); await flush();
  assert.equal(h.service.snapshot('a').latest.network_rates[0].rx_bytes_per_second, 100);
  assert.ok(h.service.snapshot('a').latest.probeMs >= 0); stop();
});
test('editing the same saved profile to another host clears history and continuity', async () => {
  const h = harness(); const stop = h.service.subscribe('a'); h.calls[0](stats(100)); await flush();
  h.service.setProfiles([profile('host-two')]);
  assert.equal(h.service.snapshot('a').history.length, 0);
  h.calls.at(-1)(stats(200)); await flush();
  assert.equal(h.service.snapshot('a').latest.network_rates[0].rx_bytes_per_second, null); stop();
});
