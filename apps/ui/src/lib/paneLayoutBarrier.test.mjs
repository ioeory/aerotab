import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import ts from 'typescript';
import { registerPaneLayoutBarrier, settlePaneLayout } from './paneLayoutBarrier.ts';
test('actual PaneGrid callback flushes actual syncPaneBoxes and awaits DOM tick before capture',async()=>{
 const text=await readFile(new URL('../components/PaneGrid.svelte',import.meta.url),'utf8');
 const sync=text.slice(text.indexOf('  function syncPaneBoxes('),text.indexOf('  function scheduleSync('));
 const registration=text.slice(text.indexOf('    const unregisterLayout ='),text.indexOf('    const unsubHit ='));
 const js=ts.transpileModule(sync+registration+'\nreturn {settle:()=>settlePaneLayout(source),cached:()=>paneBox};',{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.None}}).outputText;
 let rendered=641;const rect=h=>({x:0,y:0,left:0,top:0,width:857,height:h});
 const pane={dataset:{paneDropPane:'s'},getBoundingClientRect:()=>rect(rendered)};
 const slot={getBoundingClientRect:()=>rect(650)};
 const root={parentElement:null,offsetWidth:857,offsetHeight:650,getBoundingClientRect:()=>rect(650),querySelector:()=>slot,contains:()=>true};
 const source={parentElement:root,closest:()=>pane};
 globalThis.getComputedStyle=()=>({display:'block'});
 const prevRaf=globalThis.requestAnimationFrame;
 globalThis.requestAnimationFrame=(cb)=> { cb(0); return 0; };
 const api=new Function('gridEl','tab','paneBox','CSS','registerPaneLayoutBarrier','settlePaneLayout','source','tick',js)(root,{panes:[{id:'s'}]},{s:{height:641}},{escape:v=>v},registerPaneLayoutBarrier,settlePaneLayout,source,async()=>{rendered=650;});
 await api.settle();assert.equal(api.cached().s.height,650);assert.equal(rendered,650);
 if(prevRaf)globalThis.requestAnimationFrame=prevRaf;else delete globalThis.requestAnimationFrame;
 delete globalThis.getComputedStyle;
});
function fixture(){
 let slotHeight=650,cacheHeight=641,renderedHeight=641,flushes=0;
 const rect=h=>({x:243,y:36.25,width:857,height:h});
 const root={parentElement:null};
 const pane={parentElement:root,getBoundingClientRect:()=>rect(renderedHeight)};
 const slot={getBoundingClientRect:()=>rect(slotHeight)};
 const source={parentElement:pane};
 const unregister=registerPaneLayoutBarrier(root,async node=>{
   assert.equal(node,source);flushes++;cacheHeight=slotHeight;
   await Promise.resolve();renderedHeight=cacheHeight;
   return {pane:pane.getBoundingClientRect(),slot:slot.getBoundingClientRect()};
 });
 return {source,unregister,height:()=>renderedHeight,flushes:()=>flushes,resize:h=>{slotHeight=h;renderedHeight=h;}};
}
test('staging flushes real coordinator cache before capture without waiting for measurement rAF',async()=>{
 const f=fixture();assert.equal(f.height(),641);await settlePaneLayout(f.source);
 assert.equal(f.height(),650);assert.equal(f.flushes(),1);f.unregister();
});
test('barrier does not swallow later transaction resize or silently retry it',async()=>{
 const f=fixture();await settlePaneLayout(f.source);const captured=f.height();f.resize(660);
 assert.notEqual(captured,f.height());assert.equal(f.flushes(),1);f.unregister();
});
test('unsettled layout fails closed rather than capture stale pane',async()=>{
 const root={parentElement:null};const node={parentElement:root};
 const done=registerPaneLayoutBarrier(root,async()=>({pane:{x:0,y:0,width:857,height:641},slot:{x:0,y:0,width:857,height:650}}));
 await assert.rejects(settlePaneLayout(node),/layout.*consistent/);done();
});
test('DPI/CSS subpixel drift within one CSS px still settles',async()=>{
 const root={parentElement:null};const node={parentElement:root};
 const done=registerPaneLayoutBarrier(root,async()=>({pane:{x:0,y:36.25,width:857.4,height:650.6},slot:{x:0.25,y:36,width:857,height:650}}));
 await settlePaneLayout(node);done();
});
