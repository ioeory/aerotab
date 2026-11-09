<script lang="ts">
  import { tick } from 'svelte';
  import type { ProfileTreeFolder } from '../../lib/profileTree';
  import { MoreHorizontal, Plug, Folder } from '@lucide/svelte';
  import type { ProfileHealthResult, StoredProfile } from '../../lib/types';
  import ProfileListRow, { type ProfileQuickAction } from '../ProfileListRow.svelte';
  import { i18n } from '../../lib/i18n.svelte';
  import { tabs } from '../../lib/tabs.svelte';
  import { hostLabel } from './hostLabels';
  let { folders, onEnterFolder, onFolderMenu, onFolderDrag, onFolderOver, onFolderDrop, folderRenaming, folderRenameDraft, onFolderRenameDraft, onFolderRenameCommit, onFolderRenameCancel, profiles, view, selectedIds, focusedId, health, onOpen, onSelect, onCheck, onMenu, onDrag, onKeydown, onQuickAction, renaming, renameDraft, onRenameDraft, onRenameCommit, onRenameCancel }: {
    folders: ProfileTreeFolder[];
    onEnterFolder: (folder: ProfileTreeFolder) => void;
    onFolderMenu: (folder: ProfileTreeFolder, ev: MouseEvent) => void;
    onFolderDrag: (folder: ProfileTreeFolder, ev: DragEvent) => void;
    onFolderOver: (folder: ProfileTreeFolder, ev: DragEvent) => void;
    onFolderDrop: (folder: ProfileTreeFolder, ev: DragEvent) => void;
    folderRenaming: string | null; folderRenameDraft: string; onFolderRenameDraft: (value: string) => void; onFolderRenameCommit: (path: string) => void; onFolderRenameCancel: () => void;
    profiles: StoredProfile[]; view: 'grid' | 'list'; selectedIds: Set<string>; focusedId: string | null;
    health: Record<string, ProfileHealthResult>;
    onOpen: (profile: StoredProfile) => void; onSelect: (profile: StoredProfile, ev: MouseEvent) => void;
    onCheck: (profile: StoredProfile) => void; onMenu: (profile: StoredProfile, ev: MouseEvent) => void;
    onDrag: (profile: StoredProfile, ev: DragEvent) => void; onKeydown: (profile: StoredProfile, ev: KeyboardEvent) => void;
    onQuickAction: (profile: StoredProfile, action: ProfileQuickAction, ev: MouseEvent) => void;
    renaming: string | null; renameDraft: string; onRenameDraft: (value: string) => void;
    onRenameCommit: (profile: StoredProfile) => void; onRenameCancel: () => void;
  } = $props();
  let folderRenameInput = $state<HTMLInputElement | null>(null);
  $effect(() => {
    if (!folderRenaming) return;
    let cancelled = false;
    void tick().then(() => { if (!cancelled) { folderRenameInput?.focus({ preventScroll: true }); folderRenameInput?.select(); } });
    return () => { cancelled = true; };
  });
  const connected = $derived(new Set(tabs.tabs.flatMap(tab => tab.panes.map(pane => pane.profileId).filter(Boolean))));
</script>
<div class="resource-views" class:grid={view === 'grid'}>
  {#each folders as folder (folder.path)}
    <article class="folder-card">
      <button type="button" class="folder-open" draggable="true" ondragstart={(ev) => onFolderDrag(folder, ev)} ondragover={(ev) => onFolderOver(folder, ev)} ondrop={(ev) => onFolderDrop(folder, ev)} onclick={() => onEnterFolder(folder)} oncontextmenu={(ev) => onFolderMenu(folder, ev)}><Folder size={26} /><span>{folder.name}</span></button>
      {#if folderRenaming === folder.path}
        <input bind:this={folderRenameInput} aria-label={i18n.t('sidebar.renameGroup')} value={folderRenameDraft} oninput={(ev) => onFolderRenameDraft(ev.currentTarget.value)} onblur={() => onFolderRenameCommit(folder.path)} onkeydown={(ev) => { if (ev.key === 'Enter') onFolderRenameCommit(folder.path); if (ev.key === 'Escape') onFolderRenameCancel(); }} />
      {/if}
      <button type="button" aria-label={i18n.t('sidebar.bulkActions')} onclick={(ev) => onFolderMenu(folder, ev)}><MoreHorizontal size={18} /></button>
    </article>
  {/each}
  {#each profiles as profile (profile.id)}
    <article class:selected={selectedIds.has(profile.id)}>
      <ProfileListRow {profile} variant="resource" health={health[profile.id]} focused={focusedId === profile.id}
        selected={selectedIds.has(profile.id)} showSelection draggable onOpen={() => onOpen(profile)}
        onClick={(ev) => onSelect(profile, ev)} onCheckboxToggle={() => onCheck(profile)}
        onContextMenu={(ev) => onMenu(profile, ev)} onDragStart={(ev) => onDrag(profile, ev)}
        onKeydown={(ev) => onKeydown(profile, ev)} onQuickAction={(action, ev) => onQuickAction(profile, action, ev)}
        renaming={renaming === profile.id} {renameDraft} onRenameDraftChange={onRenameDraft}
        onRenameCommit={() => onRenameCommit(profile)} onRenameCancel={onRenameCancel} />
      <div class="resource-meta">
        <span title={profile.group ?? i18n.t('sidebar.ungrouped')}>{profile.group ?? i18n.t('sidebar.ungrouped')}</span>
        {#if connected.has(profile.id)}<span class="connected">{hostLabel('active')}</span>{/if}
        <button type="button" aria-label={i18n.t('profiles.connect')} title={i18n.t('profiles.connect')} onclick={() => onOpen(profile)}><Plug size={17} /></button>
        <button type="button" aria-label={i18n.t('sidebar.bulkActions')} title={i18n.t('sidebar.bulkActions')} onclick={(ev) => onMenu(profile, ev)}><MoreHorizontal size={18} /></button>
      </div>
    </article>
  {/each}
</div>
<style>
  .folder-card { display: flex !important; align-items: center; gap: 8px; }
  .folder-open { flex: 1; display: flex; gap: 12px; padding: 12px; justify-content: flex-start; text-align: left; }
  .resource-views { display: grid; gap: 8px; align-content: start; padding: 4px; }
  .resource-views.grid { grid-template-columns: repeat(auto-fill, minmax(min(100%, 270px), 1fr)); gap: 14px; }
  article { min-width: 0; border: 1px solid var(--color-border-soft); border-radius: 10px; background: var(--color-panel); padding: 12px; }
  article:hover { border-color: var(--color-border); }
  article.selected { border-color: var(--color-accent); }
  .resource-meta { display: flex; align-items: center; gap: 8px; margin: 8px 0 0 25px; min-width: 0; font-size: 14px; }
  .resource-meta > span:first-child { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--color-fg-muted); }
  .connected { color: var(--color-success); font-size: 12px; }
  button { display: grid; place-items: center; min-width: 34px; min-height: 34px; color: var(--color-fg); border-radius: 6px; background: var(--color-panel-2); cursor: pointer; }
  button:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; }
  .resource-views:not(.grid) article { display: grid; grid-template-columns: minmax(0, 1fr) minmax(180px, 30%); gap: 12px; align-items: center; }
  .resource-views:not(.grid) .resource-meta { margin: 0; }
  @media (max-width: 760px) { .resource-views:not(.grid) article { grid-template-columns: minmax(0, 1fr); } }
</style>
