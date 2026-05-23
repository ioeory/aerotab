<script lang="ts">
  // Profiles & connections — manage saved SSH/serial profiles + browse
  // ~/.ssh/config entries imported from the host. Mirrors the sidebar's
  // profile list but with grouping, search, and inline edit/delete.

  import { onMount } from 'svelte';
  import { Activity, Plus, Trash2, Pencil, Plug, Search, ShieldAlert, ShieldCheck, ShieldX, Star } from '@lucide/svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import type { ProfileHealthResult, ProfileHealthStatus, StoredProfile } from '../../../lib/types';
  import { i18n } from '../../../lib/i18n.svelte';
  import { tabs } from '../../../lib/tabs.svelte';
  import { matchesProfileQuery, profileEndpointLabel, profileGroupName, sortProfiles, summarizeProfiles } from '../../../lib/profileMeta';
  import ProfileModal from '../../ProfileModal.svelte';
  import ProfileIcon from '../../ProfileIcon.svelte';

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
  let healthRunning = $state(false);
  let health = $state<Record<string, ProfileHealthResult>>({});
  let profileModal: { open: (existing?: StoredProfile) => void } | null = $state(null);

  async function load() {
    loading = true;
    try {
      saved = await rpc.call<StoredProfile[]>('profile.list');
    } catch (e) {
      saved = [];
      onError(`profiles: ${(e as Error).message}`);
    }
    try {
      const r = await rpc.call<{ sshConfig: SshConfigEntry[] }>('profile.discover');
      sshConfig = Array.isArray(r.sshConfig) ? r.sshConfig : [];
    } catch { sshConfig = []; }
    loading = false;
  }

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
    if (!confirm(`Delete profile "${p.name}"?`)) return;
    try {
      await rpc.call('profile.delete', { id: p.id });
      await load();
    } catch (e) { onError(`delete: ${(e as Error).message}`); }
  }

  async function editProfile(p: StoredProfile) {
    try {
      const latest = await rpc.call<StoredProfile>('profile.get', { id: p.id });
      profileModal?.open(latest);
    } catch (e) {
      onError(`profile refresh: ${(e as Error).message}`);
      profileModal?.open(p);
    }
  }

  async function runHealthCheck() {
    if (saved.length === 0 || healthRunning) return;
    healthRunning = true;
    try {
      const results = await rpc.call<ProfileHealthResult[]>('profile.healthCheck', {
        ids: saved.map((profile) => profile.id),
        connect: true,
      });
      health = Object.fromEntries(results.map((result) => [result.id, result]));
    } catch (e) {
      onError(`profile health: ${(e as Error).message}`);
    } finally {
      healthRunning = false;
    }
  }

  function healthLabel(status: ProfileHealthStatus): string {
    if (status === 'ok') return i18n.t('profiles.healthOk');
    if (status === 'warning') return i18n.t('profiles.healthWarning');
    return i18n.t('profiles.healthError');
  }

  function healthTitle(result: ProfileHealthResult): string {
    const issues = result.checks.filter((check) => check.status !== 'ok');
    const visible = issues.length > 0 ? issues : result.checks.slice(0, 1);
    return visible.map((check) => `${check.name}: ${check.message}`).join('\n') || i18n.t('profiles.healthNoIssues');
  }

  function healthIssues(result: ProfileHealthResult) {
    return result.checks.filter((check) => check.status !== 'ok').slice(0, 3);
  }

  onMount(() => { void load(); });
</script>

<div class="settings-section">
  <div class="flex items-center justify-between gap-2">
    <h2 class="!mb-0">{i18n.t('profiles.title')}</h2>
    <div class="flex items-center gap-2">
      <button type="button" class="btn-secondary flex items-center gap-1.5"
              onclick={runHealthCheck} disabled={healthRunning || saved.length === 0}>
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
    />
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
    <div>
      <div class="section-h">{i18n.t('profiles.savedProfiles', { count: saved.length })}</div>
      {#if saved.length === 0}
        <div class="placeholder">{i18n.t('profiles.empty')}</div>
      {:else}
        {#each grouped() as [groupName, items] (groupName)}
          <div class="group-block">
            <div class="group-name">{groupName}</div>
            {#each items as p (p.id)}
              {@const h = health[p.id]}
              <div class="profile-row">
                <ProfileIcon icon={p.icon} name={p.name} size={14} />
                <div class="min-w-0 flex-1">
                  <div class="flex items-center gap-1 text-[12.5px] font-medium truncate">
                    <span class="truncate">{p.name}</span>
                    {#if h}
                      <span class={`health-chip ${h.status}`} title={healthTitle(h)} aria-label={healthLabel(h.status)}>
                        {#if h.status === 'ok'}
                          <ShieldCheck size={11} />
                        {:else if h.status === 'warning'}
                          <ShieldAlert size={11} />
                        {:else}
                          <ShieldX size={11} />
                        {/if}
                      </span>
                    {/if}
                    {#if p.favorite}
                      <Star size={11} class="shrink-0 text-[var(--color-accent)]" fill="currentColor" />
                    {/if}
                  </div>
                  <div class="text-[11px] text-[var(--color-fg-muted)] truncate font-mono">
                    {profileEndpointLabel(p)}
                  </div>
                  {#if (p.tags ?? []).length > 0}
                    <div class="tag-row">
                      {#each (p.tags ?? []).slice(0, 6) as tag (tag)}
                        <span>{tag}</span>
                      {/each}
                    </div>
                  {/if}
                  {#if h && h.status !== 'ok'}
                    <div class="health-details">
                      {#each healthIssues(h) as check (`${p.id}-${check.name}`)}
                        <span>{check.name}: {check.message}</span>
                      {/each}
                    </div>
                  {/if}
                </div>
                <div class="flex items-center gap-1">
                    <button type="button" class="icon-btn" title={i18n.t('profiles.connect')}
                          onclick={() => connect(p)}><Plug size={12} /></button>
                    <button type="button" class="icon-btn" title={i18n.t('common.edit')}
                      onclick={() => editProfile(p)}><Pencil size={12} /></button>
                    <button type="button" class="icon-btn" title={i18n.t('common.delete')}
                          onclick={() => remove(p)}><Trash2 size={12} /></button>
                </div>
              </div>
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
  .summary-strip span,
  .tag-row span {
    border: 1px solid var(--color-border-soft);
    border-radius: 999px;
    padding: 1px 7px;
    background: var(--color-panel-2);
  }
  .tag-row {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
    font-size: 10px;
    color: var(--color-fg-muted);
  }
  .health-chip {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border-radius: 999px;
    border: 1px solid var(--color-border-soft);
    color: var(--color-fg-muted);
    background: var(--color-panel);
    flex: 0 0 auto;
  }
  .health-chip.ok { color: var(--color-success); }
  .health-chip.warning { color: var(--color-warning); }
  .health-chip.error { color: var(--color-danger); }
  .health-details {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: 4px;
    color: var(--color-fg-muted);
    font-size: 10.5px;
    line-height: 1.35;
  }
  .group-block { margin-bottom: 8px; }
  .group-name {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--color-fg-muted);
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
