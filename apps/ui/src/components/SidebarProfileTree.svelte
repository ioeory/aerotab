<script lang="ts">
  import { ChevronDown, ChevronRight } from '@lucide/svelte';
  import type { ProfileHealthResult, StoredProfile } from '../lib/types';
  import type { ProfileTreeFolder } from '../lib/profileTree';
  import { groupStyle } from '../lib/profileVisuals';
  import { profileVisualsStore } from '../lib/profileVisualsStore.svelte';
  import { i18n } from '../lib/i18n.svelte';
  import ProfileListRow, { type ProfileQuickAction } from './ProfileListRow.svelte';
  import Self from './SidebarProfileTree.svelte';

  interface Props {
    folder: ProfileTreeFolder;
    depth?: number;
    collapsed: Set<string>;
    forceExpanded: Set<string>;
    focusedProfileId?: string | null;
    selectedProfileIds?: Set<string>;
    profileHealth?: Record<string, ProfileHealthResult>;
    showSelection?: boolean;
    onToggleFolder: (path: string) => void;
    onOpenProfile: (p: StoredProfile) => void;
    onProfileClick?: (p: StoredProfile, ev: MouseEvent) => void;
    onProfileCheckboxToggle?: (p: StoredProfile) => void;
    onProfileFocus: (p: StoredProfile) => void;
    onProfileKeydown: (p: StoredProfile, ev: KeyboardEvent) => void;
    onProfileContextMenu: (p: StoredProfile, ev: MouseEvent) => void;
    onProfileQuickAction?: (p: StoredProfile, action: ProfileQuickAction, ev: MouseEvent) => void;
    onProfileDragStart: (p: StoredProfile, ev: DragEvent) => void;
    onFolderContextMenu: (folder: ProfileTreeFolder, ev: MouseEvent) => void;
    onFolderDragStart: (folder: ProfileTreeFolder, ev: DragEvent) => void;
    onFolderDragOver: (folder: ProfileTreeFolder, ev: DragEvent) => void;
    onFolderDrop: (folder: ProfileTreeFolder, ev: DragEvent) => void;
    showUngroupedLabel?: boolean;
  }

  let {
    folder,
    depth = 0,
    showUngroupedLabel = false,
    collapsed,
    forceExpanded,
    focusedProfileId = null,
    selectedProfileIds = new Set(),
    profileHealth = {},
    showSelection = false,
    onToggleFolder,
    onOpenProfile,
    onProfileClick,
    onProfileCheckboxToggle,
    onProfileFocus,
    onProfileKeydown,
    onProfileContextMenu,
    onProfileQuickAction,
    onProfileDragStart,
    onFolderContextMenu,
    onFolderDragStart,
    onFolderDragOver,
    onFolderDrop,
  }: Props = $props();

  function isExpanded(path: string): boolean {
    if (forceExpanded.has(path)) return true;
    return !collapsed.has(path);
  }

  function quickAction(p: StoredProfile, action: ProfileQuickAction, ev: MouseEvent) {
    onProfileQuickAction?.(p, action, ev);
  }
</script>

{#each folder.folders as child (child.path)}
  {@const expanded = isExpanded(child.path)}
  <div class="profile-folder" style="--depth: {depth}">
    <div
      role="presentation"
      class="folder-header w-full flex items-center gap-1 py-0.5 pr-1 rounded
             hover:bg-[var(--color-panel-2)] text-[var(--color-fg-muted)]"
      draggable="true"
      ondragstart={(e) => onFolderDragStart(child, e)}
      ondragover={(e) => onFolderDragOver(child, e)}
      ondrop={(e) => onFolderDrop(child, e)}
      oncontextmenu={(e) => onFolderContextMenu(child, e)}
    >
      <button
        type="button"
        class="shrink-0 w-3.5 grid place-items-center rounded hover:text-[var(--color-fg)] cursor-pointer"
        onclick={() => onToggleFolder(child.path)}
        aria-expanded={expanded}
        aria-label={expanded ? i18n.t('sidebar.collapseGroup') : i18n.t('sidebar.expandGroup')}
      >
        {#if expanded}
          <ChevronDown size={12} />
        {:else}
          <ChevronRight size={12} />
        {/if}
      </button>
      <button
        type="button"
        class="profile-group-header flex-1 min-w-0 text-left truncate text-[11px] font-medium text-[var(--color-fg)] cursor-pointer"
        style={groupStyle(child.path, profileVisualsStore.overrides)}
        onclick={() => onToggleFolder(child.path)}
      >
        <span class="profile-group-swatch" aria-hidden="true"></span>
        <span class="truncate">{child.name}</span>
      </button>
      <span class="ml-auto shrink-0 text-[10px] opacity-70 pr-0.5">
        {child.profiles.length + child.folders.length}
      </span>
    </div>
    {#if expanded}
      <div class="folder-children profile-group-rail" style={groupStyle(child.path, profileVisualsStore.overrides)}>
        <Self
          folder={child}
          depth={depth + 1}
          {collapsed}
          {forceExpanded}
          {focusedProfileId}
          {selectedProfileIds}
          {profileHealth}
          {showSelection}
          {onToggleFolder}
          {onOpenProfile}
          {onProfileClick}
          {onProfileCheckboxToggle}
          {onProfileFocus}
          {onProfileKeydown}
          {onProfileContextMenu}
          {onProfileQuickAction}
          {onProfileDragStart}
          {onFolderContextMenu}
          {onFolderDragStart}
          {onFolderDragOver}
          {onFolderDrop}
        />
      </div>
    {/if}
  </div>
{/each}

{#if showUngroupedLabel && folder.profiles.length > 0}
  <div class="px-1 pt-2 pb-0.5 text-[10px] uppercase tracking-[0.1em] text-[var(--color-fg-muted)]">
    {i18n.t('sidebar.ungrouped')}
  </div>
{/if}

{#each folder.profiles as p (p.id)}
  <ProfileListRow
    profile={p}
    variant="sidebar"
    {depth}
    health={profileHealth[p.id]}
    focused={focusedProfileId === p.id}
    selected={selectedProfileIds.has(p.id)}
    {showSelection}
    draggable
    onOpen={() => onOpenProfile(p)}
    onClick={(ev) => (onProfileClick ? onProfileClick(p, ev) : onProfileFocus(p))}
    onCheckboxToggle={() => onProfileCheckboxToggle?.(p)}
    onContextMenu={(ev) => onProfileContextMenu(p, ev)}
    onDragStart={(ev) => onProfileDragStart(p, ev)}
    onKeydown={(ev) => onProfileKeydown(p, ev)}
    onQuickAction={(action, ev) => quickAction(p, action, ev)}
  />
{/each}

<style>
  .profile-folder {
    padding-left: calc(var(--depth, 0) * 10px);
  }
  .folder-children {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .folder-header button {
    background: transparent;
    border: none;
    font: inherit;
  }
</style>
