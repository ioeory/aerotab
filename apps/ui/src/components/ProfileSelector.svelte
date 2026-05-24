<script lang="ts">
  // M2 — Profile picker dropdown. Modelled after classic terminal's "Select profile or
  // enter an address" overlay: address input at top; grouped lists of
  // Recent, custom groups, user-saved profiles, built-in shells, and
  // entries imported from ~/.ssh/config.
  //
  // The picker is purely presentational: it asks the parent to open the
  // chosen entry via `onOpen`. Recent profiles are persisted under the
  // settings key `recentProfiles` as an array of profile-id strings.

  import { onMount, tick } from 'svelte';
  import {
    Eraser, Monitor, Terminal as TerminalIcon, Cog, ArrowRight,
  } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import type { StoredProfile } from '../lib/types';
  import { i18n } from '../lib/i18n.svelte';
  import { matchesProfileQuery, profileEndpointLabel, profileGroupName, sortProfiles } from '../lib/profileMeta';
  import ProfileIcon from './ProfileIcon.svelte';

  // Backend-supplied discovery payload.
  interface ShellEntry {
    id: string;
    label: string;
    command: string;
    args: string[];
    icon: string;
  }
  interface SshConfigEntry {
    alias: string;
    host: string;
    port: number;
    user: string | null;
    identity_file: string | null;
    proxy_jump?: string[];
  }

  // Discriminated union of every clickable item the picker can show.
  export type PickerItem =
    | { kind: 'profile'; profile: StoredProfile }
    | { kind: 'shell'; shell: ShellEntry }
    | { kind: 'ssh-config'; entry: SshConfigEntry }
    | { kind: 'address'; address: string };

  interface Props {
    rpc: RpcClient;
    onClose: () => void;
    onOpen: (item: PickerItem) => void;
  }
  let { rpc, onClose, onOpen }: Props = $props();

  let profiles = $state<StoredProfile[]>([]);
  let shells = $state<ShellEntry[]>([]);
  let sshConfig = $state<SshConfigEntry[]>([]);
  let recentIds = $state<string[]>([]);
  let query = $state('');
  let inputEl: HTMLInputElement | null = $state(null);
  let hover = $state<number>(0); // keyboard-selected row index in flatList

  // ----- load -----
  onMount(() => {
    void (async () => {
      try {
        profiles = await rpc.call<StoredProfile[]>('profile.list');
      } catch { profiles = []; }
      try {
        const d = await rpc.call<{ shells: ShellEntry[]; sshConfig: SshConfigEntry[] }>(
          'profile.discover',
        );
        shells = d.shells ?? [];
        sshConfig = d.sshConfig ?? [];
      } catch {
        shells = []; sshConfig = [];
      }
      try {
        const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'recentProfiles' });
        if (Array.isArray(r.value)) recentIds = r.value.filter((x) => typeof x === 'string') as string[];
      } catch { recentIds = []; }
      await tick();
      inputEl?.focus();
    })();
  });

  // ----- derived: filtered groups -----
  const q = $derived(query.trim().toLowerCase());
  const matchProfile = (p: StoredProfile) => matchesProfileQuery(p, query);
  const matchShell = (s: ShellEntry) =>
    !q || s.label.toLowerCase().includes(q) || s.command.toLowerCase().includes(q);
  const matchSshConfig = (s: SshConfigEntry) =>
    !q || s.alias.toLowerCase().includes(q) || s.host.toLowerCase().includes(q);

  const recentProfiles = $derived(
    recentIds
      .map((id) => profiles.find((p) => p.id === id))
      .filter((p): p is StoredProfile => !!p)
      .filter(matchProfile),
  );

  // Group user profiles by their `group` label. Ungrouped go into '' bucket.
  const groupedProfiles = $derived.by(() => {
    const map = new Map<string, StoredProfile[]>();
    for (const p of profiles) {
      if (!matchProfile(p)) continue;
      const g = profileGroupName(p);
      if (!map.has(g)) map.set(g, []);
      map.get(g)!.push(p);
    }
    // Stable order: ungrouped first, then alphabetical by group name.
    return Array.from(map.entries())
      .map(([groupName, ps]) => [groupName, sortProfiles(ps)] as [string, StoredProfile[]])
      .sort(([a], [b]) => {
        if (a === '(Ungrouped)') return -1;
        if (b === '(Ungrouped)') return 1;
        return a.localeCompare(b);
      });
  });

  const visibleShells = $derived(shells.filter(matchShell));
  const visibleSshConfig = $derived(sshConfig.filter(matchSshConfig));

  // Flatten everything to a single keyboard-navigable list.
  const flatList = $derived.by((): PickerItem[] => {
    const list: PickerItem[] = [];
    for (const p of recentProfiles) list.push({ kind: 'profile', profile: p });
    for (const [, ps] of groupedProfiles) {
      for (const p of ps) list.push({ kind: 'profile', profile: p });
    }
    for (const s of visibleShells) list.push({ kind: 'shell', shell: s });
    for (const e of visibleSshConfig) list.push({ kind: 'ssh-config', entry: e });
    return list;
  });

  $effect(() => {
    // Reset hover when filter changes.
    void q;
    hover = 0;
  });

  // ----- actions -----
  async function chooseRecord(item: PickerItem) {
    if (item.kind === 'profile') {
      pushRecent(item.profile.id);
    }
    onOpen(item);
  }
  function pushRecent(id: string) {
    const next = [id, ...recentIds.filter((x) => x !== id)].slice(0, 8);
    recentIds = next;
    void rpc.call('settings.set', { key: 'recentProfiles', value: next })
      .catch(() => {}); // best-effort
  }
  function clearRecent() {
    recentIds = [];
    void rpc.call('settings.set', { key: 'recentProfiles', value: [] }).catch(() => {});
  }
  function submitAddress() {
    const addr = query.trim();
    if (!addr) return;
    onOpen({ kind: 'address', address: addr });
  }
  function onKey(ev: KeyboardEvent) {
    if (ev.key === 'Escape') { ev.preventDefault(); onClose(); return; }
    if (ev.key === 'Enter') {
      ev.preventDefault();
      if (flatList.length > 0 && hover >= 0 && hover < flatList.length) {
        const item = flatList[hover];
        if (item) void chooseRecord(item);
      } else if (query.trim()) {
        submitAddress();
      }
      return;
    }
    if (ev.key === 'ArrowDown') {
      ev.preventDefault();
      hover = Math.min(flatList.length - 1, hover + 1);
      return;
    }
    if (ev.key === 'ArrowUp') {
      ev.preventDefault();
      hover = Math.max(0, hover - 1);
      return;
    }
  }

  function isHovered(item: PickerItem): boolean {
    const idx = flatList.indexOf(item);
    return idx === hover;
  }
</script>

<div
  role="dialog"
  aria-modal="true"
  aria-label={i18n.t('picker.aria')}
  class="fixed inset-0 z-50 grid place-items-start pt-12"
  onclick={(e) => { if (e.target === e.currentTarget) onClose(); }}
  onkeydown={(e) => { if (e.key === 'Escape') onClose(); }}
  tabindex="-1"
>
  <div class="picker-shell">
    <input
      bind:this={inputEl}
      bind:value={query}
      onkeydown={onKey}
      type="text"
      placeholder={i18n.t('picker.placeholder')}
      class="picker-search"
    />

    <div class="picker-scroll">
      {#if recentProfiles.length > 0}
        <div class="picker-cat">{i18n.t('picker.recent')}</div>
        {#each recentProfiles as p (p.id)}
          {@const item = { kind: 'profile' as const, profile: p }}
          <button
            type="button"
            class="picker-row" class:active={isHovered(item)}
            onclick={() => chooseRecord(item)}
          >
            <ProfileIcon icon={p.icon} name={p.name} size={13} />
            <span class="picker-label">{p.name}</span>
            {#if p.favorite}<span class="picker-pill">{i18n.t('picker.favorite')}</span>{/if}
            {#each (p.tags ?? []).slice(0, 2) as tag (tag)}
              <span class="picker-pill">{tag}</span>
            {/each}
            <span class="picker-sub">{profileEndpointLabel(p)}</span>
            {#if isHovered(item)}
              <span class="picker-kbd">ENTER <ArrowRight size={10} /></span>
            {/if}
          </button>
        {/each}
        <button type="button" class="picker-row text-[var(--color-fg-muted)]" onclick={clearRecent}>
          <Eraser size={13} />
          <span class="picker-label">{i18n.t('picker.clearRecent')}</span>
        </button>
      {/if}

      {#each groupedProfiles as [groupName, ps] (groupName || '__ungrouped__')}
        <div class="picker-cat">{groupName} · {ps.length}</div>
        {#each ps as p (p.id)}
          {@const item = { kind: 'profile' as const, profile: p }}
          <button
            type="button"
            class="picker-row" class:active={isHovered(item)}
            onclick={() => chooseRecord(item)}
          >
            <ProfileIcon icon={p.icon} name={p.name} size={13} />
            <span class="picker-label">{p.name}</span>
            {#if p.favorite}<span class="picker-pill">{i18n.t('picker.favorite')}</span>{/if}
            {#each (p.tags ?? []).slice(0, 2) as tag (tag)}
              <span class="picker-pill">{tag}</span>
            {/each}
            <span class="picker-sub">{profileEndpointLabel(p)}</span>
          </button>
        {/each}
      {/each}

      {#if visibleShells.length > 0}
        <div class="picker-cat">{i18n.t('picker.builtInShells')}</div>
        {#each visibleShells as s (s.id)}
          {@const item = { kind: 'shell' as const, shell: s }}
          <button
            type="button"
            class="picker-row" class:active={isHovered(item)}
            onclick={() => chooseRecord(item)}
          >
            <TerminalIcon size={13} />
            <span class="picker-label">{s.label}</span>
            <span class="picker-sub">{s.command}</span>
          </button>
        {/each}
      {/if}

      {#if visibleSshConfig.length > 0}
        <div class="picker-cat">{i18n.t('picker.importedSshConfig')}</div>
        {#each visibleSshConfig as e (e.alias)}
          {@const item = { kind: 'ssh-config' as const, entry: e }}
          <button
            type="button"
            class="picker-row" class:active={isHovered(item)}
            onclick={() => chooseRecord(item)}
          >
            <Monitor size={13} />
            <span class="picker-label">{e.alias} (.ssh/config)</span>
            <span class="picker-sub">{e.host}</span>
          </button>
        {/each}
      {/if}

      {#if flatList.length === 0 && query.trim()}
        <button type="button" class="picker-row active" onclick={submitAddress}>
          <Cog size={13} />
          <span class="picker-label">{i18n.t('picker.connectTo', { address: query.trim() })}</span>
          <span class="picker-kbd">ENTER <ArrowRight size={10} /></span>
        </button>
      {/if}
    </div>
  </div>
</div>

<style>
  .picker-shell {
    width: min(560px, calc(100vw - 24px));
    margin: 0 auto;
    background: var(--color-panel);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-panel);
    overflow: hidden;
    display: flex;
    flex-direction: column;
    max-height: 70vh;
  }
  .picker-search {
    width: 100%;
    border: none;
    outline: 2px solid var(--color-accent);
    outline-offset: -2px;
    background: transparent;
    color: var(--color-fg);
    padding: 12px 14px;
    font-size: 13px;
    font-family: inherit;
  }
  .picker-scroll {
    overflow-y: auto;
    padding: 6px 0 8px;
  }
  .picker-cat {
    padding: 8px 14px 4px;
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.12em;
    color: var(--color-fg-muted);
  }
  .picker-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 14px;
    background: transparent;
    border: none;
    color: var(--color-fg);
    cursor: pointer;
    font-size: 12.5px;
    text-align: left;
    font-family: inherit;
  }
  .picker-row:hover { background: var(--color-panel-2); }
  .picker-row.active {
    background: color-mix(in srgb, var(--color-accent) 22%, var(--color-panel-2));
    color: var(--color-fg);
    border-radius: var(--radius-md);
    margin: 1px 6px;
    padding-left: 8px;
  }
  .picker-row.active .picker-sub { color: var(--color-fg-muted); }
  .picker-label { font-weight: 500; }
  .picker-pill {
    font-size: 10px;
    color: var(--color-fg-muted);
    border: 1px solid var(--color-border-soft);
    border-radius: 999px;
    padding: 0 5px;
    line-height: 16px;
    max-width: 88px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .picker-sub {
    color: var(--color-fg-muted);
    font-size: 11.5px;
    margin-left: 4px;
  }
  .picker-kbd {
    margin-left: auto;
    font-size: 10px;
    font-family: var(--font-mono);
    background: var(--color-bg);
    color: var(--color-fg-muted);
    border: 1px solid var(--color-border);
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
</style>
