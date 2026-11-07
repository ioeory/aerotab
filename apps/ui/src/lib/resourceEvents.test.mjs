import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const js = ts.transpileModule(readFileSync(new URL('./resourceEvents.ts', import.meta.url), 'utf8'), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const api = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
test('resource notifications are scoped, unsubscribe and isolate failing subscribers', () => {
 const seen = []; const stop = api.subscribeResource('vault', () => seen.push('vault'));
 api.subscribeResource('knownHosts', () => seen.push('host'));
 api.subscribeResource('vault', () => { throw Error('broken view'); });
 api.notifyResource('vault'); assert.deepEqual(seen, ['vault']);
 stop(); api.notifyResource('vault'); assert.deepEqual(seen, ['vault']);
 api.notifyResource('knownHosts'); assert.deepEqual(seen, ['vault', 'host']);
});
test('RPC wrapper publishes only successful resource mutations from every entry point', async () => {
 let notifications = 0; const stop = api.subscribeResource('vault', () => notifications++);
 const client = api.instrumentResourceClient({ async call(method) { if (method === 'vault.remove') throw Error('referenced'); return 42; } });
 assert.equal(await client.call('vault.unlock'), 42);
 assert.equal(notifications, 1);
 await client.call('vault.list'); assert.equal(notifications, 1);
 await assert.rejects(client.call('vault.remove')); assert.equal(notifications, 1);
 stop();
});
test('SHA256 fingerprint hashes decoded SSH key bytes, not the base64 text', async () => {
 assert.equal(await api.hostKeyFingerprint('YWJj'), 'SHA256:ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0');
 await assert.rejects(api.hostKeyFingerprint('not base64!'));
});
