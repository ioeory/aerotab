<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { Ellipsis, FileDown, FilePlus, FileUp, FolderPlus, MapPin, Search } from '@lucide/svelte';
  import type { RpcClient } from '../../lib/rpc';
  import type { StoredProfile } from '../../lib/types';
  import { getNotesStore } from '../../lib/notes.svelte';
  import { nm } from '../../lib/notesMessages';
  import { exportFolder, importNotes } from '../../lib/notesFiles';
  import Splitter from '../Splitter.svelte';
  let treeSize = $state(230);
  import NotesTree from './NotesTree.svelte';
  import NoteEditor from './NoteEditor.svelte';
  let { rpc, profiles = [], onError, profileId = null, compact = false, listFooter }: { rpc: RpcClient; profiles?: StoredProfile[]; onError: (message: string) => void; profileId?: string | null; compact?: boolean; listFooter?: Snippet } = $props();
  const store = $derived(getNotesStore(rpc));
  let selected = $state<string | null>(null), folder = $state<string | null>(null), query = $state(''), currentOnly = $state(false);
  let dialog = $state<{ action: string; id?: string; isFolder?: boolean; titles?: string[]; resolve?: (yes: boolean) => void } | null>(null);
  let name = $state(''), parent = $state('');
  let selectedNote = $derived(store.snapshot.notes.find(note => note.id === selected));
  const isEmpty = $derived(!store.snapshot.notes.length && !store.snapshot.folders.length);
  onMount(() => { void store.refresh().catch(report); });
  function report(error: unknown): void { onError(String(error)); }
  async function select(id: string): Promise<void> {
    // Retain unsaved drafts in the shared store if flushing fails; selection is not blocked.
    if (selected) await store.save(selected).catch(report); selected = id;
  }
  function back(): void { const id = selected; selected = null; if (id) void store.save(id).catch(report); }
  function closeMenu(event: Event): void { (event.currentTarget as HTMLElement).closest('details')?.removeAttribute('open'); }
  let section = $state<HTMLElement>();
  function dismissMenus(event: PointerEvent): void {
    for (const menu of section?.querySelectorAll<HTMLDetailsElement>('details.menu[open]') ?? []) if (!menu.contains(event.target as Node)) menu.open = false;
  }
  async function create(isFolder: boolean): Promise<void> { name = isFolder ? nm('newFolder') : nm('newNote'); parent = folder ?? ''; dialog = { action: 'create', isFolder }; }
  async function action(action: 'rename' | 'move' | 'remove', id: string, isFolder: boolean): Promise<void> {
    const item = isFolder ? store.snapshot.folders.find(f => f.id === id)! : store.snapshot.notes.find(n => n.id === id)!;
    if (action === 'remove') {
      await store.remove(id, item.version, impact => new Promise<boolean>(resolve => { dialog = { action: 'remove', titles: [...impact.notes.map(n => n.title), ...store.snapshot.folders.filter(f => impact.folders.includes(f.id)).map(f => `${nm('folder')}: ${f.name}`)], resolve }; }));
      if (selected && !store.snapshot.notes.some(n => n.id === selected)) selected = null;
      return;
    }
    name = isFolder ? store.snapshot.folders.find(f => f.id === id)!.name : store.snapshot.notes.find(n => n.id === id)!.title;
    parent = item.parent_id ?? ''; dialog = { action, id, isFolder };
  }
  async function submit(): Promise<void> {
    const task = dialog; if (!task) return;
    if (task.action === 'remove') { task.resolve?.(true); dialog = null; return; }
    try {
      if (task.action === 'create') { const id = await store.create(parent || null, name, task.isFolder); if (!task.isFolder) await select(id); else folder = id; }
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
{#snippet list()}
  <div class="list">
    <div class="toolbar">
      <label class="search"><Search size={13} /><input aria-label={nm('search')} placeholder={nm('search')} bind:value={query} /></label>
      <button class="icon" title={nm('newNote')} aria-label={nm('newNote')} onclick={() => create(false)}><FilePlus size={15} /></button>
      <button class="icon" title={nm('newFolder')} aria-label={nm('newFolder')} onclick={() => create(true)}><FolderPlus size={15} /></button>
      {#if profileId}<button class="icon" aria-pressed={currentOnly} title={nm('current')} aria-label={nm('current')} onclick={() => currentOnly = !currentOnly}><MapPin size={15} /></button>{/if}
      <details class="menu">
        <summary class="icon" title={nm('more')} aria-label={nm('more')}><Ellipsis size={15} /></summary>
        <div class="popover">
          <button onclick={event => { closeMenu(event); importNotes(store, folder).catch(report); }}><FileUp size={14} />{nm('import')}</button>
          <button onclick={event => { closeMenu(event); store.flush().then(() => exportFolder(store.snapshot, folder)).catch(report); }}><FileDown size={14} />{nm('exportFolder')}</button>
        </div>
      </details>
    </div>
    <div class="tree">
      {#if isEmpty}<p class="empty">{nm('noNotes')}</p>
      {:else}<NotesTree snapshot={store.snapshot} {selected} {folder} {query} profileId={currentOnly ? profileId : null} onselect={id => { void select(id); }} onfolder={id => folder = id} onaction={(a, id, f) => { void action(a, id, f).catch(report); }} />{/if}
    </div>
    <p class="warning">{nm('warning')}</p>
    {@render listFooter?.()}
  </div>
{/snippet}
<svelte:window onpointerdown={dismissMenus} />
<section bind:this={section} class:compact class="workspace" aria-label={nm('title')}>
  {#if compact}
    {#if selectedNote}<NoteEditor {store} note={selectedNote} {profiles} {onError} onBack={back} />{:else}{@render list()}{/if}
  {:else}
    <div class="layout"><aside style={`width:${treeSize}px;max-width:65%;`}>{@render list()}</aside><Splitter axis="x" value={treeSize} min={180} max={600} label={nm('title')} onChange={v=>treeSize=v} storageKey="notes.tree" /><main>{#if selectedNote}<NoteEditor {store} note={selectedNote} {profiles} {onError} />{:else}<p class="empty">{nm('empty')}</p>{/if}</main></div>
  {/if}
  {#if dialog}<div class="dialog" role="dialog" aria-modal="true" aria-label={dialog.action === 'remove' ? nm('impact') : nm('edit')}>
    <form onsubmit={event => { event.preventDefault(); void submit(); }}>
      {#if dialog.action === 'remove'}<h3>{nm('impact')}</h3><ul>{#each dialog.titles ?? [] as title}<li>{title}</li>{/each}</ul>
      {:else}
        {#if dialog.action !== 'move'}<label>{nm('name')}<!-- svelte-ignore a11y_autofocus --><input required maxlength="512" bind:value={name} autofocus onfocus={event => event.currentTarget.select()} /></label>{/if}
        {#if dialog.action !== 'rename'}<label>{nm('parent')}<select bind:value={parent}><option value="">{nm('root')}</option>{#each store.snapshot.folders as f (f.id)}<option value={f.id} disabled={f.id === dialog.id}>{f.name}</option>{/each}</select></label>{/if}
      {/if}
      <footer><button type="button" onclick={() => { dialog?.resolve?.(false); dialog = null; }}>{nm('cancel')}</button><button type="submit" class="primary">{nm('confirm')}</button></footer>
    </form>
  </div>{/if}
</section>
<style>
  .workspace { display:flex; flex-direction:column; height:100%; min-height:0; position:relative; color:var(--color-fg); }
  .layout { display:flex; flex:1; min-height:0; } aside { flex-shrink:0; min-height:0; display:flex; border-right:1px solid var(--color-border); } main { flex:1; min-width:0; min-height:0; }
  .list { display:flex; flex-direction:column; flex:1; min-height:0; min-width:0; }
  .toolbar { display:flex; align-items:center; gap:2px; padding:6px 8px; border-bottom:1px solid var(--color-border-soft); flex-shrink:0; }
  .search { display:flex; align-items:center; gap:6px; flex:1; min-width:0; margin-right:4px; padding:0 8px; height:28px; border:1px solid var(--color-border); border-radius:6px; background:var(--color-bg-soft); color:var(--color-fg-subtle); }
  .search:focus-within { border-color:var(--color-accent); }
  .search input { flex:1; min-width:0; border:0; background:none; outline:none; color:var(--color-fg); font-size:12px; }
  .icon { display:inline-flex; align-items:center; justify-content:center; width:28px; height:28px; flex-shrink:0; padding:0; border:0; border-radius:5px; background:none; cursor:pointer; color:var(--color-fg-muted); }
  .icon:hover { background:var(--color-surface-hover); color:var(--color-fg); } .icon[aria-pressed='true'] { background:var(--color-surface-selected); color:var(--color-accent-text); }
  .menu { position:relative; } .menu>summary { list-style:none; } .menu>summary::-webkit-details-marker { display:none; }
  .popover { position:absolute; top:calc(100% + 4px); right:0; z-index:20; min-width:160px; display:flex; flex-direction:column; padding:4px; background:var(--color-surface-raised); border:1px solid var(--color-border); border-radius:7px; box-shadow:0 8px 24px var(--color-overlay); }
  .popover button { display:flex; align-items:center; gap:8px; padding:6px 8px; border:0; border-radius:4px; background:none; color:inherit; cursor:pointer; text-align:left; font-size:12px; white-space:nowrap; } .popover button:hover { background:var(--color-surface-hover); }
  .tree { flex:1; min-height:0; overflow:auto; }
  .warning { margin:0; padding:6px 10px; font-size:11px; color:var(--color-fg-subtle); border-top:1px solid var(--color-border-soft); flex-shrink:0; }
  .empty { padding:24px 16px; color:var(--color-fg-subtle); font-size:12px; text-align:center; }
  .dialog { position:absolute; inset:0; z-index:30; background:var(--color-overlay); display:flex; align-items:center; justify-content:center; padding:16px; }
  form { background:var(--color-panel-2); padding:16px; border:1px solid var(--color-border); border-radius:9px; max-height:90%; overflow:auto; width:min(380px,100%); } form label { display:flex; flex-direction:column; gap:6px; margin-bottom:12px; font-size:12px; }
  form h3 { margin:0 0 8px; font-size:14px; } form ul { margin:0 0 12px; padding-left:18px; font-size:12px; }
  form input,form select,form button { color:inherit; background:var(--color-bg-soft); border:1px solid var(--color-border); border-radius:5px; padding:6px 9px; font-size:13px; } form button { cursor:pointer; }
  form .primary { background:var(--color-accent); color:var(--color-accent-fg); border-color:var(--color-accent); }
  footer { display:flex; justify-content:flex-end; gap:8px; }
</style>
