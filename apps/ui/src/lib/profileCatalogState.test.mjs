import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { compileModule } from 'svelte/compiler';

const cache = new Map();
function moduleUrl(url) {
  if (cache.has(url.href)) return cache.get(url.href);
  // Data-URL modules cannot represent the valid i18n <-> message-bundle ESM
  // cycle. Keep this catalog unit harness independent of translation loading.
  const input = readFileSync(url, 'utf8').replace(/^import \{ i18n \} from '\.\/i18n\.svelte';$/m, "const i18n = { t: (key: string) => key };");
  let source = ts.transpileModule(input, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
  if (url.pathname.endsWith('.svelte.ts')) source = compileModule(source, { filename: url.pathname, generate: 'server' }).js.code;
  source = source.replace(/(from\s*['"]|import\s*['"])([^'"]+)(['"])/g, (_, prefix, specifier, quote) => {
    const next = specifier.startsWith('.') ? moduleUrl(new URL(`${specifier}.ts`, url)) : import.meta.resolve(specifier);
    return `${prefix}${next}${quote}`;
  });
  const result = `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`;
  cache.set(url.href, result);
  return result;
}
const { getProfileCatalog } = await import(moduleUrl(new URL('./profileCatalog.svelte.ts', import.meta.url)));

test('catalog is shared per RPC client and coalesces concurrent refreshes', async () => {
  let calls = 0;
  let release;
  const gate = new Promise(resolve => { release = resolve; });
  const rpc = { async call(method) { calls++; await gate; return method === 'profile.list' ? [{ id: 'a' }] : { value: [' prod / web ', 'prod/web'] }; } };
  const catalog = getProfileCatalog(rpc);
  assert.equal(getProfileCatalog(rpc), catalog);
  const first = catalog.refresh();
  const second = catalog.refresh();
  assert.equal(first, second);
  assert.equal(catalog.refreshing, true);
  release();
  await first;
  assert.equal(calls, 3);
  assert.deepEqual(catalog.profiles, [{ id: 'a' }]);
  assert.deepEqual(catalog.groups, ['prod/web']);
  assert.equal(catalog.refreshing, false);
});

test('invalidation during a load queues one fresh snapshot and all callers await it', async () => {
  const releases = [];
  let calls = 0;
  const rpc = { call(method) {
    if (method !== 'profile.list') return Promise.resolve({ value: [] });
    calls++;
    return new Promise(resolve => releases.push(resolve));
  } };
  const catalog = getProfileCatalog(rpc);
  const first = catalog.refresh();
  const changed = catalog.invalidate();
  assert.equal(changed, first);
  assert.equal(catalog.refresh({ invalidate: true }), first);
  releases.shift()([{ id: 'old' }]);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(calls, 2);
  assert.equal(catalog.refreshing, true);
  assert.equal(catalog.refresh(), first);
  releases.shift()([{ id: 'new' }]);
  await changed;
  assert.deepEqual(catalog.profiles, [{ id: 'new' }]);
  assert.equal(catalog.refreshing, false);
});

test('invalidation during a failed load still fetches a fresh snapshot', async () => {
  let reject;
  let calls = 0;
  const rpc = { call(method) {
    if (method !== 'profile.list') return Promise.resolve({ value: [] });
    calls++;
    return calls === 1 ? new Promise((_, fail) => { reject = fail; }) : Promise.resolve([{ id: 'new' }]);
  } };
  const catalog = getProfileCatalog(rpc);
  const first = catalog.refresh();
  catalog.refresh({ invalidate: true });
  reject(Error('stale request failed'));
  await first;
  assert.equal(calls, 2);
  assert.deepEqual(catalog.profiles, [{ id: 'new' }]);
});

test('refresh and group persistence failures retain last successful state', async () => {
  let fail = false;
  const rpc = { async call(method) { if (fail) throw Error('offline'); return method === 'profile.list' ? [{ id: 'a' }] : { value: ['prod'] }; } };
  const catalog = getProfileCatalog(rpc);
  await catalog.refresh();
  fail = true;
  await assert.rejects(catalog.refresh(), /offline/);
  assert.deepEqual(catalog.profiles, [{ id: 'a' }]);
  await assert.rejects(catalog.saveGroups(['new']), /offline/);
  assert.deepEqual(catalog.groups, ['prod']);
  assert.equal(catalog.refreshing, false);
});

test('profile writes refresh the shared snapshot and failed writes preserve it', async () => {
  let stored = [{ id: 'a', name: 'old' }];
  let fail = false;
  const rpc = { async call(method, params) {
    if (method === 'profile.list') return structuredClone(stored);
    if (method === 'settings.get') return { value: [] };
    if (fail) throw Error('write failed');
    if (method === 'profile.upsert') stored = [params];
    if (method === 'profile.delete') stored = stored.filter(p => p.id !== params.id);
  } };
  const catalog = getProfileCatalog(rpc);
  await catalog.refresh();
  await catalog.upsert({ id: 'a', name: 'new' });
  assert.equal(catalog.profiles[0].name, 'new');
  fail = true;
  await assert.rejects(catalog.remove('a'), /write failed/);
  assert.equal(catalog.profiles.length, 1);
  fail = false;
  await catalog.remove('a');
  assert.deepEqual(catalog.profiles, []);
});

test('optional group setting failure never hides successfully loaded profiles', async () => {
  const rpc = { async call(method) { if (method === 'settings.get') throw Error('unsupported'); return [{ id: 'a' }]; } };
  const catalog = getProfileCatalog(rpc);
  await catalog.refresh();
  assert.deepEqual(catalog.profiles, [{ id: 'a' }]);
  assert.deepEqual(catalog.groups, []);
});
