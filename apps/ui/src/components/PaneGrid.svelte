<script lang="ts">
  import { GripVertical, Maximize2, Minimize2, X } from '@lucide/svelte';
  import { onDestroy, onMount, tick } from 'svelte';
  import { nativeActivationRouter } from '../lib/nativeActivation';
  import { registerPaneLayoutBarrier } from '../lib/paneLayoutBarrier';
  import PaneNodeView from './PaneNodeView.svelte';
  import TerminalPane from './TerminalPane.svelte';
  import { tabs, type Tab } from '../lib/tabs.svelte';
  import { dispatchFitAllPanes, dispatchFocusPane } from '../lib/focusPane';
  import {
    endPaneDrag,
    getPaneDragHit,
    installPaneDragGlobalHandlers,
    isPaneDragActive,
    startPointerPaneDrag,
    subscribePaneDragHit,
    type PaneDropHit,
  } from '../lib/paneDrag';
  import { i18n } from '../lib/i18n.svelte';
  import type { PaneDropSide } from '../lib/tabs.svelte';
  import type { RpcClient } from '../lib/rpc';
  import { closeSessionsInBackground } from '../lib/sessionClose';
  import { notifySessionsClosing } from '../lib/sessionLifecycle';

  /**
   * Host TerminalPane instances in a flat `{#each tab.panes (pane.id)}` list and
   * only use the recursive layout tree for chrome / measure slots.
   *
   * Why: insertRelative wraps a leaf in a new split, which used to destroy and
   * remount TerminalPane (Svelte branch swap). Native VT then called
   * terminal.detach and wiped scrollback — the "split clears old pane" bug.
   * Keep session-keyed panes mounted across layout reparenting.
   */
  interface Props {
    rpc: RpcClient;
    tab: Tab;
    settingsRev: number;
    terminalViewChoices?: import('../lib/terminalViewChoice').TerminalViewChoices;
    nativeSurfaceOccluded?: boolean;
    onOpenSftp?: () => void;
    onSplitRight?: () => void;
    onSplitDown?: () => void;
    broadcastEnabled?: boolean;
    broadcastTargetIds?: string[];
    tabVisible?: boolean;
    onError?: (msg: string) => void;
  }

  let {
    rpc,
    tab,
    settingsRev,
    terminalViewChoices = {},
    nativeSurfaceOccluded = false,
    onOpenSftp,
    onSplitRight,
    onSplitDown,
    broadcastEnabled = false,
    broadcastTargetIds = [],
    tabVisible = true,
    onError,
  }: Props = $props();

  let gridEl: HTMLDivElement | null = $state(null);
  let paneBox = $state<Record<string, { left: number; top: number; width: number; height: number }>>({});
  let syncRaf = 0;
  let slotObserver: ResizeObserver | null = null;
  let dragHit = $state<PaneDropHit | null>(null);

  function dropBandStyle(side: PaneDropSide): string {
    switch (side) {
      case 'left': return 'left: 0; top: 0; width: 34%; height: 100%;';
      case 'right': return 'right: 0; top: 0; width: 34%; height: 100%;';
      case 'up': return 'left: 0; top: 0; width: 100%; height: 34%;';
      case 'down': return 'left: 0; bottom: 0; width: 100%; height: 34%;';
    }
  }

  function syncPaneBoxes(onlyPane?: string) {
    if (!gridEl) return;
    const gridRect = gridEl.getBoundingClientRect();
    if (gridRect.width < 2 || gridRect.height < 2) return;
    // DOMRects include ancestor CSS zoom; absolute pane styles use local CSS pixels.
    const scaleX = gridRect.width / gridEl.offsetWidth;
    const scaleY = gridRect.height / gridEl.offsetHeight;
    if (!Number.isFinite(scaleX) || !Number.isFinite(scaleY) || scaleX <= 0 || scaleY <= 0) return;
    const next: Record<string, { left: number; top: number; width: number; height: number }> = onlyPane ? { ...paneBox } : {};
    for (const pane of tab.panes) {
      if (onlyPane && pane.id !== onlyPane) continue;
      const slot = gridEl.querySelector(`[data-pane-slot="${CSS.escape(pane.id)}"]`) as HTMLElement | null;
      if (!slot) continue;
      const style = getComputedStyle(slot);
      if (style.display === 'none') continue;
      const r = slot.getBoundingClientRect();
      if (r.width < 2 || r.height < 2) continue;
      next[pane.id] = {
        left: (r.left - gridRect.left) / scaleX,
        top: (r.top - gridRect.top) / scaleY,
        width: r.width / scaleX,
        height: r.height / scaleY,
      };
    }
    paneBox = next;
  }

  function scheduleSync() {
    if (syncRaf) cancelAnimationFrame(syncRaf);
    syncRaf = requestAnimationFrame(() => {
      syncRaf = 0;
      syncPaneBoxes();
    });
  }

  $effect(() => {
    if (!tabVisible) return;
    void tab.maximizedPaneId;
    void tab.activePaneId;
    const ids = tab.panes.map((p) => p.id);
    dispatchFitAllPanes(ids);
  });

  $effect(() => {
    void tab.layout;
    void tab.panes;
    void tab.maximizedPaneId;
    void tabVisible;
    void tick().then(() => {
      scheduleSync();
      requestAnimationFrame(() => scheduleSync());
    });
  });

  onMount(() => {
    installPaneDragGlobalHandlers();
    const unregisterLayout = gridEl ? registerPaneLayoutBarrier(gridEl, async source => {
      const pane = (source as HTMLElement).closest<HTMLElement>('[data-pane-drop-pane]');
      const id = pane?.dataset.paneDropPane;
      const slot = id && gridEl?.querySelector<HTMLElement>(`[data-pane-slot="${CSS.escape(id)}"]`);
      if (!pane || !id || !slot || !gridEl?.contains(pane)) throw new Error('pane layout owner unavailable');
      syncPaneBoxes(id); // Update only the requested pane, not every pane on every frame.
      await tick(); // Explicit Svelte cache -> DOM commit, not an arbitrary paint delay.
      // WebView2 may need a frame after style commit before getBoundingClientRect matches.
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      return { pane: pane.getBoundingClientRect(), slot: slot.getBoundingClientRect() };
    }) : () => {};
    const unsubHit = subscribePaneDragHit(() => {
      dragHit = getPaneDragHit();
    });
    slotObserver = new ResizeObserver(() => scheduleSync());
    if (gridEl) slotObserver.observe(gridEl);
    const onWin = () => scheduleSync();
    window.addEventListener('resize', onWin);
    scheduleSync();
    return () => {
      unregisterLayout();
      unsubHit();
      window.removeEventListener('resize', onWin);
    };
  });

  onDestroy(() => {
    if (syncRaf) cancelAnimationFrame(syncRaf);
    slotObserver?.disconnect();
    slotObserver = null;
  });

  function focusPane(sessionId: string) {
    nativeActivationRouter.invalidate();
    tabs.focusPane(tab.id, sessionId);
    requestAnimationFrame(() => dispatchFocusPane(sessionId));
  }

  function closePane(sessionId: string, ev: Event) {
    ev.stopPropagation();
    const result = tabs.removePane(tab.id, sessionId);
    if (!result) return;
    notifySessionsClosing([sessionId]);
    closeSessionsInBackground(rpc, [sessionId]);
  }

  function toggleMaximize(sessionId: string, ev: Event) {
    ev.stopPropagation();
    focusPane(sessionId);
    tabs.toggleMaximize(tab.id, sessionId);
    dispatchFitAllPanes(tab.panes.map((p) => p.id));
  }
</script>

<div
  bind:this={gridEl}
  class="absolute inset-0 min-w-0 min-h-0 overflow-hidden"
  role="presentation"
  ondragover={(e) => {
    if (isPaneDragActive()) e.preventDefault();
  }}
>
  <!-- Session-keyed terminals: survive leaf→split reparenting -->
  {#each tab.panes as pane (pane.id)}
    {@const box = paneBox[pane.id]}
    {@const focused = tab.activePaneId === pane.id}
    {@const hidden = !!tab.maximizedPaneId && tab.maximizedPaneId !== pane.id}
    <!-- Keep mounted while maximized-away so VT/xterm are not remounted (same as layoutVisible). -->
    <div
      class="group absolute z-0 min-w-0 min-h-0 overflow-hidden bg-[var(--color-bg)]
             {focused && !hidden ? 'outline outline-1 outline-[var(--color-accent)] -outline-offset-1' : ''}"
      style={box && !hidden
        ? `left:${box.left}px;top:${box.top}px;width:${box.width}px;height:${box.height}px;`
        : 'display: none;'}
      data-pane-drop-tab={tab.id}
      data-pane-drop-pane={pane.id}
      role="group"
      tabindex="-1"
      onpointerdown={() => focusPane(pane.id)}
    >
        {#if tab.panes.length > 1}
          <div
            role="button"
            tabindex="-1"
            class="pane-drag-handle absolute top-0 left-0 z-[11] flex items-center gap-0.5 px-1 py-0.5 text-[12px]
                   bg-[var(--color-panel)]/80 backdrop-blur border border-[var(--color-border-soft)]
                   cursor-grab active:cursor-grabbing select-none touch-none
                   opacity-0 pointer-events-none transition-opacity
                   group-hover:opacity-100 group-hover:pointer-events-auto
                   group-focus-within:opacity-100 group-focus-within:pointer-events-auto"
            title={i18n.t('pane.movePane')}
            aria-label={i18n.t('pane.movePane')}
            onpointerdown={(e) => {
              e.stopPropagation();
              const handle = e.currentTarget as HTMLDivElement;
              startPointerPaneDrag(tab.id, pane.id, e, handle);
            }}
            onlostpointercapture={() => endPaneDrag()}
          >
            <GripVertical size={11} />
            <span>{tabs.paneIndex(tab, pane.id) + 1}</span>
          </div>
          <button
            type="button"
            title={tab.maximizedPaneId === pane.id ? i18n.t('pane.restorePane') : i18n.t('pane.maximizePane')}
            aria-label={tab.maximizedPaneId === pane.id ? i18n.t('pane.restorePane') : i18n.t('pane.maximizePane')}
            class="btn-ghost absolute top-1 right-7 z-[11] p-1 bg-[var(--color-panel)]/85 backdrop-blur border border-[var(--color-border-soft)]
                   opacity-0 pointer-events-none transition-opacity
                   group-hover:opacity-100 group-hover:pointer-events-auto
                   group-focus-within:opacity-100 group-focus-within:pointer-events-auto"
            onclick={(e) => toggleMaximize(pane.id, e)}
            onpointerdown={(e) => e.stopPropagation()}
          >
            {#if tab.maximizedPaneId === pane.id}<Minimize2 size={12} />{:else}<Maximize2 size={12} />{/if}
          </button>
          <button
            type="button"
            title={i18n.t('pane.closePaneShortcut')}
            aria-label={i18n.t('pane.closePane')}
            class="btn-ghost absolute top-1 right-1 z-[11] p-1 bg-[var(--color-panel)]/85 backdrop-blur border border-[var(--color-border-soft)] hover:!text-[var(--color-danger)]
                   opacity-0 pointer-events-none transition-opacity
                   group-hover:opacity-100 group-hover:pointer-events-auto
                   group-focus-within:opacity-100 group-focus-within:pointer-events-auto"
            onclick={(e) => closePane(pane.id, e)}
            onpointerdown={(e) => e.stopPropagation()}
          >
            <X size={12} />
          </button>
        {/if}
        <TerminalPane
          {rpc}
          session={pane}
          active={focused && tabVisible}
          onActivate={() => { if (tabVisible && !hidden) tabs.focusPane(tab.id, pane.id); }}
          rendererOverride={terminalViewChoices[pane.id]}
          {nativeSurfaceOccluded}
          layoutVisible={!hidden}
          {tabVisible}
          {settingsRev}
          onClosePane={() => closePane(pane.id, new Event('close'))}
          {onOpenSftp}
          {onSplitRight}
          {onSplitDown}
          onMaximize={() => toggleMaximize(pane.id, new Event('click'))}
          {broadcastEnabled}
          {broadcastTargetIds}
          {onError}
        />
        {#if dragHit?.kind === 'pane' && dragHit.paneId === pane.id && !hidden}
          <div class="absolute inset-0 z-20 pointer-events-none border-2 border-[var(--color-accent)] bg-[var(--color-accent)]/10">
            <div class="absolute bg-[var(--color-accent)]/25" style={dropBandStyle(dragHit.side)}></div>
          </div>
        {/if}
    </div>
  {/each}

  <!-- Layout measure + splitters above terminals; pointer-events only on splitters -->
  <div class="absolute inset-0 z-10 min-w-0 min-h-0 pointer-events-none">
    <PaneNodeView
      {rpc}
      {tab}
      node={tab.layout}
      {settingsRev}
      {tabVisible}
      {onOpenSftp}
      {onSplitRight}
      {onSplitDown}
      {broadcastEnabled}
      {broadcastTargetIds}
      {onError}
    />
  </div>
</div>
