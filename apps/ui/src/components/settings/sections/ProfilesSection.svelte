<script lang="ts">
  // Profiles & connections — manage saved SSH/serial profiles + browse
  // ~/.ssh/config entries imported from the host. Mirrors the sidebar's
  // profile list but with grouping, search, and inline edit/delete.

  import { onMount } from 'svelte';
  import { Plus, Trash2, Pencil, Plug, Search, Star } from '@lucide/svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import type { StoredProfile } from '../../../lib/types';
  import { i18n } from '../../../lib/i18n.svelte';
  import { tabs } from '../../../lib/tabs.svelte';
  import { matchesProfileQuery, profileGroupName, sortProfiles, summarizeProfiles } from '../../../lib/profileMeta';
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

  const filteredSshConfig = $derived(
    query.trim()
      ? sshConfig.filter((e) =>
          e.alias.toLowerCase().includes(query.toLowerCase())
          || e.host.toLowerCase().includes(query.toLowerCase()))
      : sshConfig,
  );

  async function connect(p: StoredProfile) {
    try {
      const meta = await rpc.call<{ id: string; kind: string; title: string }>(
        'session.openSshProfile', { profile_id: p.id },
      );
      tabs.add({ id: meta.id, kind: meta.kind, title: meta.title });
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

  onMount(() => { void load(); });
</script>

<div class="settings-section">
  <div class="flex items-center justify-between gap-2">
    <h2 class="!mb-0">{i18n.t('profiles.title')}</h2>
    <button type="button" class="btn-primary flex items-center gap-1.5"
            onclick={() => profileModal?.open()}>
      <Plus size={12} /> {i18n.t('profiles.newSshProfile')}
    </button>
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
              <div class="profile-row">
                <ProfileIcon icon={p.icon} name={p.name} size={14} />
                <div class="min-w-0 flex-1">
                  <div class="flex items-center gap-1 text-[12.5px] font-medium truncate">
                    <span class="truncate">{p.name}</span>
                    {#if p.favorite}
                      <Star size={11} class="shrink-0 text-[var(--color-accent)]" fill="currentColor" />
                    {/if}
                  </div>
                  <div class="text-[11px] text-[var(--color-fg-muted)] truncate font-mono">
                    {p.ssh.user ? `${p.ssh.user}@` : ''}{p.ssh.host}{p.ssh.port && p.ssh.port !== 22 ? `:${p.ssh.port}` : ''}
                  </div>
                  {#if (p.tags ?? []).length > 0}
                    <div class="tag-row">
                      {#each (p.tags ?? []).slice(0, 6) as tag (tag)}
                        <span>{tag}</span>
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
