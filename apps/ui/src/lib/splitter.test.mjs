import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import ts from 'typescript';
let src='';try{src=readFileSync(new URL('./splitter.ts',import.meta.url),'utf8')}catch{}
const m=await import(`data:text/javascript;base64,${Buffer.from(ts.transpileModule(src,{compilerOptions:{target:99,module:99}}).outputText).toString('base64')}`);
test('resize transaction clamps preview, commits once, and cancel restores start',()=>{
 const values=[],ends=[];const t=m.resizeTransaction(200,100,400,v=>values.push(v),v=>ends.push(v));t.move(900);t.cancel();t.commit();assert.deepEqual(values,[400,200]);assert.deepEqual(ends,[]);
 const c=m.resizeTransaction(200,100,400,v=>values.push(v),v=>ends.push(v));c.move(-900);c.commit();c.commit();assert.deepEqual(ends,[100]);
});
test('keyboard splitter respects orientation, step and Home/End bounds',()=>{assert.equal(m.keyboardResize('ArrowRight',200,100,400,'x',false),210);assert.equal(m.keyboardResize('ArrowUp',200,100,400,'x',false),null);assert.equal(m.keyboardResize('End',200,100,400,'y',false),400);assert.equal(m.keyboardResize('ArrowDown',200,100,400,'y',true),250)});
