<script lang="ts">
  import { onMount } from 'svelte';
  import { X } from '@lucide/svelte';
  import { i18n } from '../lib/i18n.svelte';

  export interface Action {
    id: string;
    title: string;
    subtitle?: string;
    shortcut?: string;
    run: () => void | Promise<void>;
  }

  interface Props {
    actions: Action[];
    onClose: () => void;
  }
  let { actions, onClose }: Props = $props();

  let query = $state('');
  let selected = $state(0);
  let inputEl: HTMLInputElement | null = $state(null);

  const filtered = $derived(
    actions.filter((a) => {
      const q = query.trim().toLowerCase();
      if (!q) return true;
      return a.title.toLowerCase().includes(q) || (a.subtitle ?? '').toLowerCase().includes(q);
    }),
  );

  $effect(() => {
    void filtered;
    selected = 0;
  });

  function onKey(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      selected = Math.min(selected + 1, filtered.length - 1);
      e.preventDefault();
    } else if (e.key === 'ArrowUp') {
      selected = Math.max(selected - 1, 0);
      e.preventDefault();
    } else if (e.key === 'Enter') {
      const a = filtered[selected];
      if (a) {
        onClose();
        void a.run();
      }
      e.preventDefault();
    } else if (e.key === 'Escape') {
      onClose();
      e.preventDefault();
    }
  }

  onMount(() => {
    requestAnimationFrame(() => inputEl?.focus());
  });
</script>

<div role="dialog" aria-modal="true" aria-label={i18n.t('commandPalette.aria')}
  tabindex="-1"
     class="fixed inset-0 z-[60] bg-black/50 grid place-items-start pt-[12vh] px-4"
     onclick={onClose}
     onkeydown={onKey}>
  <div class="w-full max-w-[560px] bg-[var(--color-panel)] border border-[var(--color-border)]
              rounded-lg shadow-2xl overflow-hidden"
       onclick={(e) => e.stopPropagation()}
       role="presentation">
    <div class="flex items-center gap-2 px-3 py-2 border-b border-[var(--color-border-soft)]">
      <input bind:this={inputEl} bind:value={query}
             type="text" placeholder={i18n.t('commandPalette.placeholder')}
             class="flex-1 bg-transparent text-[var(--color-fg)] outline-none text-[13.5px]" />
      <button type="button" class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]"
              onclick={onClose} aria-label={i18n.t('common.close')}><X size={13} /></button>
    </div>
    <ul class="max-h-[50vh] overflow-y-auto py-1" role="listbox">
      {#each filtered as a, i (a.id)}
        <li role="option" aria-selected={i === selected}
            tabindex="-1"
            class="px-3 py-1.5 cursor-pointer flex items-center gap-2 text-[12.5px]
                   {i === selected ? 'bg-[var(--color-panel-2)] text-[var(--color-fg)]' : 'text-[var(--color-fg)]'}"
            onmouseenter={() => (selected = i)}
            onkeydown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                onClose();
                void a.run();
                e.preventDefault();
              }
            }}
            onclick={() => { onClose(); void a.run(); }}>
          <div class="flex-1 min-w-0">
            <div class="truncate">{a.title}</div>
            {#if a.subtitle}
              <div class="text-[10.5px] text-[var(--color-fg-muted)] truncate">{a.subtitle}</div>
            {/if}
          </div>
          {#if a.shortcut}
            <kbd class="text-[10px] text-[var(--color-fg-muted)] px-1.5 py-0.5 border border-[var(--color-border)] rounded">
              {a.shortcut}
            </kbd>
          {/if}
        </li>
      {/each}
      {#if filtered.length === 0}
        <li class="px-3 py-4 text-center text-[var(--color-fg-muted)] text-[12px]">
          {i18n.t('common.noMatches')}
        </li>
      {/if}
    </ul>
  </div>
</div>
