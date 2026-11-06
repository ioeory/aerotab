import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import ts from 'typescript';
async function store(){const src=await readFile(new URL('./terminalUi.svelte.ts',import.meta.url),'utf8');const js=ts.transpileModule(src,{compilerOptions:{module:ts.ModuleKind.CommonJS}}).outputText;const exports={};new Function('exports','require','$state',js)(exports,()=>({traceView(){}}),v=>v);return exports.terminalUi;}
test('real store keeps request pending despite retained source ready until matching commit',async()=>{const s=await store();s.report('s','native');s.beginRequest('s');s.report('s','native');assert.equal(s.bySession.s,'pending');const token=s.requestEpoch('s');assert.equal(s.finishRequest('s',token,'native-surface'),true);assert.equal(s.bySession.s,'native-surface');});
test('rollback settles only current request, permits retry, stale/closed callback cannot settle newer request',async()=>{const s=await store();s.beginRequest('s');const first=s.requestEpoch('s');assert.equal(s.finishRequest('s',first,'native'),true);s.beginRequest('s');const second=s.requestEpoch('s');assert.notEqual(first,second);assert.equal(s.finishRequest('s',first,'native'),false);assert.equal(s.bySession.s,'pending');s.clear('s');assert.equal(s.finishRequest('s',second,'native'),false);assert.equal(s.bySession.s,undefined);});
