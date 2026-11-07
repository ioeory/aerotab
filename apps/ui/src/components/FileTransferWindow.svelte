<script lang="ts">
  import { onMount } from 'svelte';
  import type { RpcClient } from '../lib/rpc';
  import type { StoredProfile } from '../lib/types';
  import type { TransferBootstrap } from '../lib/tabs.svelte';
  import FilesWorkspace from './FilesWorkspace.svelte';
  import { transferTabBridge } from '../lib/transferTabBridge.svelte';
  interface Props { rpc: RpcClient; tabId?: string; transferBootstrap?: TransferBootstrap; onOpenSftpDock?: () => void; onError: (message: string) => void }
  let { rpc, tabId = '', transferBootstrap, onOpenSftpDock, onError }: Props = $props();
  let profiles = $state<StoredProfile[]>([]);
  let workspace: FilesWorkspace;
  let initialized = false;
  transferTabBridge.configure(rpc);
  onMount(() => { void rpc.call<StoredProfile[]>('profile.list').then(rows => { profiles = rows; }).catch(e => onError(String(e))); });
  $effect(() => {
    if (!workspace || initialized || !profiles.length) return;
    initialized = true;
    if (transferBootstrap?.leftId) workspace.openEndpoint('left', transferBootstrap.leftId);
    const right = transferBootstrap?.rightId ?? transferBootstrap?.rightProfileId;
    if (right) workspace.openEndpoint('right', right);
  });
</script>
<!-- Legacy transfer-tab mounting API remains supported while App migrates to FilesWorkspace. -->
<FilesWorkspace bind:this={workspace} {rpc} {profiles} {onError} />
