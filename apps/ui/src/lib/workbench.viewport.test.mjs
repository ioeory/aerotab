import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';
const source=await readFile(new URL('../components/Workbench.svelte',import.meta.url),'utf8');
const layoutSource=await readFile(new URL('./workbench.ts',import.meta.url),'utf8');
const model=await import(`data:text/javascript;base64,${Buffer.from(ts.transpileModule(layoutSource,{compilerOptions:{target:99,module:99}}).outputText).toString('base64')}`);
test('actual Workbench observer converges without scrollbar oscillation at fractional shell zoom',()=>{
 const start=source.indexOf('const obs=new ResizeObserver(');
 const end=source.indexOf(';obs.observe(host)',start);
 const observer=source.slice(start,end)+';return obs;';
 const script=ts.transpileModule(observer,{compilerOptions:{target:99,module:0}}).outputText;
 let callback;const scope={host:{clientWidth:2370,clientHeight:1319,getClientRects:()=>[{}]},meaningfulViewport:false,view:{width:1000,height:600},layout:model.defaultLayout([]),updateWorkbenchViewport:model.updateWorkbenchViewport,ResizeObserver:class{constructor(cb){callback=cb;}}};
 new Function('scope',`with(scope){${script}}`)(scope);
 const sizes=[];
 // Fractional content rounds UP in clientWidth/clientHeight. Assigning that
 // integer to extent overflows, adding 10px scrollbars; shrinking removes them.
 let scrollbars=false;
 for(let i=0;i<12;i++){
  const width=2369.6-(scrollbars?10:0),height=1318.6-(scrollbars?10:0);
  scope.host.clientWidth=Math.round(width);scope.host.clientHeight=Math.round(height);
  callback([{target:scope.host,contentRect:{width,height}}]);
  sizes.push([scope.view.width,scope.view.height]);
  const {extent}=model.measureLayout(scope.layout,scope.view,[]);
  scrollbars=extent.width>2369.6||extent.height>1318.6;
 }
 assert.deepEqual(sizes.at(-1),[2369.6,1318.6]);
 assert.deepEqual(sizes.at(-1),sizes.at(-2),'extent must converge, not alternate by scrollbar thickness');
 assert.equal(scrollbars,false);
});
