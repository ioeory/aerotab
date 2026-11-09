<script lang="ts">
  import { onMount } from 'svelte';
  import type { RpcClient } from '../../lib/rpc';
  import type { StoredProfile } from '../../lib/types';
  import { getNotesStore } from '../../lib/notes.svelte';
  import { nm } from '../../lib/notesMessages';
  import { exportFolder, importNotes } from '../../lib/notesFiles';
  import Splitter from '../Splitter.svelte';
  let treeSize = $state(230);
  import NotesTree from './NotesTree.svelte';
  import NoteEditor from './NoteEditor.svelte';
  let { rpc, profiles = [], onError, profileId = null, compact = false }: { rpc: RpcClient; profiles?: StoredProfile[]; onError: (message: string) => void; profileId?: string | null; compact?: boolean } = $props();
  const store = $derived(getNotesStore(rpc));
  let selected = $state<string | null>(null), folder = $state<string | null>(null), query = $state(''), currentOnly = $state(false);
  let dialog = $state<{ action: string; id?: string; isFolder?: boolean; titles?: string[]; resolve?: (yes: boolean) => void } | null>(null);
  let name = $state(''), parent = $state('');
  let selectedNote = $derived(store.snapshot.notes.find(note => note.id === selected));
  onMount(() => { void store.refresh().catch(report); });
  function report(error: unknown): void { onError(String(error)); }
  async function select(id: string): Promise<void> {
    // Retain unsaved drafts in the shared store if flushing fails; selection is not blocked.
    if (selected) await store.save(selected).catch(report); selected = id;
  }
  async function create(isFolder: boolean): Promise<void> { name = isFolder ? nm('newFolder') : nm('newNote'); parent = folder ?? ''; dialog = { action: 'create', isFolder }; }
  async function action(action: 'rename' | 'move' | 'remove', id: string, isFolder: boolean): Promise<void> {
    const item = isFolder ? store.snapshot.folders.find(f => f.id === id)! : store.snapshot.notes.find(n => n.id === id)!;
    if (action === 'remove') {
      await store.remove(id, item.version, impact => new Promise<boolean>(resolve => { dialog = { action: 'remove', titles: [...impact.notes.map(n => n.title), ...store.snapshot.folders.filter(f => impact.folders.includes(f.id)).map(f => `${nm('folder')}: ${f.name}`)], resolve }; }));
      return;
    }
    name = isFolder ? store.snapshot.folders.find(f => f.id === id)!.name : store.snapshot.notes.find(n => n.id === id)!.title;
    parent = item.parent_id ?? ''; dialog = { action, id, isFolder };
  }
  async function submit(): Promise<void> {
    const task = dialog; if (!task) return;
    if (task.action === 'remove') { task.resolve?.(true); dialog = null; return; }
    try {
      if (task.action === 'create') { const id = await store.create(parent || null, name, task.isFolder); if (!task.isFolder) selected = id; else folder = id; }
      else if (task.isFolder) {
        const item = store.snapshot.folders.find(f => f.id === task.id)!;
        await store.putFolder({ ...item, name: task.action === 'rename' ? name : item.name, parent_id: task.action === 'move' ? parent || null : item.parent_id });
      } else {
        const note = store.snapshot.notes.find(n => n.id === task.id)!;
        if (task.action === 'rename') { const draft = store.draft(note); draft.value.title = name; store.edit(note.id); await store.save(note.id); }
        else await store.move(note, parent || null);
      }
      dialog = null;
    } catch (error) { report(error); }
  }
</script>
<section class:compact class="workspace" aria-label={nm('title')}>
  <header><h2>{nm('title')}</h2><p>{nm('warning')}</p></header>
  <div class="commands">
    <button onclick={() => create(false)}>{nm('newNote')}</button><button onclick={() => create(true)}>{nm('newFolder')}</button>
    <button onclick={() => importNotes(store, folder).catch(report)}>{nm('import')}</button>
    <button onclick={() => store.flush().then(() => exportFolder(store.snapshot, folder)).catch(report)}>{nm('exportFolder')}</button>
    {#if profileId}<label><input type="checkbox" bind:checked={currentOnly} />{nm('current')}</label>{/if}
  </div>
  <div class="layout"><aside style={compact ? `height:${treeSize}px;max-height:65%;` : `width:${treeSize}px;max-width:65%;`}>
    <input class="search" aria-label={nm('search')} placeholder={nm('search')} bind:value={query} />
    <NotesTree snapshot={store.snapshot} {selected} {folder} {query} profileId={currentOnly ? profileId : null} onselect={id => { void select(id); }} onfolder={id => folder = id} onaction={(a, id, f) => { void action(a, id, f).catch(report); }} />
  </aside><Splitter axis={compact ? 'y' : 'x'} value={treeSize} min={100} max={600} label={nm('title')} onChange={v=>treeSize=v} storageKey={compact ? 'notes.compact.tree' : 'notes.tree'} /><main>{#if selectedNote}<NoteEditor {store} note={selectedNote} {profiles} {onError} />{:else}<p class="empty">{nm('empty')}</p>{/if}</main></div>
  {#if dialog}<div class="dialog" role="dialog" aria-modal="true" aria-label={dialog.action === 'remove' ? nm('impact') : nm('edit')}>
    <form onsubmit={event => { event.preventDefault(); void submit(); }}>
      {#if dialog.action === 'remove'}<h3>{nm('impact')}</h3><ul>{#each dialog.titles ?? [] as title}<li>{title}</li>{/each}</ul>
      {:else}
        {#if dialog.action !== 'move'}<label>{nm('name')}<input required maxlength="512" bind:value={name} /></label>{/if}
        {#if dialog.action !== 'rename'}<label>{nm('parent')}<select bind:value={parent}><option value="">{nm('root')}</option>{#each store.snapshot.folders as f (f.id)}<option value={f.id} disabled={f.id === dialog.id}>{f.name}</option>{/each}</select></label>{/if}
      {/if}
      <footer><button type="button" onclick={() => { dialog?.resolve?.(false); dialog = null; }}>{nm('cancel')}</button><button type="submit">{nm('confirm')}</button></footer>
    </form>
  </div>{/if}
</section>
<style>
  .workspace { display:flex; flex-direction:column; height:100%; min-height:0; position:relative; color:var(--color-fg); }
  header { padding:14px 16px 6px; } h2 { margin:0 0 5px; font-size:18px; } header p { margin:0; opacity:.65; font-size:12px; }
  .commands { display:flex; flex-wrap:wrap; align-items:center; gap:6px; padding:10px 16px; }
  button,input,select { color:inherit; background:var(--color-panel-2); border:1px solid var(--color-border); border-radius:5px; padding:6px 9px; }
  button { cursor:pointer; font-size:12px; } .layout { display:flex; flex:1; min-height:0; } aside { width:230px; flex-shrink:0; overflow:auto; border-right:1px solid var(--color-border); } main { flex:1; min-width:0; min-height:0; }
  .search { margin:8px; width:calc(100% - 16px); box-sizing:border-box; } .empty { padding:24px; opacity:.6; }
  .dialog { position:absolute; inset:0; z-index:10; background:var(--color-overlay); display:flex; align-items:center; justify-content:center; padding:16px; }
  form { background:var(--color-panel-2); padding:20px; border:1px solid var(--color-border); border-radius:9px; max-height:90%; overflow:auto; width:min(420px,100%); } form label { display:flex; flex-direction:column; gap:8px; margin-bottom:12px; } footer { display:flex; justify-content:flex-end; gap:8px; }
  .compact .layout { flex-direction:column; } .compact aside { width:100%; border-right:0; border-bottom:1px solid var(--color-border); } .compact header { padding:8px; } .compact .commands { padding:8px; } .compact main { overflow:auto; }
</style>
