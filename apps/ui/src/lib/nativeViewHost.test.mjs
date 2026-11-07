import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

async function api() {
  let mod;
  try { mod = await import('./nativeViewHost.ts'); } catch (error) {
    if (error.code !== 'ERR_MODULE_NOT_FOUND') throw error;
  }
  assert.equal(typeof mod?.createNativeViewHost, 'function', 'native host adapter must exist');
  // Old lifecycle mocks predate the output envelope. Supply an explicit zero
  // capture/draw for those empty-core fixtures, while preserving test watermarks.
  return {...mod,createNativeViewHost: options=>mod.createNativeViewHost({...options,invoke:async(cmd,args)=>{
    const state=await options.invoke(cmd,args);
    return cmd==='native_view_capabilities' ? state : {handoffOutputSeq:'0',drawnOutputSeq:'0',...state};
  }})};
}
const geometry = { x: 10, y: 20, width: 640, height: 480 };
const capabilities = { available: true, backend: 'gtk-glarea', geometryUnit: 'css-px' };
const flush = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); };
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { resolve, promise }; };

test('clipboard generation verifies live host replacement and invalidates on dispose', async () => {
  const { createNativeViewHost } = await api();
  let generation = 'g';
  const host = createNativeViewHost({ sessionId: 's', invoke: async (cmd) => {
    if (cmd === 'native_view_capabilities') return capabilities;
    return { generation };
  } });
  assert.equal(host.generation(), null);
  await host.start(geometry);
  assert.equal(host.generation(), 'g');
  assert.equal(await host.isCurrentGeneration('g'), true);
  generation = 'replacement';
  assert.equal(await host.isCurrentGeneration('g'), false);
  await host.dispose();
  assert.equal(host.generation(), null);
  assert.equal(await host.isCurrentGeneration('replacement'), false);
});

test('input handshake follows attach and persists monotonic ack through style updates', async () => {
  const { createNativeViewHost } = await api();
  const updates=[];
  const cfg={fontFamily:'mono',fontSize:13,fontWeight:400,linePadding:0,foreground:[1,2,3],background:[0,0,0],palette:[]};
  const host=createNativeViewHost({sessionId:'s',cfg,invoke:async(cmd,args)=>{
    if(cmd==='native_view_capabilities')return capabilities;
    if(cmd==='native_view_update')updates.push(args);
    return {generation:'g'};
  }});
  await host.start(geometry); await host.update(geometry,true,cfg);
  assert.equal(updates[0].cfg.inputReady,undefined);
  await host.acknowledgeInput(0);
  assert.equal(updates.at(-1).cfg.inputReady,true);
  assert.equal(updates.at(-1).cfg.inputGeneration,'g');
  await host.acknowledgeInput(3); await host.acknowledgeInput(1);
  await host.update(geometry,true,{...cfg,fontSize:14});
  assert.equal(updates.at(-1).cfg.inputAck,3);
  await host.dispose();
  await assert.rejects(host.acknowledgeInput(4));
});

test('ACK recovery gates native readiness and paste without thawing freeze or showing suspended owner',async()=>{
 const {createNativeViewHost}=await api();const updates=[];
 const host=createNativeViewHost({sessionId:'s',invoke:async(cmd,args)=>{
  if(cmd==='native_view_capabilities')return capabilities;
  if(cmd==='native_view_update')updates.push(args);
  return {generation:'g',inputFrozen:false,inputAccepted:1};
 }});
 await host.start(geometry);await host.update(geometry,true);await host.acknowledgeInput(0);
 await host.setInputRecovery(true);assert.equal(updates.at(-1).cfg.inputReady,false);assert.equal(await host.canPaste('g'),false);
 await host.suspendForWebview();await host.acknowledgeInput(1);assert.equal(updates.at(-1).visible,false);assert.equal(updates.at(-1).cfg.inputReady,false);
 await host.setInputRecovery(false);assert.equal(updates.at(-1).visible,false);
 await host.restoreAfterWebview();assert.equal(await host.canPaste('g'),true);await host.dispose();
});

test('quiesce freezes input before reading watermark and ACK cannot reopen it', async()=>{
  const {createNativeViewHost}=await api(); const updates=[];
  const cfg={fontFamily:'mono',fontSize:13,fontWeight:400,linePadding:0,foreground:[],background:[],palette:[]};
  const host=createNativeViewHost({sessionId:'s',cfg,invoke:async(cmd,args)=>{
    if(cmd==='native_view_capabilities')return capabilities;
    if(cmd==='native_view_update')updates.push(args.cfg);
    return {generation:'g',inputAccepted:3,inputAck:0,inputFrozen:true};
  }});
  await host.start(geometry); await host.update(geometry,true,cfg); await host.acknowledgeInput(0);
  assert.equal(await host.quiesceInput(),3);
  assert.equal(updates.at(-1).inputReady,false);
  await host.acknowledgeInput(3);
  assert.equal(updates.at(-1).inputReady,false);
  assert.equal(updates.at(-1).inputFrozen,true);
  assert.equal(host.generation(),'g'); await host.dispose();
});

test('reversible WebView suspension hides then captures watermark without freezing generation; rollback restores same widget', async()=>{
  const {createNativeViewHost}=await api();const updates=[];
  const cfg={fontFamily:'mono',fontSize:13,fontWeight:400,linePadding:0,foreground:[],background:[],palette:[]};
  const host=createNativeViewHost({sessionId:'s',cfg,invoke:async(cmd,args)=>{
    if(cmd==='native_view_capabilities')return capabilities;
    if(cmd==='native_view_update')updates.push(args);
    return {generation:'g',inputAccepted:4,inputFrozen:false};
  }});
  await host.start(geometry);await host.update(geometry,true,cfg);await host.acknowledgeInput(0);
  assert.equal(await host.suspendForWebview(),4);
  assert.equal(updates.at(-1).visible,false);assert.equal(updates.at(-1).cfg.inputFrozen,false);
  await host.restoreAfterWebview();assert.equal(updates.at(-1).visible,true);assert.equal(host.generation(),'g');
  assert.equal(await host.canPaste('g'),true);await host.dispose();
});

test('dock occlusion racing an in-flight focus does not destroy the attachment', async () => {
  const { createNativeViewHost } = await api();
  const pending = deferred(); let focusStarted = false;
  const host = createNativeViewHost({ sessionId: 's', invoke: async cmd => {
    if (cmd === 'native_view_capabilities') return capabilities;
    if (cmd === 'native_view_focus') { focusStarted = true; await pending.promise; throw { code: 'input_not_ready', message: 'inactive or frozen attachment' }; }
    return { generation: 'g' };
  }});
  await host.start(geometry); await host.update(geometry, true); await host.acknowledgeInput(0);
  host.setFocusAllowed(true); const focusing = host.focus(); await flush(); assert.equal(focusStarted, true);
  host.setFocusAllowed(false); await host.update(geometry, false);
  pending.resolve(); await assert.doesNotReject(focusing);
  assert.equal(host.generation(), 'g');
  await host.dispose();
});

// GTK consumes logical/CSS bounds, not physical backing-buffer pixels.
test('geometry remains CSS pixels at high DPR and hides invalid or clipped bounds', async () => {
  const { nativeViewGeometry } = await api();
  const rect = { left: 10.25, top: 20.5, width: 640.25, height: 480.5 };
  assert.deepEqual(nativeViewGeometry(rect, { width: 1920, height: 1080 }, 2), {
    x: 10.25, y: 20.5, width: 640.25, height: 480.5,
  });
  for (const invalid of [
    { ...rect, left: -1 }, { ...rect, top: NaN }, { ...rect, width: 0 },
    { ...rect, width: 20000 }, { ...rect, left: 1900 },
  ]) assert.equal(nativeViewGeometry(invalid, { width: 1920, height: 1080 }, 2), null);
});

test('unavailable capability fails explicitly before any attach', async () => {
  const { createNativeViewHost } = await api();
  const calls = [];
  const host = createNativeViewHost({ sessionId: 'same-core', invoke: async (cmd) => {
    calls.push(cmd); return { available: false, reason: 'unsupported_platform' };
  } });
  await assert.rejects(host.start(geometry), /native_surface_unavailable.*unsupported_platform/);
  assert.deepEqual(calls, ['native_view_capabilities']);
});

test('missing capability command does not try attach or compatibility fallback', async () => {
  const { createNativeViewHost } = await api();
  const calls = [];
  const host = createNativeViewHost({ sessionId: 'same-core', invoke: async (cmd) => {
    calls.push(cmd); throw new Error('command not found');
  } });
  await assert.rejects(host.start(geometry), /native_surface_unavailable.*command not found/);
  assert.deepEqual(calls, ['native_view_capabilities']);
});

test('lifecycle only attaches views, coalesces updates and never detaches core', async () => {
  const { createNativeViewHost } = await api();
  const calls = [];
  const pending = deferred();
  const host = createNativeViewHost({ sessionId: 'same-core', invoke: async (cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === 'native_view_capabilities') return capabilities;
    if (cmd === 'native_view_update' && calls.filter(c => c[0] === cmd).length === 1) await pending.promise;
    return { error: null };
  } });
  await host.start(geometry);
  const first = host.update(geometry, true);
  await flush();
  host.update({ ...geometry, width: 700 }, true);
  const last = host.update({ ...geometry, width: 800 }, false);
  pending.resolve();
  await Promise.all([first, last]);
  await host.dispose(); await host.dispose();
  assert.deepEqual(calls.map(c => c[0]), [
    'native_view_capabilities', 'native_view_attach', 'native_view_update', 'native_view_update', 'native_view_detach',
  ]);
  const {cfg,...lastArgs}=calls.at(-2)[1];
  assert.deepEqual(lastArgs, { sessionId: 'same-core', geometry: { ...geometry, width: 800 }, visible: false });
  assert.equal(cfg.revision,3);
});

test('unmount during in-flight attach detaches late view without touching engine', async () => {
  const { createNativeViewHost } = await api();
  const attach = deferred();
  const calls = [];
  const host = createNativeViewHost({ sessionId: 'same-core', invoke: async (cmd) => {
    calls.push(cmd);
    if (cmd === 'native_view_capabilities') return capabilities;
    if (cmd === 'native_view_attach') return attach.promise;
    return { error: null };
  } });
  const started = host.start(geometry);
  await flush();
  const stopped = host.dispose();
  attach.resolve({ error: null });
  await Promise.all([started, stopped]);
  assert.deepEqual(calls, ['native_view_capabilities', 'native_view_attach', 'native_view_detach']);
});

test('host errors are propagated with backend code and view detached', async () => {
  const { createNativeViewHost } = await api();
  const calls = [];
  const host = createNativeViewHost({ sessionId: 'same-core', invoke: async (cmd) => {
    calls.push(cmd);
    if (cmd === 'native_view_capabilities') return capabilities;
    return { error: { code: 'gl_context', message: 'context lost' } };
  } });
  await assert.rejects(host.start(geometry), /native_surface_unavailable.*gl_context.*context lost/);
  await host.dispose();
  assert.equal(calls.at(-1), 'native_view_detach');
});

test('configuration survives coalesced geometry-only updates and attach sends exact font fields', async () => {
  const { createNativeViewHost } = await api();
  const cfg = { fontFamily: 'JetBrains Mono', fontSize: 13, fontWeight: 500, linePadding: 2,
    foreground: [1, 2, 3], background: [4, 5, 6], palette: Array.from({ length: 16 }, () => [1, 2, 3]) };
  const pending = deferred(); const calls = [];
  const host = createNativeViewHost({ sessionId: 'same-core', cfg, invoke: async (cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === 'native_view_capabilities') return capabilities;
    if (cmd === 'native_view_update' && calls.filter(c => c[0] === cmd).length === 1) await pending.promise;
    return { error: null };
  } });
  await host.start(geometry);
  assert.deepEqual(calls[1][1].cfg, cfg);
  const first = host.update(geometry, true);
  await flush();
  host.update(geometry, true, { ...cfg, fontSize: 18 });
  const last = host.update({ ...geometry, width: 800 }, true);
  pending.resolve(); await Promise.all([first, last]);
  assert.equal(calls.at(-1)[1].cfg.fontSize, 18);
  assert.equal(calls.at(-1)[1].geometry.width, 800);
  await host.dispose();
});

test('focus waits for visibility update and is suppressed for inactive, hidden, occluded or released view', async () => {
  const { createNativeViewHost } = await api();
  const calls = []; const pending = deferred();
  const host = createNativeViewHost({ sessionId: 'same-core', invoke: async (cmd) => {
    calls.push(cmd);
    if (cmd === 'native_view_capabilities') return capabilities;
    if (cmd === 'native_view_update') await pending.promise;
    return { error: null };
  } });
  await host.focus();
  await host.start(geometry);
  host.setFocusAllowed(false); await host.focus();
  assert.equal(calls.includes('native_view_focus'), false);
  const update = host.update(geometry, true);
  host.setFocusAllowed(true);
  const focus = host.focus(); await flush();
  assert.equal(calls.includes('native_view_focus'), false);
  host.setFocusAllowed(false); // active pane/modal changed during the pending show.
  pending.resolve(); await Promise.all([update, focus]);
  assert.equal(calls.includes('native_view_focus'), false);
  host.setFocusAllowed(true); await host.focus();
  assert.equal(calls.filter(c => c === 'native_view_focus').length, 1);
  await host.update(geometry, false); await host.focus();
  assert.equal(calls.filter(c => c === 'native_view_focus').length, 1);
  await host.dispose(); await host.focus();
  assert.equal(calls.at(-1), 'native_view_detach');
});

test('release waits for in-flight focus before detaching view', async () => {
  const { createNativeViewHost } = await api();
  const pending = deferred(); const calls = [];
  const host = createNativeViewHost({ sessionId: 'same-core', invoke: async (cmd) => {
    calls.push(cmd);
    if (cmd === 'native_view_capabilities') return capabilities;
    if (cmd === 'native_view_focus') await pending.promise;
    return { error: null };
  } });
  await host.start(geometry); await host.update(geometry, true);
  host.setFocusAllowed(true);
  const focused = host.focus(); await flush();
  const stopped = host.dispose(); await flush();
  assert.notEqual(calls.at(-1), 'native_view_detach');
  pending.resolve(); await Promise.all([focused, stopped]);
  assert.equal(calls.at(-1), 'native_view_detach');
});

test('first draw gate stays pending after attach and accepts sequence zero', async () => {
  const { createNativeViewHost } = await api();
  let seq = null;
  let drawnRevision = null;
  const host = createNativeViewHost({ sessionId: 'same-core', invoke: async cmd =>
    cmd === 'native_view_capabilities' ? capabilities : { generation:'g', error: null, lastDrawSeq: seq, drawnRevision } });
  await host.start(geometry); await host.update(geometry, true);
  let ready = false;
  const wait = host.waitForDraw({ timeoutMs: 200, pollMs: 1 }).then(value => { ready = value; });
  await flush(); assert.equal(ready, false);
  seq = 0; drawnRevision = 1; await wait; assert.equal(ready, true);
  await host.dispose();
});

test('configuration and geometry readiness require matching epoch and newest revision, not wire seq', async()=>{
  const {createNativeViewHost}=await api(); const updates=[];
  let drawnRevision=1; let epoch='g';
  const host=createNativeViewHost({sessionId:'s',invoke:async(cmd,args)=>{
    if(cmd==='native_view_capabilities')return capabilities;
    if(cmd==='native_view_update')updates.push(args);
    return {generation:epoch,lastDrawSeq:0,drawnRevision};
  }});
  await host.start(geometry); await host.update(geometry,true);
  assert.equal(await host.waitForDraw({timeoutMs:50,pollMs:1}),true);
  await host.update({...geometry,width:641},true);
  assert.equal(updates.at(-1).cfg.expectedGeneration,'g');
  assert.equal(updates.at(-1).cfg.revision,2);
  let ready=false; const wait=host.waitForDraw({timeoutMs:300,pollMs:1}).then(v=>ready=v);
  await flush(); assert.equal(ready,false);
  drawnRevision=2; epoch='old'; await new Promise(r=>setTimeout(r,5)); assert.equal(ready,false);
  epoch='g'; await wait; assert.equal(ready,true); await host.dispose();
});

test('in-flight old draw response cannot acknowledge a newer requested update', async()=>{
  const {createNativeViewHost}=await api(); const stale=deferred(); let polls=0;
  const host=createNativeViewHost({sessionId:'s',invoke:async cmd=>{
    if(cmd==='native_view_capabilities')return capabilities;
    if(cmd==='native_view_state' && ++polls===1)return stale.promise;
    return {generation:'g',drawnRevision:2,lastDrawSeq:0};
  }});
  await host.start(geometry); await host.update(geometry,true);
  let ready=false; const wait=host.waitForDraw({timeoutMs:100,pollMs:1}).then(v=>ready=v);
  await flush(); await host.update({...geometry,height:481},true);
  stale.resolve({generation:'g',drawnRevision:1,lastDrawSeq:0}); await flush(); assert.equal(ready,false);
  await wait; assert.equal(ready,true); await host.dispose();
});

test('native output target is finite canonical capture with lossless u64 comparison', async()=>{
  const {createNativeViewHost}=await api();
  let drawn='9007199254740992'; let epoch='g'; let liveTarget='9007199254740993';
  const host=createNativeViewHost({sessionId:'s',invoke:async cmd=>{
    if(cmd==='native_view_capabilities')return capabilities;
    return {generation:epoch,handoffOutputSeq:liveTarget,drawnOutputSeq:drawn,drawnRevision:1,lastDrawSeq:0};
  }});
  await host.start(geometry); await host.update(geometry,true);
  let ready=false; const wait=host.waitForDraw({timeoutMs:300,pollMs:1}).then(v=>ready=v);
  await flush(); assert.equal(ready,false);
  liveTarget='9007199254741993'; // continuing output must not move the captured target
  drawn='9007199254740993'; epoch='old'; await new Promise(r=>setTimeout(r,5)); assert.equal(ready,false);
  epoch='g'; await wait; assert.equal(ready,true); await host.dispose();
});

test('output watermark rejects imprecise numbers, malformed values and u64 overflow', async()=>{
  const {nativeOutputSeq}=await import('./nativeViewHost.ts');
  assert.equal(nativeOutputSeq('9007199254740993'),9007199254740993n);
  for(const value of [9007199254740993,'01','-1','18446744073709551616',null]) assert.throws(()=>nativeOutputSeq(value));
  const {createNativeViewHost}=await import('./nativeViewHost.ts');
  const host=createNativeViewHost({sessionId:'s',invoke:async cmd=>cmd==='native_view_capabilities'?capabilities:{generation:'g'}});
  await assert.rejects(host.start(geometry),/invalid native output watermark/); await host.dispose();
});

test('draw gate fails immediately on host error and has an explicit finite timeout', async () => {
  const { createNativeViewHost } = await api();
  for (const error of [null, { code: 'gl_context', message: 'lost' }]) {
    const host = createNativeViewHost({ sessionId: 'same-core', invoke: async cmd =>
      cmd === 'native_view_capabilities' ? capabilities : cmd === 'native_view_state'
        ? { error, lastDrawSeq: null } : { error: null } });
    await host.start(geometry); await host.update(geometry, true);
    await assert.rejects(host.waitForDraw({ timeoutMs: 10, pollMs: 1 }), error ? /gl_context.*lost/ : /first_draw_timeout/);
    await host.dispose();
  }
});

test('hidden and dispose cancel even a stuck draw-state request without polling forever', async () => {
  const { createNativeViewHost } = await api();
  for (const action of ['hidden', 'dispose']) {
    const stuck = deferred(); let polls = 0;
    const host = createNativeViewHost({ sessionId: 'same-core', invoke: async cmd => {
      if (cmd === 'native_view_capabilities') return capabilities;
      if (cmd === 'native_view_state') { polls++; return stuck.promise; }
      return { error: null };
    } });
    await host.start(geometry); await host.update(geometry, true);
    const wait = host.waitForDraw({ timeoutMs: 100, pollMs: 1 });
    await flush();
    if (action === 'hidden') host.setDrawVisible(false); else await host.dispose();
    assert.equal(await wait, false);
    stuck.resolve({ error: null, lastDrawSeq: 0 }); await flush();
    assert.equal(polls, 1);
    await host.dispose();
  }
});

test('native surface controller skips hidden canvas renderer, global input and resize', async () => {
  const source = await readFile(new URL('./termgrid/controller.ts', import.meta.url), 'utf8');
  const start = source.slice(source.indexOf('  async start()'), source.indexOf('  setTheme('));
  assert.match(start, /if \(!this\.opts\.nativeSurface\) this\.createRenderer\(\)/);
  assert.match(start, /if \(!this\.opts\.nativeSurface\)\s*\{\s*this\.bindInput\(\)/);
  assert.ok(start.indexOf('terminal.attach') < start.indexOf('connectFrames'));
  assert.match(source, /fit\(\)\s*\{\s*if \(this\.disposed \|\| this\.opts\.nativeSurface\) return/);
  assert.match(source, /focus\(\)\s*\{\s*if \(this\.opts\.nativeSurface\) return/);
});
