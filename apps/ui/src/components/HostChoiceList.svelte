<script lang="ts" module>
  import type { StoredProfile } from '../lib/types';
  import { matchesProfileQuery, profileEndpointLabel, sortProfiles } from '../lib/profileMeta';

  export interface HostChoice {
    id: string;
    label: string;
    detail?: string;
    group?: string;
    section?: string;
    profile?: StoredProfile;
    disabled?: boolean;
  }
  export function profileChoice(profile: StoredProfile): HostChoice {
    return { id: profile.id, label: profile.name, group: profile.group ?? undefined, profile, detail: profileEndpointLabel(profile) };
  }
  export function profileChoices(profiles: StoredProfile[], sshOnly = false): HostChoice[] {
    return sortProfiles(sshOnly ? profiles.filter(profile => profile.kind === 'ssh') : profiles).map(profileChoice);
  }
</script>
<script lang="ts">
  import { i18n } from '../lib/i18n.svelte';
  let { choices, selected = [], onChange, onPick, disabled = false, limit = Infinity, groupSelection = false, roomy = false }: {
    choices: readonly HostChoice[]; selected?: readonly string[]; onChange?: (ids: string[]) => void;
    onPick?: (id: string) => void; disabled?: boolean; limit?: number; groupSelection?: boolean; roomy?: boolean;
  } = $props();
  let query = $state('');
  const groups = $derived.by(() => {
    const grouped = new Map<string, { label: string; rows: HostChoice[] }>();
    const terms = query.toLowerCase().trim().split(/\s+/).filter(Boolean);
    for (const choice of choices) {
      const text = `${choice.label} ${choice.detail ?? ''} ${choice.group ?? ''}`.toLowerCase();
      if (!terms.every(term => text.includes(term) || (choice.profile && matchesProfileQuery(choice.profile, term)))) continue;
      const label = choice.group ?? '';
      const key = JSON.stringify([choice.section ?? '', label]);
      if (!grouped.has(key)) grouped.set(key, { label, rows: [] });
      grouped.get(key)?.rows.push(choice);
    }
    return grouped;
  });
  function handleSearchKey(event: KeyboardEvent) {
    if (event.isComposing || (event.key !== 'Enter' && event.key !== 'ArrowDown')) return;
    event.preventDefault();
    const first = (event.currentTarget as HTMLInputElement).parentElement?.querySelector<HTMLElement>('.row:is(button):not(:disabled), .row input:not(:disabled)');
    if (event.key === 'Enter' && onPick) first?.click();
    else first?.focus();
  }
  function toggle(ids: string[]) {
    if (disabled || !ids.length) return;
    if (ids.every(id => selected.includes(id))) onChange?.(selected.filter(id => !ids.includes(id)));
    else onChange?.([...new Set([...selected, ...ids])].slice(0, limit));
  }
</script>
<div class="choices" class:roomy>
  <input type="search" aria-label={i18n.t('hostPicker.search')} placeholder={i18n.t('hostPicker.search')} bind:value={query} disabled={disabled} onkeydown={handleSearchKey} />
  <div class="host-list">
    {#each groups as [key, group] (key)}
      {@const enabled = group.rows.filter(row => !row.disabled).map(row => row.id)}
      {@const allSelected = enabled.every(id => selected.includes(id))}
      {#if group.label || groupSelection}<div class="group">
        <strong>{group.label || i18n.t('sidebar.ungrouped')}</strong>
        {#if groupSelection}<button type="button" disabled={disabled || !enabled.length || (!allSelected && selected.length >= limit)} onclick={() => toggle(enabled)}>{i18n.t(allSelected ? 'hostPicker.clearGroup' : 'hostPicker.selectGroup')}</button>{/if}
      </div>{/if}
      {#each group.rows as row (row.id)}
        {#snippet content()}
          <span>{row.label}{#if row.detail}<small>{row.detail}</small>{/if}</span>
        {/snippet}
        {#if onPick}
          <button class="row pick" type="button" aria-pressed={selected.includes(row.id)} disabled={disabled || row.disabled} onclick={() => onPick?.(row.id)}>{@render content()}</button>
        {:else}
          <label class="row"><input type="checkbox" checked={selected.includes(row.id)} disabled={disabled || row.disabled || (!selected.includes(row.id) && selected.length >= limit)} onchange={() => toggle([row.id])} />{@render content()}</label>
        {/if}
      {/each}
    {:else}<p>{i18n.t('common.noMatches')}</p>{/each}
  </div>
</div>
<style>
  .choices{display:grid;gap:6px;min-width:0}.host-list{max-height:240px;overflow:auto;overscroll-behavior:contain}.roomy{height:100%;grid-template-rows:auto minmax(0,1fr)}.roomy .host-list{max-height:none}.group,.row{display:flex;align-items:center;gap:8px;padding:5px}.group{justify-content:space-between}.row span{min-width:0;overflow-wrap:anywhere}small{display:block;color:var(--color-fg-muted)}input[type=search],button{min-width:0;color:inherit;background:var(--color-panel);border:1px solid var(--color-border-soft);padding:5px;border-radius:4px}.pick{width:100%;text-align:start;border-color:transparent}.pick:hover,.pick[aria-pressed=true]{background:var(--color-panel-2)}.pick[aria-pressed=true]{color:var(--color-accent)}button:disabled{opacity:.5}p{margin:4px 0;color:var(--color-fg-muted)}
</style>
