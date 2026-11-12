import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { runNativeViewTransaction } from './nativeViewTransaction.ts';
import { createNativeViewHost, nativeViewGeometryEquals } from './nativeViewHost.ts';
const text = readFileSync(new URL('../components/NativeTerminalView.svelte', import.meta.url), 'utf8');
const start = text.indexOf('    let started = false;');
const end = text.indexOf('    const ro = new ResizeObserver', start);
const body = ts.transpileModule(text.slice(start, end) + '\nreturn {sync, notify: () => syncSurface(), requestFocus: () => requestSurfaceFocus()};', {compilerOptions:{target:99,module:0}}).outputText;
function fixture() {
 const events = [];
 const scope = {surface:{getBoundingClientRect:()=>({left:0,top:0,width:600,height:400})},
  nativeViewGeometry:r=>r,nativeViewGeometryEquals,window:{innerWidth:1000,innerHeight:800,devicePixelRatio:1},document:{hidden:false},
  hostAdapter:{setDrawVisible:v=>events.push(['visible',v]),start:async()=>events.push(['start']),
   setFocusAllowed(){},update:async()=>events.push(['update']),acknowledgeInput:async()=>{},waitForDraw:async()=>true,focus:async()=>{events.push(['focus']);return 'applied';}},
  visible:true,occluded:false,globalOccluded:false,disposed:false,surfaceError:'',stagingWebview:false,committedWebview:false,stagingNative:true,
  viewConfig:()=>({fontSize:15}),canFocusSurface:()=>true,surfaceReady:false,controller:{},onReady(){},ackRecovering:false,fail:e=>{throw e;},
  completeNativeFocusHandoff(){},syncSurface:null,requestSurfaceFocus:null};
 const api = new Function('scope', `with(scope){${body}}`)(scope);
 return {scope,api,events};
}
test('explicit focus waits for prop flush and does not discard a tab being shown', async () => {
 const events=[];
 const scope={backend:'native-surface',committedWebview:false,committedNative:false,
  surfaceReady:true,active:false,visible:false,disposed:false,
  tick:async()=>{scope.active=true;scope.visible=true;},
  canTakeFocus:()=>scope.active&&scope.visible,
  canFocusSurface:()=>scope.active&&scope.visible,
  hostAdapter:{setFocusAllowed:v=>events.push(['allowed',v]),focus:async()=>{events.push(['focus']);return 'applied';}},
  controller:null,requestSurfaceFocus:()=>{},fail:e=>{throw e;}};
 const source=text.slice(text.indexOf('  export function focus('),text.indexOf('  /** Await OS view detach'))
  .replace('export function','function');
 const js=ts.transpileModule(source+'\nreturn focus;', {compilerOptions:{target:99,module:0}}).outputText;
 const focus=new Function('scope',`with(scope){${js}}`)(scope);
 await focus({force:true});
 assert.deepEqual(events,[['allowed',true],['focus']]);
});
test('explicit activation retries focus after a busy draw even when already restored', async () => {
 const f=fixture();f.scope.stagingNative=false;
 await f.api.sync();
 assert.equal(f.events.filter(e=>e[0]==='focus').length,1);
 let finishDraw;
 f.scope.hostAdapter.waitForDraw=()=>new Promise(resolve=>{finishDraw=resolve;});
 const busy=f.api.sync();
 f.api.requestFocus();
 f.scope.hostAdapter.waitForDraw=async()=>true;
 finishDraw(true);await busy;
 assert.equal(f.events.filter(e=>e[0]==='focus').length,2);
});
test('superseded activation cannot focus an inactive pane after prop flush', async () => {
 const scope={tick:async()=>{},canFocusSurface:()=>false,fail:e=>{throw e;}};
 const source=text.slice(text.indexOf('  export function focus('),text.indexOf('  /** Await OS view detach')).replace('export function','function');
 const js=ts.transpileModule(source+'\nreturn focus;', {compilerOptions:{target:99,module:0}}).outputText;
 await new Function('scope',`with(scope){${js}}`)(scope)({force:true});
});
test('identical observer notifications do not submit new geometry/config revisions', async () => {
 const f=fixture();await f.api.sync();
 for(let i=0;i<20;i++)await f.api.sync();
 assert.equal(f.events.filter(e=>e[0]==='update').length,1);
 f.scope.viewConfig=()=>({fontSize:16});await f.api.sync();
 assert.equal(f.events.filter(e=>e[0]==='update').length,2,'real font change still updates');
 f.scope.visible=false;await f.api.sync();
 assert.equal(f.events.filter(e=>e[0]==='update').length,3,'hide still reaches host');
});
test('failed staged first draw retains source canvas/input and never commits ownership', async () => {
 const events=[];
 const source={resumeWebviewInput:()=>events.push('resume'),focus:()=>events.push('focus'),
  quiesceWebviewInput:async()=>events.push('drain-source'),commitNativeSurface:()=>events.push('commit')};
 const host={generation:()=> 'g',waitForDraw:async()=>false,quiesceInput:async()=>0,dispose:async()=>events.push('detach-target')};
 const scope={backend:'webview',committedWebview:false,committedNative:false,stagingNative:false,controller:source,
  settlePaneLayout:async()=>{},surface:{getBoundingClientRect:()=>({x:0,y:0,width:600,height:400})},disposed:false,transitionClosing:false,
  viewConfig:()=>({}),traceLayout(){},traceView(){},stagingError:null,visible:true,occluded:false,globalOccluded:false,document:{hidden:false},
  runNativeViewTransaction,mountSurface:async()=>{},hostAdapter:host,inputQueue:{dispose:()=>events.push('queue.dispose')},
  quiesceAndDrainNativeInput:async()=>events.push('drain-target'),cleanupSurface:null,syncSurface:null,nativeViewGeometryEquals,
  unlistenInput:null,unlistenPaste:null,unlistenFontDelta:null,rollbackPending:false,onRecovered:()=>events.push('recovered')};
 const forward=text.slice(text.indexOf('  export async function stageNativeSurface('),text.indexOf('  /** Native -> WebView')).replace('export async','async');
 const js=ts.transpileModule(forward+'\nreturn stageNativeSurface;', {compilerOptions:{target:99,module:0}}).outputText;
 const run=new Function('scope',`with(scope){${js}}`)(scope);
 await assert.rejects(run(),/native staged draw unavailable/);
 assert.equal(scope.hostAdapter,null);assert.equal(scope.stagingNative,false);assert.equal(scope.committedNative,false);
 assert.deepEqual(events,['drain-target','detach-target','queue.dispose','resume','focus','recovered']);
});
test('subpixel width jitter near 1920 does not supersede native staging', async () => {
 const events=[];
 let width=1920;
 const rect=()=>({x:0,y:36,width,height:1044,left:0,top:36});
 const source={resumeWebviewInput:()=>events.push('resume'),focus:()=>events.push('focus'),
  quiesceWebviewInput:async()=>events.push('drain-source'),commitNativeSurface:()=>events.push('commit')};
 const host={generation:()=>'g',waitForDraw:async()=>{width=1920.0004;return true;},
  acknowledgeInput:async()=>events.push('ack0'),quiesceInput:async()=>0,setFocusAllowed(){},dispose:async()=>events.push('detach')};
 const scope={backend:'webview',committedWebview:false,committedNative:false,stagingNative:false,controller:source,
  settlePaneLayout:async()=>{},surface:{getBoundingClientRect:rect},disposed:false,transitionClosing:false,
  viewConfig:()=>({}),traceLayout(){},traceView(){},stagingError:null,visible:true,occluded:false,globalOccluded:false,document:{hidden:false},
  runNativeViewTransaction,mountSurface:async()=>{},hostAdapter:host,inputQueue:{dispose:()=>{}},
  quiesceAndDrainNativeInput:async()=>{},cleanupSurface:null,syncSurface:null,canFocusSurface:()=>true,
  unlistenInput:null,unlistenPaste:null,unlistenFontDelta:null,rollbackPending:false,nativeViewGeometryEquals,
  surfaceReady:false,onRecovered:()=>events.push('recovered')};
 const forward=text.slice(text.indexOf('  export async function stageNativeSurface('),text.indexOf('  /** Native -> WebView')).replace('export async','async');
 const js=ts.transpileModule(forward+'\nreturn stageNativeSurface;', {compilerOptions:{target:99,module:0}}).outputText;
 const run=new Function('scope',`with(scope){${js}}`)(scope);
 await assert.doesNotReject(run());
 assert.equal(scope.committedNative,true);assert.equal(scope.stagingNative,false);
 assert.ok(events.includes('commit'));assert.ok(events.includes('ack0'));
 assert.ok(!events.includes('detach'));assert.ok(!events.includes('resume'));
});
// Execute the production sync closure extracted above against a real host.
// The invoke boundary is simulated; no Win32 calls or desktop are exercised.
test('a skipped host focus must not mark focusRestored and suppress the next sync retry', async () => {
 const f=fixture();
 f.scope.stagingNative=false;
 let canFocus=true;
 let focusAttempts=0;
 let focusIpcs=0;
 const host=createNativeViewHost({sessionId:'a',invoke:async(command)=>{
  if(command==='native_view_capabilities')return {available:true,geometryUnit:'css-px'};
  if(command==='native_view_attach')return {generation:'ga',handoffOutputSeq:'0'};
  if(command==='native_view_update'||command==='native_view_detach')return {};
  if(command==='native_view_focus'){focusIpcs++;return;}
  if(command==='native_view_state')return {generation:'ga',drawnRevision:100,drawnOutputSeq:'0',focused:focusIpcs>0};
  assert.fail(`unexpected command: ${command}`);
 }});
 f.scope.canFocusSurface=()=>canFocus;
 f.scope.hostAdapter={...host,focus(){
  focusAttempts++;
  const pending=host.focus();
  if(focusAttempts===1){
   // The real host defers authorization to its queued microtask. Another pane
   // briefly owns focus before it runs, then A is eligible again before sync's
   // await resumes. No observer sync saw the intervening inactive state.
   canFocus=false;host.setFocusAllowed(false);
   return pending.then(result=>{
    canFocus=true;host.setFocusAllowed(true);
    return result;
   });
  }
  return pending;
 }};
 try{
  await f.api.sync();
  assert.equal(focusAttempts,1);
  assert.equal(focusIpcs,0,'real host skipped focus when permission changed before queue execution');
  await f.api.sync();
  assert.equal(focusIpcs,1,'eligible next sync must retry: skipped focus is not a successful restoration');
 }finally{await host.dispose();}
});

test('an old successful focus completion cannot consume a newer explicit request', async () => {
 const f=fixture();f.scope.stagingNative=false;
 let finish;
 let attempts=0;
 f.scope.hostAdapter.focus=()=>{
  attempts++;
  return attempts===1?new Promise(resolve=>{finish=resolve;}):Promise.resolve('applied');
 };
 const syncing=f.api.sync();
 for(let i=0;i<30&&!finish;i++)await Promise.resolve();
 assert.equal(typeof finish,'function');
 f.api.requestFocus();
 finish('applied');
 await syncing;
 assert.equal(attempts,2,'old completion must leave the new request pending for the dirty pass');
});

test('retained WebView cannot reopen native draw visibility on DOM mutations', async () => {
 const f=fixture();await f.api.sync();f.events.length=0;
 f.scope.committedWebview=true;f.scope.stagingNative=false;
 await f.api.sync();
 assert.equal(f.events.some(e=>e[0]==='visible'&&e[1]),false);
 assert.equal(f.events.some(e=>e[0]==='update'),false);
});
