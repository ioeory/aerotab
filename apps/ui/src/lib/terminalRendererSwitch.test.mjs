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
