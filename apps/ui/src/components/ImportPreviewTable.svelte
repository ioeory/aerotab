<script lang="ts">
  import { ChevronDown, ChevronRight } from '@lucide/svelte';
  import type { ImportCandidate } from '../lib/importTypes';
  import {
    type ImportPreviewFolder,
    type ImportPreviewRow,
    collectCandidatesInFolder,
    folderSelectionState,
    importableCandidates,
  } from '../lib/importPreviewTree';
  import { i18n } from '../lib/i18n.svelte';

  interface Props {
    rows: ImportPreviewRow[];
    collapsed: Set<string>;
    selectedIds: Set<string>;
    onToggleFolder: (path: string) => void;
    onToggleRow: (id: string, checked: boolean) => void;
    onToggleGroup: (folder: ImportPreviewFolder, checked: boolean) => void;
    statusLabel: (c: ImportCandidate) => string;
  }

  let {
    rows,
    collapsed,
    selectedIds,
    onToggleFolder,
    onToggleRow,
    onToggleGroup,
    statusLabel,
  }: Props = $props();

  function isExpanded(path: string): boolean {
    return !collapsed.has(path);
  }

  function groupCheckboxState(node: ImportPreviewFolder): boolean | 'mixed' {
    const state = folderSelectionState(node, selectedIds);
    if (state === 'partial') return 'mixed';
    return state === 'all';
  }

  function onGroupCheckboxChange(node: ImportPreviewFolder, ev: Event) {
    const input = ev.currentTarget as HTMLInputElement;
    onToggleGroup(node, input.checked);
  }
</script>

{#each rows as row (row.key)}
  {#if row.kind === 'ungrouped-header'}
    <tr class="import-group-row border-t border-[var(--color-border-soft)] bg-[var(--color-panel-2)]">
      <td colspan="5" class="px-2 py-1 text-[11px] font-medium text-[var(--color-fg-muted)]">
        {i18n.t('import.ungrouped')}
        <span class="font-normal">({row.count})</span>
      </td>
    </tr>
  {:else if row.kind === 'group'}
    {@const child = row.folder}
    {@const expanded = isExpanded(child.path)}
    {@const importableCount = importableCandidates(collectCandidatesInFolder(child)).length}
    {@const totalCount = collectCandidatesInFolder(child).length}
    <tr class="import-group-row border-t border-[var(--color-border-soft)] bg-[var(--color-panel-2)] hover:bg-[var(--color-panel)]">
      <td class="px-2 py-1 text-center" style="padding-left: {8 + row.depth * 14}px">
        {#if importableCount > 0}
          <input
            type="checkbox"
            checked={groupCheckboxState(child) === true}
            indeterminate={groupCheckboxState(child) === 'mixed'}
            onchange={(e) => onGroupCheckboxChange(child, e)}
          />
        {/if}
      </td>
      <td colspan="4" class="px-2 py-1">
        <button
          type="button"
          class="inline-flex items-center gap-1 min-w-0 max-w-full text-left text-[11px] font-medium text-[var(--color-fg)]"
          onclick={() => onToggleFolder(child.path)}
          aria-expanded={expanded}
        >
          <span class="shrink-0 w-3.5 grid place-items-center text-[var(--color-fg-muted)]">
            {#if expanded}
              <ChevronDown size={12} />
            {:else}
              <ChevronRight size={12} />
            {/if}
          </span>
          <span class="truncate">{child.name}</span>
          <span class="shrink-0 text-[var(--color-fg-muted)] font-normal">
            ({totalCount}{#if importableCount !== totalCount}, {importableCount} {i18n.t('import.importableShort')}{/if})
          </span>
        </button>
      </td>
    </tr>
  {:else}
    <tr class="border-t border-[var(--color-border-soft)] hover:bg-[var(--color-panel-2)]">
      <td class="px-2 py-1 text-center" style="padding-left: {8 + row.depth * 14}px">
        <input
          type="checkbox"
          checked={selectedIds.has(row.candidate.sourceId)}
          disabled={row.candidate.status === 'error'}
          onchange={(e) => onToggleRow(row.candidate.sourceId, e.currentTarget.checked)}
        />
      </td>
      <td class="px-2 py-1 truncate max-w-[200px]" title={row.candidate.name}>{row.candidate.name}</td>
      <td class="px-2 py-1 uppercase text-[10px]">{row.candidate.kind}</td>
      <td class="px-2 py-1 truncate max-w-[120px] text-[var(--color-fg-muted)]">{row.candidate.group ?? '—'}</td>
      <td class="px-2 py-1 text-[var(--color-fg-muted)] max-w-[220px] truncate" title={statusLabel(row.candidate)}>
        {statusLabel(row.candidate)}
      </td>
    </tr>
  {/if}
{/each}
