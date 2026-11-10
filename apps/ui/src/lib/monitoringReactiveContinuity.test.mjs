import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { compileModule } from 'svelte/compiler';
import ts from 'typescript';

// Run the actual detail subscription and shared profile-catalog effect in the
// Svelte client scheduler; snapshots/timers alone cannot expose reactive loops.
const read = file => readFileSync(new URL(file, import.meta.url), 'utf8');
const app = read('../App.svelte');
const profilesEffect = app.split('\n').find(line => line.includes('$effect(() => { getHostMonitoring(rpc).setProfiles(savedProfiles); });'));
const detail = read('../components/HostSystemPanel.svelte');
const subscriptionEffect = detail.split('\n').find(line => line.includes('$effect(() => { if (!profileId) return; return monitoring.service.subscribe('));
assert.ok(profilesEffect && subscriptionEffect, 'actual monitoring effects must be found');
const core = read('./monitoring.ts');
const store = read('./hostMonitoring.svelte.ts').replace(/^import .*;\n/gm, '');
const source = `
import { untrack, flushSync } from 'svelte';
import { effect_root } from 'TEST_RUNTIME';
${core}
const PROFILES_CHANGED = 'aerotab:profiles-changed';
${store}
let wall = 10000, mono = 1000, resolves = [], timers = new Map(), nextTimer = 0;
const rpc = { call: () => new Promise(resolve => resolves.push(resolve)) };
const monitoring = getHostMonitoring(rpc);
monitoring.service.dispose();
monitoring.service = new MonitoringService(rpc, { now: () => wall, monotonicNow: () => mono,
  setTimer: fn => { timers.set(++nextTimer, fn); return nextTimer; }, clearTimer: id => timers.delete(id) });
monitoring.service.watch(() => untrack(() => { monitoring.revision++; }));
const profile = name => ({ id: 'a', name, kind: 'ssh', ssh: { host: 'same-host', port: 22, user: 'u', jump_via: [] } });
let savedProfiles = $state([profile('original')]), profileId = $state('a');
let subscriptions = 0, releases = 0;
const subscribe = monitoring.service.subscribe.bind(monitoring.service);
monitoring.service.subscribe = (...args) => { subscriptions++; const release = subscribe(...args); return () => { releases++; release(); }; };
const stop = effect_root(() => { ${profilesEffect} ${subscriptionEffect} });
flushSync();
for (let i = 0; i < 8; i++) {
  if (resolves.length !== 1) throw Error('unexpected in-flight probes: ' + resolves.length);
  resolves.shift()({ hostname: 'same-host', uptime_seconds: 100 + i,
    network_interfaces: [{ name: 'eth0', rx_bytes: 1000 + i * 100, tx_bytes: 2000 + i * 200 }] });
  await new Promise(resolve => setImmediate(resolve));
  flushSync();
  const snapshot = monitoring.service.snapshot('a');
  if (snapshot.history.length !== i + 1) throw Error('history reset at round ' + i);
  if (i > 0 && snapshot.latest.network_rates[0].rx_bytes_per_second !== 100) throw Error('network rate lost at round ' + i);
  savedProfiles = [profile('renamed-' + i)];
  flushSync();
  if (snapshot.history.length !== monitoring.service.snapshot('a').history.length) throw Error('profile refresh reset history');
  if (subscriptions !== 1 || releases !== 0) throw Error('sampling recreated subscription');
  wall -= 5000; mono += 1000;
  const pending = [...timers.values()]; timers.clear(); pending.forEach(fn => fn());
  flushSync();
}
stop(); monitoring.service.dispose();
if (releases !== 1) throw Error('lease not released');
`;
const javascript = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const code = compileModule(javascript, { filename: 'monitoring-continuity.svelte.js', generate: 'client' }).js.code
  .replaceAll("'TEST_RUNTIME'", JSON.stringify(import.meta.resolve('svelte/internal/client')))
  .replaceAll("'svelte/internal/client'", JSON.stringify(import.meta.resolve('svelte/internal/client')))
  .replaceAll("'svelte'", JSON.stringify(new URL('../../node_modules/svelte/src/index-client.js', import.meta.url).href));

test('actual shared-catalog and detail effects retain counter continuity across repeated samples and same-endpoint refreshes', () => {
  const result = spawnSync(process.execPath, ['--conditions=browser', '--input-type=module', '-e', code], { encoding: 'utf8', timeout: 10000 });
  assert.equal(result.status, 0, result.stderr || String(result.error));
});
