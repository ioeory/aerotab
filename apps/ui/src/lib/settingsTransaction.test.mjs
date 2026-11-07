import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';

async function transaction() {
  let mod; try { mod = await import('./settingsTransaction.ts'); } catch (e) { if (e.code !== 'ERR_MODULE_NOT_FOUND') throw e; }
  assert.equal(typeof mod?.saveSettingsTransaction, 'function');
  return mod.saveSettingsTransaction;
}
for (const failed of ['appearance', 'uiAppearance']) test(`failed ${failed} write restores every persisted snapshot`, async () => {
  const save = await transaction();
  const values = { font: { size: 13, extra: true }, appearance: { customCss: 'old' }, uiAppearance: null };
  const before = JSON.parse(JSON.stringify(values));
  let rejected = false;
  const rpc = { call: async (method, params) => {
    if (method === 'settings.get') return { value: values[params.key] };
    values[params.key] = params.value;
    if (params.key === failed && !rejected) { rejected = true; throw new Error('write failed'); }
  } };
  await assert.rejects(save(rpc, [{ key: 'font', value: { size: 15 } }, { key: 'appearance', value: {} }, { key: 'uiAppearance', value: {} }]), /write failed/);
  assert.deepEqual(values, before);
});
test('rollback failure explicitly lists uncertain keys and original failure', async () => {
  const save = await transaction(); let writes = 0;
  const rpc = { call: async method => { if (method === 'settings.get') return { value: null }; if (++writes >= 2) throw new Error('offline'); } };
  await assert.rejects(save(rpc, [{ key: 'font', value: {} }, { key: 'appearance', value: {} }]), error => /rollback failed/i.test(error.message) && /font/.test(error.message) && /offline/.test(error.message));
});
test('failed initial read performs no writes', async () => {
  const save = await transaction(); let writes = 0;
  const rpc = { call: async method => { if (method === 'settings.get') throw new Error('read failed'); writes++; } };
  await assert.rejects(save(rpc, [{ key: 'font', value: {} }]), /read failed/);
  assert.equal(writes, 0);
});
test('real section savers reject before issuing any RPC when loading has failed', async () => {
  for (const name of ['Terminal', 'Ssh', 'Window']) {
    const source = await readFile(new URL(`../components/settings/sections/${name}Section.svelte`, import.meta.url), 'utf8');
    const start = source.indexOf('  async function save()');
    const end = source.indexOf('\n  }', start) + 4;
    const js = ts.transpileModule(source.slice(start, end), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
    let writes = 0;
    const save = new Function('loaded', 'loadError', 'i18n', 'rpc', `${js}; return save;`)(false, 'read failed', { t: key => key }, { call: async () => { writes++; } });
    await assert.rejects(save(), /read failed/);
    assert.equal(writes, 0, name);
  }
});

test('terminal SSH and window sections guard saves and expose disabled controls with retry', async () => {
  for (const name of ['Terminal', 'Ssh', 'Window']) {
    const source = await readFile(new URL(`../components/settings/sections/${name}Section.svelte`, import.meta.url), 'utf8');
    assert.match(source, /if \(!loaded\)/, name);
    assert.match(source, /fieldset disabled=\{!loaded/);
    assert.match(source, /onclick=\{load\}/);
    assert.match(source, /role="alert"/);
  }
});
