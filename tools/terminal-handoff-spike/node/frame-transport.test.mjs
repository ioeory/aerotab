import test from 'node:test';
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {FrameTransport} from './frame-transport.mjs';
const frames=()=>execFileSync('target/debug/view-fixture',{encoding:'utf8'}).trim().split('\n').map(JSON.parse);
test('real canonical frame full/damage codec preserves cells cursor colors and exact u64 ids',async()=>{
 const [full,damage]=frames();const acknowledgements=[];
 const receiver=new FrameTransport({session:full.session,view:'webview',epoch:full.epoch,ack:a=>acknowledgements.push(a)});
 receiver.apply(full);assert.equal(acknowledgements.length,1);assert.equal(acknowledgements[0].phase,'applied');
 assert.equal(receiver.state.rows[0][1].text,'中');assert.equal(receiver.state.rows[0][3].fg.kind,'rgb');
 receiver.apply(damage);assert.equal(receiver.state.rows[1][0].text,'D');
 const ticket=receiver.drawTicket();assert.equal(acknowledgements.length,2);
 receiver.drawComplete(ticket,'canvas-raf');assert.equal(acknowledgements.at(-1).phase,'drawn');
 assert.equal(acknowledgements.at(-1).physical_present,false);
 assert.throws(()=>receiver.drawComplete(ticket,'canvas-raf'),/stale/);
 assert.throws(()=>receiver.drawComplete({...receiver.drawTicket()},'canvas-raf'),/stale/);
 assert.equal(typeof full.revision,'string');assert.equal(typeof full.epoch,'string');
});
test('stale epoch/base/drop/malformed/wrapped wire seq and delayed draw reject without mutating state',()=>{
 const [full,damage]=frames();const receiver=new FrameTransport({session:full.session,view:'webview',epoch:full.epoch,ack:()=>{}});
 const wrapped=structuredClone(full);wrapped.wire_seq=0xffffffff;receiver.apply(wrapped);const old=receiver.drawTicket();
 const next=structuredClone(damage);next.wire_seq=0;receiver.apply(next);
 assert.throws(()=>receiver.drawComplete(old,'canvas-raf'),/stale/);
 const state=receiver.state;
 for(const change of [{epoch:'1'},{base_revision:'999'},{revision:99},{wire_seq:-1},{revision:full.revision},{rows:[{y:0,cells:[]}]}])assert.throws(()=>receiver.apply({...damage,...change}));
 for(const change of [{epoch:'999'}, {rows:[{y:0,cells:[]}]}, {cursor:{col:999,line:0,visible:true}}, {full:true,base_revision:null,rows:[]}])assert.throws(()=>receiver.apply({...damage,revision:'3',base_revision:damage.revision,...change}));
 assert.equal(receiver.state,state);assert.throws(()=>receiver.drawComplete(receiver.drawTicket(),'physical-present'),/evidence/);
});
test('consecutive revisions beyond JS safe integer stay distinct and dropped base requires full',()=>{
 const [full,damage]=frames();full.epoch=damage.epoch='9007199254740993';full.revision='9007199254740992';full.output_seq='9007199254740992';
 damage.revision='9007199254740993';damage.output_seq='9007199254740993';damage.base_revision=full.revision;
 const receiver=new FrameTransport({session:full.session,view:'webview',epoch:full.epoch,ack:()=>{}});
 receiver.apply(full);receiver.apply(damage);assert.equal(receiver.state.revision,'9007199254740993');
 assert.throws(()=>receiver.apply({...damage,revision:'9007199254740995',base_revision:'9007199254740994'}),/base/);
 assert.throws(()=>receiver.apply({...full,revision:'18446744073709551616'}),/malformed/);
});
