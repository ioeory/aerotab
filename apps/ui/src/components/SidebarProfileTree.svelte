<script lang="ts">
  import { ChevronDown, ChevronRight, ShieldAlert, ShieldCheck, ShieldX, Star, StickyNote } from '@lucide/svelte';
  import type { ProfileHealthResult, StoredProfile } from '../lib/types';
  import type { ProfileTreeFolder } from '../lib/profileTree';
  import { profileEndpointLabel } from '../lib/profileMeta';
  import { groupStyle } from '../lib/profileVisuals';
  import { profileVisualsStore } from '../lib/profileVisualsStore.svelte';
  import { i18n } from '../lib/i18n.svelte';
  import ProfileIcon from './ProfileIcon.svelte';
  import ProfileKindBadge from './ProfileKindBadge.svelte';
  import ProfileTag from './ProfileTag.svelte';
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
    onProfileNoteClick?: (p: StoredProfile, ev: MouseEvent) => void;
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
    onProfileNoteClick,
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

  function healthLabel(status: ProfileHealthResult['status']): string {
    if (status === 'ok') return i18n.t('profiles.healthOk');
    if (status === 'warning') return i18n.t('profiles.healthWarning');
    return i18n.t('profiles.healthError');
  }

  function healthTitle(result: ProfileHealthResult): string {
    const issues = result.checks.filter((c) => c.status !== 'ok');
    const visible = issues.length > 0 ? issues : result.checks.slice(0, 1);
    return visible.map((c) => `${c.name}: ${c.message}`).join('\n') || i18n.t('profiles.healthNoIssues');
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
          {onProfileNoteClick}
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
  {@const h = profileHealth[p.id]}
  {@const rowTitle = `${profileEndpointLabel(p)} — ${i18n.t('sidebar.profileRowHint')}${p.note ? `\n${p.note}` : ''}`}
  <div
    class="profile-row group flex items-center gap-1.5 rounded hover:bg-[var(--color-panel-2)] cursor-pointer
           {focusedProfileId === p.id ? 'profile-row--focused' : ''}
           {selectedProfileIds.has(p.id) ? 'profile-row--selected' : ''}"
    style="--depth: {depth}"
    role="button"
    tabindex="0"
    title={rowTitle}
    draggable="true"
    ondragstart={(e) => onProfileDragStart(p, e)}
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
    <ProfileIcon icon={p.icon} name={p.name} kind={p.kind} size={12} />
    <div class="flex-1 min-w-0 text-left py-1 text-[11.5px]">
      <div class="flex items-center gap-1 truncate text-[var(--color-fg)]">
        <span class="truncate">{p.name}</span>
        <ProfileKindBadge kind={p.kind} compact />
        {#if h}
          <span class="health-chip {h.status}" title={healthTitle(h)} aria-label={healthLabel(h.status)}>
            {#if h.status === 'ok'}
              <ShieldCheck size={10} />
            {:else if h.status === 'warning'}
              <ShieldAlert size={10} />
            {:else}
              <ShieldX size={10} />
            {/if}
          </span>
        {/if}
        {#if p.favorite}
          <Star size={10} class="shrink-0 text-[var(--color-accent)]" fill="currentColor" />
        {/if}
        <button
          type="button"
          class="note-chip shrink-0 {p.note ? 'has-note' : ''}"
          title={p.note?.trim() || i18n.t('sidebar.editNote')}
          aria-label={i18n.t('sidebar.editNote')}
          onclick={(ev) => {
            ev.stopPropagation();
            onProfileNoteClick?.(p, ev);
          }}
        >
          <StickyNote size={10} />
        </button>
      </div>
      <div class="truncate text-[10px] text-[var(--color-fg-muted)]">
        {profileEndpointLabel(p)}
      </div>
      {#if p.note}
        <div class="truncate text-[10px] text-[var(--color-fg-muted)] opacity-80">{p.note}</div>
      {/if}
      {#if (p.tags ?? []).length > 0}
        <div class="mt-0.5 flex gap-1 overflow-hidden">
          {#each (p.tags ?? []).slice(0, 3) as tag (tag)}
            <ProfileTag {tag} compact />
          {/each}
        </div>
      {/if}
    </div>
  </div>
{/each}

<style>
  .health-chip {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
  }
  .health-chip.ok { color: var(--color-success); }
  .health-chip.warning { color: var(--color-warning); }
  .health-chip.error { color: var(--color-danger); }

  .profile-folder,
  .profile-row {
    padding-left: calc(var(--depth, 0) * 10px);
  }
  .profile-row-indent {
    width: 12px;
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
    background: color-mix(in srgb, var(--color-accent) 10%, var(--color-panel-2));
    outline: 1px solid color-mix(in srgb, var(--color-accent) 45%, transparent);
    outline-offset: -1px;
  }
  .profile-row--selected {
    background: color-mix(in srgb, var(--color-accent) 18%, var(--color-panel-2));
  }
  .note-chip {
    display: inline-grid;
    place-items: center;
    width: 16px;
    height: 16px;
    border-radius: 4px;
    color: var(--color-fg-muted);
    opacity: 0;
  }
  .profile-row:hover .note-chip,
  .note-chip.has-note {
    opacity: 0.9;
  }
  .note-chip:hover {
    color: var(--color-accent);
    background: var(--color-panel-2);
  }
</style>
