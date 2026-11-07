<script lang="ts">
  import type { NotesSnapshot, NoteFolder } from '../../lib/notesCore';
  import { nm } from '../../lib/notesMessages';
  let { snapshot, selected, folder, query = '', profileId = null, onselect, onfolder, onaction }: { snapshot: NotesSnapshot; selected: string | null; folder: string | null; query?: string; profileId?: string | null; onselect: (id: string) => void; onfolder: (id: string | null) => void; onaction: (action: 'rename' | 'move' | 'remove', id: string, isFolder: boolean) => void } = $props();
  function visibleFolder(item: NoteFolder, visited = new Set<string>()): boolean {
    if (visited.has(item.id)) return false; visited.add(item.id);
    return item.name.toLowerCase().includes(query.toLowerCase()) || snapshot.notes.some(n => n.parent_id === item.id && matches(n)) || snapshot.folders.some(f => f.parent_id === item.id && visibleFolder(f, visited));
  }
  function matches(note: NotesSnapshot['notes'][number]): boolean { return (!profileId || note.profile_ids.includes(profileId)) && `${note.title} ${note.tags.join(' ')} ${note.markdown}`.toLowerCase().includes(query.toLowerCase()); }
</script>
{#snippet actions(id: string, isFolder: boolean)}
  <details class="menu"><summary aria-label={nm('edit')}>⋯</summary><div class="menu-items">
    <button onclick={() => onaction('rename', id, isFolder)}>{nm('rename')}</button>
    <button onclick={() => onaction('move', id, isFolder)}>{nm('move')}</button>
    <button onclick={() => onaction('remove', id, isFolder)}>{nm('remove')}</button>
  </div></details>
{/snippet}
{#snippet children(parent: string | null, visited: string[])}
  <ul>
    {#each snapshot.folders.filter(f => f.parent_id === parent && !visited.includes(f.id) && visibleFolder(f)) as item (item.id)}
      <li><details open={!!query}><summary><button class:active={folder === item.id} onclick={() => onfolder(item.id)}>▱ {item.name}</button>{@render actions(item.id, true)}</summary>{@render children(item.id, [...visited, item.id])}</details></li>
    {/each}
    {#each snapshot.notes.filter(n => n.parent_id === parent && matches(n)) as note (note.id)}
      <li class="note"><button class:active={selected === note.id} onclick={() => onselect(note.id)}>▤ {note.title}</button>{@render actions(note.id, false)}</li>
    {/each}
  </ul>
{/snippet}
<nav aria-label={nm('title')}><button class:active={folder === null} onclick={() => onfolder(null)}>{nm('root')}</button>{@render children(null, [])}</nav>
<style>
  nav { overflow:auto; padding:8px; } ul { list-style:none; padding-left:13px; margin:3px 0; } nav>ul { padding-left:0; }
  button { color:inherit; background:none; border:0; text-align:left; padding:6px; border-radius:4px; cursor:pointer; max-width:100%; overflow-wrap:anywhere; }
  .active { background:var(--bg-hover, #ffffff18); } summary,.note { display:flex; align-items:center; gap:3px; } summary>button,.note>button { flex:1; }
  .menu { margin-left:auto; } .menu>summary { cursor:pointer; padding:4px 7px; } .menu-items { display:flex; flex-direction:column; border:1px solid var(--border, #414656); border-radius:5px; padding:3px; }
</style>
