import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
const server=await createServer({configFile:false,optimizeDeps:{noDiscovery:true,include:[]},server:{middlewareMode:true}});
const {NativeTerminalController}=await server.ssrLoadModule('/src/lib/termgrid/controller.ts');
await server.close();
function frame(){const b=new ArrayBuffer(34);const d=new DataView(b);d.setUint8(0,1);d.setUint16(2,2,true);d.setUint16(4,1,true);return [...new Uint8Array(b)];}
function fixture({drawError=false,late=false,onDrawReceipt}={}){
 let cb; let complete;const calls=[];
 globalThis.window={devicePixelRatio:1,removeEventListener(){},clearTimeout(){}};
 globalThis.requestAnimationFrame=fn=>(cb=fn,1);globalThis.cancelAnimationFrame=()=>{};
 const rpc={call:async(method)=>{calls.push(method);if(method==='terminal.snapshotEnvelope'){if(late)await new Promise(r=>complete=r);return {version:1,frameVersion:1,outputSeq:'9007199254740993',bytes:frame()};}}};
 const ctl=new NativeTerminalController({sessionId:'same-core',rpc,canvas:{removeEventListener(){}},cursorBlink:false,onDrawReceipt});
 let draws=0;ctl.renderer={kind:'canvas2d',resize(){},dispose(){},render(){draws++;if(drawError)throw new Error('GPU draw failed');}};
 return {ctl,calls,draw:()=>cb?.(),draws:()=>draws,finish:()=>complete?.()};
}
const flush=async()=>{for(let i=0;i<20;i++)await Promise.resolve();};
test('source accepted writes drain once, paused input replays once on rollback',async()=>{
 const f=fixture();const writes=[];let finish;
 f.ctl.setWriteInput(async data=>{writes.push(data);if(data==='first')await new Promise(r=>finish=r);});
 const first=f.ctl.writePty('first');let drained=false;const drain=f.ctl.quiesceWebviewInput().then(()=>drained=true);
 const pending=f.ctl.writePty('pending');await flush();assert.equal(drained,false);assert.deepEqual(writes,['first']);
 finish();await first;await drain;f.ctl.resumeWebviewInput();await pending;assert.deepEqual(writes,['first','pending']);await f.ctl.dispose();
});
test('commit keeps same frame owner, replays pending once, never attaches/detaches core',async()=>{
 const f=fixture();const writes=[];f.ctl.setWriteInput(async d=>{writes.push(d);});await f.ctl.quiesceWebviewInput();const pending=f.ctl.writePty('pending');f.ctl.commitNativeSurface();await pending;
 assert.deepEqual(writes,['pending']);assert.equal(f.ctl.opts.nativeSurface,true);assert.equal(f.calls.length,0);await f.ctl.dispose();
});
test('successful exact draw emits frozen metadata only; throwing observer cannot reject readiness',async()=>{
 const receipts=[];const f=fixture({onDrawReceipt:r=>{receipts.push(r);assert.equal(Object.isFrozen(r),true);throw Error('observer failed');}});
 const wait=f.ctl.waitForDraw();await flush();assert.equal(receipts.length,0);f.draw();await wait;
 assert.equal(receipts.length,1);assert.deepEqual(Object.keys(receipts[0]).sort(),['appliedOutputSeq','drawnOutputSeq','generation','revision','sessionId']);
 assert.equal(receipts[0].sessionId,'same-core');assert.equal(receipts[0].generation,f.ctl.viewGeneration);assert.equal(receipts[0].revision,1);
 assert.equal(receipts[0].appliedOutputSeq,'9007199254740993');assert.equal(receipts[0].drawnOutputSeq,'9007199254740993');
 await f.ctl.dispose();
});
test('failed renderer never emits a draw receipt',async()=>{const receipts=[];const f=fixture({drawError:true,onDrawReceipt:r=>receipts.push(r)});const wait=f.ctl.waitForDraw();await flush();f.draw();await assert.rejects(wait,/GPU draw failed/);assert.equal(receipts.length,0);await f.ctl.dispose();});
test('real controller applies envelope but readiness waits until renderer.render succeeds; dispose never detaches canonical core',async()=>{
 const f=fixture();let ready=false;const wait=f.ctl.waitForDraw().then(()=>ready=true);await flush();assert.equal(ready,false);assert.equal(f.draws(),0);
 // Incoming legacy stream cannot overwrite the pending captured scene.
 f.ctl.onFrame(new Uint8Array(frame()).buffer);assert.equal(ready,false);
 f.draw();await wait;assert.equal(f.draws(),1);await f.ctl.dispose();assert.equal(f.calls.includes('terminal.detach'),false);
});
test('actual render exception rejects readiness',async()=>{const f=fixture({drawError:true});const wait=f.ctl.waitForDraw();await flush();f.draw();await assert.rejects(wait,/GPU draw failed/);await f.ctl.dispose();});
test('configuration supersession and timeout revoke late snapshot draw',async()=>{
 for(const action of ['config','timeout']) {const f=fixture({late:true});const wait=f.ctl.waitForDraw(action==='timeout'?5:1000);await flush();if(action==='config')f.ctl.setTheme({});await assert.rejects(wait,/superseded|timeout/);f.finish();await flush();f.draw();assert.equal(f.draws(),action==='config'?1:0);await f.ctl.dispose();}
});
test('late envelope after dispose cannot draw or become ready',async()=>{const f=fixture({late:true});const wait=f.ctl.waitForDraw();await flush();await f.ctl.dispose();await assert.rejects(wait,/disposed/);f.finish();await flush();f.draw();assert.equal(f.draws(),0);});
