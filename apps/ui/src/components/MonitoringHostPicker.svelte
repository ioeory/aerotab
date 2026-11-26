<script lang="ts">
  import type { StoredProfile } from '../lib/types';
  import { monitoringMessage as m } from '../lib/monitoringMessages';
  import HostChoiceList, { profileChoices } from './HostChoiceList.svelte';
  let { profiles, selected = [], onConfirm, onClose }: { profiles: StoredProfile[]; selected?: string[]; onConfirm: (ids: string[]) => void; onClose: () => void } = $props();
  let chosen = $state<string[]>((() => [...selected])());
  let root: HTMLElement;
  const choices = $derived(profileChoices(profiles).map(choice => ({ ...choice, disabled: choice.profile?.kind !== 'ssh' })));
  const supported = $derived(new Set(profiles.filter(p => p.kind === 'ssh').map(p => p.id)));
  const validChosen = $derived(chosen.filter(id => supported.has(id)));
  function keydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') { event.preventDefault(); onClose(); }
    if (event.key !== 'Tab') return;
    const nodes = [...root.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled)')];
    if (event.shiftKey && document.activeElement === nodes[0]) { event.preventDefault(); nodes.at(-1)?.focus(); }
    else if (!event.shiftKey && document.activeElement === nodes.at(-1)) { event.preventDefault(); nodes[0]?.focus(); }
  }
  $effect(() => { const previous = document.activeElement; root?.querySelector<HTMLInputElement>('input')?.focus(); return () => { if (previous instanceof HTMLElement && previous.isConnected) previous.focus(); }; });
</script>
<div class="picker-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget) onClose(); }}>
  <div class="picker" role="dialog" aria-modal="true" aria-label={m('picker')} tabindex="-1" bind:this={root} onkeydown={keydown}>
    <header><h2>{m('picker')}</h2><button onclick={onClose} aria-label={m('cancel')}>×</button></header>
    <div class="groups"><HostChoiceList {choices} selected={chosen} onChange={ids => { chosen = ids; }} groupSelection roomy /></div>
    <footer><span>{m('selected')}: {validChosen.length}</span><button onclick={onClose}>{m('cancel')}</button><button class="primary" onclick={() => onConfirm(validChosen)}>{m('confirm')}</button></footer>
  </div>
</div>
<style>
  .picker-backdrop{position:absolute;inset:0;background:#0005;z-index:70;display:flex;justify-content:flex-end}.picker{width:min(440px,100%);height:100%;display:flex;flex-direction:column;gap:16px;padding:20px;background:var(--bg,#171b24);color:var(--fg,#e2e8f0);border-left:1px solid var(--border,#384152);box-shadow:-12px 0 40px #0003}header,footer{display:flex;align-items:center;gap:12px;justify-content:space-between}h2{font-size:16px;margin:0}button{font:inherit;color:inherit;border:1px solid var(--border,#384152);border-radius:6px;background:var(--surface,#ffffff08);padding:7px 10px}button{cursor:pointer}button:disabled{opacity:.4;cursor:default}.primary{background:var(--accent,#4f8cff);color:white}.groups{overflow:auto;flex:1;min-height:0}footer{font-size:12px}footer span{flex:1}:focus-visible{outline:2px solid var(--accent,#4f8cff);outline-offset:2px}
</style>
