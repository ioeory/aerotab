<script lang="ts">
  import Sidebar from '../Sidebar.svelte';
  import type { RpcClient } from '../../lib/rpc';
  import type { StoredProfile } from '../../lib/types';
  import type { ProfileModalOpenOptions } from '../ProfileModal.svelte';

  /** Keep mounted alongside session workspace; active controls global profile shortcut ownership. */
  let { rpc, onError, openProfileModal, connectProfile, openSerialModal, openSftp, openSettings, onShowTransfer, onImportProfiles, active = true }: {
    rpc: RpcClient;
    onError: (message: string) => void;
    openProfileModal: (existing?: StoredProfile, options?: ProfileModalOpenOptions) => void;
    connectProfile: (profile: StoredProfile, target?: import('../../lib/profileConnection').ProfileSplitTarget) => Promise<boolean>;
    openSerialModal: () => void;
    openSftp: (profile: StoredProfile) => void;
    openSettings: () => void;
    onShowTransfer?: () => void;
    onImportProfiles?: () => void;
    active?: boolean;
  } = $props();
  let sidebar: Sidebar | undefined;
  export async function refresh() { await sidebar?.refresh(); }
</script>
<Sidebar bind:this={sidebar} variant="workspace" {rpc} {onError} {openProfileModal} {connectProfile} {openSerialModal} {openSftp} {openSettings} {onShowTransfer} {onImportProfiles} {active} />
