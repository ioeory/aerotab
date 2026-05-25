<script lang="ts">
  import { ChevronDown, ChevronRight, FolderOpen, Pencil, Star, Trash2 } from '@lucide/svelte';
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
    onToggleFolder: (path: string) => void;
    onOpenProfile: (p: StoredProfile) => void;
    onOpenSftp: (p: StoredProfile) => void;
    onEditProfile: (p: StoredProfile) => void;
    onDeleteProfile: (p: StoredProfile, ev: Event) => void;
    onContextMenu: (p: StoredProfile, ev: MouseEvent) => void;
    showUngroupedLabel?: boolean;
  }

  let {
    folder,
    depth = 0,
    showUngroupedLabel = false,
    collapsed,
    forceExpanded,
    onToggleFolder,
    onOpenProfile,
    onOpenSftp,
    onEditProfile,
    onDeleteProfile,
    onContextMenu,
  }: Props = $props();

  function isExpanded(path: string): boolean {
    if (forceExpanded.has(path)) return true;
    return !collapsed.has(path);
  }
</script>

{#each folder.folders as child (child.path)}
  {@const expanded = isExpanded(child.path)}
  <div class="profile-folder" style="--depth: {depth}">
    <button
      type="button"
      class="folder-header w-full flex items-center gap-1 py-1 pr-1 rounded-md text-left
             hover:bg-[var(--color-panel-2)] cursor-pointer text-[var(--color-fg-muted)]"
      onclick={() => onToggleFolder(child.path)}
      aria-expanded={expanded}
    >
      <span class="shrink-0 w-3.5 grid place-items-center">
        {#if expanded}
          <ChevronDown size={12} />
        {:else}
          <ChevronRight size={12} />
        {/if}
      </span>
      <span class="truncate text-[11px] font-medium text-[var(--color-fg)]">{child.name}</span>
      <span class="ml-auto shrink-0 text-[10px] opacity-70">
        {child.profiles.length + child.folders.length}
      </span>
    </button>
    {#if expanded}
      <div class="folder-children">
        <Self
          folder={child}
          depth={depth + 1}
          {collapsed}
          {forceExpanded}
          {onToggleFolder}
          {onOpenProfile}
          {onOpenSftp}
          {onEditProfile}
          {onDeleteProfile}
          {onContextMenu}
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
    class="profile-row group flex items-center gap-1 rounded-md hover:bg-[var(--color-panel-2)]"
    style="--depth: {depth}"
    role="presentation"
    oncontextmenu={(e) => onContextMenu(p, e)}
  >
    <div class="profile-row-indent shrink-0"></div>
    <div class="pl-0.5">
      <ProfileIcon icon={p.icon} name={p.name} size={13} />
    </div>
    <button
      type="button"
      onclick={() => onOpenProfile(p)}
      class="flex-1 min-w-0 text-left px-2 py-1.5 text-[12px]"
      title={profileEndpointLabel(p)}
    >
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
    </button>
    {#if p.kind === 'ssh'}
      <button
        type="button"
        class="opacity-0 group-hover:opacity-100 p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-accent)]"
        onclick={(e) => { e.stopPropagation(); onOpenSftp(p); }}
        title={i18n.t('sidebar.openSftpBrowser')}
        aria-label={i18n.t('sidebar.openSftpBrowser')}
      >
        <FolderOpen size={12} />
      </button>
    {/if}
    <button
      type="button"
      class="opacity-0 group-hover:opacity-100 p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-accent)]"
      onclick={(e) => { e.stopPropagation(); onEditProfile(p); }}
      title={i18n.t('common.edit')}
      aria-label={i18n.t('sidebar.editProfile')}
    >
      <Pencil size={12} />
    </button>
    <button
      type="button"
      class="opacity-0 group-hover:opacity-100 p-1 mr-1 text-[var(--color-fg-muted)] hover:text-[var(--color-danger)]"
      onclick={(e) => onDeleteProfile(p, e)}
      title={i18n.t('common.delete')}
      aria-label={i18n.t('sidebar.deleteProfile')}
    >
      <Trash2 size={12} />
    </button>
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
</style>
