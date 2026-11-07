<script lang="ts">
  import { Clock, Folder, Server, Star } from '@lucide/svelte';
  import type { ProfileTreeFolder } from '../../lib/profileTree';
  import { collectProfilesInFolder } from '../../lib/profileTree';
  import type { StoredProfile } from '../../lib/types';
  import { i18n } from '../../lib/i18n.svelte';
  import { hostLabel } from './hostLabels';
  let { groups, profiles, group, favorites, recent, onRecent, onSelect, onFavorites, onMenu, onDragStart, onDragOver, onDrop, renaming, renameDraft, onRenameDraft, onRenameCommit, onRenameCancel }: {
    groups: ProfileTreeFolder; profiles: StoredProfile[]; group: string | null; favorites: boolean; recent: boolean; onRecent: () => void;
    onSelect: (path: string | null) => void; onFavorites: () => void;
    onMenu: (folder: ProfileTreeFolder, ev: MouseEvent) => void;
    onDragStart: (folder: ProfileTreeFolder, ev: DragEvent) => void;
    onDragOver: (folder: ProfileTreeFolder, ev: DragEvent) => void;
    onDrop: (folder: ProfileTreeFolder, ev: DragEvent) => void;
    renaming: string | null; renameDraft: string; onRenameDraft: (value: string) => void;
    onRenameCommit: (path: string) => void; onRenameCancel: () => void;
  } = $props();
  function focusRename(input: HTMLInputElement) {
    input.focus({ preventScroll: true });
    input.select();
  }
</script>
<nav aria-label={hostLabel('groups')}>
  <button type="button" class:chosen={group === null && !favorites && !recent} onclick={() => onSelect(null)}><Server size={17} /><span>{hostLabel('all')}</span><small>{profiles.length}</small></button>
  <button type="button" class:chosen={favorites} onclick={onFavorites}><Star size={17} /><span>{i18n.t('profileModal.favorite')}</span><small>{profiles.filter(p => p.favorite).length}</small></button>
  <button type="button" class:chosen={recent} onclick={onRecent}><Clock size={17} /><span>{i18n.t('picker.recent')}</span></button>
  <h3>{hostLabel('groups')}</h3>
  {@render folders(groups)}
  <button type="button" class:chosen={group === '' && !favorites && !recent} onclick={() => onSelect('')} oncontextmenu={(ev) => onMenu({ name: '', path: '', folders: [], profiles: profiles.filter(p => !p.group) }, ev)}><Folder size={17} /><span>{i18n.t('sidebar.ungrouped')}</span></button>
</nav>
{#snippet folders(parent: ProfileTreeFolder, depth = 0)}
  {#each parent.folders as folder (folder.path)}
    <div role="presentation" draggable="true" ondragstart={(ev) => onDragStart(folder, ev)} ondragover={(ev) => onDragOver(folder, ev)} ondrop={(ev) => onDrop(folder, ev)} oncontextmenu={(ev) => onMenu(folder, ev)}>
      {#if renaming === folder.path}
        <input class="input" use:focusRename value={renameDraft} aria-label={i18n.t('sidebar.renameGroup')} oninput={(ev) => onRenameDraft(ev.currentTarget.value)} onblur={() => onRenameCommit(folder.path)} onkeydown={(ev) => { if (ev.key === 'Enter') { ev.preventDefault(); onRenameCommit(folder.path); } else if (ev.key === 'Escape') { ev.preventDefault(); onRenameCancel(); } }} />
      {:else}
        <button type="button" style:padding-left="{10 + depth * 14}px" class:chosen={group === folder.path && !favorites && !recent} onclick={() => onSelect(folder.path)} title={folder.path}><Folder size={17} /><span>{folder.name}</span><small>{collectProfilesInFolder(folder).length}</small></button>
      {/if}
    </div>
    {@render folders(folder, depth + 1)}
  {/each}
{/snippet}
<style>
  nav { padding: 16px 10px; overflow: auto; min-width: 0; border-right: 1px solid var(--color-border-soft); }
  button { width: 100%; min-height: 42px; display: flex; align-items: center; gap: 9px; text-align: left; color: var(--color-fg); background: transparent; padding: 8px 10px; border-radius: 6px; font-size: 14px; line-height: 1.5; cursor: pointer; }
  button:hover, button.chosen { background: var(--color-panel-2); }
  button.chosen { color: var(--color-accent); }
  button:focus-visible { outline: 2px solid var(--color-accent); outline-offset: -2px; }
  span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
  small { font-size: 12px; color: var(--color-fg-muted); }
  h3 { margin: 24px 10px 8px; font-size: 14px; font-weight: 600; }
</style>
