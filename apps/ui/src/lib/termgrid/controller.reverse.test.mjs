import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'vite';
const server=await createServer({configFile:false,optimizeDeps:{noDiscovery:true,include:[]},server:{middlewareMode:true}});
const {NativeTerminalController}=await server.ssrLoadModule('/src/lib/termgrid/controller.ts');
await server.close();
function fixture(){
 globalThis.window={devicePixelRatio:1,removeEventListener(){},clearTimeout(){}};
 globalThis.cancelAnimationFrame=()=>{};
 const writes=[];const calls=[];
 const ctl=new NativeTerminalController({sessionId:'s',nativeSurface:true,rpc:{call:async m=>calls.push(m)},canvas:{removeEventListener(){}},writeInput:async d=>writes.push(d)});
 ctl.bindInput=()=>{};ctl.observeResize=()=>{};ctl.startBlink=()=>{};ctl.fit=()=>{};
 ctl.createRenderer=()=>{ctl.renderer={dispose(){},resize(){},render(){}};};
 return {ctl,writes,calls};
}
test('reverse staging retains single owner and rollback replays accepted target writes once',async()=>{
 const {ctl,writes,calls}=fixture();
 await ctl.prepareWebviewSurface();
 ctl.beginWebviewInputHandoff();
 const a=ctl.writePty('a');const b=ctl.writePty('b');assert.deepEqual(writes,[]);
 ctl.rollbackWebviewSurface();await Promise.all([a,b]);assert.deepEqual(writes,['a','b']);assert.equal(ctl.opts.nativeSurface,true);assert.deepEqual(calls,[]);await ctl.dispose();
});
test('paused input is bounded and rejects new bytes visibly without dropping accepted data',async()=>{
 const {ctl,writes}=fixture();const pressure=[];ctl.opts.onInputBackpressure=v=>pressure.push(v);
 await ctl.prepareWebviewSurface();ctl.beginWebviewInputHandoff();
 const accepted=[];for(let i=0;i<256;i++)accepted.push(ctl.writePty('x'));
 await assert.rejects(ctl.writePty('overflow'),/backpressure/);assert.equal(ctl.pausedWrites.length,256);assert.deepEqual(pressure,[true]);
 await ctl.rollbackWebviewSurface();await Promise.all(accepted);assert.equal(writes.length,256);assert.deepEqual(pressure,[true,false]);await ctl.dispose();
});
test('paused UTF-8 byte bound rejects oversized paste before ownership',async()=>{
 const {ctl}=fixture();await ctl.prepareWebviewSurface();ctl.beginWebviewInputHandoff();
 await assert.rejects(ctl.writePty('中'.repeat(90000)),/backpressure/);assert.equal(ctl.pausedWrites.length,0);await ctl.dispose();
});
test('reverse commit resumes queued writes once and allows another native and WebView cycle',async()=>{
 const {ctl,writes,calls}=fixture();
 for(let i=0;i<2;i++){
  await ctl.prepareWebviewSurface();ctl.beginWebviewInputHandoff();const queued=ctl.writePty(`q${i}`);
  ctl.commitWebviewSurface();await queued;await ctl.paste(`p${i}`);
  await ctl.quiesceWebviewInput();ctl.commitNativeSurface();
 }
 assert.deepEqual(writes,['q0','p0','q1','p1']);assert.deepEqual(calls,[]);await ctl.dispose();
});
