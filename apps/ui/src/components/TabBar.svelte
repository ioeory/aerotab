<script lang="ts">
  import { X, Terminal as TerminalIcon, Server, Usb, Columns2, Rows2, Plus, FolderOpen } from '@lucide/svelte';
  import { tabs, type SplitDir, type Tab } from '../lib/tabs.svelte';
  import { dispatchFocusPane } from '../lib/focusPane';
  import { endPaneDrag, isPaneDragActive, readPaneDragData } from '../lib/paneDrag';
  import { getWindowSettings } from '../lib/windowSettings';
  import { i18n } from '../lib/i18n.svelte';
  import { appConfirm } from '../lib/confirm.svelte';
  import type { RpcClient } from '../lib/rpc';

  interface Props {
    rpc: RpcClient;
    onAddTab?: () => void;
    onSplit?: (direction: SplitDir) => void;
    onOpenSftp?: () => void;
    onDuplicateTab?: (tab: Tab) => void;
    onCloseOthers?: (tabId: string) => void;
    onCloseToRight?: (tabIndex: number) => void;
    onCloseAll?: () => void;
  }
  let {
    rpc,
    onAddTab,
    onSplit,
    onOpenSftp,
    onDuplicateTab,
    onCloseOthers,
    onCloseToRight,
    onCloseAll,
  }: Props = $props();

  let dragIdx: number | null = $state(null);
  let menuOpen = $state(false);
  let menuX = $state(0);
  let menuY = $state(0);
  let menuTab = $state<Tab | null>(null);
  let menuTabIndex = $state(-1);
  let paneDropTabId = $state<string | null>(null);

  function iconFor(kind: string) {
    if (kind === 'Ssh') return Server;
    if (kind === 'Serial') return Usb;
    return TerminalIcon;
  }

  async function closeTab(tab: Tab, ev?: Event) {
    ev?.stopPropagation();
    const ws = getWindowSettings();
    if (ws.confirmCloseWithMultipleTabs !== false && tab.panes.length > 1) {
      if (!(await appConfirm(i18n.t('tabbar.closeMultiPaneConfirm', { count: tab.panes.length })))) return;
    }
    const pane_ids = tab.panes.map((p) => p.id);
    tabs.remove(tab.id);
    for (const id of pane_ids) {
      try { await rpc.call('session.close', { id }); } catch (e) { console.warn(e); }
    }
  }

  function showTabMenu(tab: Tab, index: number, ev: MouseEvent) {
    ev.preventDefault();
    ev.stopPropagation();
    menuTab = tab;
    menuTabIndex = index;
    menuX = ev.clientX;
    menuY = ev.clientY;
    menuOpen = true;
  }

  function closeMenu() {
    menuOpen = false;
    menuTab = null;
    menuTabIndex = -1;
  }

  function onTabHover(tab: Tab) {
    if (getWindowSettings().focusFollowsMouse) tabs.activate(tab.id);
  }

  async function splitActive(direction: 'row' | 'col', ev: Event) {
    ev.stopPropagation();
    onSplit?.(direction);
  }

  function onDragStart(idx: number, ev: DragEvent) {
    dragIdx = idx;
    if (ev.dataTransfer) ev.dataTransfer.effectAllowed = 'move';
  }
  function onDragOver(ev: DragEvent) {
    ev.preventDefault();
    if (ev.dataTransfer) ev.dataTransfer.dropEffect = 'move';
  }

  function onTabDragOver(tabId: string, idx: number, ev: DragEvent) {
    if (isPaneDragActive()) {
      ev.preventDefault();
      ev.stopPropagation();
      if (ev.dataTransfer) ev.dataTransfer.dropEffect = 'move';
      paneDropTabId = tabId;
      return;
    }
    paneDropTabId = null;
    onDragOver(ev);
  }

  function onDrop(idx: number, tabId: string, ev: DragEvent) {
    const panePayload = readPaneDragData(ev);
    if (panePayload) {
      ev.preventDefault();
      ev.stopPropagation();
      paneDropTabId = null;
      endPaneDrag();
      tabs.mergePaneIntoTab(panePayload.tabId, panePayload.paneId, tabId);
      activateTab(tabId);
      return;
    }
    ev.preventDefault();
    if (dragIdx != null) tabs.move(dragIdx, idx);
    dragIdx = null;
    paneDropTabId = null;
  }

  function activateTab(tabId: string) {
    tabs.activate(tabId);
    const tab = tabs.tabs.find((candidate) => candidate.id === tabId);
    requestAnimationFrame(() => dispatchFocusPane(tab?.activePaneId));
  }
</script>

<svelte:window onclick={closeMenu} />

<div data-aerotab-context-menu="" class="flex items-stretch gap-1 px-2 pt-2 overflow-x-auto select-none">
  {#each tabs.tabs as tab, i (tab.id)}
    {@const first = tabs.firstPane(tab)}
    {@const Icon = iconFor(first ? first.kind : 'Local')}
    {@const isActive = tabs.activeId === tab.id}
    <div
      role="tab"
      aria-selected={isActive}
      tabindex="0"
      draggable="true"
      ondragstart={(e) => onDragStart(i, e)}
      ondragenter={(e) => onTabDragOver(tab.id, i, e)}
      ondragover={(e) => onTabDragOver(tab.id, i, e)}
      ondrop={(e) => onDrop(i, tab.id, e)}
      ondragend={() => endPaneDrag()}
      class:ring-1={paneDropTabId === tab.id}
      class:ring-[var(--color-accent)]={paneDropTabId === tab.id}
      onpointerdown={(e) => {
        if (e.button === 0) activateTab(tab.id);
      }}
      oncontextmenu={(e) => showTabMenu(tab, i, e)}
      onpointerenter={() => onTabHover(tab)}
      onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') activateTab(tab.id); }}
      class="tab-shell group flex items-center gap-2 px-3 py-1.5 rounded-t-md cursor-pointer text-[12.5px] border-t border-l border-r
             {isActive
               ? 'bg-[var(--color-bg)] border-[var(--color-border)] text-[var(--color-fg)]'
               : 'bg-[var(--color-panel)] border-transparent text-[var(--color-fg-muted)] hover:text-[var(--color-fg)] hover:bg-[var(--color-panel-2)]'}
             {dragIdx === i ? 'opacity-60 ring-1 ring-[var(--color-accent)]' : ''}"
    >
      <Icon size={13} class={isActive ? 'text-[var(--color-accent)]' : ''} />
      <span class="truncate max-w-[180px]">{tab.title}</span>
      {#if tab.panes.length > 1}
        <span class="text-[10px] px-1 rounded bg-[var(--color-panel-2)] text-[var(--color-fg-muted)]">
          {tab.panes.length}
        </span>
      {/if}
      {#if !isActive}
        {@const act = tabs.tabActivity(tab)}
        {#if act === 'bell'}
          <span class="w-1.5 h-1.5 rounded-full bg-[var(--color-danger)] animate-pulse" title={i18n.t('tabbar.bell')}></span>
        {:else if act === 'output'}
          <span class="w-1.5 h-1.5 rounded-full bg-[var(--color-accent)]" title={i18n.t('tabbar.newOutput')}></span>
        {/if}
      {/if}
      <button
        type="button"
        title={i18n.t('tabbar.closeTab')}
        aria-label={i18n.t('tabbar.closeTab')}
        class="opacity-50 group-hover:opacity-100 hover:text-[var(--color-danger)] -mr-1 p-0.5"
        onclick={(e) => closeTab(tab, e)}
      >
        <X size={12} />
      </button>
    </div>
  {/each}
  {#if tabs.tabs.length === 0}
    <div class="text-[var(--color-fg-muted)] px-3 py-1.5 text-[12px] italic">
      {i18n.t('tabbar.noOpenSessions')}
    </div>
    <div class="ml-auto flex items-center gap-1 pr-1">
      <button type="button" title={i18n.t('tabbar.newTab')} aria-label={i18n.t('tabbar.newTab')}
              class="btn-ghost p-1" onclick={() => onAddTab?.()}>
        <Plus size={14} />
      </button>
    </div>
  {:else}
    <div class="ml-auto flex items-center gap-1 pr-1">
      <button type="button" title={i18n.t('tabbar.newTab')} aria-label={i18n.t('tabbar.newTab')}
              class="btn-ghost p-1" onclick={() => onAddTab?.()}>
        <Plus size={14} />
      </button>
      <button type="button" title={i18n.t('tabbar.splitRight')} aria-label={i18n.t('tabbar.splitRight')}
              class="btn-ghost p-1" onclick={(e) => splitActive('row', e)}>
        <Columns2 size={14} />
      </button>
      <button type="button" title={i18n.t('tabbar.splitDown')} aria-label={i18n.t('tabbar.splitDown')}
              class="btn-ghost p-1" onclick={(e) => splitActive('col', e)}>
        <Rows2 size={14} />
      </button>
      <button type="button" title={i18n.t('tabbar.openSftpCurrent')} aria-label={i18n.t('tabbar.openSftpCurrent')}
              class="btn-ghost p-1" onclick={() => onOpenSftp?.()}>
        <FolderOpen size={14} />
      </button>
    </div>
  {/if}
</div>

{#if menuOpen && menuTab}
  <div
    data-aerotab-context-menu=""
    class="panel fixed z-[200] min-w-[180px] py-1 text-[12px]"
    style="left: {menuX}px; top: {menuY}px;"
    role="menu"
  >
    <button type="button" class="ctx-item" role="menuitem"
            onclick={() => { const t = menuTab!; closeMenu(); void closeTab(t); }}>
      {i18n.t('tabbar.closeTab')}
    </button>
    <button type="button" class="ctx-item" role="menuitem"
            onclick={() => { const id = menuTab!.id; closeMenu(); onCloseOthers?.(id); }}>
      {i18n.t('tabbar.closeOthers')}
    </button>
    <button type="button" class="ctx-item" role="menuitem"
            onclick={() => { const idx = menuTabIndex; closeMenu(); onCloseToRight?.(idx); }}
            disabled={menuTabIndex >= tabs.tabs.length - 1}>
      {i18n.t('tabbar.closeToRight')}
    </button>
    <button type="button" class="ctx-item" role="menuitem"
            onclick={() => { closeMenu(); onCloseAll?.(); }}>
      {i18n.t('tabbar.closeAll')}
    </button>
    <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
    <button type="button" class="ctx-item" role="menuitem"
            onclick={() => { const t = menuTab!; closeMenu(); onDuplicateTab?.(t); }}>
      {i18n.t('tabbar.duplicateTab')}
    </button>
    <button type="button" class="ctx-item" role="menuitem"
            onclick={() => { const t = menuTab!; closeMenu(); activateTab(t.id); onOpenSftp?.(); }}
            disabled={!menuTab.panes.some((p) => p.kind === 'Ssh' || p.profileId || p.sshProfile)}>
      {i18n.t('tabbar.openSftpTab')}
    </button>
  </div>
{/if}

