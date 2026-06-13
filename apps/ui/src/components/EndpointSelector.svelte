<script lang="ts">
  import { Monitor } from '@lucide/svelte';
  import type { StoredProfile } from '../lib/types';
  import { matchesProfileQuery, sortProfiles, profileEndpointLabel } from '../lib/profileMeta';
  import { i18n } from '../lib/i18n.svelte';

  interface Props {
    profiles: StoredProfile[];
    value: string;
    placeholder?: string;
    localLabel: string;
    onChange: (value: string) => void;
  }

  let {
    profiles = [],
    value = '',
    placeholder = '',
    localLabel,
    onChange,
  }: Props = $props();

  const LOCAL_ID = '__local__';
  let query = $state('');
  let open = $state(false);
  let hover = $state(0);

  const q = $derived(query.trim().toLowerCase());

  const items = $derived.by(() => {
    const sorted = sortProfiles(profiles.filter((p) => p.kind === 'ssh'));
    if (!q) return sorted;
    return sorted.filter((p) => matchesProfileQuery(p, q));
  });

  function displayName(): string {
    if (value === LOCAL_ID) return localLabel;
    const p = profiles.find((pr) => pr.id === value);
    return p?.name ?? '';
  }

  function select(id: string) {
    if (id && id !== value) onChange(id);
    open = false;
    query = '';
    hover = 0;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') { open = false; query = ''; return; }
    if (e.key === 'ArrowDown') { e.preventDefault(); hover = Math.min(hover + 1, items.length); return; }
    if (e.key === 'ArrowUp') { e.preventDefault(); hover = Math.max(hover - 1, 0); return; }
    if (e.key === 'Enter') {
      e.preventDefault();
      const f = items[hover];
      if (f) select(f.id);
    }
  }

  let wrapEl = $state<HTMLDivElement | null>(null);

  function onDocClick(e: MouseEvent) {
    if (wrapEl && !wrapEl.contains(e.target as Node)) {
      open = false;
      query = '';
    }
  }

  $effect(() => {
    if (open) {
      document.addEventListener('click', onDocClick, true);
      return () => document.removeEventListener('click', onDocClick, true);
    }
  });
</script>

<div bind:this={wrapEl} class="relative min-w-0">
  <input
    type="text"
    class="input py-1 text-[12px] w-full"
    class:ring-1={open}
    class:border-[var(--color-accent)]={open}
    placeholder={displayName() || placeholder}
    bind:value={query}
    onfocus={() => { open = true; hover = 0; }}
    onkeydown={onKey}
  />

  {#if open}
    <div class="absolute left-0 right-0 top-full mt-1 z-50 bg-[var(--color-bg)] border border-[var(--color-border)] rounded-md shadow-lg max-h-[260px] overflow-y-auto py-1">
      <button type="button"
        class="w-full flex items-center gap-2 px-2 py-1.5 text-[12px] text-left {value === LOCAL_ID ? 'text-[var(--color-accent)]' : ''} {hover === -1 ? 'bg-[var(--color-accent)]/15' : 'hover:bg-[var(--color-panel-2)]'}"
        onclick={() => select(LOCAL_ID)}
        onmouseenter={() => { hover = -1; }}
      >
        <Monitor size={13} class="shrink-0" />
        <span class="truncate">{localLabel}</span>
      </button>
      {#each items as p, i (p.id)}
        <button type="button"
          class="w-full flex items-center gap-2 px-2 py-1.5 text-[12px] text-left {value === p.id ? 'text-[var(--color-accent)]' : ''} {hover === i ? 'bg-[var(--color-accent)]/15' : 'hover:bg-[var(--color-panel-2)]'}"
          onclick={() => select(p.id)}
          onmouseenter={() => { hover = i; }}
        >
          <span class="truncate flex-1">{p.name}</span>
          <span class="shrink-0 text-[10px] text-[var(--color-fg-muted)] truncate max-w-[160px]">{profileEndpointLabel(p)}</span>
        </button>
      {/each}
      {#if items.length === 0 && q}
        <div class="px-2 py-3 text-[12px] text-[var(--color-fg-muted)] text-center">{i18n.t('common.noMatches')}</div>
      {/if}
    </div>
  {/if}
</div>
