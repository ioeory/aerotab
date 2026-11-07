import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';
const source = await readFile(new URL('./rpc.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText;
const exports = {};
new Function('exports', js)(exports);
const { selectClient } = exports;

async function callWith(response) {
  const oldWindow = globalThis.window;
  const oldFetch = globalThis.fetch;
  globalThis.window = {};
  globalThis.fetch = async () => response;
  try { return await selectClient().call('settings.get'); }
  finally { globalThis.window = oldWindow; globalThis.fetch = oldFetch; }
}

test('HTTP failures explain missing desktop backend without parsing empty JSON', async () => {
  await assert.rejects(callWith(new Response('', { status: 404 })), /RPC backend unavailable.*HTTP 404.*desktop/i);
});
test('empty successful response has actionable error rather than JSON SyntaxError', async () => {
  await assert.rejects(callWith(new Response('')), /RPC backend.*empty response.*desktop/i);
});
test('malformed response does not expose response body', async () => {
  await assert.rejects(callWith(new Response('<html>token=secret</html>')), error => /invalid JSON/.test(error.message) && !error.message.includes('secret'));
});
test('non-RPC JSON is rejected', async () => {
  await assert.rejects(callWith(Response.json({})), /invalid JSON-RPC response/);
});
test('valid RPC result and server errors retain existing semantics', async () => {
  assert.deepEqual(await callWith(Response.json({ jsonrpc: '2.0', id: 1, result: { value: null } })), { value: null });
  assert.equal(await callWith(Response.json({ jsonrpc: '2.0', id: 1, result: null })), null);
  await assert.rejects(callWith(Response.json({ jsonrpc: '2.0', id: 1, error: { code: -1, message: 'offline' } })), /-1: offline/);
});
