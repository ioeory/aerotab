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
  import { scheduleModalFieldFocus } from '../lib/modalFocus';
  import { matchesProfileQuery, profileEndpointLabel, profileGroupName, sortProfiles } from '../lib/profileMeta';
  import ProfileIcon from './ProfileIcon.svelte';
  import ProfileKindBadge from './ProfileKindBadge.svelte';
  import ProfileTag from './ProfileTag.svelte';

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

  type PickerBlock =
    | { kind: 'header'; title: string; key: string }
    | { kind: 'row'; key: string; item: PickerItem; navIndex: number }
    | { kind: 'clearRecent'; key: string };

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
  let scrollEl: HTMLDivElement | null = $state(null);
  /** Index into `navItems` (keyboard/mouse selection). */
  let hover = $state(0);

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
      scheduleModalFieldFocus(() => inputEl?.focus());
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

  const { blocks, navItems } = $derived.by((): { blocks: PickerBlock[]; navItems: PickerItem[] } => {
    const blocks: PickerBlock[] = [];
    const navItems: PickerItem[] = [];
    const pushRow = (key: string, item: PickerItem) => {
      const navIndex = navItems.length;
      navItems.push(item);
      blocks.push({ kind: 'row', key, item, navIndex });
    };

    if (recentProfiles.length > 0) {
      blocks.push({ kind: 'header', title: i18n.t('picker.recent'), key: 'hdr:recent' });
      for (const p of recentProfiles) {
        pushRow(`recent:${p.id}`, { kind: 'profile', profile: p });
      }
      blocks.push({ kind: 'clearRecent', key: 'clear-recent' });
    }

    for (const [groupName, ps] of groupedProfiles) {
      blocks.push({
        kind: 'header',
        title: `${groupName} · ${ps.length}`,
        key: `hdr:${groupName || '__ungrouped__'}`,
      });
      for (const p of ps) {
        pushRow(`group:${groupName}:${p.id}`, { kind: 'profile', profile: p });
      }
    }

    if (visibleShells.length > 0) {
      blocks.push({ kind: 'header', title: i18n.t('picker.builtInShells'), key: 'hdr:shells' });
      for (const s of visibleShells) {
        pushRow(`shell:${s.id}`, { kind: 'shell', shell: s });
      }
    }

    if (visibleSshConfig.length > 0) {
      blocks.push({ kind: 'header', title: i18n.t('picker.importedSshConfig'), key: 'hdr:ssh-config' });
      for (const e of visibleSshConfig) {
        pushRow(`ssh-config:${e.alias}`, { kind: 'ssh-config', entry: e });
      }
    }

    return { blocks, navItems };
  });

  const showAddressRow = $derived(navItems.length === 0 && query.trim().length > 0);

  $effect(() => {
    void q;
    hover = 0;
  });

  $effect(() => {
    const idx = hover;
    void navItems.length;
    void tick().then(() => {
      if (!scrollEl || idx < 0) return;
      const row = scrollEl.querySelector<HTMLElement>(`[data-picker-nav="${idx}"]`);
      row?.scrollIntoView({ block: 'nearest' });
    });
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
  function clampHover(next: number): number {
    if (navItems.length === 0) return 0;
    return Math.max(0, Math.min(navItems.length - 1, next));
  }
  function onKey(ev: KeyboardEvent) {
    if (ev.key === 'Escape') { ev.preventDefault(); onClose(); return; }
    if (ev.key === 'ArrowDown') {
      ev.preventDefault();
      hover = clampHover(hover + 1);
      return;
    }
    if (ev.key === 'ArrowUp') {
      ev.preventDefault();
      hover = clampHover(hover - 1);
      return;
    }
    if (ev.key === 'Home') {
      ev.preventDefault();
      hover = 0;
      return;
    }
    if (ev.key === 'End') {
      ev.preventDefault();
      hover = clampHover(navItems.length - 1);
      return;
    }
    if (ev.key === 'Enter') {
      ev.preventDefault();
      if (navItems.length > 0) {
        const item = navItems[hover];
        if (item) void chooseRecord(item);
      } else if (query.trim()) {
        submitAddress();
      }
      return;
    }
  }
</script>

<div
  data-aerotab-modal=""
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
      role="combobox"
      aria-expanded="true"
      aria-controls="picker-listbox"
      aria-activedescendant={navItems.length > 0 ? `picker-option-${hover}` : undefined}
      aria-autocomplete="list"
      placeholder={i18n.t('picker.placeholder')}
      class="picker-search"
    />

    <div id="picker-listbox" bind:this={scrollEl} class="picker-scroll" role="listbox">
      {#each blocks as block (block.key)}
        {#if block.kind === 'header'}
          <div class="picker-cat">{block.title}</div>
        {:else if block.kind === 'clearRecent'}
          <button type="button" class="picker-row text-[var(--color-fg-muted)]" onclick={clearRecent}>
            <Eraser size={13} />
            <span class="picker-label">{i18n.t('picker.clearRecent')}</span>
          </button>
        {:else if block.item.kind === 'profile'}
          {@const p = block.item.profile}
          <button
            id="picker-option-{block.navIndex}"
            type="button"
            role="option"
            aria-selected={hover === block.navIndex}
            data-picker-nav={block.navIndex}
            class="picker-row"
            class:active={hover === block.navIndex}
            onclick={() => chooseRecord(block.item)}
            onmouseenter={() => { hover = block.navIndex; }}
          >
            <ProfileIcon icon={p.icon} name={p.name} kind={p.kind} size={13} />
            <span class="picker-label">{p.name}</span>
            <ProfileKindBadge kind={p.kind} compact />
            {#if p.favorite}<span class="picker-pill">{i18n.t('picker.favorite')}</span>{/if}
            {#each (p.tags ?? []).slice(0, 2) as tag (tag)}
              <ProfileTag {tag} compact />
            {/each}
            <span class="picker-sub">{profileEndpointLabel(p)}</span>
            {#if hover === block.navIndex}
              <span class="picker-kbd">ENTER <ArrowRight size={10} /></span>
            {/if}
          </button>
        {:else if block.item.kind === 'shell'}
          {@const s = block.item.shell}
          <button
            id="picker-option-{block.navIndex}"
            type="button"
            role="option"
            aria-selected={hover === block.navIndex}
            data-picker-nav={block.navIndex}
            class="picker-row"
            class:active={hover === block.navIndex}
            onclick={() => chooseRecord(block.item)}
            onmouseenter={() => { hover = block.navIndex; }}
          >
            <TerminalIcon size={13} />
            <span class="picker-label">{s.label}</span>
            <span class="picker-sub">{s.command}</span>
            {#if hover === block.navIndex}
              <span class="picker-kbd">ENTER <ArrowRight size={10} /></span>
            {/if}
          </button>
        {:else if block.item.kind === 'ssh-config'}
          {@const e = block.item.entry}
          <button
            id="picker-option-{block.navIndex}"
            type="button"
            role="option"
            aria-selected={hover === block.navIndex}
            data-picker-nav={block.navIndex}
            class="picker-row"
            class:active={hover === block.navIndex}
            onclick={() => chooseRecord(block.item)}
            onmouseenter={() => { hover = block.navIndex; }}
          >
            <Monitor size={13} />
            <span class="picker-label">{e.alias} (.ssh/config)</span>
            <span class="picker-sub">{e.host}</span>
            {#if hover === block.navIndex}
              <span class="picker-kbd">ENTER <ArrowRight size={10} /></span>
            {/if}
          </button>
        {/if}
      {/each}

      {#if showAddressRow}
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
    outline: 1px solid color-mix(in srgb, var(--color-accent) 55%, transparent);
    outline-offset: -1px;
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
