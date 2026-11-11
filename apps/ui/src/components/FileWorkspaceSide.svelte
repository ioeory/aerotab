<script lang="ts">
  import { onMount } from 'svelte';
  import type { RpcClient } from '../lib/rpc';
  import { tauriInvoke } from '../lib/rpc';
  import type { StoredProfile, LocalEntry } from '../lib/types';
  import type { FileTab } from '../lib/fileWorkspace';
  import { getFileTransferService } from '../lib/fileTransferService';
  import { fileMessage } from '../lib/fileMessages';
  import { parseRemoteDrag, readSftpDragData, SFTP_DRAG_REMOTE, joinLocalPath, parentLocalPath } from '../lib/sftpLocal';
  import SftpBrowser from './SftpBrowser.svelte';
  import SftpLocalPane from './SftpLocalPane.svelte';
  import EndpointSelector from './EndpointSelector.svelte';
  interface Props { rpc: RpcClient; profiles: StoredProfile[]; tabs: FileTab[]; activeId: string; label: string; onAdd: () => void; onActivate: (id: string) => void; onClose: (id: string) => void; onChange: () => void; onError: (message: string) => void }
  let { rpc, profiles, tabs, activeId, label, onAdd, onActivate, onClose, onChange, onError }: Props = $props();
  const service = getFileTransferService(rpc);
  let localData = $state<Record<string, { entries: LocalEntry[]; loading: boolean; error: string | null }>>({});
  const sequences = new Map<string, number>();
  async function navigate(tab: FileTab, path: string) {
    tab.path = path; onChange(); const seq = (sequences.get(tab.id) ?? 0) + 1; sequences.set(tab.id, seq);
    localData[tab.id] = { entries: localData[tab.id]?.entries ?? [], loading: true, error: null };
    try { const rows = await tauriInvoke<LocalEntry[]>('local_list_dir', { path }); if (!rows) throw Error('Desktop file API unavailable'); if (sequences.get(tab.id) === seq) localData[tab.id] = { entries: rows, loading: false, error: null }; }
    catch (e) { if (sequences.get(tab.id) === seq) localData[tab.id] = { entries: [], loading: false, error: String(e) }; }
  }
  async function home(tab: FileTab) { try { const path = await tauriInvoke<string>('local_home_dir'); if (path) await navigate(tab, path); } catch (e) { onError(String(e)); } }
  function drop(tab: FileTab, event: DragEvent) {
    event.preventDefault(); const raw = readSftpDragData(event.dataTransfer, SFTP_DRAG_REMOTE); if (!raw) return; const remote = parseRemoteDrag(raw); if (!remote?.sourceSessionId) return;
    try { service.queue.enqueue({ name: remote.name, source: { kind: 'remote', sessionId: remote.sourceSessionId, path: remote.path, label: remote.sourceLabel }, destination: { kind: 'local', path: joinLocalPath(tab.path, remote.name) }, entryKind: remote.kind, size: remote.size }); } catch (e) { onError(String(e)); }
  }
  $effect(() => { for (const tab of tabs) if (tab.endpointId === '__local__' && !localData[tab.id]) { localData[tab.id] = { entries: [], loading: true, error: null }; if (tab.path) void navigate(tab, tab.path); else void home(tab); } });
</script>
<section class="file-side" aria-label={label}>
  <div class="file-tabs" role="tablist" aria-label={label}>
    {#each tabs as tab (tab.id)}
      <div class="tab-heading" class:active={activeId === tab.id}>
        <button type="button" role="tab" aria-selected={activeId === tab.id} aria-controls={`panel-${tab.id}`} onclick={() => onActivate(tab.id)}>{tab.endpointId === '__local__' ? fileMessage('local') : profiles.find(p => p.id === tab.endpointId)?.name ?? fileMessage('pick')} <span>{tab.path}</span></button>
        <button type="button" aria-label={fileMessage('close')} onclick={() => onClose(tab.id)}>×</button>
      </div>
    {/each}
    <button type="button" class="btn-ghost" aria-label={fileMessage('add')} onclick={onAdd}>+</button>
  </div>
  {#each tabs as tab (tab.id)}
    <div id={`panel-${tab.id}`} class="file-tab-panel" hidden={tab.id !== activeId} role="tabpanel">
      <EndpointSelector {profiles} value={tab.endpointId} localLabel={fileMessage('local')} placeholder={fileMessage('pick')} onChange={id => { tab.endpointId = id; tab.path = ''; delete localData[tab.id]; onChange(); }} />
      <div class="file-tab-body">
        {#if tab.endpointId === '__local__'}
          <SftpLocalPane cwd={tab.path} entries={localData[tab.id]?.entries ?? []} loading={localData[tab.id]?.loading ?? true} listError={localData[tab.id]?.error ?? null} onRefresh={() => void navigate(tab, tab.path)} onNavigate={path => void navigate(tab, path)} onGoUp={() => void navigate(tab, parentLocalPath(tab.path))} onGoHome={() => void home(tab)} onDropRemote={event => drop(tab, event)} onDropFiles={event => event.preventDefault()} onDragOverPane={event => event.preventDefault()} {onError} />
        {:else}
          {@const profile = profiles.find(p => p.id === tab.endpointId)}
          {#if profile?.kind === 'ssh'}
            {#key tab.endpointId}
              <SftpBrowser {rpc} {profile} registryId={tab.id} mode="dock" showLocalPane={false} initialPath={tab.path || '.'} onPathChange={path => { tab.path = path; onChange(); }} onClose={() => onClose(tab.id)} {onError} />
            {/key}
          {:else}<p>{fileMessage('pick')}</p>{/if}
        {/if}
      </div>
    </div>
  {/each}
</section>
<style>
  .file-side { display:flex; flex-direction:column; min-width:0; min-height:0; overflow:hidden; }
  .file-tabs { display:flex; align-items:stretch; overflow:auto; border-bottom:1px solid var(--color-border-soft); min-height:40px; flex-shrink:0; }
  .tab-heading { display:flex; flex:none; max-width:240px; border-right:1px solid var(--color-border-soft); }
  .tab-heading.active { background:var(--color-accent-soft); color:var(--color-accent); }
  .tab-heading button { padding:7px; text-align:left; min-width:28px; }
  .tab-heading span { display:block; max-width:180px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:var(--text-meta); color:var(--color-fg-muted); }
  .file-tab-panel { display:flex; flex:1; min-width:0; min-height:0; flex-direction:column; }
  .file-tab-panel[hidden] { display:none; }
  .file-tab-body { flex:1; min-width:0; min-height:0; overflow:hidden; }
  button:focus-visible { outline:2px solid var(--color-accent); outline-offset:-2px; }
</style>
