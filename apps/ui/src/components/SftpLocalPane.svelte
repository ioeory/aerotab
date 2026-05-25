<script lang="ts">
  import { Folder, FileText, RefreshCw, ChevronRight, Home, ArrowUp } from '@lucide/svelte';
  import type { LocalEntry } from '../lib/types';
  import { i18n } from '../lib/i18n.svelte';
  import {
    SFTP_DRAG_LOCAL,
    joinLocalPath,
    localBreadcrumbs,
    setSftpDragData,
    type LocalDragPayload,
  } from '../lib/sftpLocal';

  interface Props {
    cwd: string;
    entries: LocalEntry[];
    loading: boolean;
    listError: string | null;
    onRefresh: () => void;
    onNavigate: (path: string) => void;
    onGoUp: () => void;
    onGoHome: () => void;
    onDropRemote: (e: DragEvent) => void;
    onDropFiles: (e: DragEvent) => void;
    onDragOverPane: (e: DragEvent) => void;
  }
  let {
    cwd,
    entries,
    loading,
    listError,
    onRefresh,
    onNavigate,
    onGoUp,
    onGoHome,
    onDropRemote,
    onDropFiles,
    onDragOverPane,
  }: Props = $props();

  const crumbs = $derived(localBreadcrumbs(cwd));

  function formatSize(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
    return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }

  function dragPayload(entry: LocalEntry, path: string): LocalDragPayload {
    return { path, name: entry.name, kind: entry.kind, size: entry.size };
  }

  function onDragStartLocal(e: DragEvent, entry: LocalEntry, path: string) {
    setSftpDragData(e.dataTransfer, SFTP_DRAG_LOCAL, JSON.stringify(dragPayload(entry, path)));
  }
</script>

<div
  class="flex flex-col min-w-0 min-h-0 h-full border-r border-[var(--color-border-soft)]"
  role="region"
  aria-label={i18n.t('sftp.localPane')}
  ondragover={onDragOverPane}
  ondrop={(e) => {
    if (e.dataTransfer?.types.includes(SFTP_DRAG_LOCAL)) return;
    onDropRemote(e);
    onDropFiles(e);
  }}
>
  <div class="px-2 py-1 shell-section-title border-b border-[var(--color-border-soft)]">
    {i18n.t('sftp.localPane')}
  </div>
  <div class="flex items-center gap-1 px-2 py-1 border-b border-[var(--color-border-soft)] text-[12px]">
    <button type="button" class="toolbtn" onclick={onGoHome} title={i18n.t('common.home')}><Home size={13} /></button>
    <button type="button" class="toolbtn" onclick={onGoUp} title={i18n.t('common.up')}><ArrowUp size={13} /></button>
    <button type="button" class="toolbtn" onclick={onRefresh} title={i18n.t('common.refresh')}><RefreshCw size={13} /></button>
    <div class="mx-1 flex items-center gap-0.5 flex-wrap text-[11px] text-[var(--color-fg-muted)] min-w-0">
      {#each crumbs as bc, i (bc.path)}
        {#if i > 0}<ChevronRight size={11} class="text-[var(--color-border)]" />{/if}
        <button type="button" class="hover:text-[var(--color-accent)] px-0.5 truncate max-w-[80px]" onclick={() => onNavigate(bc.path)}>
          {bc.label}
        </button>
      {/each}
    </div>
  </div>

  {#if listError}
    <div class="mx-2 mt-1 px-2 py-1 text-[11px] text-[var(--color-danger)] truncate">{listError}</div>
  {/if}

  <div class="flex-1 min-h-0 overflow-y-auto">
    {#if loading && entries.length === 0 && !listError}
      <div class="px-3 py-4 text-[12px] text-[var(--color-fg-muted)]">{i18n.t('common.loading')}</div>
    {:else if entries.length === 0 && !listError}
      <div class="px-3 py-4 text-[12px] text-[var(--color-fg-muted)] italic">{i18n.t('sftp.emptyDirectory')}</div>
    {:else}
      <table class="w-full text-[12px]">
        <thead class="sticky top-0 bg-[var(--color-panel)] text-[10px] uppercase tracking-[0.12em] text-[var(--color-fg-muted)]">
          <tr>
            <th class="text-left px-2 py-1 font-normal">{i18n.t('sftp.name')}</th>
            <th class="text-right px-2 py-1 font-normal w-[72px]">{i18n.t('sftp.size')}</th>
          </tr>
        </thead>
        <tbody
          ondragover={onDragOverPane}
          ondrop={(e) => {
            e.stopPropagation();
            if (e.dataTransfer?.types.includes(SFTP_DRAG_LOCAL)) return;
            onDropRemote(e);
            onDropFiles(e);
          }}
        >
          {#each entries as e (e.name)}
            {@const fullPath = joinLocalPath(cwd, e.name)}
            <tr
              class="hover:bg-[var(--color-panel-2)]"
              draggable={e.kind === 'file' || e.kind === 'dir'}
              ondragstart={(ev) => onDragStartLocal(ev, e, fullPath)}
            >
              <td class="px-2 py-0.5 truncate">
                <button
                  type="button"
                  class="flex items-center gap-1.5 w-full text-left"
                  ondblclick={() => e.kind === 'dir' && onNavigate(fullPath)}
                  onclick={() => e.kind === 'dir' && onNavigate(fullPath)}
                >
                  {#if e.kind === 'dir'}
                    <Folder size={12} class="text-[var(--color-accent)] shrink-0" />
                  {:else}
                    <FileText size={12} class="text-[var(--color-fg-muted)] shrink-0" />
                  {/if}
                  <span class="truncate">{e.name}</span>
                </button>
              </td>
              <td class="px-2 py-0.5 text-right text-[var(--color-fg-muted)]">
                {e.kind === 'file' ? formatSize(e.size) : ''}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
</div>

