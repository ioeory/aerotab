import test from 'node:test';
import assert from 'node:assert/strict';
import { createNativeInputQueue } from './nativeInputBridge.ts';
const flush = async () => { for (let i=0;i<30;i++) await Promise.resolve(); };
const event = (seq, data = 'YQ==', generation = 'g') => ({ sessionId: 's', generation, seq, data });
function setup(write, limit=256) {
  let generation = 'g'; const acks=[]; const errors=[];
  const q = createNativeInputQueue({ sessionId:'s', generation:()=>generation,
    write, ack: async seq=>{acks.push(seq);}, fail:()=>errors.push('failed'), limit });
  return {q, acks, errors, replace:()=>generation='new'};
}
test('rapid IME and key bytes stay ordered and duplicates/old generation are rejected', async()=>{
  const writes=[]; const {q,acks}=setup(async bytes=>{await Promise.resolve();writes.push([...bytes]);});
  q.accept(event(1,'5Lit')); q.accept(event(2,'DQ==')); q.accept(event(2)); q.accept(event(3,'YQ==','old')); q.accept(event(3,'/w=='));
  await flush();
  assert.deepEqual(writes,[[228,184,173],[13],[255]]);
  assert.equal(acks.at(-1),3);
});
test('dispose or replacement cancels queued input without rerouting it', async()=>{
  let done; const blocked=new Promise(r=>done=r); const writes=[];
  const {q,replace}=setup(async b=>{writes.push([...b]);await blocked;});
  q.accept(event(1)); await flush(); q.accept(event(2)); replace(); done(); await flush();
  assert.equal(writes.length,1);
  const other=setup(async b=>writes.push([...b])); other.q.dispose(); other.q.accept(event(1)); await flush();
  assert.equal(writes.length,1);
});
test('shared writer receives intact bytes for broadcast and transfer routing without recursion', async()=>{
  const {readFile}=await import('node:fs/promises');
  const pane=await readFile(new URL('../components/TerminalPane.svelte',import.meta.url),'utf8');
  assert.match(pane,/writeNativeBytes=\{nativeWriteBytes\}/);
  assert.match(pane,/nativeWriteInput\(text, valid\)/);
  assert.match(pane,/transferFilter\.processTerminalInput\(data\)/);
  assert.match(pane,/rpc\.call\('session\.writeMany', \{ ids: targets, data \}\)/);
  assert.match(pane,/sendSessionInput\(bytes, valid\)/);
  const view=await readFile(new URL('../components/NativeTerminalView.svelte',import.meta.url),'utf8');
  assert.match(view,/await listenNativeEvent<NativeInputEvent>/);
  assert.match(view,/await hostAdapter!\.acknowledgeInput\(0\)/);
});
test('view switch drains watermark including late delivery exactly once before release', async()=>{
  const writes=[]; const {q,acks}=setup(async b=>{await Promise.resolve();writes.push([...b]);});
  q.accept(event(1,'YQ=='));
  let released=false;
  const release=q.drainThrough(3,1000).then(()=>{released=true;});
  await flush(); assert.equal(released,false);
  q.accept(event(2,'5Lit')); q.accept(event(3,'DQ==')); q.accept(event(3,'DQ=='));
  await release;
  assert.deepEqual(writes,[[97],[228,184,173],[13]]); assert.equal(acks.at(-1),3);
  q.dispose();
});
test('handoff waits for real host ACK before detach, with no cancellation on timeout', async()=>{
  const {quiesceAndDrainNativeInput}=await import('./nativeInputBridge.ts');
  let ackDone; const ackBlocked=new Promise(r=>ackDone=r); const calls=[];
  const q=createNativeInputQueue({sessionId:'s',generation:()=> 'g',write:async()=>{calls.push('write');},
    ack:async()=>{await ackBlocked;calls.push('ack');},fail:()=>assert.fail('unexpected failure')});
  q.accept(event(1));
  const host={quiesceInput:async()=>{calls.push('freeze');return 2;},dispose:async()=>calls.push('detach')};
  const handoff=quiesceAndDrainNativeInput(host,q,1000).then(()=>host.dispose());
  q.accept(event(2)); await flush(); assert.ok(!calls.includes('detach'));
  ackDone(); await handoff;
  assert.deepEqual(calls,['write','freeze','ack','write','ack','detach']);
  const blocked=setup(async()=>{});
  await assert.rejects(quiesceAndDrainNativeInput({quiesceInput:()=>new Promise(()=>{})},blocked.q,5),/timeout/);
  blocked.q.accept(event(1)); await flush(); assert.equal(blocked.acks.at(-1),1);
});
test('transient ACK rejection retries acknowledgement, never successful PTY bytes', async()=>{
  const writes=[],acks=[],errors=[];
  const q=createNativeInputQueue({sessionId:'s',generation:()=> 'g',write:async b=>writes.push([...b]),
    ack:async seq=>{acks.push(seq);if(acks.length===1)throw Error('ACK rejected');},fail:()=>errors.push('fatal'),ackRetryMs:1});
  q.accept(event(1));q.accept(event(2,'Yg=='));
  await q.drainThrough(2,200);
  assert.deepEqual(writes,[[97],[98]]);assert.deepEqual(acks,[1,1,2]);assert.deepEqual(errors,[]);q.dispose();
});
test('sustained ACK rejection retains bounded ownership across timeout then recovers without replay', async()=>{
  let reject=true;const writes=[],states=[],errors=[];
  const q=createNativeInputQueue({sessionId:'s',generation:()=> 'g',write:async b=>writes.push([...b]),
    ack:async()=>{if(reject)throw Error('ACK rejected');},fail:()=>errors.push('fatal'),ackRetryMs:1,
    onAckRecovery:value=>states.push(value)});
  q.accept(event(1));q.accept(event(2,'Yg=='));
  await assert.rejects(q.drainThrough(2,15),/timeout/);
  assert.deepEqual(writes,[[97]]);assert.deepEqual(errors,[]);assert.deepEqual(states,[true]);
  reject=false;await q.drainThrough(2,200);
  q.accept(event(3,'Yw=='));await q.drainThrough(3,200);
  assert.deepEqual(writes,[[97],[98],[99]]);assert.deepEqual(states,[true,false]);q.dispose();
});
for(const cancel of ['dispose','generation'])test(`ACK recovery ${cancel} cancels waiters and never sends stale input`,async()=>{
  let generation='g', attempts=0;const writes=[];
  const q=createNativeInputQueue({sessionId:'s',generation:()=>generation,write:async b=>writes.push([...b]),
    ack:async()=>{attempts++;throw Error('ACK rejected');},fail:()=>{},ackRetryMs:1});
  q.accept(event(1));q.accept(event(2));await flush();
  const drained=q.drainThrough(2,200);if(cancel==='dispose')q.dispose();else generation='new';
  await assert.rejects(drained,/cancelled/);
  const count=attempts;await new Promise(r=>setTimeout(r,10));assert.equal(attempts,count);assert.equal(writes.length,1);q.dispose();
});
test('closing while ACK response is pending rejects drain immediately and ignores late success',async()=>{
 let finish;const pending=new Promise(r=>finish=r);const writes=[];
 const q=createNativeInputQueue({sessionId:'s',generation:()=> 'g',write:async b=>writes.push([...b]),ack:()=>pending,fail:()=>assert.fail('close is cancellation')});
 q.accept(event(1));q.accept(event(2));await flush();const wait=q.drainThrough(2,200);q.dispose();
 await assert.rejects(wait,/cancelled/);finish();await flush();assert.equal(writes.length,1);
});
test('ACK-applied but lost response is idempotent and cannot duplicate PTY bytes',async()=>{
  let hostAck=0,lost=true;const writes=[];
  const q=createNativeInputQueue({sessionId:'s',generation:()=> 'g',write:async b=>writes.push([...b]),ack:async seq=>{
    hostAck=Math.max(hostAck,seq);if(lost){lost=false;throw Error('response lost');}
  },fail:()=>assert.fail('ACK is recoverable'),ackRetryMs:1});
  q.accept(event(1));await q.drainThrough(1,200);assert.equal(hostAck,1);assert.equal(writes.length,1);q.dispose();
});
test('never-returning ACK has one transport call, visible bounded recovery and cancellable application wait',async()=>{
 let finish;const pending=new Promise(r=>finish=r);let calls=0;const writes=[],states=[];
 const q=createNativeInputQueue({sessionId:'s',generation:()=> 'g',write:async b=>writes.push([...b]),ack:()=>{calls++;return pending;},fail:()=>assert.fail('retain owner'),ackTimeoutMs:3,ackRetryMs:1,onAckRecovery:v=>states.push(v)});
 q.accept(event(1));q.accept(event(2));await assert.rejects(q.drainThrough(2,15),/timeout/);
 assert.deepEqual(states,[true]);assert.equal(calls,1);assert.equal(writes.length,1);
 const drain=q.drainThrough(2,200);q.dispose();await assert.rejects(drain,/cancelled/);
 finish();await flush();assert.equal(calls,1);assert.equal(writes.length,1);
});
test('hanging ACK generation replacement cancels application wait and ignores late success',async()=>{
 let generation='g',finish;const writes=[];let acks=0;
 const q=createNativeInputQueue({sessionId:'s',generation:()=>generation,write:async b=>writes.push([...b]),ack:()=>{acks++;return new Promise(r=>finish=r);},fail:()=>assert.fail('not a write failure'),ackTimeoutMs:2});
 q.accept(event(1));q.accept(event(2));const waiting=q.drainThrough(2,100);await new Promise(r=>setTimeout(r,4));generation='new';await assert.rejects(waiting,/cancelled/);
 finish();await flush();assert.equal(acks,1);assert.equal(writes.length,1);q.dispose();
});
test('late ACK after deadline resumes same reservation without duplicate RPC or PTY write',async()=>{
 let finish;const pending=new Promise(r=>finish=r);let calls=0;const writes=[];
 const q=createNativeInputQueue({sessionId:'s',generation:()=> 'g',write:async b=>writes.push([...b]),ack:()=>{calls++;return pending;},fail:()=>assert.fail('retain owner'),ackTimeoutMs:2,ackRetryMs:1});
 q.accept(event(1));await assert.rejects(q.drainThrough(1,8),/timeout/);finish();await q.drainThrough(1,200);
 assert.equal(calls,1);assert.equal(writes.length,1);q.dispose();
});
test('repeated quiesce deadlines never start parallel non-cancellable host requests',async()=>{
 const {quiesceAndDrainNativeInput}=await import('./nativeInputBridge.ts');let calls=0,finish;
 const host={quiesceInput:()=>{calls++;return new Promise(r=>finish=r);}},queue={drainThrough:async()=>{}};
 for(let i=0;i<3;i++)await assert.rejects(quiesceAndDrainNativeInput(host,queue,2),/timeout/);
 assert.equal(calls,1);finish(0);await flush();
});
test('writer rejection fails drain without ACK or fake success', async()=>{
  const {q,acks,errors}=setup(async()=>{throw new Error('writer failed');});
  q.accept(event(1));
  await assert.rejects(q.drainThrough(1),/cancelled/);
  assert.deepEqual(acks,[]); assert.equal(errors.length,1);
});
test('missing watermark fails visibly instead of claiming successful switch', async()=>{
  const {q}=setup(async()=>{});
  await assert.rejects(q.drainThrough(1,5), /timeout/);
});
test('sequence gap, invalid base64 and bounded overflow fail explicitly', async()=>{
  const gap=setup(async()=>{}); gap.q.accept(event(2)); assert.equal(gap.errors.length,1);
  const bad=setup(async()=>{}); bad.q.accept(event(1,'%%%')); await flush(); assert.equal(bad.errors.length,1);
  const full=setup(async()=>new Promise(()=>{}),1); full.q.accept(event(1)); full.q.accept(event(2)); assert.equal(full.errors.length,1);
});
