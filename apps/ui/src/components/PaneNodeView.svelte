<script lang="ts">
  import { GripVertical, Maximize2, Minimize2, X } from '@lucide/svelte';
  import PaneNodeView from './PaneNodeView.svelte';
  import TerminalPane from './TerminalPane.svelte';
  import { tabs, type PaneDropSide, type PaneNode, type Tab } from '../lib/tabs.svelte';
  import { dispatchFitAllPanes, dispatchFocusPane } from '../lib/focusPane';
  import {
    endPaneDrag,
    getPaneDragHit,
    isPaneDragActive,
    startPointerPaneDrag,
    subscribePaneDragHit,
    type PaneDropHit,
  } from '../lib/paneDrag';
  import { onMount } from 'svelte';
  import { i18n } from '../lib/i18n.svelte';
  import type { RpcClient } from '../lib/rpc';
  import { closeSessionsInBackground } from '../lib/sessionClose';

  interface Props {
    rpc: RpcClient;
    tab: Tab;
    node: PaneNode;
    settingsRev: number;
    onOpenSftp?: () => void;
    onSplitRight?: () => void;
    onSplitDown?: () => void;
    broadcastEnabled?: boolean;
    broadcastTargetIds?: string[];
    tabVisible?: boolean;
  }

  let {
    rpc,
    tab,
    node,
    settingsRev,
    onOpenSftp,
    onSplitRight,
    onSplitDown,
    broadcastEnabled = false,
    broadcastTargetIds = [],
    tabVisible = true,
  }: Props = $props();
  let host: HTMLDivElement | null = $state(null);
  let dragHandle: HTMLDivElement | null = $state(null);
  let dragging: { idx: number; startPx: number; startRatios: number[] } | null = null;
  let dragHit = $state<PaneDropHit | null>(null);

  onMount(() => subscribePaneDragHit(() => {
    dragHit = getPaneDragHit();
  }));

  const maximized = $derived(tab.maximizedPaneId ?? null);
  const hiddenByMaximize = $derived(!!maximized && !tabs.nodeContains(node, maximized));

  function closePane(sessionId: string, ev: Event) {
    ev.stopPropagation();
    const result = tabs.removePane(tab.id, sessionId);
    if (!result) return;
    closeSessionsInBackground(rpc, [sessionId]);
  }

  function focusPane(sessionId: string) {
    tabs.focusPane(tab.id, sessionId);
    requestAnimationFrame(() => dispatchFocusPane(sessionId));
  }

  function toggleMaximize(sessionId: string, ev: Event) {
    ev.stopPropagation();
    focusPane(sessionId);
    tabs.toggleMaximize(tab.id, sessionId);
    dispatchFitAllPanes(tab.panes.map((p) => p.id));
  }

  const dropSide = $derived(
    node.type === 'leaf'
      && dragHit?.kind === 'pane'
      && dragHit.paneId === node.pane.id
      ? dragHit.side
      : null,
  );

  function dropBandStyle(side: PaneDropSide): string {
    switch (side) {
      case 'left': return 'left: 0; top: 0; width: 34%; height: 100%;';
      case 'right': return 'right: 0; top: 0; width: 34%; height: 100%;';
      case 'up': return 'left: 0; top: 0; width: 100%; height: 34%;';
      case 'down': return 'left: 0; bottom: 0; width: 100%; height: 34%;';
    }
  }

  function onResize(idx: number, ev: PointerEvent) {
    if (!host || node.type !== 'split') return;
    (ev.target as HTMLElement).setPointerCapture(ev.pointerId);
    const rect = host.getBoundingClientRect();
    const startPx = (node.direction === 'col' ? ev.clientY : ev.clientX)
      - (node.direction === 'col' ? rect.top : rect.left);
    dragging = { idx, startPx, startRatios: node.ratios.slice() };
    ev.preventDefault();

    const onMove = (move: PointerEvent) => {
      if (!dragging || !host || node.type !== 'split') return;
      const r2 = host.getBoundingClientRect();
      const totalNow = node.direction === 'col' ? r2.height : r2.width;
      const cur = (node.direction === 'col' ? move.clientY : move.clientX)
        - (node.direction === 'col' ? r2.top : r2.left);
      const deltaRatio = (cur - dragging.startPx) / Math.max(1, totalNow);
      const a = dragging.startRatios[dragging.idx] ?? 0;
      const b = dragging.startRatios[dragging.idx + 1] ?? 0;
      const minR = 0.06;
      const newA = Math.min(Math.max(a + deltaRatio, minR), a + b - minR);
      const newB = a + b - newA;
      const next = dragging.startRatios.slice();
      next[dragging.idx] = newA;
      next[dragging.idx + 1] = newB;
      tabs.resizeSplit(tab.id, node.id, next);
    };
    const onUp = () => {
      dragging = null;
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }
</script>

{#if node.type === 'leaf'}
  {@const focused = tab.activePaneId === node.pane.id}
  <div
    role="group"
    tabindex="-1"
    style="display: {hiddenByMaximize ? 'none' : 'block'};"
    class="relative h-full w-full min-w-0 min-h-0 bg-[var(--color-bg)] {focused ? 'outline outline-1 outline-[var(--color-accent)] -outline-offset-1' : ''}"
    data-pane-drop-tab={tab.id}
    data-pane-drop-pane={node.pane.id}
    onpointerdown={() => focusPane(node.pane.id)}
  >
    {#if tab.panes.length > 1}
      <div
        bind:this={dragHandle}
        class="pane-drag-handle absolute top-0 left-0 z-[11] flex items-center gap-0.5 px-1 py-0.5 text-[10px]
               bg-[var(--color-panel)]/80 backdrop-blur border border-[var(--color-border-soft)]
               cursor-grab active:cursor-grabbing select-none touch-none"
        title={i18n.t('pane.movePane')}
        aria-label={i18n.t('pane.movePane')}
        onpointerdown={(e) => {
          e.stopPropagation();
          if (dragHandle) startPointerPaneDrag(tab.id, node.pane.id, e, dragHandle);
        }}
        onlostpointercapture={() => endPaneDrag()}
      >
        <GripVertical size={11} />
        <span>{tabs.paneIndex(tab, node.pane.id) + 1}</span>
      </div>
    {/if}
    <TerminalPane
      {rpc}
      session={node.pane}
      active={focused}
      layoutVisible={!hiddenByMaximize}
      {tabVisible}
      {settingsRev}
      onClosePane={() => closePane(node.pane.id, new Event('close'))}
      {onOpenSftp}
      {onSplitRight}
      {onSplitDown}
      onMaximize={() => toggleMaximize(node.pane.id, new Event('click'))}
      {broadcastEnabled}
      {broadcastTargetIds}
    />
    {#if tab.panes.length > 1}
      <button
        type="button"
        title={tab.maximizedPaneId === node.pane.id ? i18n.t('pane.restorePane') : i18n.t('pane.maximizePane')}
        aria-label={tab.maximizedPaneId === node.pane.id ? i18n.t('pane.restorePane') : i18n.t('pane.maximizePane')}
        class="btn-ghost absolute top-1 right-7 z-10 p-1 bg-[var(--color-panel)]/85 backdrop-blur border border-[var(--color-border-soft)]"
        onclick={(e) => toggleMaximize(node.pane.id, e)}
        onpointerdown={(e) => e.stopPropagation()}
      >
        {#if tab.maximizedPaneId === node.pane.id}<Minimize2 size={12} />{:else}<Maximize2 size={12} />{/if}
      </button>
      <button
        type="button"
        title={i18n.t('pane.closePaneShortcut')}
        aria-label={i18n.t('pane.closePane')}
        class="btn-ghost absolute top-1 right-1 z-10 p-1 bg-[var(--color-panel)]/85 backdrop-blur border border-[var(--color-border-soft)] hover:!text-[var(--color-danger)]"
        onclick={(e) => closePane(node.pane.id, e)}
        onpointerdown={(e) => e.stopPropagation()}
      >
        <X size={12} />
      </button>
    {/if}
    {#if dropSide}
      <div class="absolute inset-0 z-20 pointer-events-none border-2 border-[var(--color-accent)] bg-[var(--color-accent)]/10">
        <div class="absolute bg-[var(--color-accent)]/25" style={dropBandStyle(dropSide)}></div>
      </div>
    {/if}
  </div>
{:else}
  <div
    bind:this={host}
    style="display: {hiddenByMaximize ? 'none' : 'flex'};"
    class="h-full w-full min-w-0 min-h-0 {maximized ? 'flex-1' : ''} {node.direction === 'col' ? 'flex-col' : 'flex-row'}"
    ondragover={(e) => {
      if (isPaneDragActive()) e.preventDefault();
    }}
  >
    {#each node.children as child, idx (tabs.nodeKey(child))}
      {@const childHidden = !!maximized && !tabs.nodeContains(child, maximized)}
      {@const childExpanded = !!maximized && tabs.nodeContains(child, maximized)}
      <div
        style={childHidden
          ? 'display: none;'
          : childExpanded
            ? 'flex: 1 1 100%; min-width: 0; min-height: 0; width: 100%; height: 100%;'
            : `flex: ${node.ratios[idx] ?? 1} ${node.ratios[idx] ?? 1} 0; min-width: 60px; min-height: 60px;`}
        class="relative min-w-0 min-h-0 {childExpanded ? 'flex-[1_1_100%]' : ''}"
      >
        <PaneNodeView
          {rpc}
          {tab}
          node={child}
          {settingsRev}
          {tabVisible}
          {onOpenSftp}
          {onSplitRight}
          {onSplitDown}
          {broadcastEnabled}
          {broadcastTargetIds}
        />
      </div>
      {#if idx < node.children.length - 1 && !maximized}
        <button
          type="button"
          aria-label={i18n.t('pane.resizePane')}
          onpointerdown={(e) => onResize(idx, e)}
          class="bg-[var(--color-border-soft)] hover:bg-[var(--color-accent)] transition-colors
                 {node.direction === 'col' ? 'h-[3px] w-full cursor-row-resize' : 'w-[3px] h-full cursor-col-resize'}"
        ></button>
      {/if}
    {/each}
  </div>
{/if}