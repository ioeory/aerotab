import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import ts from 'typescript';
import {runNativeViewTransaction,recoverNativeInputRollback} from './nativeViewTransaction.ts';
import {quiesceAndDrainNativeInput,createNativeInputQueue} from './nativeInputBridge.ts';
const text=await readFile(new URL('../components/NativeTerminalView.svelte',import.meta.url),'utf8');
const body=text.slice(text.indexOf('  export async function stageWebviewSurface('),text.indexOf('  export function cancelViewTransition')).replace('export async','async')+'\nreturn stageWebviewSurface;';
const js=ts.transpileModule(body,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.None}}).outputText;
function fixture(failure,queue){
 const events=[];let geometry=1,config=1,closed=false;
 const source={prepareWebviewSurface:async()=>events.push('prepare'),waitForDraw:async()=>{events.push('draw');if(failure==='draw')throw Error('draw timeout');if(failure==='geometry')geometry++;if(failure==='config')config++;if(failure==='close')closed=true;},beginWebviewInputHandoff:()=>events.push('buffer'),focusWebviewHandoff:()=>events.push('focus'),focus:()=>events.push('focus'),commitWebviewSurface:()=>events.push('commit'),rollbackWebviewSurface:async()=>events.push('rollback')};
 const host={generation:()=> 'g',suspendForWebview:async()=>{events.push('suspend');return 4;},restoreAfterWebview:async()=>{events.push('restore');if(failure==='restore'&&events.filter(e=>e==='restore').length<3)throw Error('native_surface_unavailable: update rejected');},dispose:async()=>events.push('detach'),setFocusAllowed(){}};
 const scope={quiesceAndDrainNativeInput:(h,q)=>quiesceAndDrainNativeInput(h,q,10),onRecovered:()=>events.push('recovered'),nativeSurfaceError:error=>error,transitionClosing:false,onError:()=>{},recoverNativeInputRollback,rollbackPending:false,controller:source,hostAdapter:host,inputQueue:{drainThrough:async t=>{assert.equal(t,4);events.push('drain');if((failure==='drain'||failure==='restore') && events.filter(e=>e==='drain').length===1)throw Error('ACK timeout');}},disposed:false,stagingWebview:false,committedWebview:false,stagingNative:false,committedNative:true,webviewHandoff:false,reverseEpoch:0,
 surface:{getBoundingClientRect:()=>({x:0,y:0,width:geometry,height:10})},viewConfig:()=>({config}),visible:true,occluded:false,globalOccluded:false,document:{hidden:false},runNativeViewTransaction,settlePaneLayout:async()=>{},tick:async()=>{},cleanupSurface:null,syncSurface:null,disposePaste:()=>events.push('listeners.dispose'),canFocusSurface:()=>true};
 // Closure accessors make supersession/close visible to the exact production body.
 Object.defineProperty(scope,'disposed',{get:()=>closed,set:v=>{closed=v;}});
 if(queue)scope.inputQueue={drainThrough:target=>queue.drainThrough(target,10)};
 const bindings=Object.keys(scope).map(k=>`let ${k}=scope.${k};`).join('\n');
 // disposed is read live rather than snapshotted for close cancellation.
 const code=js.replace(/\bdisposed\b/g,'scope.disposed').replace(/\breverseEpoch\b/g,'scope.reverseEpoch');
 const run=new Function('scope',bindings.replace('let disposed=scope.disposed;','').replace('let reverseEpoch=scope.reverseEpoch;','')+code)(scope);
 return {run,events,close:()=>{closed=true;},supersede:()=>{geometry++;},replaceRequest:()=>{scope.reverseEpoch++;}};
}
test('actual reverse transaction close during ACK recovery cancels without late rollback',async()=>{
 let generation='g';const q=createNativeInputQueue({sessionId:'s',generation:()=>generation,write:async()=>{},ack:async()=>{throw Error('ACK');},fail:()=>{},ackRetryMs:1});
 for(let seq=1;seq<=4;seq++)q.accept({sessionId:'s',generation:'g',seq,data:'YQ=='});
 const f=fixture('ack',q);const run=f.run();setTimeout(()=>{generation=null;q.dispose();},2);
 await assert.rejects(run,/cancelled/);assert.ok(!f.events.includes('rollback'));assert.ok(!f.events.includes('commit'));assert.ok(!f.events.includes('detach'));
});
test('actual reverse request epoch supersession rejects old commit after delayed ACK',async()=>{
 let finish;const pending=new Promise(r=>finish=r);const writes=[];
 const q=createNativeInputQueue({sessionId:'s',generation:()=> 'g',write:async b=>writes.push([...b]),ack:()=>pending,fail:()=>assert.fail('retain'),ackTimeoutMs:2});
 for(let seq=1;seq<=4;seq++)q.accept({sessionId:'s',generation:'g',seq,data:'YQ=='});
 const f=fixture('ack',q);const run=f.run();setTimeout(()=>{f.replaceRequest();finish();},2);
 await assert.rejects(run,/superseded/);assert.equal(writes.length,4);assert.ok(!f.events.includes('commit'));assert.ok(!f.events.includes('detach'));q.dispose();
});
test('actual reverse geometry supersession during pending ACK never commits or detaches',async()=>{
 let finish;const pending=new Promise(r=>finish=r);const writes=[];
 const q=createNativeInputQueue({sessionId:'s',generation:()=> 'g',write:async b=>writes.push([...b]),ack:()=>pending,fail:()=>assert.fail('retain'),ackTimeoutMs:2});
 for(let seq=1;seq<=4;seq++)q.accept({sessionId:'s',generation:'g',seq,data:'YQ=='});
 const f=fixture('ack',q);const run=f.run();setTimeout(()=>{f.supersede();finish();},2);
 await assert.rejects(run,/superseded/);assert.equal(writes.length,4);assert.ok(f.events.includes('restore'));assert.ok(!f.events.includes('commit'));assert.ok(!f.events.includes('detach'));q.dispose();
});
test('actual reverse transaction retains source and buffered target through sustained production ACK rejection',async()=>{
 let rejected=true;const writes=[];
 const q=createNativeInputQueue({sessionId:'s',generation:()=> 'g',write:async b=>writes.push([...b]),ack:async()=>{if(rejected)throw Error('ACK rejection');},fail:()=>assert.fail('must retain'),ackRetryMs:1});
 for(let seq=1;seq<=4;seq++)q.accept({sessionId:'s',generation:'g',seq,data:'YQ=='});
 const f=fixture('ack',q);await assert.rejects(f.run(),/timeout/);
 assert.equal(writes.length,1);assert.ok(!f.events.includes('rollback'));assert.ok(!f.events.includes('detach'));
 rejected=false;await q.drainThrough(4,200);
 for(let i=0;i<30;i++)await Promise.resolve();
 assert.equal(writes.length,4);assert.ok(f.events.includes('rollback'));assert.ok(f.events.includes('restore'));assert.ok(!f.events.includes('commit'));q.dispose();
});
for(const failure of ['draw','geometry','config','close','drain'])test(`actual reverse transaction ${failure} cannot commit/detach source`,async()=>{
 const f=fixture(failure);await assert.rejects(f.run());assert.equal(f.events.includes('rollback'),failure!=='close');assert.ok(!f.events.includes('commit'));assert.ok(!f.events.includes('detach'));
 if(failure==='drain')assert.ok(f.events.includes('restore'));
});
test('actual reverse update failure retains owner and retries restore without duplicate target input',async()=>{
 const f=fixture('restore');await assert.rejects(f.run(),/update rejected/);
 for(let i=0;i<30&&!f.events.includes('recovered');i++)await new Promise(r=>setTimeout(r,25));
 assert.equal(f.events.filter(e=>e==='restore').length,3);assert.equal(f.events.filter(e=>e==='recovered').length,1);assert.ok(!f.events.includes('commit'));assert.ok(!f.events.includes('detach'));
});
test('actual reverse transaction draws before reversible suspension and drains before detach',async()=>{
 const f=fixture();await f.run();assert.deepEqual(f.events,['prepare','draw','buffer','focus','suspend','drain','commit','listeners.dispose','detach','focus']);
});
