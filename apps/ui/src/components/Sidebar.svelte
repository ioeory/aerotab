<script lang="ts">
  import {
    Plus, Terminal as TerminalIcon, Server, Usb, Settings as SettingsIcon, Search, X, RefreshCw,
  } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import type { SessionMeta, StoredProfile } from '../lib/types';
  import { tabs } from '../lib/tabs.svelte';
  import { focusProfileInTabs } from '../lib/focusProfileSession';
  import { matchesProfileQuery } from '../lib/profileMeta';
  import type { ProfileModalOpenOptions } from './ProfileModal.svelte';
  import {
    handleProfileSidebarShortcut,
    profileSidebarBindingLabel,
    type ProfileSidebarActionKey,
  } from '../lib/profileSidebarShortcuts';
  import {
    buildProfileTree,
    expandPathsForGroup,
    expandPathsForMatches,
    loadCollapsedPaths,
    saveCollapsedPaths,
    type ProfileTreeFolder,
  } from '../lib/profileTree';
  import SidebarProfileTree from './SidebarProfileTree.svelte';
  import { i18n } from '../lib/i18n.svelte';
  import { notifyProfilesChanged, PROFILES_CHANGED, type ProfilesChangedDetail } from '../lib/profileEvents';
  import { withRpcTimeout } from '../lib/rpcTimeout';
  import { portal } from '../lib/portal';
  import { appConfirm } from '../lib/confirm.svelte';
  import {
    invertProfileSelection,
    profilesFromSelection,
    rangeSelectProfiles,
    selectAllProfiles,
    toggleProfileInSelection,
  } from '../lib/profileSelection';
  import type { ProfileHealthResult } from '../lib/types';
  import { onMount, onDestroy, tick } from 'svelte';
  import logoUrl from '../assets/logo.png';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
    openProfileModal: (existing?: StoredProfile, options?: ProfileModalOpenOptions) => void;
    openSerialModal: () => void;
    openSftp: (p: StoredProfile) => void;
    openSettings: () => void;
  }
  let { rpc, onError, openProfileModal, openSerialModal, openSftp, openSettings }: Props = $props();

  let profiles = $state<StoredProfile[]>([]);
  let profilesRefreshing = $state(false);
  let profileQuery = $state('');
  let collapsedPaths = $state<Set<string>>(loadCollapsedPaths());

  const filteredProfiles = $derived(
    profiles.filter((p) => matchesProfileQuery(p, profileQuery)),
  );
  const profileTree = $derived(buildProfileTree(filteredProfiles));
  const forceExpandedPaths = $derived(
    profileQuery.trim()
      ? expandPathsForMatches(profiles, (p) => matchesProfileQuery(p, profileQuery))
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

  function expandFoldersForGroup(group: string | null | undefined) {
    const paths = expandPathsForGroup(group);
    if (paths.size === 0) return;
    const next = new Set(collapsedPaths);
    for (const path of paths) next.delete(path);
    collapsedPaths = next;
    saveCollapsedPaths(next);
  }

  export async function refresh() {
    if (profilesRefreshing) return;
    profilesRefreshing = true;
    try {
      profiles = await withRpcTimeout(
        rpc.call<StoredProfile[]>('profile.list'),
        20_000,
        'profile.list',
      );
    } catch (e) {
      profiles = [];
      onError(`profile.list: ${(e as Error).message}`);
    } finally {
      profilesRefreshing = false;
    }
  }

  const onProfilesChanged = (ev: Event) => {
    const detail = (ev as CustomEvent<ProfilesChangedDetail>).detail;
    if (detail?.group) expandFoldersForGroup(detail.group);
    void refresh();
  };
  let hotkeyRev = $state(0);
  const onHotkeysChanged = () => { hotkeyRev += 1; };

  onMount(() => {
    void refresh();
    document.addEventListener(PROFILES_CHANGED, onProfilesChanged);
    document.addEventListener('aerotab:settings-changed', onHotkeysChanged);
    document.addEventListener('aerotab:settings-synced', onHotkeysChanged);
  });
  onDestroy(() => {
    document.removeEventListener(PROFILES_CHANGED, onProfilesChanged);
    document.removeEventListener('aerotab:settings-changed', onHotkeysChanged);
    document.removeEventListener('aerotab:settings-synced', onHotkeysChanged);
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
  let menuEl = $state<HTMLDivElement | null>(null);
  let focusedProfileId = $state<string | null>(null);
  let selectedProfileIds = $state<Set<string>>(new Set());
  let selectionAnchorId = $state<string | null>(null);
  let bulkBusy = $state(false);

  const selectedProfiles = $derived(profilesFromSelection(filteredProfiles, selectedProfileIds));
  const hasProfileSelection = $derived(selectedProfileIds.size > 0);

  function clampMenuToViewport(x: number, y: number, el: HTMLDivElement | null): { x: number; y: number } {
    if (!el) return { x, y };
    const pad = 8;
    const maxX = Math.max(pad, window.innerWidth - el.offsetWidth - pad);
    const maxY = Math.max(pad, window.innerHeight - el.offsetHeight - pad);
    return {
      x: Math.min(Math.max(pad, x), maxX),
      y: Math.min(Math.max(pad, y), maxY),
    };
  }

  async function openMenu(target: SidebarMenu, ev: MouseEvent) {
    ev.preventDefault();
    ev.stopPropagation();
    menuTarget = target;
    menuX = ev.clientX;
    menuY = ev.clientY;
    menuOpen = true;
    await tick();
    const clamped = clampMenuToViewport(menuX, menuY, menuEl);
    menuX = clamped.x;
    menuY = clamped.y;
  }

  function focusProfile(p: StoredProfile) {
    focusedProfileId = p.id;
  }

  function clearProfileSelection() {
    selectedProfileIds = new Set();
    selectionAnchorId = null;
  }

  function onProfileRowClick(p: StoredProfile, ev: MouseEvent) {
    if (ev.shiftKey && selectionAnchorId) {
      selectedProfileIds = rangeSelectProfiles(filteredProfiles, selectionAnchorId, p.id);
    } else if (ev.ctrlKey || ev.metaKey) {
      selectedProfileIds = toggleProfileInSelection(selectedProfileIds, p.id);
      selectionAnchorId = p.id;
    } else {
      focusProfile(p);
      selectionAnchorId = p.id;
    }
  }

  function toggleProfileCheckbox(p: StoredProfile) {
    selectedProfileIds = toggleProfileInSelection(selectedProfileIds, p.id);
    selectionAnchorId = p.id;
    focusProfile(p);
  }

  async function bulkConnectSelected() {
    const list = selectedProfiles.filter((p) => p.kind === 'ssh');
    if (list.length === 0) return;
    bulkBusy = true;
    let opened = 0;
    try {
      for (const p of list) {
        try {
          await openProfile(p, 'new-tab');
          opened += 1;
        } catch {
          /* onError already called in openProfile */
        }
      }
      void opened;
    } finally {
      bulkBusy = false;
    }
  }

  async function bulkHealthCheckSelected() {
    const ids = selectedProfiles.map((p) => p.id);
    if (ids.length === 0 || bulkBusy) return;
    bulkBusy = true;
    try {
      const results = await withRpcTimeout(
        rpc.call<ProfileHealthResult[]>('profile.healthCheck', { ids, connect: true }),
        60_000,
        'profile.healthCheck',
      );
      const failed = results.filter((r) => r.status === 'error').length;
      if (failed > 0) {
        onError(i18n.t('profiles.healthSummary', {
          ok: results.filter((r) => r.status === 'ok').length,
          warning: results.filter((r) => r.status === 'warning').length,
          error: failed,
        }));
      }
    } catch (e) {
      onError(`profile health: ${(e as Error).message}`);
    } finally {
      bulkBusy = false;
    }
  }

  async function bulkDeleteSelected() {
    const list = selectedProfiles;
    if (list.length === 0) return;
    if (!(await appConfirm(i18n.t('profiles.bulkDeleteConfirm', { count: list.length }), {
      danger: true,
      confirmLabel: i18n.t('common.delete'),
    }))) return;
    bulkBusy = true;
    try {
      for (const p of list) {
        await rpc.call('profile.delete', { id: p.id });
      }
      clearProfileSelection();
      notifyProfilesChanged();
      await refresh();
    } catch (e) {
      onError((e as Error).message);
    } finally {
      bulkBusy = false;
    }
  }

  function showProfileMenu(p: StoredProfile, ev: MouseEvent) {
    focusProfile(p);
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
  async function cloneProfile(p: StoredProfile) {
    closeMenu();
    try {
      const fresh = await latestProfile(p);
      openProfileModal(undefined, {
        duplicateFrom: fresh,
        existingNames: profiles.map((x) => x.name),
      });
    } catch (e) {
      onError(i18n.t('sidebar.cloneProfileFailed', { message: (e as Error).message }));
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
    focusProfileInTabs(p.id);
    openProfileModal(await latestProfile(p));
  }

  async function deleteProfile(p: StoredProfile) {
    if (!(await appConfirm(i18n.t('sidebar.deleteProfileConfirm', { name: p.name }), { danger: true, confirmLabel: i18n.t('common.delete') }))) return;
    try {
      await rpc.call('profile.delete', { id: p.id });
      if (focusedProfileId === p.id) focusedProfileId = null;
      notifyProfilesChanged();
      await refresh();
    } catch (e) {
      onError((e as Error).message);
    }
  }

  const profileShortcutHandlers = {
    onEdit: (p: StoredProfile) => { void editProfile(p); },
    onClone: (p: StoredProfile) => { void cloneProfile(p); },
    onRemove: (p: StoredProfile) => { void deleteProfile(p); },
    onOpenNewTab: (p: StoredProfile) => { void openProfile(p, 'new-tab'); },
    onSplitRight: (p: StoredProfile) => { void openProfile(p, 'split-right'); },
    onSplitDown: (p: StoredProfile) => { void openProfile(p, 'split-down'); },
    onOpenSftp: (p: StoredProfile) => { openSftp(p); },
  };

  function onProfileKeydown(p: StoredProfile, ev: KeyboardEvent) {
    focusProfile(p);
    if (handleProfileSidebarShortcut(p, ev, profileShortcutHandlers)) {
      ev.preventDefault();
      ev.stopPropagation();
    }
  }

  function shortcutKbd(key: ProfileSidebarActionKey): string {
    void hotkeyRev;
    return profileSidebarBindingLabel(key);
  }
</script>

<aside data-aerotab-context-menu="" class="w-full min-w-0 h-full bg-[var(--color-panel)] flex flex-col shadow-[inset_-1px_0_0_var(--color-border-soft)]">
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
    <div class="px-1 flex items-center gap-1 min-h-[22px]">
      <div class="shell-section-title flex-1 min-w-0">{i18n.t('sidebar.sshProfiles')}</div>
      <button
        type="button"
        class="btn-ghost p-1 shrink-0 text-[var(--color-fg-muted)] hover:text-[var(--color-accent)]"
        title={i18n.t('sidebar.refreshProfiles')}
        aria-label={i18n.t('sidebar.refreshProfiles')}
        disabled={profilesRefreshing}
        onclick={() => { void refresh(); }}
      >
        <RefreshCw size={13} class={profilesRefreshing ? 'animate-spin' : ''} />
      </button>
    </div>
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
    {#if hasProfileSelection}
      <div class="flex flex-wrap items-center gap-1 px-1 py-1 border border-[var(--color-border-soft)] rounded-md bg-[var(--color-panel-2)]">
        <span class="text-[10px] text-[var(--color-fg-muted)]">{i18n.t('profiles.selectedCount', { count: selectedProfileIds.size })}</span>
        <button type="button" class="btn-secondary text-[10px] py-0.5 px-1.5" disabled={bulkBusy}
                onclick={() => { selectedProfileIds = selectAllProfiles(filteredProfiles); }}>
          {i18n.t('profiles.selectAll')}
        </button>
        <button type="button" class="btn-secondary text-[10px] py-0.5 px-1.5" disabled={bulkBusy}
                onclick={() => { selectedProfileIds = invertProfileSelection(selectedProfileIds, filteredProfiles); }}>
          {i18n.t('profiles.invertSelection')}
        </button>
        <button type="button" class="btn-secondary text-[10px] py-0.5 px-1.5" disabled={bulkBusy}
                onclick={clearProfileSelection}>
          {i18n.t('profiles.clearSelection')}
        </button>
        <button type="button" class="btn-secondary text-[10px] py-0.5 px-1.5" disabled={bulkBusy}
                onclick={() => { void bulkConnectSelected(); }}>
          {i18n.t('profiles.bulkConnect')}
        </button>
        <button type="button" class="btn-secondary text-[10px] py-0.5 px-1.5" disabled={bulkBusy}
                onclick={() => { void bulkHealthCheckSelected(); }}>
          {i18n.t('profiles.bulkHealthCheck')}
        </button>
        <button type="button" class="btn-secondary text-[10px] py-0.5 px-1.5 text-[var(--color-danger)]" disabled={bulkBusy}
                onclick={() => { void bulkDeleteSelected(); }}>
          {i18n.t('profiles.bulkDelete')}
        </button>
      </div>
    {/if}
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
        focusedProfileId={focusedProfileId}
        selectedProfileIds={selectedProfileIds}
        showSelection={hasProfileSelection}
        onToggleFolder={toggleFolder}
        onOpenProfile={(p) => openProfile(p)}
        onProfileClick={onProfileRowClick}
        onProfileCheckboxToggle={toggleProfileCheckbox}
        onProfileFocus={focusProfile}
        onProfileKeydown={onProfileKeydown}
        onProfileContextMenu={showProfileMenu}
        onFolderContextMenu={showFolderMenu}
        showUngroupedLabel={profileTree.folders.length > 0}
      />
    {/if}
  </div>
</aside>

{#if menuOpen && menuTarget}
  <div use:portal class="contents">
  <div
    role="presentation"
    class="fixed inset-0 z-[55]"
    onclick={closeMenu}
    oncontextmenu={(e) => {
      e.preventDefault();
      closeMenu();
    }}
  ></div>
  <div
    bind:this={menuEl}
    role="menu"
    tabindex="-1"
    data-aerotab-context-menu=""
    class="panel fixed z-[56] min-w-[200px] py-1 text-[12.5px] text-[var(--color-fg)]"
    style="left: {menuX}px; top: {menuY}px;"
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
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => menuEdit(mp)}>
          <span>{i18n.t('sidebar.editProfile')}</span>
          <kbd class="kbd">{shortcutKbd('edit')}</kbd>
        </button>
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => { void cloneProfile(mp); }}>
          <span>{i18n.t('sidebar.cloneProfile')}</span>
          <kbd class="kbd">{shortcutKbd('clone')}</kbd>
        </button>
        <button type="button" class="menu-item menu-item--shortcut text-[var(--color-danger)]" onclick={() => menuDelete(mp)}>
          <span>{i18n.t('sidebar.removeProfile')}</span>
          <kbd class="kbd">{shortcutKbd('remove')}</kbd>
        </button>
        <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => menuOpenInNewTab(mp)}>
          <span>{i18n.t('sidebar.openInNewTab')}</span>
          <kbd class="kbd">{shortcutKbd('openNewTab')}</kbd>
        </button>
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => menuSplitRight(mp)}>
          <span>{i18n.t('sidebar.splitRightCurrent')}</span>
          <kbd class="kbd">{shortcutKbd('splitRight')}</kbd>
        </button>
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => menuSplitDown(mp)}>
          <span>{i18n.t('sidebar.splitDownCurrent')}</span>
          <kbd class="kbd">{shortcutKbd('splitDown')}</kbd>
        </button>
        {#if mp.kind === 'ssh'}
          <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
          <button type="button" class="menu-item menu-item--shortcut" onclick={() => menuOpenSftp(mp)}>
            <span>{i18n.t('sidebar.openSftpBrowser')}</span>
            <kbd class="kbd">{shortcutKbd('sftp')}</kbd>
          </button>
        {/if}
      {/if}
  </div>
  </div>
{/if}
