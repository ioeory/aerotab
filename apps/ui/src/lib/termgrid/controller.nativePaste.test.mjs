import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';
import { createServer } from 'vite';
const server = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true, include: [] }, server: { middlewareMode: true } });
const { NativeTerminalController } = await server.ssrLoadModule('/src/lib/termgrid/controller.ts');
const { createPasteRequestGate, preparePasteText } = await server.ssrLoadModule('/src/lib/nativePasteBridge.ts');
const { createNativeViewHost } = await server.ssrLoadModule('/src/lib/nativeViewHost.ts');
const { tracePaste } = await server.ssrLoadModule('/src/lib/nativePasteTrace.ts');
const { MODE_BRACKETED_PASTE } = await server.ssrLoadModule('/src/lib/termgrid/frame.ts');
await server.close();
const pane = await readFile(new URL('../../components/TerminalPane.svelte', import.meta.url), 'utf8');
const view = await readFile(new URL('../../components/NativeTerminalView.svelte', import.meta.url), 'utf8');
function production(source, bindings) {
  const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText;
  return new Function(...Object.keys(bindings), js)(...Object.values(bindings));
}
const cfg = { fontFamily: 'mono', fontSize: 13, fontWeight: 400, linePadding: 0, foreground: [], background: [], palette: [] };
const geometry = { x: 0, y: 0, width: 640, height: 480 };
function deferred() { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; }
async function fixture({ confirm = async () => true, occluded = false, delayShow = false, reuse, generation = 'g', delayHide = false, afterTick = async()=>{} } = {}) {
  globalThis.window = { devicePixelRatio: 1, removeEventListener() {}, clearTimeout() {} };
  globalThis.cancelAnimationFrame = () => {};
  const writes = reuse?.writes ?? [], rpcCalls = reuse?.rpcCalls ?? [];
  const ctl = reuse?.ctl ?? new NativeTerminalController({ sessionId: 's', rpc: { call: async (method) => { rpcCalls.push(method); } }, canvas: { removeEventListener() {} }, cursorBlink: false });
  // Real staging reuse: keep the writer registered before quiesce/commit.
  ctl.setWriteInput(async data => { writes.push(data); });
  await ctl.quiesceWebviewInput();
  ctl.commitNativeSurface();
  let hostGeneration = generation, frozen = false;
  let holdShow = false, releaseShow;
  const showStarted = deferred(), hideStarted = deferred(), hideRelease = deferred();
  const host = createNativeViewHost({ sessionId: 's', cfg, invoke: async (cmd,args) => {
    if (cmd === 'native_view_update' && !args.visible && delayHide) { hideStarted.resolve(); await hideRelease.promise; }
    if (cmd === 'native_view_update' && args.visible && holdShow) await new Promise(resolve => { releaseShow = resolve; showStarted.resolve(); });
    return cmd === 'native_view_capabilities'
    ? { available: true, geometryUnit: 'css-px' }
    : { generation: hostGeneration, handoffOutputSeq: '0', drawnOutputSeq: '0', inputFrozen: cmd === 'native_view_state' && frozen, inputAccepted: 0 }; } });
  await host.start(geometry); await host.update(geometry, true, cfg); await host.acknowledgeInput(0);
  const gate = createPasteRequestGate();
  let confirmations = 0;
  // Compile the actual Pane policy+dispatch function, not a reconstructed routing model.
  const pasteText = production(pane.slice(pane.indexOf('  async function pasteText('), pane.indexOf('  async function doPaste()')) + '\nreturn pasteText;', {
    session: { id: 's' }, exited: false, preparePasteText, pasteTrimWhitespace: false, pasteFlattenNewlines: false,
    pasteMultilineWarn: true, appConfirm: async () => { confirmations++; return confirm(); }, i18n: { t: () => 'confirm' },
    useNative: true, nativeCtl: ctl, bracketedPaste: false, term: null, sendSessionInput: () => { throw Error('unexpected xterm dispatch'); },
  });
  let blocked = false;
  const originalCanFocusSurface = production(view.slice(view.indexOf('  function canFocusSurface()'), view.indexOf('  /** Commit this request')) + '\nreturn canFocusSurface;', {
    active: true, visible: true, occluded, globalOccluded: false, document: { hidden: false }, disposed: false, surfaceError: '', ackRecovering:false, rollbackPending:false,
  });
  const canFocusSurface = () => !blocked && originalCanFocusSurface();
  const requestPasteVisibility = production(view.slice(view.indexOf('  function requestPasteVisibility()'),view.indexOf('  async function mountSurface()'))+'\nreturn requestPasteVisibility;',{
    hostAdapter:host,canFocusSurface,releasingInput:false,stagingWebview:false,
    surface:{getBoundingClientRect:()=>({left:0,top:0,width:640,height:480})},nativeViewGeometry:r=>({x:r.left,y:r.top,width:r.width,height:r.height}),
    window:{innerWidth:1000,innerHeight:1000,devicePixelRatio:1},viewConfig:()=>cfg,fail:e=>{throw e;},
  });
  let listener, completion;
  // Execute the real NativeTerminalView event callback and capture its async completion at the prop boundary.
  production(view.slice(view.indexOf('    const unlisten = await listenNativePaste('), view.indexOf('    if (disposed) { unlisten();')) .replace('const unlisten = await', 'const unlisten =') , {
    requestPasteVisibility, tracePaste, listenNativePaste: handler => { listener = handler; }, hostAdapter: host, pasteGate: gate, sessionId: 's', canFocusSurface, surfaceReady: true, tick:afterTick,releasingInput:false,stagingWebview:false,
    onNativePaste: (...args) => (completion = pasteText(...args)), onError: message => { throw Error(message); },
  });
  let requestId = 0;
  return { ctl, writes, rpcCalls, host, gate, showStarted:showStarted.promise, hideStarted:hideStarted.promise, releaseHide:hideRelease.resolve,
    async retireHost() { gate.dispose(); await host.dispose(); },
    confirmations: () => confirmations,
    replace() { hostGeneration = 'replacement'; },
    async hide() { await host.update(geometry,false,cfg); },
    show() { holdShow=delayShow; return host.update(geometry,true,cfg); },
    releaseShow() { holdShow=false; releaseShow?.(); },
    block() { blocked=true; },
    async freeze() { frozen = true; await host.quiesceInput(); },
    async send(text = 'echo safe', requestGeneration = generation, id = ++requestId) {
      completion = undefined; listener({ sessionId: 's', generation:requestGeneration, requestId: id, text }); await completion; await Promise.resolve();
    },
    async close() { gate.dispose(); await host.dispose(); await ctl.dispose(); },
  };
}
test('committed real controller receives policy-approved native paste once through original writer, never fallback RPC', async () => {
  const f = await fixture();
  try { await f.send(); await f.send('echo safe', 'g', 1); assert.deepEqual(f.writes, ['echo safe']); assert.deepEqual(f.rpcCalls, []); }
  finally { await f.close(); }
});
test('retired WebView key, DOM paste, IME input and ordinary controller paste cannot write', async () => {
  const f = await fixture();
  try {
    await f.ctl.onKey({ key: 'a' }); f.ctl.onPaste({ clipboardData: { getData: () => 'dom' }, preventDefault() {} });
    await f.ctl.onImeInput({ inputType: 'insertText', data: 'ime', isComposing: false });
    await f.ctl.onCompositionEnd({ data: 'composed' }); await f.ctl.paste('ordinary');
    assert.deepEqual(f.writes, []);
  } finally { await f.close(); }
});
for (const reason of ['stale-request', 'replaced-host', 'frozen', 'occluded']) {
  test(`native paste rejected for ${reason}`, async () => {
    const f = await fixture({ occluded: reason === 'occluded' });
    try {
      if (reason === 'replaced-host') f.replace();
      if (reason === 'frozen') await f.freeze();
      await f.send('blocked', reason === 'stale-request' ? 'old' : 'g');
      assert.deepEqual(f.writes, []);
    } finally { await f.close(); }
  });
}
for (const approved of [false, true]) {
  test(`long multiline native paste confirmation ${approved ? 'approval' : 'denial'} retains real policy before writer`, async () => {
    const f = await fixture({ confirm: async () => approved }); const raw = 'line\r\n'.repeat(50);
    try { await f.send(raw); assert.equal(f.confirmations(), 1); assert.deepEqual(f.writes, approved ? ['line\r'.repeat(50)] : []); }
    finally { await f.close(); }
  });
}
test('native bracketed paste uses existing encoder and same writer once', async () => {
  const f = await fixture();
  try { f.ctl.mode = MODE_BRACKETED_PASTE; await f.send('one'); assert.deepEqual(f.writes, ['\x1b[200~one\x1b[201~']); }
  finally { await f.close(); }
});
for (const invalidation of ['freeze', 'dispose']) {
  test(`${invalidation} during multiline confirmation prevents write`, async () => {
    let f; f = await fixture({ confirm: async () => {
      if (invalidation === 'freeze') await f.freeze();
      else f.gate.dispose();
      return true;
    } });
    try { await f.send('line\n'.repeat(50)); assert.equal(f.confirmations(), 1); assert.deepEqual(f.writes, []); }
    finally { await f.close(); }
  });
}
test('approved modal closure waits for same-generation visibility RPC before exactly-one writer dispatch',async()=>{
 let f,show;
 f=await fixture({delayShow:true,confirm:async()=>{await f.hide();show=f.show();return true;}});
 try {
  const sent=f.send('line\n'.repeat(50));
  for(let i=0;i<40;i++)await Promise.resolve();
  assert.deepEqual(f.writes,[]);f.releaseShow();await show;await sent;
  assert.deepEqual(f.writes,['line\r'.repeat(50)]);
 }finally{await f.close();}
});
test('stuck visibility restoration expires without writing and late completion cannot replay paste',async()=>{
 const f=await fixture({delayShow:true});
 try{await f.hide();const show=f.show();assert.equal(await f.host.canPaste('g',()=>true,0),false);
 assert.deepEqual(f.writes,[]);f.releaseShow();await show;assert.deepEqual(f.writes,[]);
 }finally{await f.close();}
});
for(const invalidation of ['other-modal','generation','detach','freeze','supersede'])test(`pending restoration ${invalidation} prevents approved paste`,async()=>{
 let f,show;
 f=await fixture({delayShow:true,confirm:async()=>{await f.hide();show=f.show();return true;}});
 try{
  const sent=f.send('line\n'.repeat(50));for(let i=0;i<40;i++)await Promise.resolve();
  let pending;
  if(invalidation==='other-modal')f.block();
  if(invalidation==='generation')f.replace();
  if(invalidation==='detach')pending=f.host.dispose();
  if(invalidation==='freeze')pending=f.freeze();
  if(invalidation==='supersede')f.gate.dispose();
  f.releaseShow();await show;await pending;await sent;assert.deepEqual(f.writes,[]);
 }finally{await f.close();}
});
test('modal close tick marks busy production sync dirty while canPaste waits old hide, not later show',async()=>{
 let f,sync;const entered=deferred();
 const state={busy:false,dirty:false,disposed:false,surfaceError:'',surface:{getBoundingClientRect:()=>({left:0,top:0,width:640,height:480})},
  visible:true,occluded:false,globalOccluded:false,stagingWebview:false,committedWebview:false,stagingNative:false,
  started:true,lastGeometry:geometry,lastConfig:'',inputAcknowledged:false,surfaceReady:true,focusRestored:false,
  nativeViewGeometry:r=>({x:r.left,y:r.top,width:r.width,height:r.height}),viewConfig:()=>cfg,
  canFocusSurface:()=>!state.occluded,document:{hidden:false},window:{innerWidth:1000,innerHeight:1000,devicePixelRatio:1},
  controller:null,onReady:()=>{},fail:e=>{throw e;}};
 f=await fixture({delayHide:true,afterTick:async()=>{sync();},confirm:async()=>{
   state.occluded=true;void sync();await f.hideStarted;state.occluded=false;return true;
 }});
 state.hostAdapter=f.host;
 const source=view.slice(view.indexOf('    const sync = async () => {'),view.indexOf('    syncSurface = () =>'));
 sync=production('with(scope){'+source+'return sync;}',{scope:state});
 const original=f.host.canPaste;f.host.canPaste=async(...args)=>{entered.resolve();return original.apply(f.host,args);};
 try{const sent=f.send('line\n'.repeat(50));await entered.promise;f.releaseHide();await sent;
  assert.deepEqual(f.writes,['line\r'.repeat(50)]);
 }finally{f.releaseHide();await f.close();}
});
test('round-trip same controller/new host then single duplicate stale denial approval and overlapping show',async()=>{
 const first=await fixture(); const ctl=first.ctl;
 ctl.bindInput=()=>{};ctl.observeResize=()=>{};ctl.startBlink=()=>{};ctl.fit=()=>{};
 ctl.createRenderer=()=>{ctl.renderer={dispose(){},resize(){},render(){}};};
 await ctl.prepareWebviewSurface();ctl.beginWebviewInputHandoff();ctl.commitWebviewSurface();
 await first.retireHost();
 let f,show,overlap;let approve=false;let confirmation=deferred();
 f=await fixture({reuse:first,generation:'g2',delayShow:true,confirm:async()=>{
   await f.hide();show=f.show();await f.showStarted;overlap=f.host.update(geometry,true,cfg);
   confirmation.resolve();return approve;
 }});
 try{
  window.__aerotabDebugDrawObserver={pasteTrace:true};
  await f.send('single','g2',10);await f.send('duplicate','g2',10);await f.send('stale','old',11);
  const denied=f.send('line\n'.repeat(50),'g2',12);await confirmation.promise;f.releaseShow();await show;await overlap;await denied;
  approve=true;confirmation=deferred();const approved=f.send('line\n'.repeat(50),'g2',13);
  await confirmation.promise;f.releaseShow();await show;await overlap;await approved;
  assert.deepEqual(f.writes,['single','line\r'.repeat(50)]);assert.equal(f.ctl,ctl);
  const evidence=window.__aerotabNativePasteTrace.snapshot();
  for(const [stage,reason] of [['request','duplicate'],['request','generation'],['confirm','denied'],['confirm','approved'],['authorization','accepted'],['writer','completed']])
   assert.ok(evidence.some(e=>e.stage===stage&&e.reason===reason),`${stage}:${reason}`);
  assert.ok(evidence.every(e=>Object.keys(e).every(k=>['order','stage','reason','request','revision','shown'].includes(k))));
 }finally{f.releaseShow();await f.close();}
});
test('host replacement during confirmation invalidates approved paste', async () => {
  let f; f = await fixture({ confirm: async () => { f.replace(); return true; } });
  try { await f.send('line\n'.repeat(50)); assert.equal(f.confirmations(), 1); assert.deepEqual(f.writes, []); }
  finally { await f.close(); }
});
