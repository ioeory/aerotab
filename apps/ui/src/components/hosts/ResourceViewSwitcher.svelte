<script lang="ts">
  import { LayoutGrid, List, ListTree } from '@lucide/svelte';
  import type { ResourceView } from '../../lib/profileCatalogHelpers';
  import { hostLabel } from './hostLabels';
  let { value, onChange }: { value: ResourceView; onChange: (value: ResourceView) => void } = $props();
</script>
<div class="view-switch" role="group" aria-label={hostLabel(value)}>
  {#each [{ key: 'grid', icon: LayoutGrid }, { key: 'list', icon: List }, { key: 'tree', icon: ListTree }] as item}
    <button type="button" class:chosen={value === item.key} aria-pressed={value === item.key}
      aria-label={hostLabel(item.key as ResourceView)} title={hostLabel(item.key as ResourceView)} onclick={() => onChange(item.key as ResourceView)}>
      <item.icon size={18} />
    </button>
  {/each}
</div>
<style>
  .view-switch { display: flex; gap: 2px; border: 1px solid var(--color-border-soft); border-radius: 8px; padding: 3px; }
  button { display: grid; place-items: center; width: 38px; min-height: 36px; border-radius: 5px; color: var(--color-fg-muted); background: transparent; cursor: pointer; }
  button:hover, button.chosen { background: var(--color-panel-2); color: var(--color-accent); }
  button:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; }
</style>
