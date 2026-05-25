<script lang="ts">
  import PaneNodeView from './PaneNodeView.svelte';
  import { tabs, type Tab } from '../lib/tabs.svelte';
  import { dispatchFitPane } from '../lib/focusPane';
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
  }: Props = $props();

  const maximizedPaneId = $derived(tab.maximizedPaneId ?? null);
  const maximizedLeaf = $derived(
    maximizedPaneId ? tabs.findLeaf(tab, maximizedPaneId) : null,
  );

  $effect(() => {
    void tab.maximizedPaneId;
    void tab.activePaneId;
    const id = tab.activePaneId;
    requestAnimationFrame(() => {
      requestAnimationFrame(() => dispatchFitPane(id));
    });
  });
</script>

<div class="absolute inset-0 min-w-0 min-h-0">
  {#if maximizedLeaf}
    <PaneNodeView {rpc} {tab} node={maximizedLeaf} {settingsRev} {onOpenSftp} {onSplitRight} {onSplitDown} {broadcastEnabled} {broadcastTargetIds} />
  {:else}
    <PaneNodeView {rpc} {tab} node={tab.layout} {settingsRev} {onOpenSftp} {onSplitRight} {onSplitDown} {broadcastEnabled} {broadcastTargetIds} />
  {/if}
</div>
