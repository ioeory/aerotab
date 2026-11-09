import test from 'node:test';import assert from 'node:assert/strict';import{readFileSync}from'node:fs';import ts from'typescript';let s='';try{s=readFileSync(new URL('./nativeFloatFallback.ts',import.meta.url),'utf8')}catch{}const m=await import(`data:text/javascript;base64,${Buffer.from(ts.transpileModule(s,{compilerOptions:{target:99,module:99}}).outputText).toString('base64')}`);
test('nonmodal float leaves WebView controller interactive and does not transition it',async()=>{const calls=[];const fallback=m.createFloatFallback({native:()=>false,toWebview:async()=>calls.push('web'),toNative:async()=>calls.push('native')});await fallback.update(true);await fallback.update(false);assert.deepEqual(calls,[]);});
test('OS surface falls back to same-engine WebView and returns only after last float closes',async()=>{const calls=[];let native=true;const fallback=m.createFloatFallback({native:()=>native,toWebview:async()=>{calls.push('web');native=false;},toNative:async()=>{calls.push('native');native=true;}});await fallback.update(true);await fallback.update(true);assert.deepEqual(calls,['web']);await fallback.update(false);assert.deepEqual(calls,['web','native']);});
test('failed native restore stays on the retained WebView across observer/timer notifications', async () => {
 let native = true, attempts = 0;
 const failure = Error('native staged draw unavailable');
 const fallback = m.createFloatFallback({native: () => native,
  toWebview: async () => { native = false; },
  toNative: async () => { attempts++; throw failure; }});
 await fallback.update(true);
 await assert.rejects(fallback.update(false), /native staged draw unavailable/);
 for (let i = 0; i < 20; i++) await assert.rejects(fallback.update(false), /native staged draw unavailable/);
 assert.equal(attempts, 1, 'DOM mutations and the 250ms timer must not repeatedly attach/detach');
 assert.equal(native, false);
 // An actual new float lifecycle permits one fresh restoration attempt.
 await fallback.update(true);
 await assert.rejects(fallback.update(false), /native staged draw unavailable/);
 assert.equal(attempts, 2);
});
test('failed WebView fallback is not retried for the same float state', async () => {
 let attempts = 0;
 const fallback = m.createFloatFallback({native: () => true,
  toWebview: async () => { attempts++; throw Error('draw timeout'); }, toNative: async () => {}});
 for (let i = 0; i < 10; i++) await assert.rejects(fallback.update(true), /draw timeout/);
 assert.equal(attempts, 1);
});
test('float closing while transition is pending serializes return without session destruction',async()=>{let resolve;const calls=[];let native=true;const fallback=m.createFloatFallback({native:()=>native,toWebview:async()=>{calls.push('web');await new Promise(r=>resolve=r);native=false;},toNative:async()=>{calls.push('native');native=true;}});const pending=fallback.update(true);await Promise.resolve();const close=fallback.update(false);resolve();await Promise.all([pending,close]);assert.deepEqual(calls,['web','native']);});
