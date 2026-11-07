<script lang="ts">
  import { onMount } from 'svelte';
  import type { RpcClient } from '../lib/rpc';
  import type { StoredProfile } from '../lib/types';
  import { FileWorkspace, type FileLayout, type FileSide } from '../lib/fileWorkspace';
  import { getFileTransferService } from '../lib/fileTransferService';
  import type { TransferRequest, TransferTask } from '../lib/transferQueue';
  import { fileMessage } from '../lib/fileMessages';
  import { transferTabBridge } from '../lib/transferTabBridge.svelte';
  import FileWorkspaceSide from './FileWorkspaceSide.svelte';
  import SftpTransferQueue from './SftpTransferQueue.svelte';
  interface Props { rpc: RpcClient; profiles: StoredProfile[]; onError: (message: string) => void }
  let { rpc, profiles, onError }: Props = $props();
  const service = getFileTransferService(rpc);
  transferTabBridge.configure(rpc);
  let layout = $state<FileLayout>({ version: 1, left: [], right: [], activeLeft: '', activeRight: '' });
  let tasks = $state<TransferTask[]>([]);
  let ready = $state(false);
  let history = $state(false);
  const storageKey = 'aerotab.files.layout.v1';
  function persist() { if (!ready) return; try { localStorage.setItem(storageKey, JSON.stringify(new FileWorkspace(layout).serialize())); } catch { /* Browsing remains available when storage is full. */ } }
  function change(side: FileSide, action: 'add' | 'activate' | 'close', id = '', endpointId = '__local__', path = '') { const model = new FileWorkspace(layout); if (action === 'add') model.add(side, endpointId, path); else if (action === 'activate') model.activate(side, id); else model.close(side, id); layout = model.serialize(); persist(); }
  export function openEndpoint(side: FileSide, profileId: string, path = '.') { change(side, 'add', '', profileId, path); }
  export function enqueue(request: TransferRequest) { return service.queue.enqueue(request); }
  onMount(() => {
    try { const stored = JSON.parse(localStorage.getItem(storageKey) ?? 'null'); if (stored) layout = new FileWorkspace(stored).serialize(); } catch { /* invalid layout is reset without restoring connections */ }
    const model = new FileWorkspace(layout); if (!model.left.length) model.add('left'); if (!model.right.length) model.add('right', profiles.find(p => p.kind === 'ssh')?.id ?? '__local__'); layout = model.serialize(); ready = true;
    const unsubscribe = service.queue.subscribe(() => { tasks = service.queue.tasks.map(t => ({ ...t })); }); return unsubscribe;
  });
  const visible = $derived(tasks.filter(t => history ? !['queued', 'running', 'canceling'].includes(t.status) : ['queued', 'running', 'canceling'].includes(t.status)));
  const items = $derived(visible.map(t => ({ id: t.id, name: t.name, status: t.status === 'canceling' ? 'running' as const : t.status === 'interrupted' ? 'error' as const : t.status, percent: t.size ? Math.min(100, Math.round(t.transferred / t.size * 100)) : t.status === 'done' ? 100 : 0, summary: t.status === 'canceling' ? fileMessage('canceling') : t.status === 'interrupted' ? fileMessage('interrupted') : t.message ?? `${t.transferred} / ${t.size} B`, routeLabel: `${t.source.label ?? t.source.path} → ${t.destination.label ?? t.destination.path}` })));
</script>
<div class="files-workspace" data-aerotab-focus-zone="">
  <header><h1>{fileMessage('title')}</h1><p>{fileMessage('partial')}</p></header>
  <div class="files-sides">
    <FileWorkspaceSide {rpc} {profiles} tabs={layout.left} activeId={layout.activeLeft} label={fileMessage('left')} onAdd={() => change('left', 'add')} onActivate={id => change('left', 'activate', id)} onClose={id => change('left', 'close', id)} onChange={persist} {onError} />
    <FileWorkspaceSide {rpc} {profiles} tabs={layout.right} activeId={layout.activeRight} label={fileMessage('right')} onAdd={() => change('right', 'add')} onActivate={id => change('right', 'activate', id)} onClose={id => change('right', 'close', id)} onChange={persist} {onError} />
  </div>
  <section class="files-queue">
    <div class="queue-switch"><button type="button" class="btn-secondary" aria-pressed={!history} onclick={() => { history = false; }}> {fileMessage('title')} · {tasks.filter(t => ['queued', 'running', 'canceling'].includes(t.status)).length}</button><button type="button" class="btn-secondary" aria-pressed={history} onclick={() => { history = true; }}>{tasks.filter(t => !['queued', 'running', 'canceling'].includes(t.status)).length} · {fileMessage('retry')}</button></div>
    <SftpTransferQueue variant="full" queueView={history ? 'history' : 'active'} tasks={items} onCancel={id => service.queue.cancel(id)} onRetry={id => { if (!service.queue.retry(id)) onError(fileMessage('interrupted')); }} onCancelAll={() => { for (const task of tasks) service.queue.cancel(task.id); }} onClearFinished={() => service.queue.clearFinished()} onClearHistory={() => service.queue.clearFinished()} />
  </section>
</div>
<style>
  .files-workspace { height:100%; display:flex; flex-direction:column; min-height:0; background:var(--color-bg); color:var(--color-fg); }
  header { display:flex; gap:16px; padding:12px 16px; align-items:center; border-bottom:1px solid var(--color-border-soft); }
  h1 { font-size:var(--text-section); font-weight:600; } p { color:var(--color-fg-muted); font-size:var(--text-compact); }
  .files-sides { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr); flex:1; min-height:0; gap:1px; background:var(--color-border-soft); }
  .files-queue { flex-shrink:0; max-height:32%; min-height:140px; display:flex; flex-direction:column; overflow:hidden; }
  .queue-switch { display:flex; gap:8px; padding:4px 12px; }
</style>
