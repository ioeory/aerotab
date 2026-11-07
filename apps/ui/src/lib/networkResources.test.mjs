import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('./networkResources.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const api = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
test('new forwarding rules never auto-start and bind loopback', () => {
 const rule = api.newForwardingRule('profile');
 assert.equal(rule.bind_host, '127.0.0.1');
 assert.equal(rule.profile_id, 'profile');
 assert.equal('running' in rule, false);
 assert.equal('autostart' in rule, false);
});
test('proxy drafts only carry Vault references, never plaintext credentials', () => {
 const proxy = api.newProxyResource();
 assert.equal(proxy.config.kind, 'socks5');
 assert.equal(proxy.config.vault_entry_id, null);
 assert.equal('password' in proxy.config, false);
});
