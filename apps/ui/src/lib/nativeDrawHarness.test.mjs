import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
const source=readFileSync(new URL('../../../../src-tauri/src/bin/aerotab-app.rs',import.meta.url),'utf8');
const helpers=source.slice(source.indexOf('const matchesTransaction='),source.indexOf('const timeline='));
const {matchesTransaction,matchingReceipt}=new Function(helpers+';return {matchesTransaction,matchingReceipt};')();
const p={sessionId:'s',generation:'source',epoch:2,label:'success'};
const transaction={sessionId:'s',sourceGeneration:'source',epoch:2,phase:'success'};
const r={sessionId:'s',generation:'target',revision:4,appliedOutputSeq:'1470',drawnOutputSeq:'1470',transaction};
const embedded=source.split('let script = r#"')[1].split('"#;')[0];
async function executeEmbedded(script=embedded){
 const reports=[];
 const context={console:{error(){}},performance:{getEntriesByType:()=>[]},location:{href:'test',search:''},MutationObserver:class{observe(){}},setTimeout,clearTimeout};
 context.window={__TAURI_INTERNALS__:{invoke:async(cmd,args)=>{if(cmd==='native_ui_e2e_report'){reports.push(args.report);return;}throw Error('stub stop');}},__TAURI__:{event:{listen:async()=>()=>{}}}};
 context.document={body:{dataset:{},innerText:'',},querySelectorAll:()=>[],querySelector:()=>null,scripts:[]};
 // Stop the untouched orchestration only after installing its real observers/helpers.
 context.setTimeout=()=>{throw Error('scope-fixture-stop');};
 const point="await wait(()=>footer()&&!footer().disabled&&surface(),'production terminal ready');";
 const exercise=`
 const ctl={},can={};window.__aerotabDebugDrawObserver.identity('s',ctl,can);
 const base={sessionId:'s',coreIdentity:1};const current={...base,channelRegistrations:1,attachCount:1,detachOrCloseCount:0,globalChannelRegistrations:2};
 const checks=continuity(base,current,identities.get('s'),can);
 if(Object.values(checks).some(v=>!v))throw Error('unrelated session miscount');
 if(continuity(base,{...current,channelRegistrations:2},identities.get('s'),can).channel)throw Error('reregister accepted');
 if(continuity(base,{...current,coreIdentity:2},identities.get('s'),can).core)throw Error('core replacement accepted');
 if(continuity(base,current,{controllers:new Set([ctl,{}]),canvases:new Set([can])},can).controller)throw Error('controller replacement accepted');
 if(continuity(base,current,{controllers:new Set([ctl]),canvases:new Set([{}])},can).canvas)throw Error('canvas replacement accepted');
 const p=beginReverse('s','success','source',0,'fixed');reverseProbe=p;
 window.__aerotabDebugDrawObserver.receipt({sessionId:'s',generation:'target',revision:4,appliedOutputSeq:'1470',drawnOutputSeq:'1470'});
 if(!matchingReceipt(p.receipts[0],p))throw Error('receipt association failed');
 const old={...p.receipts[0].transaction,epoch:p.epoch-1,phase:'fault'};
 const events=[{transaction:old},{transaction:p.receipts[0].transaction}];
 if(events.find(e=>matchesTransaction(e.transaction,p))!==events[1])throw Error('wrong hide selected');
 if(window.__aerotabDebugDrawObserver.pasteTrace!==true)throw Error('paste tracing disabled');
 window.__aerotabNativePasteTrace={snapshot:()=>[{order:1,stage:'authorization',reason:'hidden',request:1004}]};
 window.scopeResult={sessionId:p.sessionId,epoch:p.epoch,receiptCount:p.receipts.length};
 throw Error('intentional embedded failure branch');
 `;
 await vm.runInNewContext(script.replace(point,exercise),context);
 return {reports,window:context.window};
}
test('entire embedded script executes lexical helper, receipt subscription, selection and failure report',async()=>{
 const result=await executeEmbedded();
 assert.ok(result.window.scopeResult,JSON.stringify(result.reports));
 assert.deepEqual(JSON.parse(JSON.stringify(result.window.scopeResult)),{sessionId:'s',epoch:1,receiptCount:1});
 assert.equal(result.reports.at(-1).error,'Error: intentional embedded failure branch');
 assert.deepEqual(JSON.parse(JSON.stringify(result.reports.at(-1).nativePasteTrace)),[{order:1,stage:'authorization',reason:'hidden',request:1004}]);
 assert.match(embedded,/nativePasteTrace:window\.__aerotabNativePasteTrace\?\.snapshot\(\)\?\?\[\],layoutSnapshots,causalTrace:[^\n]*ok:oldWidgetLateBlocked/);
 const red=await executeEmbedded(embedded.replace('({label,sessionId,generation','({label,sessionId:id,generation'));
 assert.match(red.reports.at(-1).error,/ReferenceError.*id/);
 assert.equal(red.window.scopeResult,undefined);
});
test('ACK fault orchestration releases rejected update channel before waiting for hide',async()=>{
 const section=embedded.slice(embedded.indexOf(" phase='production ACK rejection and recovery';"),embedded.indexOf("await invoke('native_ui_e2e_buffer',{sessionId:id,inject:{generation:state.generation,phase:'initial native input'}});",embedded.indexOf("limitations:['Synthetic GTK input")));
 const body=section.slice(0,section.lastIndexOf('}'));
 let fault=false,clicked=false,geometry=false,released=false,reports=0;
 const base={sessionWriteCount:2,coreIdentity:1,channelRegistrations:1,gdkMarkerCount:0,flow:{ack:0,reserved:0,pending:0}};
 const target={getBoundingClientRect:()=>({width:geometry?92:100}),style:{set width(v){geometry=true;}}};
 const invoke=async(cmd,args)=>{
  if(cmd==='native_ui_e2e_report'){reports++;return;}
  if(cmd==='native_view_state')return {generation:'g',visible:true,inputFrozen:false};
  if(args?.inject?.ackFault===true){fault=true;return;}
  if(args?.inject?.ackFault===false){assert.ok(clicked&&geometry);fault=false;released=true;return;}
  if(!clicked&&!fault)return base;
  return {...base,sessionWriteCount:released?30:3,gdkMarkerCount:released?1:0,ackFault:{rejects:3},flow:{ack:released?28:0,reserved:28,pending:released?0:28}};
 };
 const wait=async(fn,label)=>{const value=await fn();assert.ok(value,`deadlocked ACK scenario: ${label}`);return value;};
 const run=new Function('invoke','inv','wait','footer','surface','window','backend','continuity','identities','id','state',`return (async()=>{let phase;${body}})()`);
 await run(invoke,invoke,wait,()=>({click(){clicked=true;}}),()=>target,{__aerotabViewCausalTrace:{snapshot:()=>clicked?[{stage:'transaction.drain'}]:[]},__stagingSourceCanvas:{}},async()=>{},()=>({channel:true}),new Map(),'s',{generation:'g'});
 assert.ok(released);assert.equal(reports,1);
});
test('captured startup invoke delays a real completed ACK until released',async()=>{
 const start=embedded.indexOf('window.__TAURI_INTERNALS__.invoke=async(cmd,args)=>{try{');
 const end=embedded.indexOf('const invoke=(cmd,args)=>',start);
 let release,settled=false;const gate={id:'s',baseline:0,held:false,dispatched:[],promise:new Promise(r=>{release=r;})};
 const window={__TAURI_INTERNALS__:{},__ackCloseGate:gate};
 new Function('window','inv','calls','reverseProbe',embedded.slice(start,end))(window,async()=>({real:true}),[],null);
 const captured=window.__TAURI_INTERNALS__.invoke;
 const pending=captured('native_view_update',{sessionId:'s',cfg:{inputAck:1}}).then(value=>{settled=true;return value;});
 for(let i=0;i<10;i++)await Promise.resolve();
 assert.ok(gate.held);assert.equal(settled,false);release();assert.deepEqual(await pending,{real:true});assert.equal(gate.dispatched.length,1);
});
test('isolated ACK close releases late host response only after real session removal and observes three seconds',async()=>{
 const start=embedded.indexOf("if(location.search.includes('ackClose=1')){");
 assert.notEqual(start,-1,'close case must be independently selectable');
 const body=embedded.slice(start+"if(location.search.includes('ackClose=1')){".length,embedded.indexOf("if(location.search.includes('ackRecovery=1'))",start)).trim().replace(/\}\s*$/,'');
 let closed=false,released=false,report;const order=[];let wrapped;
 const invoke=async(cmd,args)=>{
  if(cmd==='native_ui_e2e_buffer')return {flow:{reserved:0},sessionWriteCount:2};
  if(cmd==='native_view_state')throw Error('not_attached');
  if(cmd==='native_ui_e2e_report'){report=args.report;return;}
  return {};
 };
 const win={__TAURI_INTERNALS__:{invoke:async()=>({})},__aerotabViewCausalTrace:{snapshot:()=>[]}};
 const rpc=async()=>closed?[]:[{id:'s'}];
 const sleep=async ms=>{assert.equal(ms,3000);for(let i=0;i<10;i++)await Promise.resolve();assert.ok(released);order.push('observe');};
 const wait=async(fn)=>{
  if(!wrapped){wrapped=true;const gate=win.__ackCloseGate;gate.held=true;void gate.promise.then(()=>{released=true;});}
  for(let i=0;i<20;i++){const v=await fn();if(v)return v;await Promise.resolve();}throw Error('condition not met');
 };
 const doc={querySelector:()=>({click(){closed=true;order.push('close');}})};
 const AsyncFunction=Object.getPrototypeOf(async function(){}).constructor;
 await new AsyncFunction('invoke','inv','window','document','rpc','wait','sleep','footer','id','state','calls',body)(invoke,invoke,win,doc,rpc,wait,sleep,()=>({click(){}}),'s',{generation:'g'},[]);
 assert.equal(report.ok,true);assert.equal(report.observationMs,3000);assert.deepEqual(order,['close','observe']);
});
test('same-generation rollback hide cannot satisfy later success transaction',()=>{
 const events=[{generation:'source',transaction:{...transaction,epoch:1,phase:'fault'}},{generation:'source',transaction}];
 assert.notEqual(events.find(e=>e.generation===p.generation),events[1]); // old selector red control
 assert.equal(events.find(e=>matchesTransaction(e.transaction,p)),events[1]);
});
test('missing, stale source draw, wrong epoch and phase cannot pass; actual matching draw can',()=>{
 assert.equal([].some(x=>matchingReceipt(x,p)),false);
 for(const bad of [{...r,generation:'source'},{...r,transaction:{...transaction,epoch:1}},{...r,transaction:{...transaction,phase:'fault'}},{...r,revision:0},{...r,drawnOutputSeq:'1469'}])assert.equal(matchingReceipt(bad,p),false);
 assert.equal(matchingReceipt(r,p),true);
});
test('observer resolves debug sink at actual render publication, not mount; stores transaction receipt',()=>{
 const view=readFileSync(new URL('../components/NativeTerminalView.svelte',import.meta.url),'utf8');
 assert.match(view,/onDrawReceipt: receipt =>/);
 assert.match(source,/if\(p\)p.receipts.push\(receipt\)/);
 const controller=readFileSync(new URL('./termgrid/controller.ts',import.meta.url),'utf8');
 assert.ok(controller.indexOf('this.renderer.render({')<controller.indexOf('this.opts.onDrawReceipt?.('));
});
