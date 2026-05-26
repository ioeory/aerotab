<script lang="ts">
  import { onMount } from 'svelte';
  import PaneNodeView from './PaneNodeView.svelte';
  import { tabs, type Tab } from '../lib/tabs.svelte';
  import { dispatchFitAllPanes } from '../lib/focusPane';
  import { installPaneDragGlobalHandlers, isPaneDragActive } from '../lib/paneDrag';
  import type { RpcClient } from '../lib/rpc';

  interface Props {
    rpc: RpcClient;
    tab: Tab;
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
    settingsRev,
    onOpenSftp,
    onSplitRight,
    onSplitDown,
    broadcastEnabled = false,
    broadcastTargetIds = [],
    tabVisible = true,
  }: Props = $props();

  $effect(() => {
    if (!tabVisible) return;
    void tab.maximizedPaneId;
    void tab.activePaneId;
    const ids = tab.panes.map((p) => p.id);
    dispatchFitAllPanes(ids);
  });

  onMount(() => {
    installPaneDragGlobalHandlers();
  });
</script>

<div
  class="absolute inset-0 min-w-0 min-h-0"
  role="presentation"
  ondragover={(e) => {
    if (isPaneDragActive()) e.preventDefault();
  }}
>
  <PaneNodeView {rpc} {tab} node={tab.layout} {settingsRev} {tabVisible} {onOpenSftp} {onSplitRight} {onSplitDown} {broadcastEnabled} {broadcastTargetIds} />
</div>
