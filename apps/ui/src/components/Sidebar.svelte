<script lang="ts">
  import { Plus, Terminal as TerminalIcon, Server, Usb, Settings as SettingsIcon, Search, X } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import type { SessionMeta, StoredProfile } from '../lib/types';
  import { tabs } from '../lib/tabs.svelte';
  import { dispatchFocusPane } from '../lib/focusPane';
  import { cloneProfileAsNew, matchesProfileQuery, suggestDuplicateProfileName } from '../lib/profileMeta';
  import { notifyProfilesChanged } from '../lib/profileEvents';
  import { uuidv4 } from '../lib/rpc';
  import {
    buildProfileTree,
    expandPathsForMatches,
    loadCollapsedPaths,
    saveCollapsedPaths,
    type ProfileTreeFolder,
  } from '../lib/profileTree';
  import SidebarProfileTree from './SidebarProfileTree.svelte';
  import { i18n } from '../lib/i18n.svelte';
  import { PROFILES_CHANGED } from '../lib/profileEvents';
  import { withRpcTimeout } from '../lib/rpcTimeout';
  import { onMount, onDestroy } from 'svelte';
  import logoUrl from '../assets/logo.png';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
    openProfileModal: (existing?: StoredProfile, options?: { group?: string }) => void;
    openSerialModal: () => void;
    openSftp: (p: StoredProfile) => void;
    openSettings: () => void;
  }
  let { rpc, onError, openProfileModal, openSerialModal, openSftp, openSettings }: Props = $props();

  let profiles = $state<StoredProfile[]>([]);
  let profileQuery = $state('');
  let collapsedPaths = $state<Set<string>>(loadCollapsedPaths());

  const sshProfiles = $derived(profiles.filter((p) => p.kind === 'ssh'));
  const filteredProfiles = $derived(
    sshProfiles.filter((p) => matchesProfileQuery(p, profileQuery)),
  );
  const profileTree = $derived(buildProfileTree(filteredProfiles));
  const forceExpandedPaths = $derived(
    profileQuery.trim()
      ? expandPathsForMatches(sshProfiles, (p) => matchesProfileQuery(p, profileQuery))
      : new Set<string>(),
  );
  const hasVisibleProfiles = $derived(
    filteredProfiles.length > 0,
  );

  function toggleFolder(path: string) {
    const next = new Set(collapsedPaths);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    collapsedPaths = next;
    saveCollapsedPaths(next);
  }

  function clearProfileSearch() {
    profileQuery = '';
  }

  export async function refresh() {
    try {
      profiles = await withRpcTimeout(
        rpc.call<StoredProfile[]>('profile.list'),
        20_000,
        'profile.list',
      );
    } catch {
      profiles = [];
    }
  }
  const onProfilesChanged = () => { void refresh(); };
  onMount(() => {
    void refresh();
    document.addEventListener(PROFILES_CHANGED, onProfilesChanged);
  });
  onDestroy(() => {
    document.removeEventListener(PROFILES_CHANGED, onProfilesChanged);
  });

  async function openLocal() {
    try {
      const meta = await rpc.call<SessionMeta>('session.openLocal', { title: 'local' });
      tabs.add(meta);
    } catch (e) {
      onError(`local: ${(e as Error).message}`);
    }
  }

  async function openProfile(p: StoredProfile, mode: 'new-tab' | 'split-right' | 'split-down' = 'new-tab') {
    if (p.kind === 'rdp' || p.kind === 'vnc') {
      try {
        await rpc.call('remote.openProfile', { profile_id: p.id });
      } catch (e) {
        onError(`remote: ${(e as Error).message}`);
      }
      return;
    }
    try {
      const meta = await rpc.call<SessionMeta>('session.openSsh', {
        title: p.name,
        rows: 24,
        cols: 80,
        profile: p.ssh,
      });
      const activeTab = tabs.tabs.find((t) => t.id === tabs.activeId);
      if (mode !== 'new-tab' && activeTab) {
        tabs.addPane(activeTab.id, { ...meta, profileId: p.id, sshProfile: p.ssh }, mode === 'split-down' ? 'col' : 'row');
      } else {
        tabs.add({ ...meta, profileId: p.id, sshProfile: p.ssh });
      }
    } catch (e) {
      onError(`ssh: ${(e as Error).message}`);
    }
  }

  type SidebarMenu =
    | { kind: 'profile'; profile: StoredProfile }
    | { kind: 'group'; groupPath: string; groupLabel: string };

  let menuOpen = $state(false);
  let menuX = $state(0);
  let menuY = $state(0);
  let menuTarget = $state<SidebarMenu | null>(null);

  function openMenu(target: SidebarMenu, ev: MouseEvent) {
    ev.preventDefault();
    ev.stopPropagation();
    menuTarget = target;
    menuX = ev.clientX;
    menuY = ev.clientY;
    menuOpen = true;
  }

  function showProfileMenu(p: StoredProfile, ev: MouseEvent) {
    openMenu({ kind: 'profile', profile: p }, ev);
  }

  function showFolderMenu(folder: ProfileTreeFolder, ev: MouseEvent) {
    openMenu({ kind: 'group', groupPath: folder.path, groupLabel: folder.name }, ev);
  }

  function closeMenu() {
    menuOpen = false;
    menuTarget = null;
  }

  function menuOpenInNewTab(p: StoredProfile) {
    closeMenu();
    void openProfile(p, 'new-tab');
  }
  function menuSplitRight(p: StoredProfile) {
    closeMenu();
    void openProfile(p, 'split-right');
  }
  function menuSplitDown(p: StoredProfile) {
    closeMenu();
    void openProfile(p, 'split-down');
  }
  function menuOpenSftp(p: StoredProfile) {
    closeMenu();
    openSftp(p);
  }
  function menuEdit(p: StoredProfile) {
    closeMenu();
    void editProfile(p);
  }
  async function menuDuplicateProfile(p: StoredProfile) {
    closeMenu();
    try {
      const fresh = await latestProfile(p);
      const newName = suggestDuplicateProfileName(
        fresh.name,
        profiles.map((x) => x.name),
      );
      const clone = cloneProfileAsNew(fresh, newName, uuidv4());
      await rpc.call('profile.upsert', clone);
      notifyProfilesChanged();
      await refresh();
    } catch (e) {
      onError(i18n.t('sidebar.duplicateProfileFailed', { message: (e as Error).message }));
    }
  }
  function menuDelete(p: StoredProfile) {
    closeMenu();
    void deleteProfile(p);
  }
  function menuNewProfileInGroup(groupPath: string) {
    closeMenu();
    openProfileModal(undefined, { group: groupPath });
  }

  async function latestProfile(p: StoredProfile): Promise<StoredProfile> {
    try {
      return await rpc.call<StoredProfile>('profile.get', { id: p.id });
    } catch (e) {
      onError(`profile refresh: ${(e as Error).message}`);
      return p;
    }
  }

  async function editProfile(p: StoredProfile) {
    const tab = tabs.tabs.find((candidate) =>
      candidate.panes.some((pane) => pane.profileId === p.id),
    );
    if (tab) {
      tabs.activate(tab.id);
      const pane = tab.panes.find((candidate) => candidate.profileId === p.id);
      if (pane) {
        tabs.focusPane(tab.id, pane.id);
        requestAnimationFrame(() => dispatchFocusPane(pane.id));
      }
    }
    openProfileModal(await latestProfile(p));
  }

  async function deleteProfile(p: StoredProfile) {
    if (!confirm(i18n.t('sidebar.deleteProfileConfirm', { name: p.name }))) return;
    try {
      await rpc.call('profile.delete', { id: p.id });
      await refresh();
    } catch (e) {
      onError((e as Error).message);
    }
  }
</script>

<aside data-aerotab-context-menu="" class="w-[240px] shrink-0 border-r border-[var(--color-border-soft)] bg-[var(--color-panel)] flex flex-col shadow-[inset_-1px_0_0_var(--color-border-soft)]">
  <div class="px-4 py-3 border-b border-[var(--color-border-soft)] flex items-center gap-2">
    <img src={logoUrl} alt="" class="aerotab-logo" width="24" height="24" />
    <h1 class="text-[13px] font-semibold tracking-wide font-mono">AeroTab</h1>
    <button
      type="button"
      onclick={openSettings}
      class="btn-ghost ml-auto"
      title={i18n.t('sidebar.settings')}
      aria-label={i18n.t('sidebar.settings')}
    >
      <SettingsIcon size={14} />
    </button>
  </div>

  <div class="px-2 py-2 flex flex-col gap-1">
    <button
      type="button"
      onclick={openLocal}
      class="list-item w-full text-[12.5px] text-left"
    >
      <TerminalIcon size={14} class="text-[var(--color-accent)]" />
      <span class="flex-1">{i18n.t('sidebar.newLocalShell')}</span>
      <Plus size={12} class="text-[var(--color-fg-muted)]" />
    </button>
    <button
      type="button"
      onclick={() => openProfileModal()}
      class="list-item w-full text-[12.5px] text-left"
    >
      <Server size={14} class="text-[var(--color-accent)]" />
      <span class="flex-1">{i18n.t('sidebar.newSshProfile')}</span>
      <Plus size={12} class="text-[var(--color-fg-muted)]" />
    </button>
    <button
      type="button"
      onclick={openSerialModal}
      class="list-item w-full text-[12.5px] text-left"
    >
      <Usb size={14} class="text-[var(--color-accent)]" />
      <span class="flex-1">{i18n.t('sidebar.newSerialConnection')}</span>
      <Plus size={12} class="text-[var(--color-fg-muted)]" />
    </button>
  </div>

  <div class="px-2 pt-2 pb-1 flex flex-col gap-1.5">
    <div class="px-1 shell-section-title">{i18n.t('sidebar.sshProfiles')}</div>
    <div class="relative">
      <Search size={12} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--color-fg-muted)] pointer-events-none" />
      <input
        type="search"
        bind:value={profileQuery}
        placeholder={i18n.t('sidebar.searchProfiles')}
        class="input w-full pl-8 pr-7 py-1.5 text-[12px]"
        aria-label={i18n.t('sidebar.searchProfiles')}
      />
      {#if profileQuery.trim()}
        <button
          type="button"
          class="btn-ghost absolute right-1 top-1/2 -translate-y-1/2 p-0.5"
          onclick={clearProfileSearch}
          aria-label={i18n.t('sidebar.clearSearch')}
        >
          <X size={12} />
        </button>
      {/if}
    </div>
    <div class="help px-1 text-[10px] leading-snug">{i18n.t('sidebar.groupPathHint')}</div>
  </div>
  <div class="flex-1 overflow-y-auto px-2 pb-3 flex flex-col gap-0.5 min-h-0">
    {#if !hasVisibleProfiles}
      <div class="px-3 py-2 text-[11.5px] text-[var(--color-fg-muted)] italic">
        {profileQuery.trim() ? i18n.t('sidebar.noSearchResults') : i18n.t('sidebar.noProfiles')}
      </div>
    {:else}
      <SidebarProfileTree
        folder={profileTree}
        collapsed={collapsedPaths}
        forceExpanded={forceExpandedPaths}
        onToggleFolder={toggleFolder}
        onOpenProfile={(p) => openProfile(p)}
        onProfileContextMenu={showProfileMenu}
        onFolderContextMenu={showFolderMenu}
        showUngroupedLabel={profileTree.folders.length > 0}
      />
    {/if}
  </div>
</aside>

{#if menuOpen && menuTarget}
  <div
    role="presentation"
    data-aerotab-context-menu=""
    class="fixed inset-0 z-[55]"
    onclick={closeMenu}
    oncontextmenu={(e) => {
      e.preventDefault();
      closeMenu();
    }}
  >
    <div
      role="menu"
      tabindex="-1"
      class="absolute min-w-[200px] panel py-1 text-[12.5px] text-[var(--color-fg)]"
      style="left:{menuX}px; top:{menuY}px;"
      onkeydown={(e) => e.stopPropagation()}
      onclick={(e) => e.stopPropagation()}
    >
      {#if menuTarget.kind === 'group'}
        {@const groupPath = menuTarget.groupPath}
        <button type="button" class="menu-item" onclick={() => menuNewProfileInGroup(groupPath)}>
          {i18n.t('sidebar.newProfileInGroup')}
        </button>
      {:else}
        {@const mp = menuTarget.profile}
        <button type="button" class="menu-item" onclick={() => menuEdit(mp)}>
          {i18n.t('sidebar.editProfile')}
        </button>
        <button type="button" class="menu-item" onclick={() => { void menuDuplicateProfile(mp); }}>
          {i18n.t('sidebar.duplicateProfile')}
        </button>
        <button type="button" class="menu-item text-[var(--color-danger)]" onclick={() => menuDelete(mp)}>
          {i18n.t('sidebar.removeProfile')}
        </button>
        <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
        <button type="button" class="menu-item" onclick={() => menuOpenInNewTab(mp)}>
          {i18n.t('sidebar.openInNewTab')}
        </button>
        <button type="button" class="menu-item" onclick={() => menuSplitRight(mp)}>
          {i18n.t('sidebar.splitRightCurrent')}
        </button>
        <button type="button" class="menu-item" onclick={() => menuSplitDown(mp)}>
          {i18n.t('sidebar.splitDownCurrent')}
        </button>
        {#if mp.kind === 'ssh'}
          <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
          <button type="button" class="menu-item" onclick={() => menuOpenSftp(mp)}>
            {i18n.t('sidebar.openSftpBrowser')}
          </button>
        {/if}
      {/if}
    </div>
  </div>
{/if}
