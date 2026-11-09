<script lang="ts">
  import { ChevronRight } from '@lucide/svelte';
  import { profileGroupBreadcrumbs } from '../../lib/profileCatalogHelpers';
  import { hostLabel } from './hostLabels';
  import { i18n } from '../../lib/i18n.svelte';
  let { group, favorites, recent, onSelect }: { group: string | null; favorites: boolean; recent: boolean; onSelect: (group: string | null) => void } = $props();
  const crumbs = $derived(profileGroupBreadcrumbs(group));
</script>
<nav aria-label={i18n.t('profileModal.group')}>
  <button type="button" aria-current={group === null && !favorites && !recent ? 'page' : undefined} onclick={() => onSelect(null)}>{hostLabel('all')}</button>
  {#if group !== null && !favorites && !recent}<ChevronRight size={14} aria-hidden="true" /><button type="button" aria-current={group === '' ? 'page' : undefined} onclick={() => onSelect('')}>{i18n.t('hosts.root')}</button>{/if}
  {#if favorites || recent}
    <ChevronRight size={14} aria-hidden="true" /><span aria-current="page">{recent ? i18n.t('picker.recent') : favorites ? i18n.t('profileModal.favorite') : i18n.t('sidebar.ungrouped')}</span>
  {:else}
    {#each crumbs as crumb, index (crumb.path)}
      <ChevronRight size={14} aria-hidden="true" /><button type="button" aria-current={index === crumbs.length - 1 ? 'page' : undefined} title={crumb.path} onclick={() => onSelect(crumb.path)}>{crumb.name}</button>
    {/each}
  {/if}
</nav>
<style>
  nav { display: flex; align-items: center; flex-wrap: wrap; min-width: 0; gap: 4px; font-size: 14px; line-height: 1.5; }
  button { background: transparent; color: var(--color-fg-muted); border-radius: 4px; min-height: 36px; padding: 4px 6px; overflow-wrap: anywhere; text-align: left; cursor: pointer; }
  button:hover { color: var(--color-accent); background: var(--color-panel-2); }
  [aria-current='page'] { color: var(--color-fg); font-weight: 500; }
  button:focus-visible { outline: 2px solid var(--color-accent); }
</style>
