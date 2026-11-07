import type { RpcClient } from './rpc';
import { b64decode, b64encode, tauriInvoke } from './rpc';
import type { LocalEntry, SftpEntry, SshProfileSpec } from './types';
import { SftpConnections } from './sftpConnections';
import { TransferQueue, copyChunks, type FileEndpoint, type TransferTask, type TransferContext } from './transferQueue';
import { appConfirm } from './confirm.svelte';
import { fileMessage } from './fileMessages';
import { joinLocalPath, parentLocalPath } from './sftpLocal';
import { i18n } from './i18n.svelte';
export interface FileConnectionTarget { profile?: SshProfileSpec; profile_id?: string; sudo?: boolean }
export interface FileTransferService { connections: SftpConnections<FileConnectionTarget>; queue: TransferQueue }
const services = new WeakMap<RpcClient, FileTransferService>();
const join = (endpoint: FileEndpoint, base: string, name: string) => endpoint.kind === 'local' ? joinLocalPath(base, name) : `${base.replace(/\/$/, '')}/${name}`;
async function local<T>(method: string, args: Record<string, unknown> = {}): Promise<T> { const promise = tauriInvoke<T>(method, args); if (!promise) throw new Error('Desktop file API unavailable'); return promise; }
export function getFileTransferService(rpc: RpcClient): FileTransferService {
  const existing = services.get(rpc); if (existing) return existing;
  const connections = new SftpConnections<FileConnectionTarget>(async target => (await rpc.call<{ id: string }>('sftp.open', target)).id, id => rpc.call('sftp.close', { id }));
  async function stat(endpoint: FileEndpoint, path: string): Promise<{ kind: string; size: number } | null> {
    // Listing the parent distinguishes absence from permission/connection errors; do not treat every stat failure as ENOENT.
    const parent = endpoint.kind === 'local' ? parentLocalPath(path) : path.slice(0, path.lastIndexOf('/')) || '/';
    const name = path.replace(/\\/g, '/').split('/').pop();
    const entries = endpoint.kind === 'local' ? await local<LocalEntry[]>('local_list_dir', { path: parent }) : await rpc.call<SftpEntry[]>('sftp.list', { id: endpoint.sessionId, path: parent });
    return entries.find(e => e.name === name) ?? null;
  }
  async function conflict(task: TransferTask, path: string): Promise<boolean> {
    const found = await stat(task.destination, path); if (!found) return true;
    if (task.conflict === 'skip') return false;
    return task.conflict === 'overwrite' || appConfirm(fileMessage('conflict', path), { danger: true });
  }
  async function mkdir(endpoint: FileEndpoint, path: string) {
    if (endpoint.kind === 'local') { await local('local_mkdir', { path }); return; }
    const found = await stat(endpoint, path); if (found) { if (found.kind !== 'Dir') throw new Error('Destination is not a directory'); return; }
    await rpc.call('sftp.mkdir', { id: endpoint.sessionId, path });
  }
  async function execute(task: TransferTask, context: TransferContext) {
    let transferred = 0; let total = task.size;
    const source = task.source; const dest = task.destination;
    if (task.mode === 'direct' && source.kind === 'remote' && dest.kind === 'remote') {
      if (!await conflict(task, dest.path)) return;
      context.checkCanceled();
      const preflight = await rpc.call<{ can_direct: boolean; dest_auth_ok: boolean; source_host_key: string; reasons: string[] }>('sftp.directPreflight', { source_session_id: source.sessionId, dest_session_id: dest.sessionId, probe_tools: true });
      if (!preflight.can_direct || !preflight.dest_auth_ok) throw new Error(preflight.reasons.join('; '));
      // Always obtain explicit consent; never infer it from selecting a transport mode.
      if (!await appConfirm(i18n.t('transfer.directConsentBody', { host: preflight.source_host_key }), { title: i18n.t('transfer.directConsentTitle') })) throw new Error(i18n.t('transfer.directConsentDenied'));
      context.checkCanceled();
      await rpc.call('sftp.directTransfer', { transfer_id: task.id, source_session_id: source.sessionId, dest_session_id: dest.sessionId, source_path: source.path, kind: task.entryKind, dest_path: dest.path, consent_granted: true });
      context.checkCanceled();
      const verified = await rpc.call<SftpEntry>('sftp.stat', { id: dest.sessionId, path: dest.path });
      if (task.entryKind === 'File' && (verified.kind !== 'File' || verified.size !== task.size)) throw new Error(i18n.t('transfer.directVerifyFailed'));
      context.progress(task.size); return;
    }
    async function copyFile(src: string, dst: string, size: number) {
      context.checkCanceled(); if (!await conflict(task, dst)) return; context.checkCanceled();
      const base = transferred;
      await copyChunks(size, async (offset, len) => {
        if (source.file) return new Uint8Array(await source.file.slice(offset, offset + len).arrayBuffer());
        const value = source.kind === 'local' ? await local<{ data: string }>('local_read_chunk', { path: src, offset, len }) : await rpc.call<{ data: string }>('sftp.readChunk', { id: source.sessionId, path: src, offset, len }); return b64decode(value.data);
      }, async (offset, bytes) => {
        if (dest.kind === 'local') await local('local_write_chunk', { path: dst, offset, data: b64encode(bytes), create: offset === 0 });
        else await rpc.call('sftp.writeChunk', { id: dest.sessionId, path: dst, offset, data: b64encode(bytes), create: offset === 0 });
      }, bytes => context.progress(base + bytes, total), context.checkCanceled);
      transferred += size;
    }
    const files: { src: string; dst: string; size: number }[] = []; const dirs: string[] = [];
    async function plan(src: string, dst: string) {
      context.checkCanceled(); dirs.push(dst);
      const entries = source.kind === 'local' ? await local<LocalEntry[]>('local_list_dir', { path: src }) : await rpc.call<SftpEntry[]>('sftp.list', { id: source.sessionId, path: src });
      for (const entry of entries) {
        if (entry.name === '.' || entry.name === '..' || /[/\\]/.test(entry.name)) throw new Error('Unsafe directory entry');
        const childSource = join(source, src, entry.name); const childDest = join(dest, dst, entry.name);
        if (entry.kind === 'Dir' || entry.kind === 'dir') await plan(childSource, childDest);
        else if (entry.kind === 'File' || entry.kind === 'file') files.push({ src: childSource, dst: childDest, size: entry.size });
        else throw new Error(fileMessage('unsupported'));
      }
    }
    if (task.entryKind === 'Dir') { await plan(source.path, dest.path); total = files.reduce((sum, f) => sum + f.size, 0); context.progress(0, total); for (const dir of dirs) { context.checkCanceled(); await mkdir(dest, dir); } }
    else files.push({ src: source.path, dst: dest.path, size: task.size });
    for (const file of files) await copyFile(file.src, file.dst, file.size);
  }
  const queue = new TransferQueue(execute, id => connections.retainSession(id), id => rpc.call('sftp.cancelDirectTransfer', { transfer_id: id }));
  const service = { connections, queue }; services.set(rpc, service);
  if (typeof localStorage !== 'undefined') {
    try { queue.restore(JSON.parse(localStorage.getItem('aerotab.files.queue.v2') ?? '[]')); } catch { /* corrupt history must not prevent browsing */ }
    queue.subscribe(() => { try { localStorage.setItem('aerotab.files.queue.v2', JSON.stringify(queue.serialize())); } catch { /* storage quota does not stop active transfers */ } });
  }
  return service;
}
