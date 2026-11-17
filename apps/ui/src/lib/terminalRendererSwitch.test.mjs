import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../components/TerminalPane.svelte', import.meta.url), 'utf8');
const deferred = () => {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
};
const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };

// Execute the real component transition, with DOM/backend boundaries held pending.
// This reproduces tick() completing before NativeTerminalView's async start().
test('native transition stays pending until attach/snapshot and replay are ready', async () => {
  const start = source.indexOf('  async function switchToNativeFrontend(');
  const end = source.indexOf('  async function switchToXtermFrontend(', start);
  const fn = source.slice(start, end).replace(/cfg: Awaited<ReturnType<typeof loadTermSettings>>,/, 'cfg,');
  const ready = deferred();
  const env = {
    switchingFrontend: false, seedNativeFromReplay: false, session: { id: 'same-session' },
    cachedReplayChunks: () => [{ bytes: new Uint8Array([65]) }],
    teardownXtermInstance: async () => {}, applyNativeCfg: () => {},
    applyTransferDetectionFromCfg: () => {}, configureTransferFilter: async () => {},
    useNative: false, frontendReady: true, reportFrontendKind: () => {},
    tick: async () => {}, waitForNativeReady: () => ready.promise,
    beginNativeReadiness: () => ready.promise,
    destroyed: false,
  };
  const transition = new Function('env', `with (env) { ${fn}; return switchToNativeFrontend; }`)(env);
  let complete = false;
  const pending = transition({ experimentalTransferDetection: false }).then(() => { complete = true; });
  await flush();
  assert.equal(complete, false, 'tick is not native readiness: attach/snapshot still pending');
  ready.resolve();
  await pending;
  assert.equal(complete, true);
});

async function coordinator() {
  let mod;
  try { mod = await import('./terminalRendererSwitch.ts'); } catch (error) {
    if (error.code !== 'ERR_MODULE_NOT_FOUND') throw error;
  }
  assert.equal(typeof mod?.createTerminalSettingsReload, 'function', 'serialized reload coordinator must exist');
  return mod.createTerminalSettingsReload;
}

test('coalesces notifications, skips stale reads, and serializes transitions through readiness', async () => {
  const create = await coordinator();
  const read = deferred();
  const nativeReady = deferred();
  let desired = 'native';
  let reads = 0;
  let concurrent = 0;
  let peak = 0;
  const applied = [];
  const queue = create({
    load: async () => { reads++; return reads === 1 ? read.promise : desired; },
    apply: async (value) => {
      concurrent++; peak = Math.max(peak, concurrent);
      applied.push(value);
      if (value === 'native') await nativeReady.promise;
      concurrent--;
    },
  });
  const first = queue.request();
  desired = 'dom';
  queue.request(); queue.request();
  read.resolve('native');
  await first;
  assert.deepEqual(applied, ['dom'], 'stale settings must not start a renderer');
  desired = 'native';
  const next = queue.request();
  await flush();
  desired = 'dom';
  queue.request(); queue.request();
  await flush();
  assert.deepEqual(applied, ['dom', 'native']);
  nativeReady.resolve();
  await next;
  assert.deepEqual(applied, ['dom', 'native', 'dom']);
  assert.equal(peak, 1);
});

test('destroy suppresses pending loads and later notifications', async () => {
  const create = await coordinator();
  const read = deferred();
  const applied = [];
  const queue = create({ load: () => read.promise, apply: async (cfg) => { applied.push(cfg); } });
  const pending = queue.request();
  queue.dispose();
  read.resolve('native');
  await pending;
  await queue.request();
  assert.deepEqual(applied, []);
});

test('failed reload reports rejection and a later notification can retry', async () => {
  const create = await coordinator();
  let failed = true;
  const applied = [];
  const queue = create({
    load: async () => 'native',
    apply: async (value) => { if (failed) throw new Error('attach failed'); applied.push(value); },
  });
  await assert.rejects(queue.request(), /attach failed/);
  failed = false;
  await queue.request();
  assert.deepEqual(applied, ['native']);
});

// A superseded view transaction is a designed race loss: the transaction rolled
// back, the retained renderer stayed interactive, and diagnostics.record
// already captured the failure enum. reloadSettingsLive must filter these via
// isBenignViewRace instead of re-raising into a red status toast.
test('benign view race is classified without matching real surface failures', async () => {
  const { isBenignViewRace, viewTransactionSuperseded } = await import('./nativeViewTransaction.ts');
  assert.equal(isBenignViewRace(viewTransactionSuperseded()), true, 'marked transaction race');
  assert.equal(isBenignViewRace(new Error('native staging superseded during layout barrier')), true);
  assert.equal(isBenignViewRace(new Error('WebView staging superseded')), true);
  assert.equal(isBenignViewRace({ benignViewRace: true }), true);
  assert.equal(isBenignViewRace(null), false);
  assert.equal(isBenignViewRace(undefined), false);
  assert.equal(isBenignViewRace('view transaction superseded or session closed'), false, 'bare string is not an error');
  assert.equal(isBenignViewRace(new Error('native commit revision not drawn')), false, 'real draw failure must surface');
  assert.equal(isBenignViewRace(new Error('WebView input drain timeout')), false);
  assert.equal(isBenignViewRace(new Error('native_surface_unavailable: stale_generation: attachment replaced')), false);
});

test('reloadSettingsLive filters benign view races instead of calling onError', async () => {
  const source2 = await readFile(new URL('../components/TerminalPane.svelte', import.meta.url), 'utf8');
  const start = source2.indexOf('  function reloadSettingsLive(');
  const end = source2.indexOf('  async function applyTermSettingsLive(', start);
  const fn = source2.slice(start, end);
  assert.ok(fn.includes('isBenignViewRace'), 'filter must consult isBenignViewRace');
  assert.ok(fn.includes("traceView({ stage: 'reload.superseded' });"), 'race must stay traceable');
  // The production body calls onError only on the non-benign path.
  const benignReturn = fn.indexOf('return;');
  const onErrorCall = fn.indexOf('onError?.(String(error))');
  assert.ok(benignReturn > 0 && onErrorCall > benignReturn, 'benign path must return before onError');
});
