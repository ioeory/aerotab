import test from 'node:test';
import assert from 'node:assert/strict';
import { traceLayout, createViewCausalBuffer } from './viewCausalTrace.ts';
test('layout probe is inert without explicit debug capability (no rect/style reads)', () => {
 globalThis.window={}; const element={getBoundingClientRect(){throw Error('must not read');}};
 assert.doesNotThrow(()=>traceLayout('capture',element));delete globalThis.window;
});
test('layout samples retain only bounded numeric geometry and safe tokens, never DOM text/classes',()=>{
 const b=createViewCausalBuffer();
 b.record({stage:'layout.sample',status:'geometry',layout:[{role:'surface',rect:[243,36.25,857,641],client:[857,641],scroll:[857,641],display:'block',position:'absolute',overflowX:'hidden',overflowY:'hidden',fontSize:13,lineHeight:19.5,paddingY:0,borderY:0,hidden:false,classes:'SECRET',text:'SECRET'}]});
 const e=b.snapshot()[0];assert.equal(e.stage,'layout.sample');assert.equal(e.layout.length,1);
 assert.deepEqual(e.layout[0].rect,[243,36.25,857,641]);assert.ok(!JSON.stringify(e).includes('SECRET'));
});
