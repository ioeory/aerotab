<script lang="ts">
  // Profiles & connections — manage saved SSH/serial profiles + browse
  // ~/.ssh/config entries imported from the host. Mirrors the sidebar's
  // profile list but with grouping, search, and inline edit/delete.

  import { onMount, onDestroy, tick } from 'svelte';
  import { Activity, Plus, Plug, Search } from '@lucide/svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import type { ProfileHealthResult, StoredProfile } from '../../../lib/types';
  import { i18n } from '../../../lib/i18n.svelte';
  import { appConfirm, appPrompt } from '../../../lib/confirm.svelte';
  import {
    BULK_OPEN_CONFIRM_THRESHOLD,
    openProfilesEachInNewTab,
    openProfilesInSameTab,
  } from '../../../lib/profileBulkOpen';
  import {
    defaultGroupForMove,
    normalizeProfileGroupInput,
    upsertProfilesGroup,
  } from '../../../lib/profileGroupMove';
  import { tabs } from '../../../lib/tabs.svelte';
  import { matchesProfileQuery, profileGroupName, sortProfiles, summarizeProfiles, displayGroupName, isUngroupedGroupKey } from '../../../lib/profileMeta';
  import {
    invertProfileSelection,
    profilesFromSelection,
    rangeSelectProfiles,
    selectAllProfiles,
    toggleProfileInSelection,
  } from '../../../lib/profileSelection';
  import { notifyProfilesChanged, PROFILES_CHANGED } from '../../../lib/profileEvents';
  import { requestImportConnections } from '../../../lib/importConnections';
  import { healthIssueDetailText, summarizeHealthResults } from '../../../lib/profileHealthUi';
  import { withRpcTimeout } from '../../../lib/rpcTimeout';
  import { focusProfileInTabs } from '../../../lib/focusProfileSession';
  import ProfileModal from '../../ProfileModal.svelte';
  import ProfileListRow from '../../ProfileListRow.svelte';
  import ProfileTag from '../../ProfileTag.svelte';
  import { groupStyle, normalizeGroupKey, normalizeTagKey } from '../../../lib/profileVisuals';
  import { profileVisualsStore } from '../../../lib/profileVisualsStore.svelte';
  import VisualColorPicker from '../../VisualColorPicker.svelte';

  interface Props {
    rpc: RpcClient;
    onError: (msg: string) => void;
  }
  let { rpc, onError }: Props = $props();

  interface SshConfigEntry {
    alias: string;
    host: string;
    port: number;
    user?: string | null;
    identity_file?: string | null;
  }

  let saved = $state<StoredProfile[]>([]);
  let sshConfig = $state<SshConfigEntry[]>([]);
  let query = $state('');
  let loading = $state(true);
  let loadError = $state('');
  let loadGen = 0;
  let profilesChangedTimer: ReturnType<typeof setTimeout> | null = null;
  let healthRunning = $state(false);
  let visualStatus = $state('');
  let visualImportInput: HTMLInputElement | null = null;
  let health = $state<Record<string, ProfileHealthResult>>({});
  let selectedProfileIds = $state<Set<string>>(new Set());
  let selectionAnchorId = $state<string | null>(null);
  let bulkBusy = $state(false);

  const visibleProfiles = $derived(
    sortProfiles(saved).filter((profile) => matchesProfileQuery(profile, query)),
  );
  const selectedProfiles = $derived(profilesFromSelection(visibleProfiles, selectedProfileIds));
  const hasProfileSelection = $derived(selectedProfileIds.size > 0);
  let profileModal: {
    open: (existing?: StoredProfile, options?: { group?: string }) => void;
  } | null = $state(null);

  async function load(refresh = false) {
    const gen = ++loadGen;
    const showLoading = !refresh && saved.length === 0;
    if (showLoading) loading = true;
    loadError = '';
    const failsafe = setTimeout(() => {
      if (gen === loadGen && loading) {
        loading = false;
        loadError = i18n.t('profiles.loadTimeout');
      }
    }, 25_000);
    try {
      const listResult = await withRpcTimeout(
        rpc.call<StoredProfile[]>('profile.list'),
        12_000,
        'profile.list',
      ).catch((e) => {
        const msg = (e as Error).message;
        if (gen === loadGen) {
          loadError = msg;
          onError(`profiles: ${msg}`);
        }
        return [] as StoredProfile[];
      });
      if (gen !== loadGen) return;
      saved = listResult;

      void withRpcTimeout(
        rpc.call<{ sshConfig: SshConfigEntry[] }>('profile.discover'),
        6_000,
        'profile.discover',
      )
        .then((r) => {
          if (gen === loadGen) {
            sshConfig = Array.isArray(r.sshConfig) ? r.sshConfig : [];
          }
        })
        .catch(() => {
          if (gen === loadGen) sshConfig = [];
        });
    } catch (e) {
      if (gen === loadGen) {
        loadError = (e as Error).message;
        saved = [];
      }
    } finally {
      clearTimeout(failsafe);
      if (gen === loadGen) loading = false;
    }
  }

  const onProfilesChanged = () => {
    if (profilesChangedTimer) clearTimeout(profilesChangedTimer);
    profilesChangedTimer = setTimeout(() => {
      profilesChangedTimer = null;
      void load(true);
    }, 80);
  };

  const grouped = $derived(() => {
    const groups = new Map<string, StoredProfile[]>();
    for (const p of sortProfiles(saved).filter((profile) => matchesProfileQuery(profile, query))) {
      const g = profileGroupName(p);
      const arr = groups.get(g) ?? [];
      arr.push(p);
      groups.set(g, arr);
    }
    return [...groups.entries()].sort(([a], [b]) => a.localeCompare(b));
  });

  const summary = $derived(() => summarizeProfiles(saved));

  const healthSummary = $derived(() => {
    const values = Object.values(health);
    return {
      checked: values.length,
      ok: values.filter((item) => item.status === 'ok').length,
      warning: values.filter((item) => item.status === 'warning').length,
      error: values.filter((item) => item.status === 'error').length,
    };
  });

  const filteredSshConfig = $derived(
    query.trim()
      ? sshConfig.filter((e) =>
          e.alias.toLowerCase().includes(query.toLowerCase())
          || e.host.toLowerCase().includes(query.toLowerCase()))
      : sshConfig,
  );

  async function connect(p: StoredProfile) {
    try {
      if (p.kind === 'rdp' || p.kind === 'vnc') {
        await rpc.call('remote.openProfile', { profile_id: p.id });
        return;
      }
      const meta = await rpc.call<{ id: string; kind: string; title: string }>(
        'session.openSshProfile', { profile_id: p.id },
      );
      tabs.add({ id: meta.id, kind: meta.kind, title: meta.title, profileId: p.id, sshProfile: p.ssh });
    } catch (e) { onError(`connect: ${(e as Error).message}`); }
  }

  async function connectSshConfig(e: SshConfigEntry) {
    const profile = {
      host: e.host,
      port: e.port,
      user: e.user ?? 'root',
      auth: e.identity_file
        ? { PublicKey: { key_path: e.identity_file } }
        : 'Agent',
      jump_via: [],
    };
    try {
      const meta = await rpc.call<{ id: string; kind: string; title: string }>(
        'session.openSsh', { title: e.alias, profile },
      );
      tabs.add({ id: meta.id, kind: meta.kind, title: meta.title });
    } catch (err) { onError(`ssh-config: ${(err as Error).message}`); }
  }

  async function remove(p: StoredProfile) {
    if (!(await appConfirm(i18n.t('sidebar.deleteProfileConfirm', { name: p.name }), { danger: true, confirmLabel: i18n.t('common.delete') }))) return;
    try {
      await rpc.call('profile.delete', { id: p.id });
      notifyProfilesChanged();
      await load();
    } catch (e) { onError(`delete: ${(e as Error).message}`); }
  }

  async function editProfile(p: StoredProfile) {
    focusProfileInTabs(p.id);
    try {
      const latest = await rpc.call<StoredProfile>('profile.get', { id: p.id });
      profileModal?.open(latest);
    } catch (e) {
      onError(`profile refresh: ${(e as Error).message}`);
      profileModal?.open(p);
    }
  }

  function clearProfileSelection() {
    selectedProfileIds = new Set();
    selectionAnchorId = null;
  }

  function onProfileRowClick(p: StoredProfile, ev: MouseEvent) {
    if (ev.shiftKey && selectionAnchorId) {
      selectedProfileIds = rangeSelectProfiles(visibleProfiles, selectionAnchorId, p.id);
    } else if (ev.ctrlKey || ev.metaKey) {
      selectedProfileIds = toggleProfileInSelection(selectedProfileIds, p.id);
      selectionAnchorId = p.id;
    } else {
      selectedProfileIds = new Set([p.id]);
      selectionAnchorId = p.id;
    }
  }

  function toggleProfileCheckbox(p: StoredProfile) {
    selectedProfileIds = toggleProfileInSelection(selectedProfileIds, p.id);
    selectionAnchorId = p.id;
  }

  async function runHealthCheck(ids?: string[]) {
    const targetIds = ids ?? saved.map((profile) => profile.id);
    if (targetIds.length === 0 || healthRunning) return;
    healthRunning = true;
    try {
      const results = await rpc.call<ProfileHealthResult[]>('profile.healthCheck', {
        ids: targetIds,
        connect: true,
      });
      health = { ...health, ...Object.fromEntries(results.map((result) => [result.id, result])) };
    } catch (e) {
      onError(`profile health: ${(e as Error).message}`);
    } finally {
      healthRunning = false;
    }
  }

  const bulkOpenDeps = {
    rpc,
    onError,
    onSummary: async (message: string) => {
      await appConfirm(message, { confirmLabel: i18n.t('common.ok') });
    },
  };

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
    if (ids.length === 0 || healthRunning || bulkBusy) return;
    healthRunning = true;
    try {
      const results = await rpc.call<ProfileHealthResult[]>('profile.healthCheck', {
        ids,
        connect: true,
      });
      health = { ...health, ...Object.fromEntries(results.map((r) => [r.id, r])) };
      const summary = summarizeHealthResults(results);
      let message = i18n.t('profiles.healthSummary', summary);
      const details = healthIssueDetailText(results);
      if (details) message += `\n\n${details}`;
      await appConfirm(message, { confirmLabel: i18n.t('common.ok') });
    } catch (e) {
      onError(`profile health: ${(e as Error).message}`);
    } finally {
      healthRunning = false;
    }
  }

  async function bulkMoveSelected() {
    const list = selectedProfiles;
    if (list.length === 0 || bulkBusy) return;
    const value = await appPrompt(
      i18n.t('profiles.moveToGroupPrompt', { count: list.length }),
      {
        defaultValue: defaultGroupForMove(list),
        placeholder: i18n.t('profileModal.groupPlaceholder'),
        confirmLabel: i18n.t('profiles.moveToGroup'),
      },
    );
    if (value === null) return;
    bulkBusy = true;
    try {
      const group = normalizeProfileGroupInput(value);
      const moved = await upsertProfilesGroup(rpc, list, group);
      if (moved > 0) notifyProfilesChanged({ group });
      await load();
    } catch (e) {
      onError(i18n.t('profiles.moveToGroupFailed', { message: (e as Error).message }));
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
      await load();
    } catch (e) {
      onError(`delete: ${(e as Error).message}`);
    } finally {
      bulkBusy = false;
    }
  }

  function exportVisualSettings() {
    const json = JSON.stringify(profileVisualsStore.exportPayload(), null, 2);
    const blob = new Blob([json], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = 'aerotab-profile-visuals.json';
    document.body.appendChild(link);
    link.click();
    link.remove();
    URL.revokeObjectURL(url);
    visualStatus = i18n.t('profiles.exportVisualsDone');
  }

  function triggerVisualImport() {
    visualImportInput?.click();
  }

  async function importVisualSettings(ev: Event) {
    const input = ev.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    try {
      const text = await file.text();
      const parsed = JSON.parse(text) as unknown;
      await profileVisualsStore.importPayload(rpc, parsed, true);
      visualStatus = i18n.t('profiles.importVisualsDone', { name: file.name });
    } catch (e) {
      onError(i18n.t('profiles.importVisualsFailed', { message: (e as Error).message }));
    }
  }

  function healthIssues(result: ProfileHealthResult) {
    return result.checks.filter((check) => check.status !== 'ok').slice(0, 3);
  }

  onMount(() => {
    document.addEventListener(PROFILES_CHANGED, onProfilesChanged);
    void load();
  });
  onDestroy(() => {
    if (profilesChangedTimer) clearTimeout(profilesChangedTimer);
    document.removeEventListener(PROFILES_CHANGED, onProfilesChanged);
  });
</script>

<div class="settings-section">
  <div class="flex items-center justify-between gap-2">
    <h2 class="!mb-0">{i18n.t('profiles.title')}</h2>
    <div class="flex items-center gap-2">
      <button type="button" class="btn-secondary flex items-center gap-1.5"
              onclick={() => { requestImportConnections(); }}>
        <Plug size={12} /> {i18n.t('profiles.importConnections')}
      </button>
      <button type="button" class="btn-secondary flex items-center gap-1.5"
              onclick={() => { void runHealthCheck(); }} disabled={healthRunning || saved.length === 0}>
        <Activity size={12} /> {healthRunning ? i18n.t('profiles.healthChecking') : i18n.t('profiles.healthCheck')}
      </button>
      <button type="button" class="btn-primary flex items-center gap-1.5"
              onclick={() => profileModal?.open()}>
        <Plus size={12} /> {i18n.t('profiles.newSshProfile')}
      </button>
    </div>
  </div>

  <div class="relative">
    <Search size={12} class="absolute left-2 top-1/2 -translate-y-1/2 opacity-60" />
    <input
      type="search" bind:value={query} placeholder={i18n.t('profiles.filterPlaceholder')}
      class="input pl-7"
      aria-label={i18n.t('profiles.filterPlaceholder')}
    />
  </div>

  {#if hasProfileSelection}
    <div class="flex flex-wrap items-center gap-2 py-1">
      <span class="text-[11px] text-[var(--color-fg-muted)]">{i18n.t('profiles.selectedCount', { count: selectedProfileIds.size })}</span>
      <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" disabled={bulkBusy || healthRunning}
              onclick={() => { selectedProfileIds = selectAllProfiles(visibleProfiles); }}>
        {i18n.t('profiles.selectAll')}
      </button>
      <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" disabled={bulkBusy || healthRunning}
              onclick={() => { selectedProfileIds = invertProfileSelection(selectedProfileIds, visibleProfiles); }}>
        {i18n.t('profiles.invertSelection')}
      </button>
      <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" disabled={bulkBusy}
              onclick={clearProfileSelection}>
        {i18n.t('profiles.clearSelection')}
      </button>
      <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" disabled={bulkBusy}
              onclick={() => { void bulkOpenSelected('new-same'); }}>
        {i18n.t('profiles.bulkConnectSameNewTab')}
      </button>
      <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" disabled={bulkBusy}
              onclick={() => { void bulkOpenSelected('new-each'); }}>
        {i18n.t('profiles.bulkConnectEachNewTab')}
      </button>
      <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" disabled={bulkBusy}
              onclick={() => { void bulkOpenSelected('active'); }}>
        {i18n.t('profiles.bulkConnect')}
      </button>
      <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" disabled={bulkBusy || healthRunning}
              onclick={() => { void bulkHealthCheckSelected(); }}>
        {healthRunning ? i18n.t('profiles.healthChecking') : i18n.t('profiles.bulkHealthCheck')}
      </button>
      <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" disabled={bulkBusy}
              onclick={() => { void bulkMoveSelected(); }}>
        {i18n.t('profiles.bulkMoveToGroup')}
      </button>
      <button type="button" class="btn-secondary text-[11px] py-0.5 px-2 text-[var(--color-danger)]" disabled={bulkBusy}
              onclick={() => { void bulkDeleteSelected(); }}>
        {i18n.t('profiles.bulkDelete')}
      </button>
    </div>
  {/if}

  <div class="visual-settings panel mb-3">
    <div class="section-h">{i18n.t('profiles.visualTitle')}</div>
    <p class="hint mb-2">{i18n.t('profiles.visualHelp')}</p>
    <label class="row">
      <span class="row-label">{i18n.t('profiles.showSshKindBadge')}</span>
      <input
        type="checkbox"
        checked={profileVisualsStore.showSshKindBadge}
        onchange={(e) => {
          void profileVisualsStore.setShowSshKindBadge(rpc, (e.currentTarget as HTMLInputElement).checked);
        }}
      />
    </label>
    {#if Object.keys(profileVisualsStore.groupColors).length > 0}
      <div class="visual-custom-list">
        <span class="visual-custom-label">{i18n.t('profiles.customGroupColors', { count: Object.keys(profileVisualsStore.groupColors).length })}</span>
        <button type="button" class="visual-color-reset" onclick={() => { void profileVisualsStore.resetGroupColors(rpc); }}>
          {i18n.t('profiles.resetGroupColors')}
        </button>
      </div>
      <div class="visual-color-items">
        {#each Object.entries(profileVisualsStore.groupColors) as [key, color] (key)}
          <div class="visual-color-item">
            <span class="visual-color-item-label" title={key}>{key}</span>
            <VisualColorPicker
              compact
              value={color}
              onPick={(c) => {
                void profileVisualsStore.setGroupColor(rpc, key, c);
              }}
            />
          </div>
        {/each}
      </div>
    {/if}
    {#if Object.keys(profileVisualsStore.tagColors).length > 0}
      <div class="visual-custom-list">
        <span class="visual-custom-label">{i18n.t('profiles.customTagColors', { count: Object.keys(profileVisualsStore.tagColors).length })}</span>
        <button type="button" class="visual-color-reset" onclick={() => { void profileVisualsStore.resetTagColors(rpc); }}>
          {i18n.t('profiles.resetTagColors')}
        </button>
      </div>
      <div class="visual-color-items">
        {#each Object.entries(profileVisualsStore.tagColors) as [key, color] (key)}
          <div class="visual-color-item">
            <ProfileTag tag={key} compact />
            <VisualColorPicker
              compact
              value={color}
              onPick={(c) => {
                void profileVisualsStore.setTagColor(rpc, key, c);
              }}
            />
          </div>
        {/each}
      </div>
    {/if}
    <div class="visual-actions">
      <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" onclick={exportVisualSettings}>
        {i18n.t('profiles.exportVisuals')}
      </button>
      <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" onclick={triggerVisualImport}>
        {i18n.t('profiles.importVisuals')}
      </button>
      <input bind:this={visualImportInput} type="file" accept="application/json,.json" class="hidden" onchange={(e) => { void importVisualSettings(e); }} />
    </div>
    {#if visualStatus}
      <p class="hint mt-1">{visualStatus}</p>
    {/if}
    <p class="hint mt-2">{i18n.t('profiles.visualContextHint')}</p>
    <p class="hint">{i18n.t('profiles.visualSyncHint')}</p>
  </div>

  <div class="summary-strip">
    <span>{i18n.t('profiles.groups', { count: summary().groups })}</span>
    <span>{i18n.t('profiles.tags', { count: summary().tags })}</span>
    <span>{i18n.t('profiles.favorites', { count: summary().favorites })}</span>
    {#if healthSummary().checked > 0}
      <span>{i18n.t('profiles.healthSummary', healthSummary())}</span>
    {/if}
  </div>

  {#if loading}
    <div class="placeholder">{i18n.t('common.loading')}</div>
  {:else}
    {#if loadError}
      <div class="placeholder text-[var(--color-danger)]">
        {loadError}
        <button type="button" class="btn-secondary ml-2 text-[11px] py-0.5 px-2"
                onclick={() => { void load(); }}>
          {i18n.t('profiles.retryLoad')}
        </button>
      </div>
    {/if}
    <div>
      <div class="section-h">{i18n.t('profiles.savedProfiles', { count: saved.length })}</div>
      {#if saved.length === 0 && !loadError}
        <div class="placeholder">{i18n.t('profiles.empty')}</div>
      {:else}
        {#each grouped() as [groupName, items] (groupName)}
          <div class="group-block">
            <div class="group-name profile-group-header" style={groupStyle(isUngroupedGroupKey(groupName) ? '' : groupName, profileVisualsStore.overrides)}>
              <span class="profile-group-swatch" aria-hidden="true"></span>
              <span>{displayGroupName(groupName, (key) => i18n.t(key))}</span>
            </div>
            {#each items as p (p.id)}
              {@const h = health[p.id]}
              <ProfileListRow
                profile={p}
                variant="settings"
                health={h}
                healthIssues={h ? healthIssues(h) : []}
                selected={selectedProfileIds.has(p.id)}
                showSelection={hasProfileSelection}
                onOpen={() => connect(p)}
                onClick={(ev) => onProfileRowClick(p, ev)}
                onCheckboxToggle={() => toggleProfileCheckbox(p)}
                onConnect={() => connect(p)}
                onEdit={() => editProfile(p)}
                onRemove={() => remove(p)}
              />
            {/each}
          </div>
        {/each}
      {/if}
    </div>

    {#if sshConfig.length > 0}
      <div>
        <div class="section-h">~/.ssh/config ({filteredSshConfig.length})</div>
        {#each filteredSshConfig as e (e.alias)}
          <div class="profile-row">
            <div class="min-w-0 flex-1">
              <div class="text-[12.5px] font-medium truncate">{e.alias}</div>
              <div class="text-[11px] text-[var(--color-fg-muted)] truncate font-mono">
                {e.user ? `${e.user}@` : ''}{e.host}{e.port && e.port !== 22 ? `:${e.port}` : ''}
              </div>
            </div>
            <button type="button" class="icon-btn" title={i18n.t('profiles.connect')}
                    onclick={() => connectSshConfig(e)}><Plug size={12} /></button>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<ProfileModal {rpc} bind:this={profileModal} onSaved={() => load()} {onError} />

<style>
  .profile-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-radius: 4px;
    border: 1px solid var(--color-border-soft);
    background: var(--color-panel-2);
    margin-bottom: 4px;
  }
  .summary-strip {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    color: var(--color-fg-muted);
    font-size: 11px;
  }
  .summary-strip span {
    border: 1px solid var(--color-border-soft);
    border-radius: 999px;
    padding: 1px 7px;
    background: var(--color-panel-2);
  }
  .group-block { margin-bottom: 8px; }
  .visual-settings {
    padding: 10px 12px;
  }
  .visual-custom-list {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-top: 6px;
    font-size: 11px;
    color: var(--color-fg-muted);
  }
  .visual-custom-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .visual-color-reset {
    flex-shrink: 0;
    font-size: 10.5px;
    color: var(--color-fg-muted);
    background: transparent;
    border: none;
    padding: 0;
    cursor: pointer;
  }
  .visual-color-reset:hover {
    color: var(--color-accent);
    text-decoration: underline;
  }
  .visual-color-items {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 4px;
    margin-bottom: 6px;
  }
  .visual-color-item {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .visual-color-item-label {
    min-width: 72px;
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 10.5px;
    color: var(--color-fg-muted);
  }
  .visual-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 8px;
  }
  .group-name {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--profile-tone-fg, var(--color-fg-muted));
    padding: 4px 0;
  }
  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px; height: 22px;
    border-radius: 4px;
    color: var(--color-fg-muted);
    background: transparent;
    border: 1px solid transparent;
  }
  .icon-btn:hover {
    background: var(--color-panel);
    color: var(--color-fg);
    border-color: var(--color-border-soft);
  }
</style>
