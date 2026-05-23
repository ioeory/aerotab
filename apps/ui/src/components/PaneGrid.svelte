<script lang="ts">
  import PaneNodeView from './PaneNodeView.svelte';
  import type { Tab } from '../lib/tabs.svelte';
  import type { RpcClient } from '../lib/rpc';

  interface Props {
    rpc: RpcClient;
    tab: Tab;
    settingsRev: number;
    onOpenSftp?: () => void;
  }

  let { rpc, tab, settingsRev, onOpenSftp }: Props = $props();
</script>

<div class="absolute inset-0 min-w-0 min-h-0">
  <PaneNodeView {rpc} {tab} node={tab.layout} {settingsRev} {onOpenSftp} />
</div>