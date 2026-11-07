import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { compileModule } from 'svelte/compiler';

// Execute App's actual effect with the client reactive scheduler, not SSR (which
// erases effects). No desktop session or remote host is touched by this harness.
const app = readFileSync(new URL('../App.svelte', import.meta.url), 'utf8');
const effectStart = app.lastIndexOf('  $effect(() => {', app.indexOf('const key = activeHostStatsKey();'));
assert.ok(effectStart >= 0, 'host stats effect must exist');
const effect = app.slice(effectStart, app.indexOf('\n  });', effectStart) + '\n  });'.length);
const source = `
import { untrack, flushSync } from 'svelte';
import { effect_root } from 'TEST_RUNTIME';
let hostStats = $state(null), hostStatsStatus = $state('idle'), hostStatsUpdatedAt = $state(null);
let hostStatsEnabled = $state(true), hostStatsIntervalSec = $state(30);
let tabs = { revision: 0, activeId: 'tab' }, workspacePage = 'sessions';
let hostStatsSeq = 0, hostStatsPollHandle, savedProfiles = [], rpc = {};
const activeHostStatsKey = () => 'profile:a';
const currentActivePane = () => ({ profileId: 'a' });
const clearHostStatsPoll = () => {};
const refreshHostStats = () => {};
let subscriptions = 0, releases = 0, listeners = new Set();
let latest, error;
const service = {
  snapshot: () => ({ latest, error }),
  watch: fn => { listeners.add(fn); return () => listeners.delete(fn); },
  subscribe: () => { subscriptions++; return () => { releases++; for (const fn of listeners) fn(); }; }
};
const getHostMonitoring = () => ({ service, setProfiles: () => {} });
const stop = effect_root(() => { ${effect} });
flushSync();
if (subscriptions !== 1) throw Error('initial subscription count: ' + subscriptions);
latest = { stats: { cpu_percent: 17 }, at: 123 };
for (const fn of listeners) fn();
flushSync();
if (subscriptions !== 1 || releases !== 0) throw Error('sample restarted subscription: ' + subscriptions + '/' + releases);
if (hostStats?.cpu_percent !== 17 || hostStatsStatus !== 'ok' || hostStatsUpdatedAt !== 123) throw Error('sample lost');
for (let i = 0; i < 50; i++) {
  latest = { stats: { cpu_percent: i }, at: 124 + i };
  error = i % 2 ? 'probeFailed' : undefined;
  for (const fn of listeners) fn();
  flushSync();
  if (hostStatsStatus !== (error ? 'unavailable' : 'ok')) throw Error('sample status lost');
}
if (subscriptions !== 1 || releases !== 0) throw Error('later samples restarted subscription');
stop();
if (releases !== 1 || listeners.size !== 0) throw Error('lease cleanup failed');
`;
const code = compileModule(source, { filename: 'host-stats-regression.svelte.js', generate: 'client' }).js.code
  .replaceAll("'TEST_RUNTIME'", JSON.stringify(import.meta.resolve('svelte/internal/client')))
  .replaceAll("'svelte/internal/client'", JSON.stringify(import.meta.resolve('svelte/internal/client')))
  .replaceAll("'svelte'", JSON.stringify(new URL('../../node_modules/svelte/src/index-client.js', import.meta.url).href));

test('footer stats samples must not retrigger the subscription-owning effect', () => {
  const result = spawnSync(process.execPath, ['--conditions=browser', '--input-type=module', '-e', code], { encoding: 'utf8', timeout: 10000 });
  assert.equal(result.status, 0, result.stderr || String(result.error));
});
