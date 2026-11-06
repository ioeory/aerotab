import test from 'node:test';
import assert from 'node:assert/strict';
import { createWebviewDrawBarrier } from './webviewDrawBarrier.ts';

test('snapshot RPC/apply/queued callback are not drawn; exact successful draw is required', async()=>{
 const gate=createWebviewDrawBarrier('epoch'); const wait=gate.begin(1); const ticket=gate.apply('epoch',1,'9007199254740993');
 let ready=false; wait.then(()=>ready=true); await Promise.resolve(); assert.equal(ready,false);
 assert.equal(gate.complete({...ticket,outputSeq:9007199254740992n}),false);
 assert.equal(gate.complete({...ticket,generation:'old'}),false);
 assert.equal(gate.complete(ticket),true); assert.equal(await wait,9007199254740993n);
});
test('fixed target ignores continuing output and rejects old revision and dispose',async()=>{
 const gate=createWebviewDrawBarrier('epoch'); const wait=gate.begin(2);
 assert.throws(()=>gate.apply('epoch',1,'100'),/revision/);
 const ticket=gate.apply('epoch',2,'43'); assert.equal(gate.complete(ticket),true); assert.equal(await wait,43n);
 const second=gate.begin(3); gate.dispose(); await assert.rejects(second,/disposed/); assert.equal(gate.complete(ticket),false);
});
test('render errors fail readiness; precise u64 and invalid inputs fail closed',async()=>{
 const gate=createWebviewDrawBarrier('g'); const wait=gate.begin(1); gate.apply('g',1,'18446744073709551615'); gate.fail(new Error('draw failed')); await assert.rejects(wait,/draw failed/);
 for(const seq of ['18446744073709551616',1,'01']) {const g=createWebviewDrawBarrier('g'); const p=g.begin(1); assert.throws(()=>g.apply('g',1,seq)); g.dispose(); await assert.rejects(p);}
});
