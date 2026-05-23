<script lang="ts">
  import { GripVertical, Maximize2, Minimize2, X } from '@lucide/svelte';
  import PaneNodeView from './PaneNodeView.svelte';
  import TerminalPane from './TerminalPane.svelte';
  import { tabs, type PaneDropSide, type PaneNode, type Tab } from '../lib/tabs.svelte';
  import { i18n } from '../lib/i18n.svelte';
  import type { RpcClient } from '../lib/rpc';

  interface Props {
    rpc: RpcClient;
    tab: Tab;
    node: PaneNode;
    settingsRev: number;
  }

  let { rpc, tab, node, settingsRev }: Props = $props();
  let host: HTMLDivElement | null = $state(null);
  let dragging: { idx: number; startPx: number; startRatios: number[] } | null = null;
  let dropSide = $state<PaneDropSide | null>(null);

  const PANE_DRAG_MIME = 'application/x-tabby-pane';

  const maximized = $derived(tab.maximizedPaneId ?? null);
  const hiddenByMaximize = $derived(!!maximized && !tabs.nodeContains(node, maximized));

  async function closePane(sessionId: string, ev: Event) {
    ev.stopPropagation();
    const result = tabs.removePane(tab.id, sessionId);
    if (!result) return;
    try { await rpc.call('session.close', { id: sessionId }); } catch (e) { console.warn(e); }
  }

  function toggleMaximize(sessionId: string, ev: Event) {
    ev.stopPropagation();
    tabs.toggleMaximize(tab.id, sessionId);
  }

  function dragPayload(ev: DragEvent): { tabId: string; paneId: string } | null {
    const raw = ev.dataTransfer?.getData(PANE_DRAG_MIME);
    if (!raw) return null;
    try {
      const parsed = JSON.parse(raw) as Record<string, unknown>;
      if (typeof parsed.tabId === 'string' && typeof parsed.paneId === 'string') {
        return { tabId: parsed.tabId, paneId: parsed.paneId };
      }
    } catch {
      return null;
    }
    return null;
  }

  function dropSideFromEvent(el: HTMLElement, ev: DragEvent): PaneDropSide {
    const rect = el.getBoundingClientRect();
    const x = (ev.clientX - rect.left) / Math.max(1, rect.width);
    const y = (ev.clientY - rect.top) / Math.max(1, rect.height);
    const distances = [
      ['left', x],
      ['right', 1 - x],
      ['up', y],
      ['down', 1 - y],
    ] as Array<[PaneDropSide, number]>;
    distances.sort((a, b) => a[1] - b[1]);
    return distances[0]?.[0] ?? 'right';
  }

  function dropBandStyle(side: PaneDropSide): string {
    switch (side) {
      case 'left': return 'left: 0; top: 0; width: 34%; height: 100%;';
      case 'right': return 'right: 0; top: 0; width: 34%; height: 100%;';
      case 'up': return 'left: 0; top: 0; width: 100%; height: 34%;';
      case 'down': return 'left: 0; bottom: 0; width: 100%; height: 34%;';
    }
  }

  function onPaneDragStart(sessionId: string, ev: DragEvent) {
    ev.stopPropagation();
    ev.dataTransfer?.setData(PANE_DRAG_MIME, JSON.stringify({ tabId: tab.id, paneId: sessionId }));
    if (ev.dataTransfer) {
      ev.dataTransfer.effectAllowed = 'move';
      ev.dataTransfer.dropEffect = 'move';
    }
  }

  function onPaneDragOver(sessionId: string, ev: DragEvent) {
    if (maximized) return;
    const payload = dragPayload(ev);
    if (!payload || payload.tabId !== tab.id || payload.paneId === sessionId) return;
    ev.preventDefault();
    ev.stopPropagation();
    if (ev.dataTransfer) ev.dataTransfer.dropEffect = 'move';
    dropSide = dropSideFromEvent(ev.currentTarget as HTMLElement, ev);
  }

  function onPaneDragLeave(ev: DragEvent) {
    const next = ev.relatedTarget;
    if (next instanceof Node && (ev.currentTarget as HTMLElement).contains(next)) return;
    dropSide = null;
  }

  function onPaneDrop(sessionId: string, ev: DragEvent) {
    const payload = dragPayload(ev);
    if (!payload || payload.tabId !== tab.id || payload.paneId === sessionId) return;
    ev.preventDefault();
    ev.stopPropagation();
    const side = dropSide ?? dropSideFromEvent(ev.currentTarget as HTMLElement, ev);
    dropSide = null;
    tabs.movePane(tab.id, payload.paneId, sessionId, side);
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
    onpointerdown={() => tabs.focusPane(tab.id, node.pane.id)}
    ondragover={(e) => onPaneDragOver(node.pane.id, e)}
    ondragleave={onPaneDragLeave}
    ondrop={(e) => onPaneDrop(node.pane.id, e)}
  >
    <TerminalPane
      {rpc}
      session={node.pane}
      active={focused && !hiddenByMaximize}
      {settingsRev}
      onClosePane={() => closePane(node.pane.id, new Event('close'))}
    />
    {#if tab.panes.length > 1}
      <button
        type="button"
        title={tab.maximizedPaneId === node.pane.id ? i18n.t('pane.restorePane') : i18n.t('pane.maximizePane')}
        aria-label={tab.maximizedPaneId === node.pane.id ? i18n.t('pane.restorePane') : i18n.t('pane.maximizePane')}
        class="absolute top-1 right-7 z-10 p-1 rounded bg-[var(--color-panel)]/85 backdrop-blur
               text-[var(--color-fg-muted)] hover:text-[var(--color-accent)] hover:bg-[var(--color-panel)]
               border border-[var(--color-border-soft)]"
        onclick={(e) => toggleMaximize(node.pane.id, e)}
        onpointerdown={(e) => e.stopPropagation()}
      >
        {#if tab.maximizedPaneId === node.pane.id}<Minimize2 size={12} />{:else}<Maximize2 size={12} />{/if}
      </button>
      <button
        type="button"
        title={i18n.t('pane.closePaneShortcut')}
        aria-label={i18n.t('pane.closePane')}
        class="absolute top-1 right-1 z-10 p-1 rounded bg-[var(--color-panel)]/85 backdrop-blur
               text-[var(--color-fg-muted)] hover:text-[var(--color-danger)] hover:bg-[var(--color-panel)]
               border border-[var(--color-border-soft)]"
        onclick={(e) => closePane(node.pane.id, e)}
        onpointerdown={(e) => e.stopPropagation()}
      >
        <X size={12} />
      </button>
      <button
        type="button"
        draggable="true"
        title={i18n.t('pane.movePane')}
        aria-label={i18n.t('pane.movePane')}
        class="absolute top-1 left-1 z-10 px-1 py-0.5 rounded text-[10px] bg-[var(--color-panel)]/80
               backdrop-blur text-[var(--color-fg-muted)] hover:text-[var(--color-accent)] hover:bg-[var(--color-panel)]
               border border-[var(--color-border-soft)] inline-flex items-center gap-0.5 cursor-grab active:cursor-grabbing"
        ondragstart={(e) => onPaneDragStart(node.pane.id, e)}
        ondragend={() => (dropSide = null)}
        onclick={(e) => e.stopPropagation()}
        onpointerdown={(e) => e.stopPropagation()}
      >
        <GripVertical size={11} />
        <span>{tabs.paneIndex(tab, node.pane.id) + 1}</span>
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
    class="h-full w-full min-w-0 min-h-0 {node.direction === 'col' ? 'flex-col' : 'flex-row'}"
  >
    {#each node.children as child, idx (tabs.nodeKey(child))}
      {@const childHidden = !!maximized && !tabs.nodeContains(child, maximized)}
      <div
        style="display: {childHidden ? 'none' : 'block'}; flex: {node.ratios[idx] ?? 1} {node.ratios[idx] ?? 1} 0; min-width: 60px; min-height: 60px;"
        class="relative min-w-0 min-h-0"
      >
        <PaneNodeView {rpc} {tab} node={child} {settingsRev} />
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