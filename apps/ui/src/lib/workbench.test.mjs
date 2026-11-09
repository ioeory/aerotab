import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
let source = ''; try { source = readFileSync(new URL('./workbench.ts', import.meta.url), 'utf8'); } catch {}
const m = await import(`data:text/javascript;base64,${Buffer.from(ts.transpileModule(source, {compilerOptions:{target:99,module:99}}).outputText).toString('base64')}`);
test('stable registered panel IDs survive dock, reorder and float without changing content identity', () => {
 const registry = new m.PanelRegistry(); registry.register({id:'notes:one',type:'notes'}); registry.register({id:'ai:future',type:'ai'});
 assert.throws(()=>registry.register({id:'notes:one',type:'notes'}));
 let layout = m.defaultLayout(registry.ids());
 layout = m.placePanel(layout,'notes:one','left'); layout = m.placePanel(layout,'ai:future','left',0);
 assert.deepEqual(layout.panels.filter(p=>p.side==='left').map(p=>p.id),['ai:future','notes:one']);
 layout = m.placePanel(layout,'notes:one','float'); assert.equal(layout.panels.find(p=>p.id==='notes:one').side,'float'); assert.equal(registry.get('notes:one').type,'notes');
});
test('restore validates versions, duplicates, unknown IDs and nonfinite geometry; serializes layout only',()=>{
 const ids=['notes','system']; const fallback=m.defaultLayout(ids);
 assert.deepEqual(m.restoreLayout({version:99,panels:[]},ids),fallback);
 const restored=m.restoreLayout({version:1,secret:'no',panels:[{id:'notes',side:'float',size:Infinity,rect:{x:-500,y:2,width:NaN,height:4},secret:'no'},{id:'notes',side:'left'},{id:'unknown',side:'right'}]},ids);
 assert.deepEqual(restored.panels.map(p=>p.id),['notes','unknown','system']); assert.equal(JSON.stringify(restored).includes('secret'),false); assert.equal(Number.isFinite(restored.panels[0].size),true);
});
test('float clamp handles tiny viewport and CSS-pixel/high DPI coordinates',()=>{
 assert.deepEqual(m.clampRect({x:-3,y:900,width:1000,height:700},{width:320,height:180}),{x:0,y:0,width:320,height:180});
 const r=m.resizeRect({x:100,y:100,width:400,height:300},'nw',900,900,{width:900,height:700});
 assert.equal(r.x+r.width,500); assert.equal(r.y+r.height,400); assert.ok(r.width>=180); assert.ok(r.height>=100);
});
test('all float resize edges/corners stay in bounds and preserve opposite edge',()=>{
 for(const edge of ['n','s','e','w','ne','nw','se','sw']) {const r=m.resizeRect({x:100,y:100,width:400,height:300},edge,-1000,-1000,{width:800,height:600});assert.ok(r.x>=0&&r.y>=0&&r.x+r.width<=800&&r.y+r.height<=600);}
});
test('layout allocates all four docks without overlap and leaves usable terminal space',()=>{
 let l=m.defaultLayout(['a','b','c','d']); for(const [i,side] of ['left','right','top','bottom'].entries())l=m.placePanel(l,['a','b','c','d'][i],side);
 const boxes=m.measureLayout(l,{width:1200,height:800},['a','b','c','d']);
 assert.ok(boxes.main.width>=240); assert.ok(boxes.main.height>=120);
 for(const p of Object.values(boxes.panels))assert.ok(p.width>0&&p.height>0);
});
