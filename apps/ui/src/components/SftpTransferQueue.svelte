<script lang="ts">
  import {
    CheckCircle2, CircleX, Clock3, Loader2, Pause, Play, RotateCw, Trash2, X,
  } from '@lucide/svelte';
  import { i18n } from '../lib/i18n.svelte';

  export type TransferQueueStatus = 'queued' | 'running' | 'paused' | 'done' | 'error' | 'canceled';

  export interface TransferQueueItem {
    id: string;
    name: string;
    status: TransferQueueStatus;
    percent: number;
    summary: string;
    kindLabel?: string;
    directionLabel?: string;
    modeLabel?: string;
    routeLabel?: string;
  }

  interface Props {
    variant?: 'compact' | 'full';
    queueView?: 'active' | 'history';
    tasks: TransferQueueItem[];
    selectedIds?: Set<string>;
    showToolbar?: boolean;
    maxHeight?: string;
    onSelectAll?: () => void;
    onInvertSelection?: () => void;
    onClearSelection?: () => void;
    onRemoveSelected?: () => void;
    onCancelAll?: () => void;
    onClearFinished?: () => void;
    onClearAll?: () => void;
    onRetryFailed?: () => void;
    onClearHistory?: () => void;
    onToggleSelect?: (id: string) => void;
    onPause?: (id: string) => void;
    onResume?: (id: string) => void;
    onCancel?: (id: string) => void;
    onRetry?: (id: string) => void;
    onDelete?: (id: string) => void;
    onOpenTransferCenter?: () => void;
  }

  let {
    variant = 'compact',
    queueView = 'active',
    tasks,
    selectedIds = new Set<string>(),
    showToolbar = true,
    maxHeight,
    onSelectAll,
    onInvertSelection,
    onClearSelection,
    onRemoveSelected,
    onCancelAll,
    onClearFinished,
    onClearAll,
    onRetryFailed,
    onClearHistory,
    onToggleSelect,
    onPause,
    onResume,
    onCancel,
    onRetry,
    onDelete,
    onOpenTransferCenter,
  }: Props = $props();

  function statusIcon(status: TransferQueueStatus) {
    if (status === 'queued') return Clock3;
    if (status === 'paused') return Pause;
    if (status === 'running') return Loader2;
    if (status === 'done') return CheckCircle2;
    return CircleX;
  }

  function statusClass(status: TransferQueueStatus): string {
    if (status === 'running') return 'text-[var(--color-accent)] animate-spin';
    if (status === 'paused') return 'text-[var(--color-warning)]';
    if (status === 'done') return 'text-[var(--color-success)]';
    if (status === 'error' || status === 'canceled') return 'text-[var(--color-danger)]';
    return 'text-[var(--color-fg-muted)]';
  }

  const isActive = $derived(queueView === 'active');
</script>

<div
  class="sftp-transfer-queue border-t border-[var(--color-border-soft)] bg-[var(--color-panel)] flex flex-col min-h-0"
  style:max-height={maxHeight}
>
  {#if showToolbar}
    <div class="sticky top-0 z-10 flex flex-wrap items-center gap-2 px-3 py-1.5 bg-[var(--color-panel)] border-b border-[var(--color-border-soft)] text-[11px]">
      <span class="uppercase tracking-[0.12em] text-[var(--color-fg-muted)]">
        {isActive ? i18n.t('transfer.queueActive') : i18n.t('transfer.history')}
      </span>
      <span class="text-[var(--color-fg-muted)]">{tasks.length}</span>
      {#if isActive && variant === 'compact'}
        <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={onSelectAll}>{i18n.t('sftp.transferSelectAll')}</button>
        <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={onInvertSelection}>{i18n.t('sftp.transferInvert')}</button>
        {#if selectedIds.size > 0}
          <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={onClearSelection}>{i18n.t('sftp.transferClearSelection')}</button>
          <button type="button" class="text-[var(--color-danger)] hover:underline" onclick={onRemoveSelected}>{i18n.t('sftp.transferRemoveSelected')}</button>
        {/if}
        <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={onCancelAll}>{i18n.t('sftp.cancelAll')}</button>
        <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={onClearFinished}>{i18n.t('common.clearFinished')}</button>
        {#if onOpenTransferCenter}
          <button type="button" class="text-[var(--color-accent)] hover:underline" onclick={onOpenTransferCenter}>{i18n.t('transfer.openCenter')}</button>
        {/if}
        <button type="button" class="ml-auto text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={onClearAll}>{i18n.t('sftp.transferClearAll')}</button>
      {:else if isActive}
        <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={onCancelAll}>{i18n.t('sftp.cancelAll')}</button>
        <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={onClearFinished}>{i18n.t('transfer.clearCompleted')}</button>
        {#if onRetryFailed}
          <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={onRetryFailed}>{i18n.t('transfer.retryFailed')}</button>
        {/if}
      {:else}
        {#if onRetryFailed}
          <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={onRetryFailed}>{i18n.t('transfer.retryFailed')}</button>
        {/if}
        {#if onClearHistory}
          <button type="button" class="ml-auto text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={onClearHistory}>{i18n.t('transfer.clearHistory')}</button>
        {/if}
      {/if}
    </div>
  {/if}

  <div class="flex-1 min-h-0 overflow-y-auto divide-y divide-[var(--color-border-soft)]">
    {#if tasks.length === 0}
      <div class="px-4 py-6 text-[12px] text-[var(--color-fg-muted)] text-center">{i18n.t('transfer.queueEmpty')}</div>
    {:else}
      {#each tasks as task (task.id)}
        {@const Icon = statusIcon(task.status)}
        <div class="px-3 py-2 text-[11.5px] {selectedIds.has(task.id) ? 'bg-[var(--color-panel-2)]' : ''}">
          <div class="flex items-center gap-2 min-w-0">
            {#if onToggleSelect}
              <input
                type="checkbox"
                class="shrink-0"
                checked={selectedIds.has(task.id)}
                onchange={() => onToggleSelect?.(task.id)}
                aria-label={i18n.t('transfer.selectTask', { name: task.name })}
              />
            {/if}
            <Icon size={13} class={`shrink-0 ${statusClass(task.status)}`} />
            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-2 min-w-0">
                {#if task.directionLabel}
                  <span class="uppercase text-[9.5px] text-[var(--color-fg-muted)] shrink-0">{task.directionLabel}</span>
                {/if}
                {#if task.modeLabel}
                  <span class="uppercase text-[9.5px] text-[var(--color-accent)] shrink-0">{task.modeLabel}</span>
                {/if}
                {#if task.kindLabel}
                  <span class="uppercase text-[9.5px] text-[var(--color-fg-muted)] shrink-0">{task.kindLabel}</span>
                {/if}
                <span class="truncate text-[var(--color-fg)]">{task.name}</span>
                <span class="ml-auto text-[10.5px] text-[var(--color-fg-muted)] shrink-0">{task.percent}%</span>
              </div>
              {#if task.routeLabel}
                <div class="mt-0.5 truncate text-[10px] font-mono text-[var(--color-fg-muted)]">{task.routeLabel}</div>
              {/if}
              <div
                class="mt-1 h-1 rounded bg-[var(--color-panel-2)] overflow-hidden"
                role="progressbar"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={task.percent}
                aria-label={task.name}
              >
                <div class="h-full bg-[var(--color-accent)] transition-[width] duration-150" style="width: {task.percent}%"></div>
              </div>
              <div class="mt-1 truncate text-[10.5px] text-[var(--color-fg-muted)]">{task.summary}</div>
            </div>
            {#if isActive && (task.status === 'queued' || task.status === 'running' || task.status === 'paused')}
              {#if task.status === 'paused'}
                <button type="button" class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-accent)]" title={i18n.t('sftp.resumeTransfer')} aria-label={i18n.t('sftp.resumeTransfer')} onclick={() => onResume?.(task.id)}><Play size={12} /></button>
              {:else}
                <button type="button" class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-warning)]" title={i18n.t('sftp.pauseTransfer')} aria-label={i18n.t('sftp.pauseTransfer')} onclick={() => onPause?.(task.id)}><Pause size={12} /></button>
              {/if}
              <button type="button" class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-danger)]" title={i18n.t('sftp.cancelTransfer')} aria-label={i18n.t('sftp.cancelTransfer')} onclick={() => onCancel?.(task.id)}><X size={12} /></button>
            {:else if !isActive && (task.status === 'error' || task.status === 'canceled')}
              <button type="button" class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-accent)]" title={i18n.t('transfer.retry')} aria-label={i18n.t('transfer.retry')} onclick={() => onRetry?.(task.id)}><RotateCw size={12} /></button>
              <button type="button" class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-danger)]" title={i18n.t('common.delete')} aria-label={i18n.t('common.delete')} onclick={() => onDelete?.(task.id)}><Trash2 size={12} /></button>
            {/if}
          </div>
        </div>
      {/each}
    {/if}
  </div>
</div>
