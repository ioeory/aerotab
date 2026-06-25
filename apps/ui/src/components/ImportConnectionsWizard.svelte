<script lang="ts">
  import { X, FolderOpen, ChevronLeft, Loader2 } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import { tauriInvoke } from '../lib/rpc';
  import { i18n } from '../lib/i18n.svelte';
  import { portal } from '../lib/portal';
  import {
    IMPORT_SOURCE_CARDS,
    type ImportCandidate,
    type ImportDetectPath,
    type ImportPreviewResult,
    type ImportSourceKind,
  } from '../lib/importTypes';
  import {
    buildImportPreviewTree,
    collectFolderPaths,
    collectCandidatesInFolder,
    flattenVisibleImportRows,
    importableCandidates,
    invertImportSelection,
    type ImportPreviewFolder,
  } from '../lib/importPreviewTree';
  import ImportPreviewTable from './ImportPreviewTable.svelte';
  import ImportAuthBatchPanel from './ImportAuthBatchPanel.svelte';
  import { notifyProfilesChanged } from '../lib/profileEvents';
  import type { StoredProfile } from '../lib/types';
  import {
    applyBatchAuthToCandidates,
    buildImportApplyItems,
    matchAuthFromExistingProfiles,
    type ImportBatchAuthConfig,
  } from '../lib/importAuth';

  interface Props {
    rpc: RpcClient;
    open: boolean;
    onClose: () => void;
    onError: (msg: string) => void;
    onSummary?: (msg: string) => void;
  }
  let { rpc, open, onClose, onError, onSummary }: Props = $props();

  type Step = 'source' | 'file' | 'preview';

  let step = $state<Step>('source');
  let source = $state<ImportSourceKind>('windterm');
  let detectPaths = $state<ImportDetectPath[]>([]);
  let selectedPath = $state<string | null>(null);
  let preview = $state<ImportPreviewResult | null>(null);
  let loading = $state(false);
  let applying = $state(false);
  let selectedIds = $state<Set<string>>(new Set());
  let previewTree = $state<ImportPreviewFolder | null>(null);
  let collapsedGroups = $state<Set<string>>(new Set());
  let browseBusy = $state(false);
  let existingProfiles = $state<StoredProfile[]>([]);

  $effect(() => {
    if (!open) {
      step = 'source';
      source = 'windterm';
      detectPaths = [];
      selectedPath = null;
      preview = null;
      loading = false;
      applying = false;
      selectedIds = new Set();
      previewTree = null;
      collapsedGroups = new Set();
      browseBusy = false;
      existingProfiles = [];
    }
  });

  function defaultSelected(candidates: ImportCandidate[]): Set<string> {
    return new Set(
      candidates
        .filter((c) => c.status === 'ready')
        .map((c) => c.sourceId),
    );
  }

  function fileStepHint(): string {
    if (source === 'windterm') return i18n.t('import.fileStepHint.windterm');
    if (source === 'termius') return i18n.t('import.fileStepHint.termius');
    if (source === 'ssh-config') return i18n.t('import.fileStepHint.sshConfig');
    if (source === 'csv') return i18n.t('import.fileStepHint.csv');
    if (source === 'putty') return i18n.t('import.fileStepHint.putty');
    if (source === 'mobaxterm') return i18n.t('import.fileStepHint.mobaxterm');
    if (source === 'xshell') return i18n.t('import.fileStepHint.xshell');
    if (source === 'securecrt') return i18n.t('import.fileStepHint.securecrt');
    if (source === 'tabby') return i18n.t('import.fileStepHint.tabby');
    return i18n.t('import.fileStepHint.windterm');
  }

  function noAutoDetectHint(): string {
    if (source === 'ssh-config') return i18n.t('import.noAutoDetect.sshConfig');
    if (source === 'csv') return i18n.t('import.noAutoDetect.csv');
    if (source === 'putty') return i18n.t('import.noAutoDetect.putty');
    if (source === 'mobaxterm') return i18n.t('import.noAutoDetect.mobaxterm');
    if (source === 'xshell') return i18n.t('import.noAutoDetect.xshell');
    if (source === 'securecrt') return i18n.t('import.noAutoDetect.securecrt');
    if (source === 'tabby') return i18n.t('import.noAutoDetect.tabby');
    if (source === 'termius') return i18n.t('import.noAutoDetect.termius');
    return i18n.t('import.noAutoDetect');
  }

  function browseUsesDirectory(): boolean {
    return source === 'xshell';
  }

  function canPreview(): boolean {
    return Boolean(selectedPath);
  }

  async function pickSource(id: ImportSourceKind, enabled: boolean) {
    if (!enabled) return;
    source = id;
    step = 'file';
    loading = true;
    try {
      const r = await rpc.call<{ paths: ImportDetectPath[] }>('profile.importDetect', { source: id });
      detectPaths = r.paths ?? [];
      selectedPath = detectPaths[0]?.path ?? null;
    } catch (e) {
      onError(`import detect: ${(e as Error).message}`);
      detectPaths = [];
    } finally {
      loading = false;
    }
  }

  async function browseFile(asDirectory = browseUsesDirectory()) {
    if (browseBusy) return;
    browseBusy = true;
    try {
      const paths = await tauriInvoke<string[] | null>('pick_open_files', { directory: asDirectory });
      const path = paths?.[0];
      if (path) selectedPath = path;
    } catch (e) {
      onError(`import browse: ${(e as Error).message}`);
    } finally {
      browseBusy = false;
    }
  }

  async function runPreview() {
    if (!selectedPath && detectPaths.length === 0) {
      onError(i18n.t('import.noFileSelected'));
      return;
    }
    loading = true;
    try {
      const r = await rpc.call<ImportPreviewResult>('profile.importPreview', {
        source,
        path: selectedPath ?? undefined,
      });
      if (!r.candidates?.length) {
        onError(i18n.t('import.previewEmpty'));
        return;
      }
      preview = r;
      previewTree = buildImportPreviewTree(r.candidates);
      collapsedGroups = new Set(collectFolderPaths(previewTree));
      selectedIds = defaultSelected(r.candidates);
      try {
        existingProfiles = await rpc.call<StoredProfile[]>('profile.list');
      } catch {
        existingProfiles = [];
      }
      step = 'preview';
    } catch (e) {
      onError(`import preview: ${(e as Error).message}`);
    } finally {
      loading = false;
    }
  }

  function toggleRow(id: string, checked: boolean) {
    const next = new Set(selectedIds);
    if (checked) next.add(id);
    else next.delete(id);
    selectedIds = next;
  }

  function toggleAllImportable() {
    if (!preview) return;
    const importable = importableCandidates(preview.candidates);
    const allSelected = importable.every((c) => selectedIds.has(c.sourceId));
    if (allSelected) {
      selectedIds = new Set();
    } else {
      selectedIds = new Set(importable.map((c) => c.sourceId));
    }
  }

  function invertSelection() {
    if (!preview) return;
    selectedIds = invertImportSelection(selectedIds, preview.candidates);
  }

  function toggleFolder(path: string) {
    const next = new Set(collapsedGroups);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    collapsedGroups = next;
  }

  function expandAllGroups() {
    collapsedGroups = new Set();
  }

  function collapseAllGroups() {
    if (!previewTree) return;
    collapsedGroups = new Set(collectFolderPaths(previewTree));
  }

  function toggleGroup(folder: ImportPreviewFolder, checked: boolean) {
    const next = new Set(selectedIds);
    for (const c of importableCandidates(collectCandidatesInFolder(folder))) {
      if (checked) next.add(c.sourceId);
      else next.delete(c.sourceId);
    }
    selectedIds = next;
  }

  function statusLabel(c: ImportCandidate): string {
    if (c.status === 'ready') return i18n.t('import.statusReady');
    if (c.status === 'duplicate') return i18n.t('import.statusDuplicate');
    return c.errorMessage ?? i18n.t('import.statusError');
  }

  let previewRows = $derived(
    previewTree
      ? flattenVisibleImportRows(
          previewTree,
          collapsedGroups,
          previewTree.candidates.length > 0 && previewTree.folders.length > 0,
        )
      : [],
  );

  function handleBatchAuth(config: ImportBatchAuthConfig) {
    if (!preview) return;
    const count = applyBatchAuthToCandidates(preview.candidates, selectedIds, config);
    preview = { ...preview, candidates: [...preview.candidates] };
    onSummary?.(i18n.t('import.batchAuth.applied', { count }));
  }

  function handleMatchExisting() {
    if (!preview) return;
    const { matched, unmatched } = matchAuthFromExistingProfiles(
      preview.candidates,
      selectedIds,
      existingProfiles,
    );
    preview = { ...preview, candidates: [...preview.candidates] };
    onSummary?.(i18n.t('import.batchAuth.matchResult', { matched, unmatched }));
  }

  async function applyImport() {
    if (!preview || selectedIds.size === 0) return;
    applying = true;
    try {
      const items = buildImportApplyItems(preview.candidates, selectedIds);
      const r = await rpc.call<{ created: number; skipped: number; updated: number; errors: string[] }>(
        'profile.importApply',
        { source, path: preview.path ?? selectedPath ?? undefined, items },
      );
      notifyProfilesChanged();
      const msg = i18n.t('import.applyDone', {
        created: r.created,
        skipped: r.skipped,
        updated: r.updated,
      });
      if (r.created === 0 && r.updated === 0) {
        onError(
          r.errors.length > 0
            ? `${msg} ${r.errors.join('; ')}`
            : i18n.t('import.applyNothing', { skipped: r.skipped }),
        );
        return;
      }
      onSummary?.(msg);
      if (r.errors.length > 0) {
        onError(r.errors.join('; '));
      }
      onClose();
    } catch (e) {
      onError(`import apply: ${(e as Error).message}`);
    } finally {
      applying = false;
    }
  }
</script>

{#if open}
  <div use:portal class="contents">
    <div
      role="presentation"
      class="fixed inset-0 z-[70] bg-black/55"
      data-aerotab-modal=""
      onclick={onClose}
    ></div>
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="import-wizard-title"
      class="panel fixed z-[71] left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-[min(920px,calc(100vw-32px))] max-h-[min(720px,calc(100vh-48px))] flex flex-col overflow-hidden"
      onclick={(e) => e.stopPropagation()}
    >
      <header class="flex items-center gap-2 px-4 py-3 border-b border-[var(--color-border-soft)]">
        {#if step !== 'source'}
          <button type="button" class="btn-ghost p-1" aria-label={i18n.t('common.back')} onclick={() => {
            if (step === 'preview') step = 'file';
            else step = 'source';
          }}>
            <ChevronLeft size={16} />
          </button>
        {/if}
        <h2 id="import-wizard-title" class="text-[14px] font-semibold flex-1">{i18n.t('import.title')}</h2>
        <button type="button" class="btn-ghost p-1" aria-label={i18n.t('common.close')} onclick={onClose}>
          <X size={16} />
        </button>
      </header>

      <div class="flex-1 min-h-0 overflow-y-auto p-4">
        {#if step === 'source'}
          <p class="text-[12px] text-[var(--color-fg-muted)] mb-4">{i18n.t('import.subtitle')}</p>
          <div class="grid grid-cols-2 sm:grid-cols-4 gap-3">
            {#each IMPORT_SOURCE_CARDS as card (card.id)}
              <button
                type="button"
                class="import-source-card text-left p-3 rounded-lg border {card.enabled
                  ? 'border-[var(--color-border)] hover:border-[var(--color-accent)] cursor-pointer'
                  : 'border-[var(--color-border-soft)] opacity-50 cursor-not-allowed'}"
                disabled={!card.enabled}
                onclick={() => { void pickSource(card.id, card.enabled); }}
              >
                <div class="text-[12px] font-semibold text-[var(--color-fg)]">{i18n.t(card.titleKey)}</div>
                <div class="text-[10.5px] text-[var(--color-fg-muted)] mt-1 line-clamp-2">{i18n.t(card.descKey)}</div>
                {#if !card.enabled}
                  <div class="text-[10px] text-[var(--color-accent)] mt-2">{i18n.t('import.comingSoon')}</div>
                {/if}
              </button>
            {/each}
          </div>
        {:else if step === 'file'}
          <p class="text-[12px] text-[var(--color-fg-muted)] mb-3">{fileStepHint()}</p>
          {#if loading}
            <div class="flex items-center gap-2 text-[12px] text-[var(--color-fg-muted)]">
              <Loader2 size={14} class="animate-spin" />
              {i18n.t('common.loading')}
            </div>
          {:else}
            {#if detectPaths.length > 0}
              <div class="space-y-1 mb-4">
                {#each detectPaths as p (p.path)}
                  <label class="flex items-start gap-2 p-2 rounded border border-[var(--color-border-soft)] cursor-pointer hover:bg-[var(--color-panel-2)]">
                    <input type="radio" bind:group={selectedPath} value={p.path} class="mt-1" />
                    <span class="min-w-0">
                      <span class="block text-[12px] font-mono truncate">{p.path}</span>
                      <span class="block text-[10.5px] text-[var(--color-fg-muted)]">{p.label}</span>
                    </span>
                  </label>
                {/each}
              </div>
            {:else}
              <p class="text-[12px] text-[var(--color-fg-muted)] mb-3">{noAutoDetectHint()}</p>
            {/if}
            <button type="button" class="btn-secondary text-[12px] px-3 py-1.5 inline-flex items-center gap-1.5" onclick={() => { void browseFile(); }}>
              <FolderOpen size={14} />
              {source === 'xshell' ? i18n.t('import.browseFolder') : i18n.t('import.browseFile')}
            </button>
            {#if source === 'xshell'}
              <button type="button" class="btn-ghost text-[12px] px-3 py-1.5 ml-2" onclick={() => { void browseFile(false); }}>
                {i18n.t('import.browseSingleFile')}
              </button>
            {/if}
            {#if selectedPath && !detectPaths.some((p) => p.path === selectedPath)}
              <p class="text-[11px] font-mono text-[var(--color-fg-muted)] mt-2 truncate" title={selectedPath}>{selectedPath}</p>
            {/if}
          {/if}
        {:else if preview && previewTree}
          <div class="flex flex-wrap items-center gap-2 mb-3 text-[11px] text-[var(--color-fg-muted)]">
            <span>{i18n.t('import.statsTotal', { count: preview.stats.total })}</span>
            <span class="text-[var(--color-success)]">{i18n.t('import.statsReady', { count: preview.stats.ready })}</span>
            <span>{i18n.t('import.statsDuplicate', { count: preview.stats.duplicate })}</span>
            <span class="text-[var(--color-danger)]">{i18n.t('import.statsError', { count: preview.stats.error })}</span>
            <span class="w-px h-3 bg-[var(--color-border-soft)]" aria-hidden="true"></span>
            <button type="button" class="btn-ghost text-[11px] px-2 py-0.5" onclick={toggleAllImportable}>
              {i18n.t('profiles.selectAll')}
            </button>
            <button type="button" class="btn-ghost text-[11px] px-2 py-0.5" onclick={invertSelection}>
              {i18n.t('profiles.invertSelection')}
            </button>
            <button type="button" class="btn-ghost text-[11px] px-2 py-0.5" onclick={expandAllGroups}>
              {i18n.t('import.expandAll')}
            </button>
            <button type="button" class="btn-ghost text-[11px] px-2 py-0.5" onclick={collapseAllGroups}>
              {i18n.t('import.collapseAll')}
            </button>
          </div>
          <ImportAuthBatchPanel
            {rpc}
            selectedCount={selectedIds.size}
            disabled={applying}
            onApply={handleBatchAuth}
            onMatchExisting={handleMatchExisting}
            {onError}
            {onSummary}
          />
          <div class="border border-[var(--color-border-soft)] rounded overflow-hidden max-h-[min(480px,50vh)] overflow-y-auto">
            <table class="w-full text-[11.5px]">
              <thead class="bg-[var(--color-panel-2)] text-[var(--color-fg-muted)] sticky top-0 z-[1]">
                <tr>
                  <th class="w-8 px-2 py-1.5"></th>
                  <th class="text-left px-2 py-1.5 font-normal">{i18n.t('import.colName')}</th>
                  <th class="text-left px-2 py-1.5 font-normal">{i18n.t('import.colKind')}</th>
                  <th class="text-left px-2 py-1.5 font-normal">{i18n.t('import.colGroup')}</th>
                  <th class="text-left px-2 py-1.5 font-normal">{i18n.t('import.colStatus')}</th>
                </tr>
              </thead>
              <tbody>
                <ImportPreviewTable
                  rows={previewRows}
                  collapsed={collapsedGroups}
                  {selectedIds}
                  onToggleFolder={toggleFolder}
                  onToggleRow={toggleRow}
                  onToggleGroup={toggleGroup}
                  statusLabel={statusLabel}
                />
              </tbody>
            </table>
          </div>
          <p class="text-[10.5px] text-[var(--color-fg-muted)] mt-2">{i18n.t('import.duplicateHint')}</p>
        {/if}
      </div>

      <footer class="flex justify-end gap-2 px-4 py-3 border-t border-[var(--color-border-soft)]">
        <button type="button" class="btn-secondary text-[12px] px-3 py-1.5" onclick={onClose}>
          {i18n.t('common.cancel')}
        </button>
        {#if step === 'file'}
          <button
            type="button"
            class="btn-secondary text-[12px] px-3 py-1.5 text-[var(--color-accent)]"
            disabled={loading || !canPreview()}
            onclick={() => { void runPreview(); }}
          >
            {i18n.t('import.preview')}
          </button>
        {:else if step === 'preview'}
          <button
            type="button"
            class="btn-secondary text-[12px] px-3 py-1.5 text-[var(--color-accent)]"
            disabled={applying || selectedIds.size === 0}
            onclick={() => { void applyImport(); }}
          >
            {#if applying}
              <Loader2 size={14} class="inline animate-spin mr-1" />
            {/if}
            {i18n.t('import.apply', { count: selectedIds.size })}
          </button>
        {/if}
      </footer>
    </div>
  </div>
{/if}

<style>
  .import-source-card {
    background: var(--color-panel-2);
  }
</style>
