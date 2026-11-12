<script lang="ts">
  import { Ellipsis, FileText, Folder, FolderOpen, Pencil, FolderInput, Trash2 } from '@lucide/svelte';
  import type { NotesSnapshot, NoteFolder } from '../../lib/notesCore';
  import { nm } from '../../lib/notesMessages';
  let { snapshot, selected, folder, query = '', profileId = null, onselect, onfolder, onaction }: { snapshot: NotesSnapshot; selected: string | null; folder: string | null; query?: string; profileId?: string | null; onselect: (id: string) => void; onfolder: (id: string | null) => void; onaction: (action: 'rename' | 'move' | 'remove', id: string, isFolder: boolean) => void } = $props();
  function visibleFolder(item: NoteFolder, visited = new Set<string>()): boolean {
    if (visited.has(item.id)) return false; visited.add(item.id);
    return item.name.toLowerCase().includes(query.toLowerCase()) || snapshot.notes.some(n => n.parent_id === item.id && matches(n)) || snapshot.folders.some(f => f.parent_id === item.id && visibleFolder(f, visited));
  }
  function matches(note: NotesSnapshot['notes'][number]): boolean { return (!profileId || note.profile_ids.includes(profileId)) && `${note.title} ${note.tags.join(' ')} ${note.markdown}`.toLowerCase().includes(query.toLowerCase()); }
  let expanded = $state<Record<string, boolean>>({});
  const anyVisible = $derived(snapshot.notes.some(matches) || snapshot.folders.some(f => visibleFolder(f)));
  function run(event: Event, action: 'rename' | 'move' | 'remove', id: string, isFolder: boolean): void {
    (event.currentTarget as HTMLElement).closest('details')?.removeAttribute('open'); onaction(action, id, isFolder);
  }
</script>
{#snippet actions(id: string, isFolder: boolean)}
  <details class="menu"><summary aria-label={nm('edit')} title={nm('edit')}><Ellipsis size={14} /></summary><div class="menu-items">
    <button onclick={event => run(event, 'rename', id, isFolder)}><Pencil size={13} />{nm('rename')}</button>
    <button onclick={event => run(event, 'move', id, isFolder)}><FolderInput size={13} />{nm('move')}</button>
    <button class="danger" onclick={event => run(event, 'remove', id, isFolder)}><Trash2 size={13} />{nm('remove')}</button>
  </div></details>
{/snippet}
{#snippet children(parent: string | null, visited: string[])}
  <ul>
    {#each snapshot.folders.filter(f => f.parent_id === parent && !visited.includes(f.id) && visibleFolder(f)) as item (item.id)}
      {@const open = !!query || !!expanded[item.id]}
      <li><div class="row" class:active={folder === item.id}><button aria-expanded={open} onclick={() => { expanded[item.id] = !expanded[item.id]; onfolder(item.id); }}>{#if open}<FolderOpen size={14} />{:else}<Folder size={14} />{/if}<span class="label">{item.name}</span></button>{@render actions(item.id, true)}</div>{#if open}{@render children(item.id, [...visited, item.id])}{/if}</li>
    {/each}
    {#each snapshot.notes.filter(n => n.parent_id === parent && matches(n)) as note (note.id)}
      <li class="row" class:active={selected === note.id}><button onclick={() => onselect(note.id)}><FileText size={14} /><span class="label">{note.title || nm('newNote')}</span></button>{@render actions(note.id, false)}</li>
    {/each}
  </ul>
{/snippet}
<nav aria-label={nm('title')}>
  <button class="root" class:active={folder === null} onclick={() => onfolder(null)}>{nm('root')}</button>
  {@render children(null, [])}
  {#if !anyVisible}<p class="none">{nm('noMatches')}</p>{/if}
</nav>
<style>
  nav { padding:4px 6px 8px; } ul { list-style:none; padding-left:12px; margin:0; } nav>ul { padding-left:0; }
  button { color:inherit; background:none; border:0; text-align:left; cursor:pointer; font:inherit; }
  .root { width:100%; padding:4px 8px; border-radius:5px; font-size:11px; text-transform:uppercase; letter-spacing:.04em; color:var(--color-fg-subtle); }
  .root.active { color:var(--color-accent-text); }
  .row { display:flex; align-items:center; gap:2px; border-radius:5px; }
  .row:hover { background:var(--color-surface-hover); } .row.active { background:var(--color-surface-selected); }
  .row>button { flex:1; min-width:0; display:flex; align-items:center; gap:7px; padding:5px 8px; font-size:13px; color:var(--color-fg); }
  .row :global(svg) { flex-shrink:0; color:var(--color-fg-muted); }
  .label { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .menu { position:relative; margin-left:auto; } .menu>summary { list-style:none; cursor:pointer; padding:4px 6px; border-radius:4px; display:flex; opacity:0; } .menu>summary::-webkit-details-marker { display:none; }
  .row:hover .menu>summary, .row.active .menu>summary, .menu[open]>summary, .menu>summary:focus-visible { opacity:1; }
  .menu-items { position:absolute; right:0; top:calc(100% + 2px); z-index:15; min-width:130px; display:flex; flex-direction:column; padding:4px; background:var(--color-surface-raised); border:1px solid var(--color-border); border-radius:7px; box-shadow:0 8px 24px var(--color-overlay); }
  .menu-items button { display:flex; align-items:center; gap:8px; padding:6px 8px; border-radius:4px; font-size:12px; white-space:nowrap; } .menu-items button:hover { background:var(--color-surface-hover); } .menu-items .danger { color:var(--color-danger); }
  .none { padding:16px; text-align:center; font-size:12px; color:var(--color-fg-subtle); }
</style>
