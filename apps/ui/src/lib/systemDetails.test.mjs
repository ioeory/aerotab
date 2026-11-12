import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const path = new URL('./systemDetails.ts', import.meta.url);
let api = {};
try { api = await import(`data:text/javascript;base64,${Buffer.from(ts.transpileModule(readFileSync(path, 'utf8'), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText).toString('base64')}`); } catch (error) { if (error.code !== 'ENOENT') throw error; }
// Synthetic fixtures only: these are not real remote SSH samples.
test('network totals require complete real interface rates and exclude loopback / bridge duplicates', () => {
  assert.equal(typeof api.networkSummary, 'function');
  const row = (name, rx, tx) => ({ name, rx_bytes: 1000, tx_bytes: 2000, rx_bytes_per_second: rx, tx_bytes_per_second: tx });
  assert.deepEqual(api.networkSummary([row('eth0', 20, 30), row('lo', 99, 99), row('eth0-ovs', 20, 30)]), { rx: 20, tx: 30 });
  assert.deepEqual(api.networkSummary([row('eth0', null, null)]), { rx: null, tx: null });
  assert.deepEqual(api.networkSummary([]), { rx: null, tx: null });
});
test('network state distinguishes unavailable, filtered, warmup, invalid counters and incomplete totals', () => {
  const row = (name, rx, tx) => ({ name, rx_bytes: 10, tx_bytes: 20, rx_bytes_per_second: rx, tx_bytes_per_second: tx });
  const snapshot = (rows, extra = {}) => ({ history: [], sampling: false, failures: 0, latest: { stats: {}, network_rates: rows }, ...extra });
  assert.equal(typeof api.networkState, 'function');
  assert.equal(api.networkState({ history: [], sampling: true, failures: 0 }).status, 'waitingFirst');
  assert.equal(api.networkState(snapshot([])).status, 'interfacesUnavailable');
  assert.equal(api.networkState(snapshot([row('lo', 0, 0)])).status, 'noEligibleInterfaces');
  assert.equal(api.networkState(snapshot([row('eth0', null, null)], { latest: { stats: {}, network_rates: [row('eth0', null, null)], continuityReason: 'firstSample' } })).status, 'waitingSecond');
  assert.equal(api.networkState(snapshot([row('eth0', null, null)])).status, 'invalidCounters');
  const partial = api.networkState(snapshot([row('eth0', 5, 10), row('eth1', null, null), row('lo', 0, 0)]));
  assert.equal(partial.status, 'incomplete');
  assert.equal(partial.filteredCount, 1);
  assert.equal(partial.eligibleCount, 2);
  assert.equal(partial.readyCount, 1);
  assert.equal(partial.rx, null, 'strict overview must not silently undercount a missing interface');
  assert.equal(api.networkState(snapshot([row('eth0', 0, 0)])).status, 'ready');
  assert.equal(api.networkState(snapshot([], { error: 'probeFailed' })).status, 'probeFailed');
});
test('network diagnostic payload contains only bounded counts and enum reasons, not host or interface identities', () => {
  assert.equal(typeof api.networkDiagnostic, 'function');
  const snapshot = { sampling: false, failures: 0, history: [{}, {}], latest: { continuityReason: 'probeFailed', stats: { hostname: '10.1.2.3', profile: { password: 'SECRET' } }, network_rates: [{ name: 'secret-interface', rx_bytes_per_second: null, tx_bytes_per_second: null }] } };
  const diagnostic = api.networkDiagnostic(snapshot);
  assert.equal(diagnostic.sampleCount, 2);
  assert.equal(diagnostic.interfaceCount, 1);
  assert.equal(diagnostic.continuityReason, 'probeFailed');
  assert.doesNotMatch(JSON.stringify(diagnostic), /SECRET|10\.1\.2\.3|secret-interface|hostname|password/);
});
test('sparklines never invent first-sample trends across breakBefore, or exceed bounded history', () => {
  assert.equal(typeof api.sparklinePaths, 'function');
  assert.deepEqual(api.sparklinePaths([{ at: 1, value: 50 }], 100), []);
  // Mid-series nulls compress without fragmenting into undrawable single-point islands.
  const acrossNull = api.sparklinePaths([{ at: 1, value: 50 }, { at: 2, value: null }, { at: 3, value: 10 }], 100);
  assert.equal(acrossNull.length, 1);
  assert.match(acrossNull[0], /^M0\.0,/);
  assert.match(acrossNull[0], / L200\.0,/);
  const paths = api.sparklinePaths([{ at: 1, value: 50 }, { at: 2, value: 60 }, { at: 3, value: 20, breakBefore: true }, { at: 4, value: 40 }], 100);
  assert.equal(paths.length, 2);
  assert.ok(paths.every(p => !p.includes('NaN')));
  const bounded = api.sparklinePaths(Array.from({ length: 120 }, (_, i) => ({ at: i, value: 50 })), 100);
  assert.equal((bounded.join('').match(/ L/g) ?? []).length, 59);
  assert.deepEqual(api.sparklinePaths([{ at: 2, value: 50 }, { at: 1, value: 60 }], 100), []);
});
test('sparklines plot by sample order so a paused terminal switch does not leave a blank middle', () => {
  // Three samples, a 10-minute pause while another terminal was active, then three more.
  const before = [0, 5000, 10000].map(at => ({ at, value: 50 }));
  const after = [610000, 615000, 620000].map((at, i) => ({ at, value: 50, breakBefore: i === 0 }));
  const paths = api.sparklinePaths([...before, ...after], 100);
  assert.equal(paths.length, 2, 'the pause remains a visible break, not an invented line');
  const xs = paths.map(p => p.slice(1).split(' L').map(pt => Number(pt.split(',')[0])));
  assert.equal(xs[0][0], 0);
  assert.equal(xs[1].at(-1), 200);
  assert.ok(xs[1][0] - xs[0].at(-1) <= 200 / 5 + 0.1, `gap ${xs[1][0] - xs[0].at(-1)} must be one sample step, not wall-clock time`);
});
test('null network rates after resume do not leave a large blank region on the right', () => {
  // History kept old rates; first sample after switch-back has no rate (needs 2 probes).
  const before = Array.from({ length: 15 }, (_, i) => ({ at: i * 1000, value: 50 }));
  const warmup = { at: 15_000, value: null, breakBefore: true };
  const after = [16_000, 17_000, 18_000, 19_000].map(at => ({ at, value: 40 }));
  const paths = api.sparklinePaths([...before, warmup, ...after], 100);
  assert.equal(paths.length, 2);
  const xs = paths.map(p => p.slice(1).split(' L').map(pt => Number(pt.split(',')[0])));
  assert.equal(xs[0][0], 0);
  assert.equal(xs[1].at(-1), 200, 'drawable samples must span the full sparkline width');
  const gap = xs[1][0] - xs[0].at(-1);
  assert.ok(gap <= 200 / 18 + 0.1, `null warmup must be a one-step break (${gap}), not empty trailing width`);
});
test('null,null,rate,rate spans full width without a right-edge stub', () => {
  const paths = api.sparklinePaths([
    { at: 1, value: null }, { at: 2, value: null }, { at: 3, value: 40 }, { at: 4, value: 50 },
  ], 100);
  assert.equal(paths.length, 1);
  const xs = paths[0].slice(1).split(' L').map(pt => Number(pt.split(',')[0]));
  assert.equal(xs[0], 0);
  assert.equal(xs.at(-1), 200);
});
test('alternating null rates do not fragment into a short right-edge dash', () => {
  // Incomplete interface totals / warmup nulls used to set breakPending per null,
  // leaving only the final consecutive run as a stub on the right.
  const points = [];
  for (let i = 0; i < 20; i++) {
    points.push({ at: i * 1000, value: i % 2 === 0 ? 50 : null });
  }
  points.push({ at: 20_000, value: 40 }, { at: 21_000, value: 45 });
  const paths = api.sparklinePaths(points, 100);
  assert.equal(paths.length, 1, 'nulls compress without segment breaks');
  const xs = paths[0].slice(1).split(' L').map(pt => Number(pt.split(',')[0]));
  assert.equal(xs[0], 0);
  assert.equal(xs.at(-1), 200);
  assert.ok(xs.length >= 10, `expected a full series, got ${xs.length} vertices`);
});
test('virtual filesystem filters and capacity warning thresholds are explicit', () => {
  assert.equal(typeof api.isVirtualMount, 'function');
  assert.ok(api.isVirtualMount({ device: 'tmpfs', mount_point: '/run' }));
  assert.ok(api.isVirtualMount({ device: 'udev', mount_point: '/dev' }));
  assert.equal(api.isVirtualMount({ device: '/dev/mapper/root', mount_point: '/' }), false);
  assert.equal(api.capacityTone(79), 'normal');
  assert.equal(api.capacityTone(80), 'warning');
  assert.equal(api.capacityTone(90), 'danger');
});
