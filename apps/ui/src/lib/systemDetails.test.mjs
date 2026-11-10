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
test('sparklines never invent first-sample trends, join missing data, or exceed bounded history', () => {
  assert.equal(typeof api.sparklinePaths, 'function');
  assert.deepEqual(api.sparklinePaths([{ at: 1, value: 50 }], 100), []);
  assert.deepEqual(api.sparklinePaths([{ at: 1, value: 50 }, { at: 2, value: null }, { at: 3, value: 10 }], 100), []);
  const paths = api.sparklinePaths([{ at: 1, value: 50 }, { at: 2, value: 60 }, { at: 3, value: 20, breakBefore: true }, { at: 4, value: 40 }], 100);
  assert.equal(paths.length, 2);
  assert.ok(paths.every(p => !p.includes('NaN')));
  const bounded = api.sparklinePaths(Array.from({ length: 120 }, (_, i) => ({ at: i, value: 50 })), 100);
  assert.equal((bounded.join('').match(/ L/g) ?? []).length, 59);
  assert.deepEqual(api.sparklinePaths([{ at: 2, value: 50 }, { at: 1, value: 60 }], 100), []);
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
