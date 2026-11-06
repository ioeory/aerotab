import test from 'node:test';
import assert from 'node:assert/strict';
import { tracePaste } from './nativePasteTrace.ts';
test('paste diagnostics default off and bounded allowlist survives failures without text',()=>{
 globalThis.window={};tracePaste('request','accepted',1);assert.equal(window.__aerotabNativePasteTrace,undefined);
 window.__aerotabDebugDrawObserver={pasteTrace:true};
 for(let i=0;i<300;i++)tracePaste('request','secret clipboard',i);
 const b=window.__aerotabNativePasteTrace;const rows=b.snapshot();assert.equal(rows.length,256);
 assert.equal(rows[0].reason,'unknown');assert.deepEqual(Object.keys(rows[0]).sort(),['order','reason','request','stage']);
 rows[0].reason='mutated';assert.equal(b.snapshot()[0].reason,'unknown');
 tracePaste('visibility-update','start',undefined,{revision:42,shown:true,text:'secret'});
 assert.deepEqual(b.snapshot().at(-1),{order:301,stage:'visibility-update',reason:'start',revision:42,shown:true});
 assert.doesNotThrow(()=>tracePaste('writer','accepted',2));delete globalThis.window;
});
