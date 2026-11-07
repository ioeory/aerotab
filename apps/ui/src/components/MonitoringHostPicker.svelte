<script lang="ts">
  import type { StoredProfile } from '../lib/types';
  import { monitoringMessage as m } from '../lib/monitoringMessages';
  let { profiles, selected = [], onConfirm, onClose }: { profiles: StoredProfile[]; selected?: string[]; onConfirm: (ids: string[]) => void; onClose: () => void } = $props();
  let chosen = $state<string[]>((() => [...selected])());
  let query = $state('');
  let root: HTMLElement;
  let groups = $derived.by(() => {
    const grouped = new Map<string, StoredProfile[]>();
    for (const profile of profiles.filter(p => `${p.name} ${p.group ?? ''} ${p.kind === 'ssh' ? p.ssh.host : ''}`.toLowerCase().includes(query.toLowerCase()))) {
      const group = profile.group || m('ungrouped'); grouped.set(group, [...(grouped.get(group) ?? []), profile]);
    }
    return [...grouped.entries()].sort(([a], [b]) => a.localeCompare(b));
  });
  const supported = $derived(new Set(profiles.filter(p => p.kind === 'ssh').map(p => p.id)));
  const validChosen = $derived(chosen.filter(id => supported.has(id)));
  function toggle(id: string): void { chosen = chosen.includes(id) ? chosen.filter(item => item !== id) : [...chosen, id]; }
  function toggleGroup(rows: StoredProfile[]): void {
    const ids = rows.filter(p => p.kind === 'ssh').map(p => p.id);
    chosen = ids.every(id => chosen.includes(id)) ? chosen.filter(id => !ids.includes(id)) : [...new Set([...chosen, ...ids])];
  }
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
    <input aria-label={m('search')} placeholder={m('search')} bind:value={query} />
    <div class="groups">
      {#each groups as [group, rows] (group)}
        <section class="group">
          <header><strong>{group}</strong><button disabled={!rows.some(p => p.kind === 'ssh')} onclick={() => toggleGroup(rows)}>{rows.filter(p => p.kind === 'ssh').every(p => chosen.includes(p.id)) ? m('clearGroup') : m('selectGroup')}</button></header>
          {#each rows as profile (profile.id)}
            <label class:unsupported={profile.kind !== 'ssh'}><input type="checkbox" disabled={profile.kind !== 'ssh'} checked={chosen.includes(profile.id)} onchange={() => toggle(profile.id)} /><span><strong>{profile.name}</strong><small>{profile.kind === 'ssh' ? `${profile.ssh.user}@${profile.ssh.host}:${profile.ssh.port}` : m('unsupported')}</small></span></label>
          {/each}
        </section>
      {:else}<p>{m('noResults')}</p>{/each}
    </div>
    <footer><span>{m('selected')}: {validChosen.length}</span><button onclick={onClose}>{m('cancel')}</button><button class="primary" onclick={() => onConfirm(validChosen)}>{m('confirm')}</button></footer>
  </div>
</div>
<style>
  .picker-backdrop{position:absolute;inset:0;background:#0005;z-index:70;display:flex;justify-content:flex-end}.picker{width:min(440px,100%);height:100%;display:flex;flex-direction:column;gap:16px;padding:20px;background:var(--bg,#171b24);color:var(--fg,#e2e8f0);border-left:1px solid var(--border,#384152);box-shadow:-12px 0 40px #0003}header,footer{display:flex;align-items:center;gap:12px;justify-content:space-between}h2{font-size:16px;margin:0}button,input{font:inherit;color:inherit;border:1px solid var(--border,#384152);border-radius:6px;background:var(--surface,#ffffff08);padding:7px 10px}button{cursor:pointer}button:disabled{opacity:.4;cursor:default}.primary{background:var(--accent,#4f8cff);color:white}.groups{overflow:auto;flex:1;min-height:0}.group{margin-bottom:20px}.group header{font-size:12px;padding-bottom:8px}.group header button{font-size:11px}label{display:flex;align-items:center;gap:10px;padding:10px 6px;border-bottom:1px solid var(--border,#ffffff12)}label span{min-width:0}small{display:block;opacity:.65;font-size:11px;overflow-wrap:anywhere}.unsupported{opacity:.5}footer{font-size:12px}footer span{flex:1}:focus-visible{outline:2px solid var(--accent,#4f8cff);outline-offset:2px}
</style>
