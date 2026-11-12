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
test('saved-host stats requests use the same ID-only payload as successful MCP probes', async () => {
  const h = harness(); const stop = h.service.subscribe('a');
  assert.deepEqual(h.calls[0].params, { profile_id: 'a' });
  h.calls[0].resolve({ cpu_percent: 10 }); await flush(); stop();
});
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
  assert.ok(h.calls.slice(3).some(c => c.params.profile_id === 'a'));
  assert.ok(h.calls.every(c => !('profile' in c.params)));
  releases.forEach(release => release());
});
test('repeated slower UI leases cannot postpone the next detail sample', async () => {
  const h = harness(); const detail = h.service.subscribe('a', 5000);
  h.calls[0].resolve({ cpu_percent: 10 }); await flush();
  const [scheduledId] = h.timers.keys();
  for (let i = 0; i < 20; i++) {
    const footer = h.service.subscribe('a', 30000);
    assert.equal([...h.timers.keys()][0], scheduledId, 'a slower lease reset the detail sampling deadline');
    footer();
  }
  h.advance(5000);
  assert.equal(h.calls.length, 2);
  h.calls[1].resolve({ cpu_percent: 20 }); await flush();
  assert.equal(h.service.snapshot('a').latest.stats.cpu_percent, 20);
  assert.equal(h.timers.size, 1);
  detail();
});
test('a faster detail lease advances the deadline without later slow leases delaying it', async () => {
  const h = harness(); const footer = h.service.subscribe('a', 30000);
  h.calls[0].resolve({ cpu_percent: 10 }); await flush();
  assert.equal([...h.timers.values()][0].ms, 30000);
  const detail = h.service.subscribe('a', 5000);
  const [detailTimer] = h.timers.keys();
  assert.equal(h.timers.size, 1);
  assert.equal([...h.timers.values()][0].ms, 5000);
  const extraFooter = h.service.subscribe('a', 60000);
  assert.equal([...h.timers.keys()][0], detailTimer);
  h.advance(5000); h.calls[1].resolve({ cpu_percent: 20 }); await flush();
  detail(); extraFooter();
  h.advance(5000); h.calls[2].resolve({ cpu_percent: 30 }); await flush();
  assert.equal([...h.timers.values()][0].ms, 30000);
  footer();
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
test('switching terminals away and back keeps the host history and marks only the pause as a break', async () => {
  const h = harness();
  let stopA = h.service.subscribe('a', 1000);
  h.calls[0].resolve({ cpu_percent: 10 }); await flush();
  h.advance(); h.calls[1].resolve({ cpu_percent: 20 }); await flush();
  stopA();
  const stopB = h.service.subscribe('b', 1000); h.calls.at(-1).resolve({ cpu_percent: 5 }); await flush(); stopB();
  h.advance(600000);
  stopA = h.service.subscribe('a', 1000); h.calls.at(-1).resolve({ cpu_percent: 30 }); await flush();
  const history = h.service.snapshot('a').history;
  assert.deepEqual(history.map(s => s.stats.cpu_percent), [10, 20, 30]);
  assert.deepEqual(history.map(s => s.breakBefore), [true, false, true]);
  stopA();
});
test('throwing watchers cannot strand sampling or block other watchers and queued hosts', async () => {
  const h = harness({ maxConcurrent: 1 });
  let updates = 0;
  h.service.watch(() => { throw Error('broken UI watcher'); });
  h.service.watch(() => { updates++; });
  const stopA = h.service.subscribe('a', 1000);
  const stopB = h.service.subscribe('b', 1000);
  assert.equal(h.calls.length, 1);
  h.calls[0].resolve({ cpu_percent: 10 }); await flush();
  assert.equal(h.calls.length, 2, 'queued host must acquire released slot');
  h.calls[1].resolve({ cpu_percent: 20 }); await flush();
  h.advance(); h.calls[2].resolve({ cpu_percent: 30 }); await flush();
  assert.equal(h.service.snapshot('a').history.length, 2);
  assert.ok(updates >= 4);
  stopA(); stopB();
});
for (const missingProfile of [false, true]) {
  test(`a hung ${missingProfile ? 'profile lookup' : 'stats probe'} times out, releases its slot and ignores late replies`, async () => {
    const h = harness({ maxConcurrent: 1, probeTimeoutMs: 100 });
    const id = missingProfile ? 'missing' : 'a';
    const stop = h.service.subscribe(id, 1000);
    const stopB = h.service.subscribe('b', 1000);
    assert.equal(h.calls[0].method, missingProfile ? 'profile.get' : 'ssh.hostStats');
    assert.equal(h.timers.size, 1, 'in-flight RPC must have a watchdog');
    h.advance(100); await flush();
    assert.equal(h.service.snapshot(id).sampling, false);
    assert.equal(h.service.snapshot(id).error, 'probeFailed');
    assert.equal(h.service.snapshot(id).failureStage, 'timeout');
    assert.equal(h.calls.length, 2, 'timeout must release global concurrency slot');
    h.calls[0].resolve(missingProfile ? profile(id) : { cpu_percent: 99 });
    h.calls[1].resolve({ cpu_percent: 20 }); await flush();
    assert.equal(h.calls.length, 2, 'late profile lookup must not start a stats RPC');
    assert.equal(h.service.snapshot(id).history.length, 0);
    h.advance(2000); await flush();
    const retry = h.calls[2];
    if (missingProfile) { retry.resolve(profile(id)); await flush(); }
    h.calls.at(-1).resolve({ cpu_percent: 30 }); await flush();
    assert.equal(h.service.snapshot(id).latest.stats.cpu_percent, 30);
    assert.equal(h.service.snapshot(id).error, undefined);
    stop(); stopB(); h.service.dispose();
  });
}
test('profile change notifications preserve unrelated history but fetched endpoint edits reset it', async () => {
  const h = harness(); const stop = h.service.subscribe('a', 1000);
  h.calls[0].resolve({ cpu_percent: 10 }); await flush();
  h.service.invalidateProfiles();
  assert.equal(h.service.snapshot('a').history.length, 1);
  h.advance();
  assert.equal(h.calls[1].method, 'profile.get');
  h.calls[1].resolve({ ...profile('a'), name: 'renamed' }); await flush();
  h.calls[2].resolve({ cpu_percent: 20 }); await flush();
  assert.equal(h.service.snapshot('a').history.length, 2);
  h.service.invalidateProfiles(); h.advance();
  h.calls[3].resolve({ ...profile('a'), ssh: { ...profile('a').ssh, host: 'changed' } }); await flush();
  h.calls[4].resolve({ cpu_percent: 30 }); await flush();
  assert.equal(h.service.snapshot('a').history.length, 1);
  assert.equal(h.service.snapshot('a').latest.continuityReason, 'profileChanged');
  stop();
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
test('default browser timers are invoked unbound so WebView never throws Illegal invocation', async () => {
  const realSet = globalThis.setTimeout, realClear = globalThis.clearTimeout;
  const pending = [];
  // Chromium/WebView2 rejects timer calls whose receiver is not the global object.
  globalThis.setTimeout = function (fn, ms) {
    if (this !== undefined && this !== globalThis) throw new TypeError('Illegal invocation');
    pending.push({ fn, ms }); return pending.length;
  };
  globalThis.clearTimeout = function () {
    if (this !== undefined && this !== globalThis) throw new TypeError('Illegal invocation');
  };
  try {
    const calls = [];
    const service = new api.MonitoringService({ call(method, params) { return new Promise((resolve, reject) => calls.push({ method, params, resolve, reject })); } });
    service.setProfiles([profile('a')]);
    const stop = service.subscribe('a', 1000);
    await flush();
    assert.equal(calls.length, 1);
    calls[0].resolve({ cpu_percent: 7 }); await flush();
    const snapshot = service.snapshot('a');
    assert.equal(snapshot.error, undefined);
    assert.equal(snapshot.sampling, false);
    assert.equal(snapshot.history.length, 1);
    assert.ok(pending.some(t => t.ms === 1000), 'next sample must be scheduled');
    stop();
  } finally { globalThis.setTimeout = realSet; globalThis.clearTimeout = realClear; }
});
