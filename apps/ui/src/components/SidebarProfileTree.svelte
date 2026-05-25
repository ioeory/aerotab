<script lang="ts">
  import { ChevronDown, ChevronRight, Star } from '@lucide/svelte';
  import type { StoredProfile } from '../lib/types';
  import type { ProfileTreeFolder } from '../lib/profileTree';
  import { profileEndpointLabel } from '../lib/profileMeta';
  import { i18n } from '../lib/i18n.svelte';
  import ProfileIcon from './ProfileIcon.svelte';
  import Self from './SidebarProfileTree.svelte';

  interface Props {
    folder: ProfileTreeFolder;
    depth?: number;
    collapsed: Set<string>;
    forceExpanded: Set<string>;
    focusedProfileId?: string | null;
    selectedProfileIds?: Set<string>;
    showSelection?: boolean;
    onToggleFolder: (path: string) => void;
    onOpenProfile: (p: StoredProfile) => void;
    onProfileClick?: (p: StoredProfile, ev: MouseEvent) => void;
    onProfileCheckboxToggle?: (p: StoredProfile) => void;
    onProfileFocus: (p: StoredProfile) => void;
    onProfileKeydown: (p: StoredProfile, ev: KeyboardEvent) => void;
    onProfileContextMenu: (p: StoredProfile, ev: MouseEvent) => void;
    onFolderContextMenu: (folder: ProfileTreeFolder, ev: MouseEvent) => void;
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
    showSelection = false,
    onToggleFolder,
    onOpenProfile,
    onProfileClick,
    onProfileCheckboxToggle,
    onProfileFocus,
    onProfileKeydown,
    onProfileContextMenu,
    onFolderContextMenu,
  }: Props = $props();

  function isExpanded(path: string): boolean {
    if (forceExpanded.has(path)) return true;
    return !collapsed.has(path);
  }
</script>

{#each folder.folders as child (child.path)}
  {@const expanded = isExpanded(child.path)}
  <div class="profile-folder" style="--depth: {depth}">
    <div
      role="presentation"
      class="folder-header w-full flex items-center gap-1 py-1 pr-1 rounded-md
             hover:bg-[var(--color-panel-2)] text-[var(--color-fg-muted)]"
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
        class="flex-1 min-w-0 text-left truncate text-[11px] font-medium text-[var(--color-fg)] cursor-pointer"
        onclick={() => onToggleFolder(child.path)}
      >
        {child.name}
      </button>
      <span class="ml-auto shrink-0 text-[10px] opacity-70 pr-0.5">
        {child.profiles.length + child.folders.length}
      </span>
    </div>
    {#if expanded}
      <div class="folder-children">
        <Self
          folder={child}
          depth={depth + 1}
          {collapsed}
          {forceExpanded}
          {focusedProfileId}
          {selectedProfileIds}
          {showSelection}
          {onToggleFolder}
          {onOpenProfile}
          {onProfileClick}
          {onProfileCheckboxToggle}
          {onProfileFocus}
          {onProfileKeydown}
          {onProfileContextMenu}
          {onFolderContextMenu}
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
  <div
    class="profile-row group flex items-center gap-1.5 rounded-md hover:bg-[var(--color-panel-2)] cursor-pointer
           {focusedProfileId === p.id ? 'profile-row--focused' : ''}
           {selectedProfileIds.has(p.id) ? 'profile-row--selected' : ''}"
    style="--depth: {depth}"
    role="button"
    tabindex="0"
    title="{profileEndpointLabel(p)} — {i18n.t('sidebar.profileRowHint')}"
    onclick={(ev) => (onProfileClick ? onProfileClick(p, ev) : onProfileFocus(p))}
    onfocus={() => onProfileFocus(p)}
    ondblclick={() => onOpenProfile(p)}
    onkeydown={(e) => {
      onProfileKeydown(p, e);
      if (!e.defaultPrevented && e.key === 'Enter') {
        onOpenProfile(p);
        e.preventDefault();
      }
    }}
    oncontextmenu={(e) => onProfileContextMenu(p, e)}
  >
    <div class="profile-row-indent shrink-0"></div>
    <input
      type="checkbox"
      class="shrink-0 opacity-60 group-hover:opacity-100 {selectedProfileIds.has(p.id) || showSelection ? 'opacity-100' : ''}"
      checked={selectedProfileIds.has(p.id)}
      onclick={(ev) => {
        ev.stopPropagation();
        onProfileCheckboxToggle?.(p);
      }}
      aria-label={p.name}
    />
    <ProfileIcon icon={p.icon} name={p.name} size={13} />
    <div class="flex-1 min-w-0 text-left py-1.5 text-[12px]">
      <div class="flex items-center gap-1 truncate text-[var(--color-fg)]">
        <span class="truncate">{p.name}</span>
        {#if p.favorite}
          <Star size={10} class="shrink-0 text-[var(--color-accent)]" fill="currentColor" />
        {/if}
      </div>
      <div class="truncate text-[10.5px] text-[var(--color-fg-muted)]">
        {profileEndpointLabel(p)}
      </div>
      {#if (p.tags ?? []).length > 0}
        <div class="mt-1 flex gap-1 overflow-hidden">
          {#each (p.tags ?? []).slice(0, 3) as tag (tag)}
            <span
              class="shrink-0 max-w-[64px] truncate rounded-full border border-[var(--color-border-soft)]
                     px-1.5 text-[9.5px] text-[var(--color-fg-muted)]"
            >{tag}</span>
          {/each}
        </div>
      {/if}
    </div>
  </div>
{/each}

<style>
  .profile-folder,
  .profile-row {
    padding-left: calc(var(--depth, 0) * 10px);
  }
  .profile-row-indent {
    width: 14px;
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
  .profile-row--focused {
    background: var(--color-panel-2);
    outline: 1px solid color-mix(in srgb, var(--color-accent) 45%, transparent);
    outline-offset: -1px;
  }
  .profile-row--selected {
    background: color-mix(in srgb, var(--color-accent) 12%, var(--color-panel-2));
  }
</style>
