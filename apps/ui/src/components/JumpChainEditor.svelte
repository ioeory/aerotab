<script lang="ts">
  import { ChevronDown, ChevronUp, GripVertical, Plus, Search, X } from '@lucide/svelte';
  import { i18n } from '../lib/i18n.svelte';
  import {
    formatManualJumpLine,
    isProfileInJumpChain,
    jumpLineForProfile,
    jumpLineSubtitle,
    jumpLineTitle,
    profileMatchesJumpSearch,
    reorderJumpLines,
  } from '../lib/jumpProfiles';
  import { profileEndpointLabel } from '../lib/profileMeta';
  import type { StoredProfile } from '../lib/types';

  interface Props {
    profiles: StoredProfile[];
    excludeProfileId?: string;
    jumpChainLines?: string[];
    onError?: (msg: string) => void;
  }

  let {
    profiles,
    excludeProfileId = '',
    jumpChainLines = $bindable([]),
    onError,
  }: Props = $props();

  let pickerSearch = $state('');
  let showManualForm = $state(false);
  let manualUser = $state('');
  let manualHost = $state('');
  let manualPort = $state(22);
  let dragFromIndex = $state<number | null>(null);
  let dragOverIndex = $state<number | null>(null);

  const pickableProfiles = $derived(
    profiles.filter(
      (p) => p.kind === 'ssh'
        && p.id !== excludeProfileId
        && !isProfileInJumpChain(p, jumpChainLines),
    ),
  );

  const filteredPickableProfiles = $derived(
    pickableProfiles.filter((p) => profileMatchesJumpSearch(p, pickerSearch)),
  );

  function addProfileToChain(profile: StoredProfile) {
    if (profile.kind !== 'ssh') return;
    if (isProfileInJumpChain(profile, jumpChainLines)) return;
    jumpChainLines = [...jumpChainLines, jumpLineForProfile(profile)];
  }

  function removeHop(index: number) {
    jumpChainLines = jumpChainLines.filter((_, i) => i !== index);
  }

  function moveHop(index: number, delta: -1 | 1) {
    const target = index + delta;
    if (target < 0 || target >= jumpChainLines.length) return;
    jumpChainLines = reorderJumpLines(jumpChainLines, index, target);
  }

  function clearChain() {
    jumpChainLines = [];
  }

  function submitManualHop() {
    try {
      const line = formatManualJumpLine(manualUser, manualHost, manualPort);
      const exists = jumpChainLines.some((l) => l.toLowerCase() === line.toLowerCase());
      if (exists) return;
      jumpChainLines = [...jumpChainLines, line];
      manualUser = '';
      manualHost = '';
      manualPort = 22;
      showManualForm = false;
    } catch (e) {
      onError?.((e as Error).message);
    }
  }

  function onDragStart(index: number, e: DragEvent) {
    dragFromIndex = index;
    dragOverIndex = index;
    e.dataTransfer?.setData('text/plain', String(index));
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }

  function onDragOver(index: number, e: DragEvent) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragOverIndex = index;
  }

  function onDrop(index: number, e: DragEvent) {
    e.preventDefault();
    const raw = e.dataTransfer?.getData('text/plain');
    const from = dragFromIndex ?? (raw ? Number(raw) : NaN);
    if (!Number.isFinite(from)) return;
    jumpChainLines = reorderJumpLines(jumpChainLines, from, index);
    dragFromIndex = null;
    dragOverIndex = null;
  }

  function onDragEnd() {
    dragFromIndex = null;
    dragOverIndex = null;
  }
</script>

<div class="jump-chain-editor mt-2 space-y-2">
  <section class="border border-[var(--color-border-soft)] rounded-md overflow-hidden">
    <div class="px-2 py-1.5 text-[10.5px] text-[var(--color-fg-muted)] bg-[var(--color-panel-2)] border-b border-[var(--color-border-soft)]">
      {i18n.t('profileModal.jumpChainTitle')}
    </div>
    {#if jumpChainLines.length === 0}
      <div class="px-3 py-4 text-[11px] text-[var(--color-fg-muted)] text-center">
        {i18n.t('profileModal.jumpChainEmpty')}
      </div>
    {:else}
      <ol class="jump-chain-list list-none m-0 p-1 space-y-0.5" aria-label={i18n.t('profileModal.jumpChainTitle')}>
        {#each jumpChainLines as line, index (line + index)}
          <li
            class="jump-chain-item flex items-center gap-1 px-1.5 py-1 rounded text-[11.5px]
                   {dragOverIndex === index ? 'bg-[var(--color-accent)]/15 ring-1 ring-[var(--color-accent)]/40' : 'hover:bg-[var(--color-panel-2)]'}"
            draggable="true"
            ondragstart={(e) => onDragStart(index, e)}
            ondragover={(e) => onDragOver(index, e)}
            ondrop={(e) => onDrop(index, e)}
            ondragend={onDragEnd}
          >
            <span
              class="jump-drag-handle shrink-0 p-0.5 text-[var(--color-fg-muted)] cursor-grab active:cursor-grabbing touch-none"
              title={i18n.t('profileModal.jumpDragReorder')}
              aria-hidden="true"
            >
              <GripVertical size={13} />
            </span>
            <span class="w-4 shrink-0 text-center text-[10px] font-medium text-[var(--color-accent)]">{index + 1}</span>
            <div class="min-w-0 flex-1">
              <div class="truncate text-[var(--color-fg)]">{jumpLineTitle(line, profiles)}</div>
              <div class="truncate text-[10px] text-[var(--color-fg-muted)]">{jumpLineSubtitle(line, profiles)}</div>
            </div>
            <button
              type="button"
              class="btn-ghost p-0.5 shrink-0"
              disabled={index === 0}
              title={i18n.t('profileModal.jumpMoveUp')}
              aria-label={i18n.t('profileModal.jumpMoveUp')}
              onclick={() => moveHop(index, -1)}
            >
              <ChevronUp size={14} />
            </button>
            <button
              type="button"
              class="btn-ghost p-0.5 shrink-0"
              disabled={index === jumpChainLines.length - 1}
              title={i18n.t('profileModal.jumpMoveDown')}
              aria-label={i18n.t('profileModal.jumpMoveDown')}
              onclick={() => moveHop(index, 1)}
            >
              <ChevronDown size={14} />
            </button>
            <button
              type="button"
              class="btn-ghost p-0.5 shrink-0 text-[var(--color-danger)]"
              title={i18n.t('profileModal.jumpRemoveHop')}
              aria-label={i18n.t('profileModal.jumpRemoveHop')}
              onclick={() => removeHop(index)}
            >
              <X size={14} />
            </button>
          </li>
        {/each}
      </ol>
    {/if}
    {#if jumpChainLines.length > 0}
      <div class="px-2 py-1.5 border-t border-[var(--color-border-soft)] flex justify-end">
        <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" onclick={clearChain}>
          {i18n.t('profileModal.jumpClearChain')}
        </button>
      </div>
    {/if}
  </section>

  {#if pickableProfiles.length > 0}
    <section class="border border-[var(--color-border-soft)] rounded-md overflow-hidden">
      <div class="px-2 py-1.5 text-[10.5px] text-[var(--color-fg-muted)] bg-[var(--color-panel-2)] border-b border-[var(--color-border-soft)]">
        {i18n.t('profileModal.jumpPickFromList')}
      </div>
      <div class="px-2 py-1.5 border-b border-[var(--color-border-soft)]">
        <div class="flex items-center gap-1.5 rounded border border-[var(--color-border-soft)] bg-[var(--color-panel)] px-2 py-1 focus-within:ring-1 focus-within:ring-[var(--color-accent)]/40">
          <Search size={13} class="shrink-0 text-[var(--color-fg-muted)]" />
          <input
            type="search"
            bind:value={pickerSearch}
            placeholder={i18n.t('profileModal.jumpSearchPlaceholder')}
            class="flex-1 min-w-0 border-0 bg-transparent text-[11.5px] outline-none"
            autocomplete="off"
          />
        </div>
      </div>
      <div class="jump-picker-list max-h-[160px] overflow-y-auto px-1 py-1">
        {#if filteredPickableProfiles.length === 0}
          <div class="px-2 py-3 text-[11px] text-[var(--color-fg-muted)] text-center">
            {i18n.t('profileModal.jumpSearchEmpty')}
          </div>
        {:else}
          {#each filteredPickableProfiles as jp (jp.id)}
            <div class="jump-picker-row flex items-center gap-2 px-2 py-1 rounded hover:bg-[var(--color-panel-2)] text-[11.5px]">
              <div class="min-w-0 flex-1">
                <div class="truncate text-[var(--color-fg)]">{jp.name}</div>
                <div class="truncate text-[10px] text-[var(--color-fg-muted)]">{profileEndpointLabel(jp)}</div>
              </div>
              <button
                type="button"
                class="btn-secondary shrink-0 p-1"
                title={i18n.t('profileModal.jumpAddProfile')}
                aria-label={i18n.t('profileModal.jumpAddProfile')}
                onclick={() => addProfileToChain(jp)}
              >
                <Plus size={14} />
              </button>
            </div>
          {/each}
        {/if}
      </div>
    </section>
  {/if}

  <div>
    {#if !showManualForm}
      <button
        type="button"
        class="text-[11px] text-[var(--color-accent)] hover:underline"
        onclick={() => { showManualForm = true; }}
      >
        {i18n.t('profileModal.jumpAddManual')}
      </button>
    {:else}
      <div class="manual-hop-form border border-[var(--color-border-soft)] rounded-md p-2 space-y-2">
        <div class="text-[10.5px] text-[var(--color-fg-muted)]">{i18n.t('profileModal.jumpManualHint')}</div>
        <div class="grid grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_72px] gap-2">
          <input bind:value={manualUser} placeholder={i18n.t('profileModal.user')} class="input py-1 text-[11.5px]" />
          <input bind:value={manualHost} placeholder={i18n.t('profileModal.host')} class="input py-1 text-[11.5px]" />
          <input bind:value={manualPort} type="number" min="1" max="65535" class="input py-1 text-[11.5px]" />
        </div>
        <div class="flex justify-end gap-1.5">
          <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" onclick={() => { showManualForm = false; }}>
            {i18n.t('common.cancel')}
          </button>
          <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" onclick={submitManualHop}>
            {i18n.t('profileModal.jumpAddManualSubmit')}
          </button>
        </div>
      </div>
    {/if}
  </div>

  <p class="text-[10.5px] text-[var(--color-fg-muted)]">{i18n.t('profileModal.jumpChainHint')}</p>
</div>

<style>
  .jump-chain-item[draggable='true'] {
    user-select: none;
  }
</style>
