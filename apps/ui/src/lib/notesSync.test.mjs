import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { selectedSyncGroups } from './syncConfig.ts';

test('Notes requires explicit opt-in and never inherits Appearance', () => {
  assert.ok(!selectedSyncGroups(null).includes('Notes'));
  assert.ok(!selectedSyncGroups({enabledGroups:{Appearance:true}}).includes('Notes'));
  assert.ok(selectedSyncGroups({enabledGroups:{Notes:true}}).includes('Notes'));
});

test('sync after failed draft save aborts, retains text, then retries before apply', async () => {
  const server = await createServer({configFile:false,optimizeDeps:{noDiscovery:true,include:[]},ssr:{optimizeDeps:{noDiscovery:true,include:[]}},server:{middlewareMode:true,hmr:false,ws:false},plugins:[svelte()]});
  try {
    const { getNotesStore } = await server.ssrLoadModule('/src/lib/notes.svelte.ts');
    const calls=[]; const editStates=[]; let fail=true;
    const note={id:'n',parent_id:null,title:'N',markdown:'old',tags:[],profile_ids:[],version:1,updated_at:0};
    const rpc={async call(method,params){
      calls.push(method);
      if(method==='notes.editState') editStates.push(params.dirty);
      if(method==='notes.put') {if(fail) throw Error('disk write failure');Object.assign(note,params.note,{version:note.version+1});return {...note};}
      if(method==='notes.list') return {notes:[{...note}],folders:[],revision:note.version};
      return {};
    }};
    const store=getNotesStore(rpc);
    await store.refresh();
    const draft=store.draft(note);draft.value.markdown='local unsaved text';store.edit('n');
    await assert.rejects(rpc.call('sync.now',{groups:['Notes']}),/draft retained/);
    assert.ok(!calls.includes('sync.now'));
    assert.equal(draft.value.markdown,'local unsaved text');assert.equal(store.dirty,true);
    assert.ok(calls.includes('notes.editState'), 'dirty drafts must gate backend automatic Notes sync');
    fail=false;await rpc.call('sync.now',{groups:['Notes']});
    assert.equal(note.markdown,'local unsaved text');assert.equal(store.dirty,false);
    assert.ok(calls.indexOf('notes.put') < calls.indexOf('sync.now'));
    assert.equal(editStates[0],true);assert.equal(editStates.at(-1),false);
  } finally {await server.close();}
});
