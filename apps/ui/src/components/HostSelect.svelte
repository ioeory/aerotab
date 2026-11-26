<script lang="ts">
  import { tick } from 'svelte';
  import { i18n } from '../lib/i18n.svelte';
  import HostChoiceList, { type HostChoice } from './HostChoiceList.svelte';

  let { choices, value, onChange, label, placeholder, emptyLabel, missingLabel, disabled = false, inline = false }: {
    choices: readonly HostChoice[]; value: string; onChange: (id: string) => void; label: string;
    placeholder?: string; emptyLabel?: string; missingLabel?: string; disabled?: boolean; inline?: boolean;
  } = $props();
  let open = $state(false);
  let root: HTMLDivElement;
  let trigger: HTMLButtonElement;
  const display = $derived(choices.find(choice => choice.id === value)?.label
    ?? (value ? missingLabel ?? value : emptyLabel ?? placeholder ?? i18n.t('hostPicker.choose')));
  const items = $derived(emptyLabel === undefined ? choices : [{ id: '', label: emptyLabel }, ...choices]);
  async function toggle() {
    open = !open;
    if (open) { await tick(); root?.querySelector<HTMLInputElement>('input[type=search]')?.focus(); }
  }
  function close(restore = false) {
    open = false;
    if (restore) trigger?.focus();
  }
  function dismissOutside(event: PointerEvent | FocusEvent) {
    if (open && event.target instanceof Node && !root?.contains(event.target)) close();
  }
  function onKey(event: KeyboardEvent) {
    if (open && event.key === 'Escape' && event.target instanceof Node && root?.contains(event.target)) {
      event.preventDefault(); event.stopPropagation(); close(true);
    }
  }
  $effect(() => { if (disabled) open = false; });
</script>
<svelte:document onpointerdown={dismissOutside} onfocusin={dismissOutside} onkeydowncapture={onKey} />
<div class="host-select" bind:this={root} role="group" aria-label={label}>
  <button type="button" class="trigger" {disabled} aria-label={`${label}: ${display}`} aria-expanded={open} bind:this={trigger} onclick={toggle}>{display}<span aria-hidden="true">▾</span></button>
  {#if open}
    <div class="menu" class:inline>
      <HostChoiceList choices={items} selected={[value]} onPick={id => { close(true); if (id !== value) onChange(id); }} />
    </div>
  {/if}
</div>
<style>
  .host-select{position:relative;min-width:0;flex-shrink:0}.trigger{display:flex;justify-content:space-between;gap:6px;width:100%;text-align:start;overflow-wrap:anywhere;padding:6px 8px;color:inherit;background:var(--color-panel);border:1px solid var(--color-border-soft);border-radius:5px}.trigger:disabled{opacity:.55}.menu{position:absolute;inset-inline:0;top:100%;margin-top:4px;z-index:50;min-width:min(180px,100%);padding:6px;background:var(--color-panel);border:1px solid var(--color-border);border-radius:6px;box-shadow:var(--shadow-lg)}.menu.inline{position:static}
</style>
