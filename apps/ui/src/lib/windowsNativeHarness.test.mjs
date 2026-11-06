import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
const backend=readFileSync(new URL('../../../../src-tauri/src/windows_ui_e2e.rs',import.meta.url),'utf8');
test('bounded finish records actual reverse failure and ordered source boundaries',()=>{
 assert.match(backend,/snapshot_fault/);
 assert.match(backend,/before-hide/);
 assert.match(backend,/after-hide/);
 assert.match(backend,/before-detach/);
 assert.match(backend,/osVisible/);
});
const script=readFileSync(new URL('../../../../src-tauri/src/windows_ui_e2e.js',import.meta.url),'utf8');
test('Windows harness failure path persists report without a startup session',async()=>{
 const reports=[];
 const context={document:{querySelector:()=>null,querySelectorAll:()=>[]},setTimeout:()=>{throw Error('bounded fixture stop');},window:{__TAURI_INTERNALS__:{invoke:async(cmd,args)=>{if(args?.action==='report')reports.push(args.report);}}}};
 await vm.runInNewContext(script,context);
 assert.equal(reports.length,1);assert.equal(reports[0].ok,false);assert.match(reports[0].error,/bounded fixture stop/);assert.equal(reports[0].publicAvailable,false);
 assert.equal(context.window.__aerotabDebugDrawObserver.causalTrace,true);
});
test('Windows acceptance reports synthetic messages and waits for host ACK and exact marker',()=>{
 assert.match(script,/synthetic:true,hardware:false/);
 assert.match(script,/s\.inputAck===s\.inputAccepted/);
 assert.match(script,/s\.markers===1/);
 assert.match(script,/round<2/);
 assert.match(script,/channelRegistrations!==1/);
});
