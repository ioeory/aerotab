import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('./vaultBootstrap.ts', import.meta.url), 'utf8').replace(/^import .* from '\.\/syncConfig';$/m, 'const loadPersistedSyncSettings = async (rpc) => (await rpc.call("settings.get", {key: "sync"})).value;');
const js = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText;
const { bootstrapVault, unlockVaultWithOptions } = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);

test('startup never reads saved secrets or auto unlocks a locked vault', async () => {
  const calls = [];
  const rpc = { async call(method) { calls.push(method); if (method === 'vault.status') return { configured: true, initialized: true, unlocked: false }; return { unlocked: true }; } };
  assert.equal(await bootstrapVault(rpc), 'needs_password');
  assert.deepEqual(calls, ['vault.status']);
});

test('explicit unlock preserves password bytes and never persists even legacy save option', async () => {
  const calls = [];
  const rpc = { async call(method, params) { calls.push([method, params]); return { unlocked: true }; } };
  assert.equal(await unlockVaultWithOptions(rpc, ' password ', { saveToKeyring: true, account: 'sync.master' }), true);
  assert.deepEqual(calls, [['sync.ensureVaultUnlock', { password: ' password ' }]]);
});

test('vault UI has no saved-password controls or secret RPCs', () => {
  for (const file of ['../components/VaultUnlockModal.svelte', '../components/settings/sections/ConfigSyncSection.svelte']) {
    const source = readFileSync(new URL(file, import.meta.url), 'utf8');
    if (file.includes('VaultUnlockModal')) assert.doesNotMatch(source, /saveToKeyring|hasVaultKeyringSecret|loadVaultKeyringAccount/);
    else assert.doesNotMatch(source, /saveVaultSecret|clearVaultSecret|refreshVaultSecretStatus|vaultSecretParams/);
  }
});

test('true app exit explicitly locks vault', () => {
  const source = readFileSync(new URL('../../../../src-tauri/src/bin/aerotab-app.rs', import.meta.url), 'utf8');
  assert.match(source, /RunEvent::Exit\)[\s\S]*?vault\.lock\(\)\.await/);
});
