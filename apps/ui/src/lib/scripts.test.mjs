import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('./scripts.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const api = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
test('draft metadata contains only script fields and stable profile id', () => {
  assert.deepEqual(api.scriptPayload({id:'s',name:' N ',command:'echo ok',description:'d',profile_id:'p',ssh:{auth:{Password:{secret:'hidden'}}},version:2}), {id:'s',name:'N',command:'echo ok',description:'d',profile_id:'p'});
});
test('run requires a prepared explicit confirmation and never uses terminal write', async () => {
  const calls = []; const rpc = {call: async (method, params) => { calls.push([method,params]); return method === 'scripts.prepare' ? {token:'t',profile_id:'p',target:'root@host:22',script_name:'N',command:'echo ok'} : {id:'task'}; }};
  const runner = new api.ScriptRunner(rpc);
  await assert.rejects(runner.confirm(), /confirmation/);
  assert.deepEqual(calls, []);
  await runner.prepare('s', 'p'); assert.equal(calls.length, 1);
  await runner.confirm();
  assert.deepEqual(calls.map(x=>x[0]), ['scripts.prepare','scripts.run']);
  assert.deepEqual(calls[1][1], {token:'t'});
  await assert.rejects(runner.confirm(), /confirmation/);
});
test('late status response cannot restore cancelled task', async () => {
  let release;
  const runner = new api.ScriptRunner({call: async (method) => method === 'scripts.status' ? await new Promise(resolve => { release = resolve; }) : {id:'task',status:'cancelled'}});
  runner.task = {id:'task',status:'running'};
  const pending = runner.refresh(); await runner.cancel(); release({id:'task',status:'running'}); await pending;
  assert.equal(runner.task.status,'cancelled');
});
test('dismissing confirmation cannot run and failed cancel retains task', async () => {
  const runner = new api.ScriptRunner({call: async (method) => { if(method === 'scripts.cancel') throw Error('offline'); return {token:'t'}; }});
  await runner.prepare('s','p'); runner.dismiss(); await assert.rejects(runner.confirm());
  runner.task = {id:'task',status:'running'};
  await assert.rejects(runner.cancel()); assert.equal(runner.task.status,'running');
});
