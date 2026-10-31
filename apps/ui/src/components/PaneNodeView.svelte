<script lang="ts">
  import PaneNodeView from './PaneNodeView.svelte';
  import { tabs, type PaneNode, type Tab } from '../lib/tabs.svelte';
  import { isPaneDragActive } from '../lib/paneDrag';
  import { i18n } from '../lib/i18n.svelte';
  import type { RpcClient } from '../lib/rpc';

  /**
   * Layout chrome only (flex splitters + measure slots).
   * TerminalPane + pane buttons live in PaneGrid, keyed by session id, so
   * leaf→split reparenting never remounts / clears VT state.
   */
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
    onError?: (msg: string) => void;
  }

  let {
    rpc,
    tab,
    node,
    settingsRev: _settingsRev,
    onOpenSftp: _onOpenSftp,
    onSplitRight: _onSplitRight,
    onSplitDown: _onSplitDown,
    broadcastEnabled: _broadcastEnabled = false,
    broadcastTargetIds: _broadcastTargetIds = [],
    tabVisible: _tabVisible = true,
    onError: _onError,
  }: Props = $props();
  let host: HTMLDivElement | null = $state(null);
  let dragging: { idx: number; startPx: number; startRatios: number[] } | null = null;

  const maximized = $derived(tab.maximizedPaneId ?? null);
  const hiddenByMaximize = $derived(!!maximized && !tabs.nodeContains(node, maximized));

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
  <div
    role="presentation"
    style="display: {hiddenByMaximize ? 'none' : 'block'};"
    class="relative h-full w-full min-w-0 min-h-0 pointer-events-none"
    data-pane-slot={node.pane.id}
  ></div>
{:else}
  <div
    bind:this={host}
    role="presentation"
    style="display: {hiddenByMaximize ? 'none' : 'flex'};"
    class="h-full w-full min-w-0 min-h-0 pointer-events-none {maximized ? 'flex-1' : ''} {node.direction === 'col' ? 'flex-col' : 'flex-row'}"
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
          settingsRev={_settingsRev}
          tabVisible={_tabVisible}
          onOpenSftp={_onOpenSftp}
          onSplitRight={_onSplitRight}
          onSplitDown={_onSplitDown}
          broadcastEnabled={_broadcastEnabled}
          broadcastTargetIds={_broadcastTargetIds}
          onError={_onError}
        />
      </div>
      {#if idx < node.children.length - 1 && !maximized}
        <button
          type="button"
          aria-label={i18n.t('pane.resizePane')}
          onpointerdown={(e) => onResize(idx, e)}
          class="pointer-events-auto z-20 bg-[var(--color-border-soft)] hover:bg-[var(--color-accent)] transition-colors
                 {node.direction === 'col' ? 'h-[3px] w-full cursor-row-resize' : 'w-[3px] h-full cursor-col-resize'}"
        ></button>
      {/if}
    {/each}
  </div>
{/if}
