import test from 'node:test';
import assert from 'node:assert/strict';
import { runNativeViewTransaction } from './nativeViewTransaction.ts';
import { nativeViewGeometryEquals } from './nativeViewHost.ts';
test('deferred rollback retains target until ACK drain recovers, and stops on close',async()=>{
 const {recoverNativeInputRollback}=await import('./nativeViewTransaction.ts');
 for(const close of [false,true]){
  const calls=[];let live=true,attempt=0;
  await recoverNativeInputRollback({current:()=>live,drain:async()=>{calls.push('drain');if(++attempt===1){if(close)live=false;throw Error('native input drain timeout');}},rollback:async()=>calls.push('rollback')});
  assert.deepEqual(calls,close?['drain']:['drain','drain','rollback']);
 }
});
test('background rollback retries failed quiesce without releasing owner then notifies completion',async()=>{
 const {recoverNativeInputRollback}=await import('./nativeViewTransaction.ts');let attempts=0;const calls=[];
 await recoverNativeInputRollback({current:()=>true,retryMs:1,drain:async()=>{if(++attempts<3)throw Error('native_surface_unavailable: update rejected');},rollback:async()=>calls.push('restore')});
 assert.equal(attempts,3);assert.deepEqual(calls,['restore']);
});
for(const failure of ['lost','close','supersede'])test(`actual forward rollback ${failure} cannot replay or detach unresolved accepted input`,async()=>{
 const {readFile}=await import('node:fs/promises');const ts=(await import('typescript')).default;
 const {createNativeInputQueue,quiesceAndDrainNativeInput}=await import('./nativeInputBridge.ts');
 const {recoverNativeInputRollback}=await import('./nativeViewTransaction.ts');
 const text=await readFile(new URL('../components/NativeTerminalView.svelte',import.meta.url),'utf8');
 const body=text.slice(text.indexOf('  export async function stageNativeSurface('),text.indexOf('  /** Native -> WebView')).replace('export async','async')+'\nreturn stageNativeSurface;';
 const js=ts.transpileModule(body,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.None}}).outputText;
 const calls=[];let lost=true;
 const q=createNativeInputQueue({sessionId:'s',generation:()=> 'g',write:async()=>calls.push('write'),ack:async()=>{calls.push('ack');if(lost){lost=false;throw Error('lost ACK response');}},fail:()=>assert.fail('must recover'),ackRetryMs:1});
 const source={quiesceWebviewInput:async()=>{},resumeWebviewInput:()=>calls.push('resume'),focus:()=>{},commitNativeSurface:()=>assert.fail('no commit')};
 const host={generation:()=> 'g',waitForDraw:async()=>true,acknowledgeInput:async()=>{q.accept({sessionId:'s',generation:'g',seq:1,data:'YQ=='});if(failure==='close'){scope.disposed=true;q.dispose();}if(failure==='supersede')rect.width++;throw Error('ACK zero response lost');},quiesceInput:async()=>{calls.push('freeze');return 1;},dispose:async()=>calls.push('detach')};
 const rect={x:0,y:0,width:100,height:10};
 const scope={onRecovered:(ctl,backend,request)=>{assert.equal(ctl,source);assert.equal(backend,'webview');assert.equal(request,17);calls.push('recovered');},backend:'webview',committedWebview:false,committedNative:false,stagingNative:false,controller:source,surface:{getBoundingClientRect:()=>rect},settlePaneLayout:async()=>{},disposed:false,transitionClosing:false,viewConfig:()=>({}),traceLayout:()=>{},traceView:()=>{},stagingError:null,visible:true,occluded:false,globalOccluded:false,document:{hidden:false},runNativeViewTransaction,recoverNativeInputRollback,quiesceAndDrainNativeInput,mountSurface:async()=>{},hostAdapter:host,inputQueue:q,surfaceReady:false,cleanupSurface:null,syncSurface:null,unlistenInput:null,unlistenPaste:null,unlistenFontDelta:null,rollbackPending:false,nativeSurfaceError:e=>e,onError:()=>{},canFocusSurface:()=>true,nativeViewGeometryEquals};
 const bindings=Object.keys(scope).map(k=>`let ${k}=scope.${k};`).join('\n');const code=js.replace(/\bdisposed\b/g,'scope.disposed');const run=new Function('scope',bindings.replace('let disposed=scope.disposed;','')+code)(scope);
 await assert.rejects(run(17),/ACK zero response lost/);
 assert.ok(calls.filter(c=>c==='write').length<=1);
 if(failure==='close'){assert.ok(!calls.includes('detach'));assert.ok(!calls.includes('resume'));assert.ok(!calls.includes('recovered'));}
 else {assert.equal(calls.filter(c=>c==='write').length,1);assert.ok(calls.lastIndexOf('ack')<calls.indexOf('detach'));assert.ok(calls.indexOf('detach')<calls.indexOf('resume'));assert.ok(calls.indexOf('recovered')>calls.indexOf('resume'));}q.dispose();
});
function fixture(){const log=[];let current=true;return {log,setCurrent:v=>current=v,options:{current:()=>current,prepare:async()=>{log.push('draw');return 'receipt';},validate:r=>r==='receipt',drain:async()=>log.push('drain'),commit:async()=>log.push('commit'),rollback:async()=>log.push('rollback')}};}
test('target failure retains source and never drains or commits',async()=>{const f=fixture();f.options.prepare=async()=>{throw Error('target failed');};await assert.rejects(runNativeViewTransaction(f.options));assert.deepEqual(f.log,['rollback']);});
test('stale receipt and source supersession cannot commit',async()=>{for(const kind of ['receipt','config']){const f=fixture();if(kind==='receipt')f.options.validate=()=>false;else f.setCurrent(false);await assert.rejects(runNativeViewTransaction(f.options));assert.equal(f.log.includes('commit'),false);}});
test('drains accepted source exactly once after target draw before commit',async()=>{const f=fixture();await runNativeViewTransaction(f.options);assert.deepEqual(f.log,['draw','drain','commit']);});
test('session close during drain cancels commit and rolls back target',async()=>{const f=fixture();f.options.drain=async()=>{f.log.push('drain');f.setCurrent(false);};await assert.rejects(runNativeViewTransaction(f.options));assert.deepEqual(f.log,['draw','drain','rollback']);});
