<script lang="ts">
  import { Maximize2, Minimize2, X } from '@lucide/svelte';
  import PaneNodeView from './PaneNodeView.svelte';
  import TerminalPane from './TerminalPane.svelte';
  import { tabs, type PaneNode, type Tab } from '../lib/tabs.svelte';
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
        title={tab.maximizedPaneId === node.pane.id ? 'Restore pane' : 'Maximize pane'}
        aria-label={tab.maximizedPaneId === node.pane.id ? 'Restore pane' : 'Maximize pane'}
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
        title="Close pane (Ctrl+W)"
        aria-label="Close pane"
        class="absolute top-1 right-1 z-10 p-1 rounded bg-[var(--color-panel)]/85 backdrop-blur
               text-[var(--color-fg-muted)] hover:text-[var(--color-danger)] hover:bg-[var(--color-panel)]
               border border-[var(--color-border-soft)]"
        onclick={(e) => closePane(node.pane.id, e)}
        onpointerdown={(e) => e.stopPropagation()}
      >
        <X size={12} />
      </button>
      <div class="absolute top-1 left-1 z-10 px-1.5 py-0.5 rounded text-[10px] bg-[var(--color-panel)]/70
                  backdrop-blur text-[var(--color-fg-muted)] pointer-events-none">
        {tabs.paneIndex(tab, node.pane.id) + 1}
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
          aria-label="Resize pane"
          onpointerdown={(e) => onResize(idx, e)}
          class="bg-[var(--color-border-soft)] hover:bg-[var(--color-accent)] transition-colors
                 {node.direction === 'col' ? 'h-[3px] w-full cursor-row-resize' : 'w-[3px] h-full cursor-col-resize'}"
        ></button>
      {/if}
    {/each}
  </div>
{/if}