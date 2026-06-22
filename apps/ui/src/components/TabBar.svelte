<script lang="ts">
  import { X, Terminal as TerminalIcon, Server, Usb, Columns2, Rows2, Plus, FolderOpen, ListTree, ArrowLeftRight, PanelLeft, PanelTop } from '@lucide/svelte';
  import { tabs, type SplitDir, type Tab } from '../lib/tabs.svelte';
  import { dispatchFocusPane } from '../lib/focusPane';
  import {
    endPaneDrag,
    getPaneDragHit,
    isPaneDragActive,
    readPaneDragData,
    subscribePaneDragHit,
  } from '../lib/paneDrag';
  import { getWindowSettings } from '../lib/windowSettings';
  import { onMount, tick } from 'svelte';
  import { i18n } from '../lib/i18n.svelte';
  import { clampMenuToViewport } from '../lib/contextMenuPosition';
  import { appConfirm, appPrompt } from '../lib/confirm.svelte';
  import type { RpcClient } from '../lib/rpc';

  interface Props {
    rpc: RpcClient;
    onAddTab?: () => void;
    onAddTransferTab?: () => void;
    onSplit?: (direction: SplitDir) => void;
    onSplitLeft?: () => void;
    onSplitUp?: () => void;
    onOpenSftp?: () => void;
    onDuplicateTab?: (tab: Tab) => void;
    onCloseTab?: (tab: Tab) => void;
    onCloseOthers?: (tabId: string) => void;
    onCloseToRight?: (tabIndex: number) => void;
    onCloseAll?: () => void;
  }
  let {
    rpc,
    onAddTab,
    onAddTransferTab,
    onSplit,
    onSplitLeft,
    onSplitUp,
    onOpenSftp,
    onDuplicateTab,
    onCloseTab,
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
  let menuEl = $state<HTMLDivElement | null>(null);
  let paneDropTabId = $state<string | null>(null);
  let tabListOpen = $state(false);

  onMount(() => subscribePaneDragHit(() => {
    const hit = getPaneDragHit();
    paneDropTabId = hit?.kind === 'tab' ? hit.tabId : null;
  }));

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
    if (!tabs.tabs.some((t) => t.id === tab.id)) return;
    onCloseTab?.(tab);
  }

  async function showTabMenu(tab: Tab, index: number, ev: MouseEvent) {
    ev.preventDefault();
    ev.stopPropagation();
    activateTab(tab.id);
    menuTab = tab;
    menuTabIndex = index;
    const anchor = ev.currentTarget instanceof HTMLElement ? ev.currentTarget.getBoundingClientRect() : null;
    menuX = anchor ? Math.round(anchor.left) : ev.clientX;
    menuY = anchor ? Math.round(anchor.bottom + 2) : ev.clientY;
    menuOpen = true;
    await tick();
    const clamped = clampMenuToViewport(menuX, menuY, menuEl);
    menuX = clamped.x;
    menuY = clamped.y;
  }

  function closeMenu() {
    menuOpen = false;
    menuTab = null;
    menuTabIndex = -1;
    tabListOpen = false;
  }

  function toggleTabList(ev: MouseEvent) {
    ev.stopPropagation();
    menuOpen = false;
    tabListOpen = !tabListOpen;
  }

  function onTabHover(tab: Tab) {
    if (getWindowSettings().focusFollowsMouse) {
      tabs.activate(tab.id);
      requestAnimationFrame(() => dispatchFocusPane(tab.activePaneId));
    }
  }

  function onTabListKeydown(ev: KeyboardEvent) {
    if (ev.key === 'Escape' && tabListOpen) {
      ev.preventDefault();
      ev.stopPropagation();
      tabListOpen = false;
    }
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

  async function renameTab(tab: Tab) {
    const value = await appPrompt(i18n.t('tabbar.renameTabPrompt'), {
      defaultValue: tabs.displayTitle(tab),
      placeholder: i18n.t('tabbar.renameTabPlaceholder'),
      confirmLabel: i18n.t('common.save'),
    });
    if (value === null) return;
    tabs.setCustomTitle(tab.id, value);
  }

  function resetTabTitle(tab: Tab) {
    tabs.clearCustomTitle(tab.id);
  }
</script>

<svelte:window onclick={closeMenu} onkeydown={onTabListKeydown} />

<div data-aerotab-context-menu="" class="tabbar-shell flex items-stretch gap-1 px-2 pt-2 select-none">
  <div class="tab-strip flex items-stretch gap-1 min-w-0 overflow-x-auto">
  {#each tabs.tabs as tab, i (tab.id)}
    {@const tabChromeRev = tabs.revision}
    {@const first = tabs.firstPane(tab)}
    {@const Icon = tab.kind === 'transfer' ? ArrowLeftRight : iconFor(first ? first.kind : 'Local')}
    {@const isActive = tabs.activeId === tab.id}
    <div
      role="tab"
      aria-selected={isActive}
      tabindex="0"
      data-tab-drop={tab.id}
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
      ondblclick={(e) => { e.stopPropagation(); void renameTab(tab); }}
      onpointerenter={() => onTabHover(tab)}
      onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') activateTab(tab.id); }}
      class="tab-shell group flex items-center gap-2 px-3 py-1.5 rounded-t-md cursor-pointer text-[12.5px] border-t border-l border-r
             {isActive
               ? 'bg-[var(--color-bg)] border-[var(--color-border)] text-[var(--color-fg)]'
               : 'bg-[var(--color-panel)] border-transparent text-[var(--color-fg-muted)] hover:text-[var(--color-fg)] hover:bg-[var(--color-panel-2)]'}
             {dragIdx === i ? 'opacity-60 ring-1 ring-[var(--color-accent)]' : ''}"
    >
      <Icon size={13} class={isActive ? 'text-[var(--color-accent)]' : ''} />
      <span class="truncate max-w-[180px]" title={tabs.displayTitle(tab)}>{tabs.displayTitle(tab)}</span>
      {#if tab.kind !== 'transfer' && tab.panes.length > 1}
        <span class="text-[10px] px-1 rounded bg-[var(--color-panel-2)] text-[var(--color-fg-muted)]">
          {tab.panes.length}
        </span>
      {/if}
      {#if !isActive && tab.kind !== 'transfer'}
        {@const act = tabs.tabActivity(tab)}
        {#if act === 'bell'}
          <span class="w-1.5 h-1.5 rounded-full bg-[var(--color-danger)] animate-pulse" title={i18n.t('tabbar.bell')}></span>
        {:else if act === 'output'}
          <span class="w-1.5 h-1.5 rounded-full bg-[var(--color-accent)]" title={i18n.t('tabbar.newOutput')}></span>
        {/if}
      {/if}
      <button
        type="button"
        title={tab.kind === 'transfer' ? i18n.t('tabbar.closeTransferTab') : i18n.t('tabbar.closeTab')}
        aria-label={tab.kind === 'transfer' ? i18n.t('tabbar.closeTransferTab') : i18n.t('tabbar.closeTab')}
        class="opacity-50 group-hover:opacity-100 hover:text-[var(--color-danger)] -mr-1 p-0.5"
        onclick={(e) => closeTab(tab, e)}
      >
        <X size={12} />
      </button>
    </div>
  {/each}
  </div>
  {#if tabs.tabs.length === 0}
    <div class="text-[var(--color-fg-muted)] px-3 py-1.5 text-[12px] italic shrink-0">
      {i18n.t('tabbar.noOpenSessions')}
    </div>
  {/if}
  <button type="button" title={i18n.t('tabbar.newTab')} aria-label={i18n.t('tabbar.newTab')}
          class="tab-new-button btn-ghost p-1 shrink-0" onclick={() => onAddTab?.()}>
    <Plus size={14} />
  </button>
  <button type="button" title={i18n.t('tabbar.newTransfer')} aria-label={i18n.t('tabbar.newTransfer')}
          class="tab-new-button btn-ghost p-1 shrink-0" onclick={() => onAddTransferTab?.()}>
    <ArrowLeftRight size={14} />
  </button>
  {#if tabs.tabs.length > 0}
    <div class="tab-actions ml-auto flex items-center gap-1 pr-1 shrink-0">
      <button type="button" title={i18n.t('tabbar.tabList')} aria-label={i18n.t('tabbar.tabList')}
              class="btn-ghost p-1" onclick={toggleTabList}>
        <ListTree size={14} />
      </button>
      <button type="button" title={i18n.t('action.splitLeft')} aria-label={i18n.t('action.splitLeft')}
              class="btn-ghost p-1" onclick={(e) => { e.stopPropagation(); onSplitLeft?.(); }}>
        <PanelLeft size={14} />
      </button>
      <button type="button" title={i18n.t('tabbar.splitRight')} aria-label={i18n.t('tabbar.splitRight')}
              class="btn-ghost p-1" onclick={(e) => splitActive('row', e)}>
        <Columns2 size={14} />
      </button>
      <button type="button" title={i18n.t('action.splitUp')} aria-label={i18n.t('action.splitUp')}
              class="btn-ghost p-1" onclick={(e) => { e.stopPropagation(); onSplitUp?.(); }}>
        <PanelTop size={14} />
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

{#if tabListOpen}
  <div
    role="presentation"
    data-aerotab-menu-open=""
    class="fixed inset-0 z-[calc(var(--z-menu)-1)] bg-black/20"
    onclick={() => { tabListOpen = false; }}
  ></div>
  <div
    data-aerotab-tab-list=""
    data-aerotab-context-menu=""
    data-aerotab-menu-open=""
    class="panel fixed right-2 top-10 w-[min(360px,calc(100vw-16px))] max-h-[min(520px,calc(100vh-64px))] overflow-y-auto py-1 text-[12px]"
    style="z-index: var(--z-menu);"
    role="menu"
  >
    <div class="px-3 py-1.5 text-[10px] uppercase tracking-wide text-[var(--color-fg-muted)]">
      {i18n.t('tabbar.tabListCount', { count: tabs.tabs.length })}
    </div>
    {#each tabs.tabs as tab, i (tab.id)}
      {@const first = tabs.firstPane(tab)}
      {@const Icon = tab.kind === 'transfer' ? ArrowLeftRight : iconFor(first ? first.kind : 'Local')}
      {@const isActive = tabs.activeId === tab.id}
      {@const act = tabs.tabActivity(tab)}
      <button
        type="button"
        role="menuitem"
        class="ctx-item w-full gap-2 {isActive ? 'text-[var(--color-accent)] bg-[var(--color-panel-2)]' : ''}"
        onclick={() => { tabListOpen = false; activateTab(tab.id); }}
      >
        <Icon size={13} class="shrink-0" />
        <span class="min-w-0 flex-1 truncate text-left">{i + 1}. {tabs.displayTitle(tab)}</span>
        {#if tab.panes.length > 1}
          <span class="shrink-0 text-[10px] text-[var(--color-fg-muted)]">{tab.panes.length}</span>
        {/if}
        {#if act === 'bell'}
          <span class="shrink-0 w-1.5 h-1.5 rounded-full bg-[var(--color-danger)]"></span>
        {:else if act === 'output'}
          <span class="shrink-0 w-1.5 h-1.5 rounded-full bg-[var(--color-accent)]"></span>
        {/if}
      </button>
    {/each}
  </div>
{/if}

{#if menuOpen && menuTab}
  <div
    bind:this={menuEl}
    data-aerotab-context-menu=""
    data-aerotab-menu-open=""
    class="panel fixed z-[200] min-w-[180px] py-1 text-[12px]"
    style="left: {menuX}px; top: {menuY}px;"
    role="menu"
  >
    <button type="button" class="ctx-item" role="menuitem"
            onclick={() => { const t = menuTab!; closeMenu(); void renameTab(t); }}>
      {i18n.t('tabbar.renameTab')}
    </button>
    {#if menuTab.customTitle?.trim()}
      <button type="button" class="ctx-item" role="menuitem"
              onclick={() => { const t = menuTab!; closeMenu(); resetTabTitle(t); }}>
        {i18n.t('tabbar.resetTabTitle')}
      </button>
    {/if}
    <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
    <button type="button" class="ctx-item" role="menuitem"
            onclick={() => { const t = menuTab!; closeMenu(); void closeTab(t); }}>
      {menuTab.kind === 'transfer' ? i18n.t('tabbar.closeTransferTab') : i18n.t('tabbar.closeTab')}
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
    {#if menuTab.kind === 'terminal'}
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
    {/if}
  </div>
{/if}


<style>
  .tabbar-shell {
    min-width: 0;
  }
  .tab-strip {
    scrollbar-width: thin;
  }
  .tab-actions {
    position: sticky;
    right: 0;
    background: var(--color-panel);
    box-shadow: -10px 0 12px color-mix(in srgb, var(--color-panel) 80%, transparent);
  }
  .tab-shell {
    flex: 0 0 auto;
  }
</style>
