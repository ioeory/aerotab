import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('./monitoring.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const api = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
const flush = () => new Promise(resolve => setImmediate(resolve));
const profile = id => ({ id, name: id, kind: 'ssh', ssh: { host: id, port: 22, user: 'u', auth: 'Agent', jump_via: [] } });
function harness(options = {}) {
  let now = 1000;
  const timers = new Map(); let timerId = 0;
  const calls = [];
  const service = new api.MonitoringService({ call(method, params) { return new Promise((resolve, reject) => calls.push({ method, params, resolve, reject })); } }, {
    now: () => now, setTimer: (fn, ms) => { const id = ++timerId; timers.set(id, { fn, ms }); return id; }, clearTimer: id => timers.delete(id), ...options,
  });
  service.setProfiles([profile('a'), profile('b'), profile('c')]);
  return { service, calls, timers, advance(ms = 1000) { now += ms; const pending = [...timers.values()]; timers.clear(); for (const t of pending) t.fn(); } };
}
test('shared subscriptions deduplicate and late results cannot revive stopped sampling', async () => {
  const h = harness();
  const a = h.service.subscribe('a', 1000), b = h.service.subscribe('a', 2000);
  assert.equal(h.calls.length, 1);
  a(); assert.equal(h.service.activeProfileIds().length, 1);
  b(); h.calls[0].resolve({ cpu_percent: 25 }); await flush();
  assert.equal(h.service.snapshot('a').history.length, 0);
  assert.equal(h.timers.size, 0);
});
test('global concurrency limits isolate slow and failed hosts, latest profile read on each probe', async () => {
  const h = harness({ maxConcurrent: 2 });
  const releases = ['a', 'b', 'c'].map(id => h.service.subscribe(id, 1000));
  assert.equal(h.calls.length, 2);
  h.calls[0].reject(Error('offline')); await flush();
  assert.equal(h.calls.length, 3);
  h.calls[1].resolve({ cpu_percent: 12 }); h.calls[2].resolve({ cpu_percent: 4 }); await flush();
  assert.equal(h.service.snapshot('b').history.length, 1);
  h.service.setProfiles([{ ...profile('a'), ssh: { ...profile('a').ssh, host: 'new-host' } }, profile('b'), profile('c')]);
  h.advance(3000); await flush();
  assert.ok(h.calls.some(c => c.params.profile.host === 'new-host'));
  releases.forEach(release => release());
});
test('bounded history and counter resets never fabricate network rates', async () => {
  const h = harness({ maxSamples: 2 }); const stop = h.service.subscribe('a', 1000);
  const sample = rx => ({ uptime_seconds: 100, network_interfaces: [{ name: 'eth0', rx_bytes: rx, tx_bytes: rx * 2 }] });
  h.calls[0].resolve(sample(1000)); await flush();
  assert.equal(h.service.snapshot('a').latest.network_rates[0].rx_bytes_per_second, null);
  h.advance(); h.calls[1].resolve(sample(2000)); await flush();
  assert.equal(h.service.snapshot('a').latest.network_rates[0].rx_bytes_per_second, 1000);
  h.advance(); h.calls[2].resolve(sample(1)); await flush();
  assert.equal(h.service.snapshot('a').latest.network_rates[0].rx_bytes_per_second, null);
  assert.equal(h.service.snapshot('a').history.length, 2); stop();
});
test('stopping then restarting while a request is in flight never overlaps the host', async () => {
  const h = harness(); const stop = h.service.subscribe('a', 1000); stop();
  const stop2 = h.service.subscribe('a', 1000);
  assert.equal(h.calls.length, 1);
  h.calls[0].resolve({ cpu_percent: 50 }); await flush();
  assert.equal(h.calls.length, 2);
  assert.equal(h.service.snapshot('a').history.length, 0); stop2();
  h.calls[1].resolve({}); await flush();
});
test('restarting a subscription resets counter continuity', async () => {
  const h = harness(); const stop = h.service.subscribe('a', 1000);
  const sample = rx => ({ network_interfaces: [{ name: 'eth0', rx_bytes: rx, tx_bytes: rx }] });
  h.calls[0].resolve(sample(100)); await flush(); stop();
  h.advance(); const stop2 = h.service.subscribe('a', 1000);
  h.calls[1].resolve(sample(1000)); await flush();
  assert.equal(h.service.snapshot('a').latest.network_rates[0].rx_bytes_per_second, null); stop2();
});
test('paused windows release timers and resume without fabricating counter rates', async () => {
  const h = harness(); const stop = h.service.subscribe('a', 1000);
  h.service.setPaused(true); h.calls[0].resolve({ cpu_percent: 5 }); await flush();
  assert.equal(h.service.snapshot('a').history.length, 0); assert.equal(h.timers.size, 0);
  h.service.setPaused(false); assert.equal(h.calls.length, 2);
  h.calls[1].resolve({ cpu_percent: 7 }); await flush();
  assert.equal(h.service.snapshot('a').history.length, 1); stop();
});
test('diagnostic callbacks are bounded and continuity reasons survive failures, restarts and reboot', async () => {
  const events = []; const h = harness({ onDiagnostic: (snapshot, stream) => events.push({ snapshot, stream }) });
  const stop = h.service.subscribe('a', 1000);
  const sample = (rx, uptime = 100) => ({ uptime_seconds: uptime, network_interfaces: [{ name: 'eth0', rx_bytes: rx, tx_bytes: rx }] });
  h.calls[0].resolve(sample(100)); await flush();
  assert.equal(h.service.snapshot('a').latest.continuityReason, 'firstSample');
  h.advance(); h.calls[1].resolve(sample(200)); await flush();
  assert.equal(h.service.snapshot('a').latest.continuityReason, undefined);
  h.advance(); h.calls[2].reject(Error('password=SECRET host=10.1.2.3')); await flush();
  h.advance(); h.calls[3].resolve(sample(400)); await flush();
  assert.equal(h.service.snapshot('a').latest.continuityReason, 'probeFailed');
  h.advance(); h.calls[4].resolve(sample(500, 1)); await flush();
  assert.equal(h.service.snapshot('a').latest.continuityReason, 'reboot');
  for (let i = 5; i < 35; i++) { h.advance(); h.calls[i].resolve(sample(500 + i, 2)); await flush(); }
  assert.equal(events.length, 1, 'never emit diagnostics at every sampling tick');
  h.advance(60000); h.calls.at(-1).resolve(sample(1000, 3)); await flush();
  assert.equal(events.length, 2);
  assert.equal(typeof events[0].stream, 'number');
  stop(); const stop2 = h.service.subscribe('a', 1000); h.calls.at(-1).resolve(sample(2000, 4)); await flush();
  assert.equal(h.service.snapshot('a').latest.continuityReason, 'subscriptionRestart');
  stop2();
});
test('panel persistence includes definitions only, validates preferences and strips credentials', () => {
  const panels = api.normalizePanels([{ id: 'p', name: 'Prod', profileIds: ['a', 'a'], running: true, history: [1], auth: 'secret', intervalMs: 2, rangeMinutes: 15, view: 'chart' }]);
  assert.deepEqual(panels[0].profileIds, ['a']);
  assert.ok(panels[0].intervalMs >= 1000);
  assert.equal(JSON.stringify(panels).includes('secret'), false);
  assert.equal('running' in panels[0], false);
  assert.equal('history' in panels[0], false);
});
test('interface diagnostics distinguish new interfaces, reset counters and invalid counters without exposing names', () => {
  const previous = { at: 1000, stats: { network_interfaces: [{ name: 'eth0', rx_bytes: 100, tx_bytes: 100 }] } };
  const rows = api.networkRates({ network_interfaces: [{ name: 'eth0', rx_bytes: 1, tx_bytes: 200 }, { name: 'eth1', rx_bytes: 10, tx_bytes: 10 }, { name: 'bad', rx_bytes: NaN, tx_bytes: 10 }] }, previous, 2000);
  assert.equal(rows[0].unavailableReason, 'counterReset');
  assert.equal(rows[1].unavailableReason, 'newInterface');
  assert.equal(rows[2].unavailableReason, 'invalidCounter');
});
test('rate calculation rejects reboot, invalid counters, and first samples', () => {
  const prev = { at: 1000, stats: { uptime_seconds: 100, network_interfaces: [{ name: 'e', rx_bytes: 5, tx_bytes: 10 }] } };
  const next = { uptime_seconds: 1, network_interfaces: [{ name: 'e', rx_bytes: 1000, tx_bytes: 2000 }] };
  assert.equal(api.networkRates(next, prev, 2000)[0].rx_bytes_per_second, null);
  assert.equal(api.networkRates(next, undefined, 2000)[0].tx_bytes_per_second, null);
});
