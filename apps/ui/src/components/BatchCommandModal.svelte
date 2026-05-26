<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { X } from '@lucide/svelte';
  import { i18n } from '../lib/i18n.svelte';
  import { isSshPane } from '../lib/broadcast';
  import { scheduleModalFieldFocus } from '../lib/modalFocus';
  import type { Tab } from '../lib/tabs.svelte';

  export interface BatchPaneOption {
    sessionId: string;
    title: string;
    tabTitle: string;
  }

  interface Props {
    tabs: Tab[];
    activeTabId: string | null;
    onSend: (sessionIds: string[], command: string) => Promise<void>;
    onClose: () => void;
  }
  let { tabs: tabList, activeTabId, onSend, onClose }: Props = $props();

  let command = $state('');
  let scope = $state<'active-tab' | 'all-tabs'>('active-tab');
  let selectedIds = $state<Set<string>>(new Set());
  let busy = $state(false);
  let commandInput = $state<HTMLTextAreaElement | null>(null);

  const paneOptions = $derived.by((): BatchPaneOption[] => {
    const out: BatchPaneOption[] = [];
    for (const tab of tabList) {
      if (scope === 'active-tab' && tab.id !== activeTabId) continue;
      for (const pane of tab.panes) {
        if (!isSshPane(pane)) continue;
        out.push({ sessionId: pane.id, title: pane.title, tabTitle: tab.title });
      }
    }
    return out;
  });

  $effect(() => {
    void scope;
    void activeTabId;
    selectedIds = new Set(paneOptions.map((p) => p.sessionId));
  });

  $effect(() => {
    if (paneOptions.length === 0) {
      selectedIds = new Set();
      return;
    }
    const allowed = new Set(paneOptions.map((p) => p.sessionId));
    const next = new Set<string>();
    for (const id of selectedIds) {
      if (allowed.has(id)) next.add(id);
    }
    if (next.size !== selectedIds.size) selectedIds = next;
  });

  function focusCommandInput() {
    commandInput?.focus();
  }

  onMount(async () => {
    await tick();
    scheduleModalFieldFocus(focusCommandInput);
  });

  function togglePane(id: string) {
    const next = new Set(selectedIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selectedIds = next;
  }

  function selectAllPanes() {
    selectedIds = new Set(paneOptions.map((p) => p.sessionId));
  }

  function invertPanes() {
    const next = new Set<string>();
    for (const p of paneOptions) {
      if (!selectedIds.has(p.sessionId)) next.add(p.sessionId);
    }
    selectedIds = next;
  }

  async function sendCommand() {
    const text = command;
    if (!text.trim() || selectedIds.size === 0 || busy) return;
    busy = true;
    try {
      await onSend([...selectedIds], text.endsWith('\n') ? text : `${text}\n`);
      onClose();
    } finally {
      busy = false;
    }
  }

  async function submit(ev: Event) {
    ev.preventDefault();
    await sendCommand();
  }

  function onDialogKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onClose();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      void sendCommand();
    }
  }

  function onCommandKeydown(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
      e.preventDefault();
      void sendCommand();
    }
  }
</script>

<div
  data-aerotab-modal=""
  data-aerotab-batch-command=""
  class="fixed inset-0 z-[70] bg-black/55 grid place-items-center p-6"
  role="dialog"
  aria-modal="true"
  aria-labelledby="batch-cmd-title"
  tabindex="-1"
  onkeydown={onDialogKeydown}
>
  <form
    class="panel w-full max-w-[520px] max-h-[min(640px,90vh)] flex flex-col overflow-hidden"
    onsubmit={submit}
  >
    <header class="flex items-center gap-2 px-4 py-3 border-b border-[var(--color-border-soft)]">
      <h2 id="batch-cmd-title" class="text-[14px] font-semibold text-[var(--color-accent)]">
        {i18n.t('batchCommand.title')}
      </h2>
      <button type="button" class="btn-ghost ml-auto p-1" onclick={onClose} aria-label={i18n.t('common.close')}>
        <X size={14} />
      </button>
    </header>

    <div class="p-4 flex flex-col gap-3 overflow-y-auto min-h-0">
      <label class="block text-[11px] text-[var(--color-fg-muted)]" for="batch-cmd-input">
        {i18n.t('batchCommand.command')}
      </label>
      <textarea
        id="batch-cmd-input"
        bind:this={commandInput}
        class="input w-full min-h-[88px] font-mono text-[12px] resize-y"
        bind:value={command}
        placeholder={i18n.t('batchCommand.placeholder')}
        spellcheck="false"
        onkeydown={onCommandKeydown}
      ></textarea>
      <p class="text-[10px] text-[var(--color-fg-muted)] -mt-2">
        {i18n.t('batchCommand.sendShortcut')}
      </p>

      <fieldset class="flex flex-wrap gap-3 text-[12px]">
        <label class="inline-flex items-center gap-1.5 cursor-pointer">
          <input type="radio" name="batch-scope" value="active-tab" bind:group={scope} />
          {i18n.t('batchCommand.scopeActiveTab')}
        </label>
        <label class="inline-flex items-center gap-1.5 cursor-pointer">
          <input type="radio" name="batch-scope" value="all-tabs" bind:group={scope} />
          {i18n.t('batchCommand.scopeAllTabs')}
        </label>
      </fieldset>

      <div class="flex flex-wrap gap-2 text-[11px]">
        <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" onclick={selectAllPanes}>
          {i18n.t('profiles.selectAll')}
        </button>
        <button type="button" class="btn-secondary text-[11px] py-0.5 px-2" onclick={invertPanes}>
          {i18n.t('profiles.invertSelection')}
        </button>
        <span class="text-[var(--color-fg-muted)] self-center">
          {i18n.t('batchCommand.selectedCount', { count: selectedIds.size })}
        </span>
      </div>

      <div class="border border-[var(--color-border-soft)] rounded-md max-h-[220px] overflow-y-auto divide-y divide-[var(--color-border-soft)]">
        {#if paneOptions.length === 0}
          <div class="px-3 py-4 text-[12px] text-[var(--color-fg-muted)] italic">
            {i18n.t('batchCommand.noSshPanes')}
          </div>
        {:else}
          {#each paneOptions as opt (opt.sessionId)}
            <label class="flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--color-panel-2)] cursor-pointer text-[12px]">
              <input
                type="checkbox"
                checked={selectedIds.has(opt.sessionId)}
                onchange={() => togglePane(opt.sessionId)}
              />
              <span class="truncate text-[var(--color-fg-muted)]">{opt.tabTitle}</span>
              <span class="truncate font-mono text-[var(--color-fg)]">{opt.title}</span>
            </label>
          {/each}
        {/if}
      </div>
    </div>

    <footer class="flex justify-end gap-2 px-4 py-3 border-t border-[var(--color-border-soft)]">
      <button type="button" class="btn-secondary" onclick={onClose} disabled={busy}>
        {i18n.t('common.cancel')}
      </button>
      <button
        type="submit"
        class="btn-primary"
        disabled={busy || !command.trim() || selectedIds.size === 0}
      >
        {busy ? i18n.t('batchCommand.sending') : i18n.t('batchCommand.send')}
      </button>
    </footer>
  </form>
</div>
