<script lang="ts">
  import {
    Plus, Terminal as TerminalIcon, Server, Usb, Settings as SettingsIcon, Search, X, RefreshCw, ArrowLeftRight,
  } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import type { SessionMeta, StoredProfile } from '../lib/types';
  import { tabs } from '../lib/tabs.svelte';
  import { healthIssueDetailText, summarizeHealthResults } from '../lib/profileHealthUi';
  import { focusProfileInTabs } from '../lib/focusProfileSession';
  import { formatTags, matchesProfileQuery, parseProfileIconInput, parseTagsInput, profileEndpointLabel } from '../lib/profileMeta';
  import { pickIconFilePath } from '../lib/localFiles';
  import type { ProfileModalOpenOptions } from './ProfileModal.svelte';
  import {
    handleProfileSidebarShortcut,
    profileSidebarBindingLabel,
    type ProfileSidebarActionKey,
  } from '../lib/profileSidebarShortcuts';
  import {
    BULK_OPEN_CONFIRM_THRESHOLD,
    openProfilesEachInNewTab,
    openProfilesInSameTab,
  } from '../lib/profileBulkOpen';
  import {
    buildProfileTree,
    collectProfileGroupPaths,
    collectProfilesInFolder,
    expandPathsForGroup,
    expandPathsForMatches,
    loadCollapsedPaths,
    normalizeGroupPath,
    saveCollapsedPaths,
    type ProfileTreeFolder,
  } from '../lib/profileTree';
  import SidebarProfileTree, { type ProfileQuickAction } from './SidebarProfileTree.svelte';
  import VisualColorPicker from './VisualColorPicker.svelte';
  import ProfileTag from './ProfileTag.svelte';
  import { normalizeGroupKey, normalizeTagKey } from '../lib/profileVisuals';
  import { profileVisualsStore } from '../lib/profileVisualsStore.svelte';
  import { i18n } from '../lib/i18n.svelte';
  import { notifyProfilesChanged, PROFILES_CHANGED, type ProfilesChangedDetail } from '../lib/profileEvents';
  import { withRpcTimeout } from '../lib/rpcTimeout';
  import { portal } from '../lib/portal';
  import { appConfirm, appPrompt, dialogAnchorFromEvent } from '../lib/confirm.svelte';
  import { jumpHostCandidates, profileSpecViaJump } from '../lib/jumpProfiles';
  import {
    defaultGroupForMove,
    normalizeProfileGroupInput,
    upsertProfilesGroup,
  } from '../lib/profileGroupMove';
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
    onShowTransfer?: () => void;
  }
  let {
    rpc,
    onError,
    openProfileModal,
    openSerialModal,
    openSftp,
    openSettings,
    onShowTransfer,
  }: Props = $props();

  let profiles = $state<StoredProfile[]>([]);
  let profilesRefreshing = $state(false);
  let profileQuery = $state('');
  let collapsedPaths = $state<Set<string>>(loadCollapsedPaths());
  let explicitGroupPaths = $state<string[]>([]);

  const PROFILE_GROUPS_SETTINGS_KEY = 'profile.groups';

  const filteredProfiles = $derived(
    profiles.filter((p) => matchesProfileQuery(p, profileQuery)),
  );
  const allGroupPaths = $derived.by(() => {
    const groups = new Set<string>(explicitGroupPaths);
    for (const group of collectProfileGroupPaths(profiles)) groups.add(group);
    return [...groups].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' }));
  });
  const visibleExplicitGroupPaths = $derived.by(() => {
    const query = profileQuery.trim().toLowerCase();
    if (!query) return explicitGroupPaths;
    return explicitGroupPaths.filter((group) => group.toLowerCase().includes(query));
  });
  const profileTree = $derived(buildProfileTree(filteredProfiles, visibleExplicitGroupPaths));
  const forceExpandedPaths = $derived(
    profileQuery.trim()
      ? expandPathsForMatches(profiles, (p) => matchesProfileQuery(p, profileQuery))
      : new Set<string>(),
  );
  const hasVisibleProfiles = $derived(
    filteredProfiles.length > 0 || profileTree.folders.length > 0,
  );

  function normalizeGroupList(value: unknown): string[] {
    if (!Array.isArray(value)) return [];
    const out = new Set<string>();
    for (const raw of value) {
      if (typeof raw !== 'string') continue;
      const normalized = normalizeGroupPath(raw);
      if (normalized) out.add(normalized);
    }
    return [...out].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' }));
  }

  async function loadExplicitGroups() {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: PROFILE_GROUPS_SETTINGS_KEY });
      explicitGroupPaths = normalizeGroupList(r.value);
    } catch {
      explicitGroupPaths = [];
    }
  }

  async function saveExplicitGroups(paths: string[]) {
    const normalized = normalizeGroupList(paths);
    explicitGroupPaths = normalized;
    await rpc.call('settings.set', { key: PROFILE_GROUPS_SETTINGS_KEY, value: normalized });
  }

  async function ensureExplicitGroup(group: string | null | undefined) {
    const normalized = normalizeGroupPath(group);
    if (!normalized || explicitGroupPaths.includes(normalized)) return;
    await saveExplicitGroups([...explicitGroupPaths, normalized]);
  }

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

  async function createGroup(parentPath: string | null = null) {
    closeMenu();
    const prefix = normalizeGroupPath(parentPath);
    const defaultValue = prefix ? `${prefix}/` : '';
    const value = await appPrompt(i18n.t('sidebar.createGroupPrompt'), {
      defaultValue,
      placeholder: i18n.t('profileModal.groupPlaceholder'),
      confirmLabel: i18n.t('sidebar.createGroup'),
      position: lastMenuPosition ?? undefined,
    });
    if (value === null) return;
    const normalized = normalizeGroupPath(value);
    if (!normalized) return;
    try {
      await saveExplicitGroups([...explicitGroupPaths, normalized]);
      expandFoldersForGroup(normalized);
      notifyProfilesChanged({ group: normalized });
    } catch (e) {
      onError(i18n.t('sidebar.createGroupFailed', { message: (e as Error).message }));
    }
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
      const [profileList] = await Promise.all([
        withRpcTimeout(
          rpc.call<StoredProfile[]>('profile.list'),
          20_000,
          'profile.list',
        ),
        loadExplicitGroups(),
      ]);
      profiles = profileList;
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

  function setDialogAnchor(ev?: MouseEvent) {
    if (ev) lastMenuPosition = dialogAnchorFromEvent(ev);
  }

  function onProfileQuickAction(p: StoredProfile, action: ProfileQuickAction, ev: MouseEvent) {
    setDialogAnchor(ev);
    switch (action) {
      case 'note':
        void promptEditNote(p);
        break;
      case 'tags':
        void promptEditTags(p);
        break;
      case 'icon':
        void promptEditIcon(p);
        break;
      case 'edit':
        void editProfile(p);
        break;
      case 'sftp':
        openSftp(p);
        break;
    }
  }

  async function openProfileViaJump(
    target: StoredProfile,
    jump: StoredProfile,
    mode: 'new-tab' | 'split-right' | 'split-down' = 'new-tab',
  ) {
    const freshTarget = await latestProfile(target);
    const freshJump = profiles.find((p) => p.id === jump.id) ?? jump;
    if (freshTarget.kind !== 'ssh' || freshJump.kind !== 'ssh') return;
    try {
      const profile = profileSpecViaJump(freshTarget, freshJump, profiles);
      const meta = await rpc.call<SessionMeta>('session.openSsh', {
        title: freshTarget.name,
        rows: 24,
        cols: 80,
        profile,
      });
      const activeTab = tabs.tabs.find((t) => t.id === tabs.activeId);
      const paneMeta = { ...meta, profileId: freshTarget.id, sshProfile: profile };
      if (mode !== 'new-tab' && activeTab) {
        tabs.addPane(activeTab.id, paneMeta, mode === 'split-down' ? 'col' : 'row');
      } else {
        tabs.add(paneMeta);
      }
    } catch (e) {
      onError(`ssh ${freshTarget.name} via ${freshJump.name}: ${(e as Error).message}`);
    }
  }

  function jumpHostOptions(target: StoredProfile): Array<StoredProfile & { kind: 'ssh' }> {
    return jumpHostCandidates(target, profiles);
  }

  function menuConnectViaJump(target: StoredProfile, jump: StoredProfile, ev: MouseEvent) {
    setDialogAnchor(ev);
    closeMenu();
    void openProfileViaJump(target, jump);
  }

  async function promptEditTags(p: StoredProfile) {
    const fresh = await latestProfile(p);
    const value = await appPrompt(i18n.t('sidebar.editTagsPrompt'), {
      defaultValue: formatTags(fresh.tags),
      placeholder: 'prod, db, singapore',
      confirmLabel: i18n.t('common.save'),
      position: lastMenuPosition ?? undefined,
    });
    if (value === null) return;
    try {
      await rpc.call('profile.upsert', { ...fresh, tags: parseTagsInput(value) });
      notifyProfilesChanged({ profileId: p.id, group: fresh.group });
      await refresh();
    } catch (e) {
      onError(i18n.t('profileModal.saveFailed', { message: (e as Error).message }));
    }
  }

  async function promptEditNote(p: StoredProfile) {
    const fresh = await latestProfile(p);
    const value = await appPrompt(i18n.t('sidebar.editNotePrompt'), {
      defaultValue: fresh.note ?? '',
      placeholder: i18n.t('profileModal.notePlaceholder'),
      confirmLabel: i18n.t('common.save'),
      multiline: true,
      rows: 4,
      position: lastMenuPosition ?? undefined,
    });
    if (value === null) return;
    try {
      await rpc.call('profile.upsert', { ...fresh, note: value.trim() || null });
      notifyProfilesChanged({ profileId: p.id, group: fresh.group });
      await refresh();
    } catch (e) {
      onError(i18n.t('profileModal.saveFailed', { message: (e as Error).message }));
    }
  }

  async function promptEditIcon(p: StoredProfile) {
    const fresh = await latestProfile(p);
    const defaultValue = fresh.icon?.kind === 'selfhst'
      ? `selfhst:${fresh.icon.value}`
      : fresh.icon?.kind === 'remote'
        ? `remote:${fresh.icon.value}`
        : fresh.icon?.value ?? '';
    const value = await appPrompt(i18n.t('sidebar.editIconPrompt'), {
      defaultValue,
      placeholder: 'selfhst:home-assistant, server, emoji:rocket, remote:https://host/a.svg|https://host/b.png',
      confirmLabel: i18n.t('common.save'),
      position: lastMenuPosition ?? undefined,
    });
    if (value === null) return;
    try {
      await rpc.call('profile.upsert', {
        ...fresh,
        icon: parseProfileIconInput(value),
      });
      notifyProfilesChanged({ profileId: p.id, group: fresh.group });
      await refresh();
    } catch (e) {
      onError(i18n.t('profileModal.saveFailed', { message: (e as Error).message }));
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
      onError(`ssh ${p.name} (${profileEndpointLabel(p)}): ${(e as Error).message}`);
    }
  }

  type SidebarMenu =
    | { kind: 'profile'; profile: StoredProfile }
    | { kind: 'group'; folder: ProfileTreeFolder; groupLabel: string };

  let menuOpen = $state(false);
  let menuX = $state(0);
  let menuY = $state(0);
  let menuTarget = $state<SidebarMenu | null>(null);
  let menuEl = $state<HTMLDivElement | null>(null);
  let jumpSubmenuQuery = $state('');
  let submenuLeft = $state(8);
  let submenuTop = $state(8);
  let focusedProfileId = $state<string | null>(null);
  let focusedGroupPath = $state<string | null>(null);
  let lastMenuPosition = $state<{ x: number; y: number } | null>(null);
  let selectedProfileIds = $state<Set<string>>(new Set());
  let selectionAnchorId = $state<string | null>(null);
  let bulkBusy = $state(false);
  let healthRunning = $state(false);
  let profileHealth = $state<Record<string, ProfileHealthResult>>({});

  const bulkOpenDeps = $derived({
    rpc,
    onError,
    onSummary: async (message: string) => {
      await appConfirm(message, { confirmLabel: i18n.t('common.ok') });
    },
  });

  function bulkOpenConfirmMany(
    count: number,
    messageKey:
      | 'profiles.bulkOpenManyConfirm'
      | 'profiles.bulkOpenManyConfirmSameNewTab'
      | 'profiles.bulkOpenManyConfirmEachNewTab',
  ): Promise<boolean> {
    return appConfirm(i18n.t(messageKey, { count }), {
      confirmLabel: i18n.t('profiles.bulkOpenConfirm'),
    });
  }

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
    lastMenuPosition = { x: ev.clientX, y: ev.clientY };
    menuTarget = target;
    menuX = ev.clientX;
    menuY = ev.clientY;
    menuOpen = true;
    await tick();
    const clamped = clampMenuToViewport(menuX, menuY, menuEl);
    menuX = clamped.x;
    menuY = clamped.y;
    lastMenuPosition = { x: menuX, y: menuY };
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

  async function bulkOpenSelected(mode: 'active' | 'new-same' | 'new-each') {
    const list = selectedProfiles;
    if (list.length === 0) {
      onError(i18n.t('profiles.bulkConnectNone'));
      return;
    }
    const sshList = list.filter((p) => p.kind === 'ssh');
    const confirmKey =
      mode === 'new-each'
        ? 'profiles.bulkOpenManyConfirmEachNewTab'
        : mode === 'new-same'
          ? 'profiles.bulkOpenManyConfirmSameNewTab'
          : 'profiles.bulkOpenManyConfirm';
    const deps = {
      ...bulkOpenDeps,
      confirmMany:
        sshList.length > BULK_OPEN_CONFIRM_THRESHOLD
          ? async (count: number) => bulkOpenConfirmMany(count, confirmKey)
          : undefined,
    };
    bulkBusy = true;
    try {
      if (mode === 'new-each') {
        await openProfilesEachInNewTab(list, deps);
      } else {
        await openProfilesInSameTab(list, deps, {
          tabTarget: mode === 'new-same' ? 'new' : 'active',
        });
      }
    } catch (e) {
      onError(`connect: ${(e as Error).message}`);
    } finally {
      bulkBusy = false;
    }
  }

  async function bulkHealthCheckSelected() {
    const ids = selectedProfiles.map((p) => p.id);
    if (ids.length === 0 || bulkBusy || healthRunning) return;
    healthRunning = true;
    bulkBusy = true;
    try {
      const results = await withRpcTimeout(
        rpc.call<ProfileHealthResult[]>('profile.healthCheck', { ids, connect: true }),
        120_000,
        'profile.healthCheck',
      );
      profileHealth = {
        ...profileHealth,
        ...Object.fromEntries(results.map((r) => [r.id, r])),
      };
      const summary = summarizeHealthResults(results);
      let message = i18n.t('profiles.healthSummary', summary);
      const details = healthIssueDetailText(results);
      if (details) message += `\n\n${details}`;
      await appConfirm(message, { confirmLabel: i18n.t('common.ok') });
    } catch (e) {
      onError(`profile health: ${(e as Error).message}`);
    } finally {
      healthRunning = false;
      bulkBusy = false;
    }
  }

  function profilesToMove(context: StoredProfile): StoredProfile[] {
    if (selectedProfileIds.has(context.id) && selectedProfileIds.size > 0) {
      return selectedProfiles;
    }
    return [context];
  }


  function profilesForAction(context: StoredProfile): StoredProfile[] {
    return selectedProfileIds.has(context.id) && selectedProfileIds.size > 0 ? selectedProfiles : [context];
  }

  async function deleteProfilesForAction(context: StoredProfile) {
    const list = profilesForAction(context);
    if (list.length > 1) {
      await bulkDeleteProfiles(list);
      return;
    }
    await deleteProfile(context);
  }

  async function openProfilesForAction(context: StoredProfile, mode: 'new-tab' | 'split-right' | 'split-down') {
    const list = profilesForAction(context);
    if (list.length <= 1) {
      await openProfile(context, mode);
      return;
    }
    bulkBusy = true;
    try {
      const deps = {
        ...bulkOpenDeps,
        confirmMany:
          list.filter((p) => p.kind === 'ssh').length > BULK_OPEN_CONFIRM_THRESHOLD
            ? async (count: number) => bulkOpenConfirmMany(count, 'profiles.bulkOpenManyConfirmEachNewTab')
            : undefined,
      };
      if (mode === 'new-tab') {
        await openProfilesEachInNewTab(list, deps);
      } else {
        await openProfilesInSameTab(list, deps, { tabTarget: 'active' });
      }
    } catch (e) {
      onError(`connect: ${(e as Error).message}`);
    } finally {
      bulkBusy = false;
    }
  }

  async function moveProfilesToGroup(profiles: StoredProfile[], group: string | null) {
    if (profiles.length === 0 || bulkBusy) return;
    closeMenu();
    bulkBusy = true;
    try {
      const moved = await upsertProfilesGroup(rpc, profiles, group);
      if (moved > 0) {
        await ensureExplicitGroup(group);
        notifyProfilesChanged({ group });
        expandFoldersForGroup(group);
      }
      await refresh();
    } catch (e) {
      onError(i18n.t('profiles.moveToGroupFailed', { message: (e as Error).message }));
    } finally {
      bulkBusy = false;
    }
  }

  async function promptAndMoveProfiles(profiles: StoredProfile[]) {
    if (profiles.length === 0) return;
    const value = await appPrompt(
      i18n.t('profiles.moveToGroupPrompt', { count: profiles.length }),
      {
        defaultValue: defaultGroupForMove(profiles),
        placeholder: i18n.t('profileModal.groupPlaceholder'),
        confirmLabel: i18n.t('profiles.moveToGroup'),
        position: lastMenuPosition ?? undefined,
      },
    );
    if (value === null) return;
    await moveProfilesToGroup(profiles, normalizeProfileGroupInput(value));
  }

  async function moveProfilesToExistingGroup(list: StoredProfile[], group: string | null) {
    await moveProfilesToGroup(list, group);
  }

  async function bulkMoveSelected() {
    await promptAndMoveProfiles(selectedProfiles);
  }

  async function bulkDeleteProfiles(list: StoredProfile[]) {
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

  async function bulkDeleteSelected() {
    await bulkDeleteProfiles(selectedProfiles);
  }

  function showProfileMenu(p: StoredProfile, ev: MouseEvent) {
    if (!selectedProfileIds.has(p.id)) {
      selectedProfileIds = new Set([p.id]);
      selectionAnchorId = p.id;
    }
    focusProfile(p);
    openMenu({ kind: 'profile', profile: p }, ev);
  }

  function showFolderMenu(folder: ProfileTreeFolder, ev: MouseEvent) {
    focusedGroupPath = folder.path;
    openMenu({ kind: 'group', folder, groupLabel: folder.name || i18n.t('sidebar.ungrouped') }, ev);
  }

  function closeMenu() {
    menuOpen = false;
    menuTarget = null;
    jumpSubmenuQuery = '';
  }

  function filteredJumpHosts(target: StoredProfile): Array<StoredProfile & { kind: 'ssh' }> {
    const needle = jumpSubmenuQuery.trim().toLowerCase();
    const list = jumpHostOptions(target);
    if (!needle) return list;
    return list.filter((p) =>
      [p.name, p.ssh.host, p.ssh.user, String(p.ssh.port), ...(p.tags ?? [])]
        .join(' ')
        .toLowerCase()
        .includes(needle),
    );
  }

  async function alignSubmenu(ev: MouseEvent) {
    const wrap = ev.currentTarget as HTMLElement;
    const trigger = wrap.querySelector(':scope > .menu-item') as HTMLElement | null;
    const panel = wrap.querySelector(':scope > .submenu-panel') as HTMLElement | null;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    await tick();
    const pad = 8;
    const pw = panel?.offsetWidth ?? 260;
    const ph = panel?.offsetHeight ?? 280;
    let x = rect.right;
    let y = rect.top;
    if (x + pw + pad > window.innerWidth) {
      x = rect.left - pw;
    }
    if (y + ph + pad > window.innerHeight) {
      y = Math.max(pad, window.innerHeight - ph - pad);
    }
    submenuLeft = Math.max(pad, x);
    submenuTop = Math.max(pad, y);
  }

  function menuOpenInNewTab(p: StoredProfile) {
    closeMenu();
    void openProfilesForAction(p, 'new-tab');
  }
  function menuSplitRight(p: StoredProfile) {
    closeMenu();
    void openProfilesForAction(p, 'split-right');
  }
  function menuSplitDown(p: StoredProfile) {
    closeMenu();
    void openProfilesForAction(p, 'split-down');
  }
  function menuOpenSftp(p: StoredProfile) {
    closeMenu();
    openSftp(p);
  }
  function menuEdit(p: StoredProfile) {
    closeMenu();
    void editProfile(p);
  }
  function groupDepth(path: string): number {
    return path.split('/').filter(Boolean).length;
  }

  function groupLabel(path: string): string {
    const parts = path.split('/').filter(Boolean);
    return parts[parts.length - 1] ?? path;
  }

  function menuMoveToGroup(p: StoredProfile) {
    void promptAndMoveProfiles(profilesToMove(p));
  }
  function menuMoveToExistingGroup(p: StoredProfile, group: string | null) {
    void moveProfilesToExistingGroup(profilesToMove(p), group);
  }
  async function menuEditTags(p: StoredProfile, ev: MouseEvent) {
    setDialogAnchor(ev);
    closeMenu();
    await promptEditTags(p);
  }
  async function menuEditNote(p: StoredProfile, ev: MouseEvent) {
    setDialogAnchor(ev);
    closeMenu();
    await promptEditNote(p);
  }
  async function menuViewNote(p: StoredProfile, ev: MouseEvent) {
    setDialogAnchor(ev);
    closeMenu();
    await appConfirm(p.note?.trim() || i18n.t('sidebar.noNote'), {
      title: p.name,
      confirmLabel: i18n.t('common.ok'),
      position: lastMenuPosition ?? undefined,
    });
  }
  async function menuEditIcon(p: StoredProfile, ev: MouseEvent) {
    setDialogAnchor(ev);
    closeMenu();
    await promptEditIcon(p);
  }
  async function menuChooseIconFile(p: StoredProfile) {
    closeMenu();
    const path = await pickIconFilePath();
    if (!path) return;
    try {
      const fresh = await latestProfile(p);
      await rpc.call('profile.upsert', { ...fresh, icon: { kind: 'file', value: path } });
      notifyProfilesChanged({ profileId: p.id, group: fresh.group });
      await refresh();
    } catch (e) {
      onError(i18n.t('profileModal.saveFailed', { message: (e as Error).message }));
    }
  }
  function menuMoveSelectedToFolder(groupPath: string) {
    const group = groupPath.trim() ? groupPath.trim() : null;
    void moveProfilesToGroup(selectedProfiles, group);
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
  function menuDelete(p: StoredProfile, ev: MouseEvent) {
    setDialogAnchor(ev);
    closeMenu();
    void deleteProfilesForAction(p);
  }
  function menuNewProfileInGroup(groupPath: string) {
    closeMenu();
    openProfileModal(undefined, { group: groupPath });
  }
  function menuCreateGroup(parentPath: string | null = null) {
    void createGroup(parentPath);
  }

  async function menuOpenGroupProfiles(folder: ProfileTreeFolder) {
    closeMenu();
    const list = collectProfilesInFolder(folder);
    if (list.length === 0) {
      onError(i18n.t('sidebar.groupEmpty'));
      return;
    }
    bulkBusy = true;
    try {
      await openProfilesEachInNewTab(list, bulkOpenDeps);
    } finally {
      bulkBusy = false;
    }
  }

  async function menuOpenGroupInSameTab(folder: ProfileTreeFolder) {
    closeMenu();
    const list = collectProfilesInFolder(folder).filter((p) => p.kind === 'ssh');
    if (list.length === 0) {
      onError(i18n.t('sidebar.groupNoSsh'));
      return;
    }
    const deps = {
      ...bulkOpenDeps,
      confirmMany:
        list.length > BULK_OPEN_CONFIRM_THRESHOLD
          ? async (count: number) =>
              bulkOpenConfirmMany(count, 'profiles.bulkOpenManyConfirmSameNewTab')
          : undefined,
    };
    bulkBusy = true;
    try {
      await openProfilesInSameTab(list, deps, { tabTarget: 'new' });
    } catch (e) {
      onError(`connect: ${(e as Error).message}`);
    } finally {
      bulkBusy = false;
    }
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
    if (!(await appConfirm(i18n.t('sidebar.deleteProfileConfirm', { name: p.name }), { danger: true, confirmLabel: i18n.t('common.delete'), position: lastMenuPosition ?? undefined }))) return;
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
    onRemove: (p: StoredProfile) => { void deleteProfilesForAction(p); },
    onOpenNewTab: (p: StoredProfile) => { void openProfilesForAction(p, 'new-tab'); },
    onSplitRight: (p: StoredProfile) => { void openProfilesForAction(p, 'split-right'); },
    onSplitDown: (p: StoredProfile) => { void openProfilesForAction(p, 'split-down'); },
    onOpenSftp: (p: StoredProfile) => { openSftp(p); },
  };

  function onProfileKeydown(p: StoredProfile, ev: KeyboardEvent) {
    focusProfile(p);
    if (handleProfileSidebarShortcut(p, ev, profileShortcutHandlers)) {
      ev.preventDefault();
      ev.stopPropagation();
    }
  }

  type ProfileDragPayload = { kind: 'profiles'; ids: string[] } | { kind: 'group'; path: string };
  const PROFILE_DND_MIME = 'application/x-aerotab-profile-drag';

  function setProfileDragData(ev: DragEvent, payload: ProfileDragPayload) {
    ev.dataTransfer?.setData(PROFILE_DND_MIME, JSON.stringify(payload));
    ev.dataTransfer?.setData('text/plain', payload.kind === 'group' ? payload.path : payload.ids.join(','));
    if (ev.dataTransfer) ev.dataTransfer.effectAllowed = 'move';
  }

  function readProfileDragData(ev: DragEvent): ProfileDragPayload | null {
    const raw = ev.dataTransfer?.getData(PROFILE_DND_MIME);
    if (!raw) return null;
    try {
      const parsed = JSON.parse(raw) as ProfileDragPayload;
      if (parsed.kind === 'profiles' && Array.isArray(parsed.ids)) return parsed;
      if (parsed.kind === 'group' && typeof parsed.path === 'string') return parsed;
    } catch {
      return null;
    }
    return null;
  }

  function profileDragList(p: StoredProfile): StoredProfile[] {
    return selectedProfileIds.has(p.id) && selectedProfileIds.size > 0 ? selectedProfiles : [p];
  }

  function onProfileDragStart(p: StoredProfile, ev: DragEvent) {
    const list = profileDragList(p);
    selectedProfileIds = new Set(list.map((x) => x.id));
    selectionAnchorId = p.id;
    focusProfile(p);
    setProfileDragData(ev, { kind: 'profiles', ids: list.map((x) => x.id) });
  }

  function onFolderDragStart(folder: ProfileTreeFolder, ev: DragEvent) {
    setProfileDragData(ev, { kind: 'group', path: folder.path });
  }

  function onFolderDragOver(_folder: ProfileTreeFolder, ev: DragEvent) {
    const dataTransfer = ev.dataTransfer;
    if (!Array.from(dataTransfer?.types ?? []).includes(PROFILE_DND_MIME)) return;
    ev.preventDefault();
    if (dataTransfer) dataTransfer.dropEffect = 'move';
  }

  async function moveGroupTo(sourcePath: string, targetParentPath: string | null) {
    const source = normalizeGroupPath(sourcePath);
    const targetParent = normalizeGroupPath(targetParentPath);
    if (!source) return;
    if (targetParent === source || (targetParent?.startsWith(`${source}/`) ?? false)) {
      onError(i18n.t('sidebar.moveGroupIntoSelf'));
      return;
    }
    const name = source.split('/').pop() ?? source;
    const target = targetParent ? `${targetParent}/${name}` : name;
    if (target === source) return;
    bulkBusy = true;
    try {
      const changedProfiles = profiles.filter((p) => {
        const group = normalizeGroupPath(p.group);
        return group === source || group?.startsWith(`${source}/`);
      });
      for (const profile of changedProfiles) {
        const group = normalizeGroupPath(profile.group);
        if (!group) continue;
        const nextGroup = `${target}${group.slice(source.length)}`;
        await rpc.call('profile.upsert', { ...profile, group: nextGroup });
      }
      const nextGroups = explicitGroupPaths.map((group) => {
        if (group === source || group.startsWith(`${source}/`)) return `${target}${group.slice(source.length)}`;
        return group;
      });
      await saveExplicitGroups([...nextGroups, target]);
      expandFoldersForGroup(target);
      notifyProfilesChanged({ group: target });
      await refresh();
    } catch (e) {
      onError(i18n.t('profiles.moveToGroupFailed', { message: (e as Error).message }));
    } finally {
      bulkBusy = false;
    }
  }

  async function handleDropToGroup(targetGroup: string | null, ev: DragEvent) {
    const payload = readProfileDragData(ev);
    if (!payload) return;
    ev.preventDefault();
    ev.stopPropagation();
    if (payload.kind === 'profiles') {
      const ids = new Set(payload.ids);
      const list = profiles.filter((p) => ids.has(p.id));
      await moveProfilesToGroup(list, targetGroup);
    } else {
      await moveGroupTo(payload.path, targetGroup);
    }
  }

  function onFolderDrop(folder: ProfileTreeFolder, ev: DragEvent) {
    void handleDropToGroup(folder.path, ev);
  }

  function onRootDragOver(ev: DragEvent) {
    const dataTransfer = ev.dataTransfer;
    if (!Array.from(dataTransfer?.types ?? []).includes(PROFILE_DND_MIME)) return;
    ev.preventDefault();
    if (dataTransfer) dataTransfer.dropEffect = 'move';
  }

  function onRootDrop(ev: DragEvent) {
    void handleDropToGroup(null, ev);
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

  <div class="px-2 pt-2 pb-1 border-b border-[var(--color-border-soft)] grid grid-cols-1 gap-1">
    <button
      type="button"
      onclick={() => onShowTransfer?.()}
      class="workspace-switch"
      title={i18n.t('workspace.fileTransfer')}
      aria-label={i18n.t('workspace.fileTransfer')}
    >
      <ArrowLeftRight size={14} />
      <span>{i18n.t('workspace.fileTransfer')}</span>
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
        title={i18n.t('sidebar.createGroup')}
        aria-label={i18n.t('sidebar.createGroup')}
        onclick={() => menuCreateGroup(null)}
      >
        <Plus size={13} />
      </button>
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
                onclick={() => { void bulkOpenSelected('new-same'); }}>
          {i18n.t('profiles.bulkConnectSameNewTab')}
        </button>
        <button type="button" class="btn-secondary text-[10px] py-0.5 px-1.5" disabled={bulkBusy}
                onclick={() => { void bulkOpenSelected('new-each'); }}>
          {i18n.t('profiles.bulkConnectEachNewTab')}
        </button>
        <button type="button" class="btn-secondary text-[10px] py-0.5 px-1.5" disabled={bulkBusy}
                onclick={() => { void bulkOpenSelected('active'); }}>
          {i18n.t('profiles.bulkConnect')}
        </button>
        <button type="button" class="btn-secondary text-[10px] py-0.5 px-1.5" disabled={bulkBusy || healthRunning}
                onclick={() => { void bulkHealthCheckSelected(); }}>
          {healthRunning ? i18n.t('profiles.healthChecking') : i18n.t('profiles.bulkHealthCheck')}
        </button>
        <button type="button" class="btn-secondary text-[10px] py-0.5 px-1.5" disabled={bulkBusy}
                onclick={() => { void bulkMoveSelected(); }}>
          {i18n.t('profiles.bulkMoveToGroup')}
        </button>
        <button type="button" class="btn-secondary text-[10px] py-0.5 px-1.5 text-[var(--color-danger)]" disabled={bulkBusy}
                onclick={() => { void bulkDeleteSelected(); }}>
          {i18n.t('profiles.bulkDelete')}
        </button>
      </div>
    {/if}
  </div>
  <div role="presentation" class="flex-1 overflow-y-auto px-2 pb-3 flex flex-col gap-0.5 min-h-0" ondragover={onRootDragOver} ondrop={onRootDrop}>
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
        profileHealth={profileHealth}
        showSelection={hasProfileSelection}
        onToggleFolder={toggleFolder}
        onOpenProfile={(p) => { void openProfilesForAction(p, 'new-tab'); }}
        onProfileClick={onProfileRowClick}
        onProfileCheckboxToggle={toggleProfileCheckbox}
        onProfileFocus={focusProfile}
        onProfileKeydown={onProfileKeydown}
        onProfileContextMenu={showProfileMenu}
        onProfileQuickAction={onProfileQuickAction}
        onProfileDragStart={onProfileDragStart}
        onFolderContextMenu={showFolderMenu}
        onFolderDragStart={onFolderDragStart}
        onFolderDragOver={onFolderDragOver}
        onFolderDrop={onFolderDrop}
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
    class="panel context-menu-scroll fixed z-[56] min-w-[220px] py-1 text-[12.5px] text-[var(--color-fg)]"
    style="left: {menuX}px; top: {menuY}px;"
    onkeydown={(e) => e.stopPropagation()}
    onclick={(e) => e.stopPropagation()}
  >
      {#if menuTarget.kind === 'group'}
        {@const folder = menuTarget.folder}
        {@const groupPath = folder.path}
        {@const groupCount = collectProfilesInFolder(folder).length}
        {#if groupCount > 0}
          <button type="button" class="menu-item" onclick={() => { void menuOpenGroupProfiles(folder); }}>
            {i18n.t('sidebar.openGroupProfiles', { count: groupCount })}
          </button>
          <button type="button" class="menu-item" onclick={() => { void menuOpenGroupInSameTab(folder); }}>
            {i18n.t('sidebar.openGroupInSameTab')}
          </button>
          <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
        {/if}
        {#if hasProfileSelection}
          <button type="button" class="menu-item" onclick={() => menuMoveSelectedToFolder(groupPath)}>
            {i18n.t('sidebar.moveSelectedToGroup', {
              count: selectedProfileIds.size,
              group: groupPath || i18n.t('sidebar.ungrouped'),
            })}
          </button>
          <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
        {/if}
        <div class="menu-section-label px-3 py-1 text-[10px] uppercase tracking-[0.08em] text-[var(--color-fg-muted)]">
          {i18n.t('profiles.groupColor')}
        </div>
        <VisualColorPicker
          menu
          value={profileVisualsStore.groupColors[normalizeGroupKey(groupPath)] ?? null}
          onPick={(color) => {
            void profileVisualsStore.setGroupColor(rpc, groupPath, color);
          }}
        />
        <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
        <button type="button" class="menu-item" onclick={() => menuCreateGroup(groupPath)}>{i18n.t('sidebar.createSubgroup')}</button>
        <button type="button" class="menu-item" onclick={() => menuNewProfileInGroup(groupPath)}>
          {i18n.t('sidebar.newProfileInGroup')}
        </button>
      {:else}
        {@const mp = menuTarget.profile}
        {@const moveCount = selectedProfileIds.has(mp.id) ? selectedProfileIds.size : 1}
        <div class="menu-with-submenu" onmouseenter={alignSubmenu}>
          <button type="button" class="menu-item menu-item--submenu" onclick={() => menuMoveToGroup(mp)}>
            <span>{moveCount > 1
              ? i18n.t('sidebar.moveProfilesToGroup', { count: moveCount })
              : i18n.t('sidebar.moveProfileToGroup')}</span>
            <span class="submenu-arrow">›</span>
          </button>
          <div class="submenu-panel" role="menu" style="left: {submenuLeft}px; top: {submenuTop}px;">
            <button type="button" class="menu-item" onclick={() => menuMoveToExistingGroup(mp, null)}>{i18n.t('sidebar.ungrouped')}</button>
            {#each allGroupPaths as group (group)}
              <button
                type="button"
                class="menu-item group-menu-item"
                style="padding-left: {10 + Math.max(0, groupDepth(group) - 1) * 12}px"
                title={group}
                onclick={() => menuMoveToExistingGroup(mp, group)}
              >
                <span class="truncate">{groupLabel(group)}</span>
              </button>
            {/each}
          </div>
        </div>
        <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
        <button type="button" class="menu-item" onclick={(ev) => { void menuEditTags(mp, ev); }}>{i18n.t('sidebar.editTags')}</button>
        {#if (mp.tags ?? []).length > 0}
          <div class="menu-with-submenu" onmouseenter={alignSubmenu}>
            <button type="button" class="menu-item menu-item--submenu">
              <span>{i18n.t('profiles.tagColors')}</span>
              <span class="submenu-arrow">›</span>
            </button>
            <div class="submenu-panel tag-color-submenu" role="menu" style="left: {submenuLeft}px; top: {submenuTop}px;">
              {#each (mp.tags ?? []) as tag (tag)}
                <div class="menu-tag-color-row">
                  <ProfileTag {tag} compact />
                  <VisualColorPicker
                    menu
                    value={profileVisualsStore.tagColors[normalizeTagKey(tag)] ?? null}
                    onPick={(color) => {
                      void profileVisualsStore.setTagColor(rpc, tag, color);
                    }}
                  />
                </div>
              {/each}
            </div>
          </div>
        {/if}
        <button type="button" class="menu-item" onclick={(ev) => { void menuEditNote(mp, ev); }}>{i18n.t('sidebar.editNote')}</button>
        <button type="button" class="menu-item" onclick={(ev) => { void menuViewNote(mp, ev); }}>{i18n.t('sidebar.viewNote')}</button>
        <button type="button" class="menu-item" onclick={(ev) => { void menuEditIcon(mp, ev); }}>{i18n.t('sidebar.editIcon')}</button>
        <button type="button" class="menu-item" onclick={() => { void menuChooseIconFile(mp); }}>{i18n.t('sidebar.chooseIconFile')}</button>
        <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => menuEdit(mp)}>
          <span>{i18n.t('sidebar.editProfile')}</span>
          <kbd class="kbd">{shortcutKbd('edit')}</kbd>
        </button>
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => { void cloneProfile(mp); }}>
          <span>{i18n.t('sidebar.cloneProfile')}</span>
          <kbd class="kbd">{shortcutKbd('clone')}</kbd>
        </button>
        <button type="button" class="menu-item menu-item--shortcut text-[var(--color-danger)]" onclick={(ev) => menuDelete(mp, ev)}>
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
          <div class="menu-with-submenu" onmouseenter={alignSubmenu}>
            <button type="button" class="menu-item menu-item--submenu">
              <span>{i18n.t('sidebar.connectViaJump')}</span>
              <span class="submenu-arrow">›</span>
            </button>
            <div class="submenu-panel jump-host-submenu" role="menu" style="left: {submenuLeft}px; top: {submenuTop}px;" onclick={(e) => e.stopPropagation()}>
              <div class="jump-host-search px-2 py-1.5">
                <input
                  type="search"
                  class="input text-[11px] py-1"
                  placeholder={i18n.t('sidebar.jumpHostSearch')}
                  bind:value={jumpSubmenuQuery}
                  onclick={(e) => e.stopPropagation()}
                />
              </div>
              {#each filteredJumpHosts(mp) as jump (jump.id)}
                <button
                  type="button"
                  class="menu-item jump-menu-item"
                  title={profileEndpointLabel(jump)}
                  onclick={(ev) => menuConnectViaJump(mp, jump, ev)}
                >
                  <span class="truncate">{jump.name}</span>
                  <span class="jump-menu-sub truncate">{profileEndpointLabel(jump)}</span>
                </button>
              {:else}
                <div class="px-3 py-2 text-[11px] text-[var(--color-fg-muted)]">{i18n.t('sidebar.connectViaJumpEmpty')}</div>
              {/each}
            </div>
          </div>
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

<style>
  .workspace-switch {
    min-width: 0;
    height: 30px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border: 1px solid transparent;
    border-radius: 6px;
    color: var(--color-fg-muted);
    background: transparent;
    font-size: 12px;
    cursor: pointer;
  }
  .workspace-switch span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .workspace-switch:hover {
    color: var(--color-fg);
    background: var(--color-panel-2);
  }
  .context-menu-scroll {
    max-height: calc(100vh - 16px);
    overflow-y: auto;
    overflow-x: visible;
  }
  .menu-with-submenu {
    position: relative;
  }
  .menu-item--submenu {
    display: flex;
    justify-content: space-between;
    gap: 12px;
  }
  .submenu-arrow {
    color: var(--color-fg-muted);
  }
  .submenu-panel {
    display: none;
    position: fixed;
    z-index: 57;
    min-width: 220px;
    max-width: min(320px, calc(100vw - 24px));
    max-height: min(320px, calc(100vh - 16px));
    overflow: auto;
    padding: 4px 0;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-panel);
    box-shadow: var(--shadow-lg);
  }
  .menu-with-submenu:hover .submenu-panel,
  .menu-with-submenu:focus-within .submenu-panel {
    display: block;
  }
  .group-menu-item {
    padding-left: calc(10px + max(0, var(--group-depth, 1) - 1) * 12px);
  }
  .jump-menu-item {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    min-width: 200px;
    max-width: 280px;
  }
  .jump-menu-sub {
    font-size: 10px;
    color: var(--color-fg-muted);
    max-width: 100%;
  }
  .tag-color-submenu {
    min-width: 240px;
    max-width: min(300px, calc(100vw - 24px));
  }
  .menu-tag-color-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    min-width: 0;
  }
  .jump-host-submenu {
    min-width: 260px;
  }
  .jump-host-search {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--color-panel);
    border-bottom: 1px solid var(--color-border-soft);
  }
</style>
