export type TransferStatus = 'queued' | 'running' | 'canceling' | 'done' | 'error' | 'canceled' | 'interrupted';
export interface FileEndpoint { kind: 'local' | 'remote'; path: string; sessionId?: string; profileId?: string; label?: string; file?: Blob }
export interface TransferRequest { name: string; source: FileEndpoint; destination: FileEndpoint; entryKind: 'File' | 'Dir'; size: number; mode?: 'relay' | 'direct'; conflict?: 'ask' | 'skip' | 'overwrite' }
export interface TransferTask extends TransferRequest { id: string; status: TransferStatus; transferred: number; message?: string; attempts: number }
export interface TransferContext { checkCanceled(): void; progress(bytes: number, total?: number): void }
type Lease = { release(): void };
export class TransferQueue {
  tasks: TransferTask[] = [];
  private listeners = new Set<() => void>();
  private leases = new Map<string, Lease[]>();
  private processing?: Promise<void>;
  private canceled = new Set<string>();
  constructor(private execute: (task: TransferTask, context: TransferContext) => Promise<void>, private retain: (id: string) => Lease, private cancelExecutor?: (id: string) => Promise<unknown>) {}
  subscribe(listener: () => void) { this.listeners.add(listener); listener(); return () => { this.listeners.delete(listener); }; }
  private emit() { for (const fn of this.listeners) fn(); }
  enqueue(request: TransferRequest): string {
    if (request.source.kind === 'local' && request.destination.kind === 'local') throw new Error('Local-to-local transfer is not supported');
    const held: Lease[] = [];
    try { for (const e of [request.source, request.destination]) if (e.sessionId) held.push(this.retain(e.sessionId)); } catch (e) { held.forEach(l => l.release()); throw e; }
    const id = `transfer-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    this.leases.set(id, held); this.tasks.push({ ...request, source: { ...request.source }, destination: { ...request.destination }, id, status: 'queued', transferred: 0, attempts: 1 }); this.emit(); this.start(); return id;
  }
  cancel(id: string) {
    const task = this.tasks.find(t => t.id === id); if (!task || !['queued', 'running'].includes(task.status)) return;
    this.canceled.add(id);
    if (task.status === 'queued') { task.status = 'canceled'; this.release(id); }
    else { task.status = 'canceling'; void this.cancelExecutor?.(id).catch(() => {}); }
    this.emit();
  }
  retry(id: string): boolean {
    const t = this.tasks.find(t => t.id === id); if (!t || !['error', 'canceled', 'interrupted'].includes(t.status)) return false;
    try { this.enqueue(t); return true; } catch (e) { t.message = String(e); this.emit(); return false; }
  }
  clearFinished() { this.tasks = this.tasks.filter(t => ['queued', 'running', 'canceling'].includes(t.status)); this.emit(); }
  private release(id: string) { this.leases.get(id)?.forEach(l => l.release()); this.leases.delete(id); }
  private start() { if (this.processing) return; this.processing = this.run().finally(() => { this.processing = undefined; if (this.tasks.some(t => t.status === 'queued')) this.start(); }); }
  async idle() { while (this.processing) await this.processing; }
  private async run() {
    for (;;) {
      const task = this.tasks.find(t => t.status === 'queued'); if (!task) return;
      task.status = 'running'; this.emit();
      const context: TransferContext = { checkCanceled: () => { if (this.canceled.has(task.id)) throw new Error('Canceled'); }, progress: (bytes, total) => { task.transferred = bytes; if (total !== undefined) task.size = total; this.emit(); } };
      try { await this.execute(task, context); context.checkCanceled(); task.status = 'done'; }
      catch (e) { task.status = this.canceled.has(task.id) ? 'canceled' : 'error'; task.message = e instanceof Error ? e.message : String(e); }
      finally { this.release(task.id); this.canceled.delete(task.id); this.emit(); }
    }
  }
  serialize() { return this.tasks.slice(-200).map(t => ({ id: t.id, name: t.name, entryKind: t.entryKind, size: t.size, transferred: t.transferred, attempts: t.attempts, status: ['queued', 'running', 'canceling'].includes(t.status) ? 'interrupted' : t.status, source: { kind: t.source.kind, profileId: t.source.profileId, path: t.source.path }, destination: { kind: t.destination.kind, profileId: t.destination.profileId, path: t.destination.path } })); }
  restore(rows: unknown) { if (!Array.isArray(rows) || this.tasks.length) return; for (const row of rows.slice(-200)) { if (!row || typeof row.name !== 'string' || !row.source || !row.destination) continue; this.tasks.push({ id: String(row.id), name: row.name, entryKind: row.entryKind === 'Dir' ? 'Dir' : 'File', size: Number(row.size) || 0, transferred: Number(row.transferred) || 0, attempts: Number(row.attempts) || 1, status: ['done', 'error', 'canceled'].includes(row.status) ? row.status : 'interrupted', source: { kind: row.source.kind, profileId: row.source.profileId, path: row.source.path }, destination: { kind: row.destination.kind, profileId: row.destination.profileId, path: row.destination.path } }); } this.emit(); }
}
/** Short reads are legal; zero bytes before the declared EOF are not. No 32-bit coercion. */
export async function copyChunks(size: number, read: (offset: number, len: number) => Promise<Uint8Array>, write: (offset: number, bytes: Uint8Array) => Promise<unknown>, progress: (bytes: number) => void, check = () => {}, chunkSize = 256 * 1024): Promise<number> {
  if (!Number.isSafeInteger(size) || size < 0) throw new Error('Invalid file size');
  let offset = 0;
  if (size === 0) { check(); await write(0, new Uint8Array()); }
  while (offset < size) { check(); const len = Math.min(chunkSize, size - offset); const bytes = await read(offset, len); check(); if (bytes.byteLength === 0) throw new Error(`Unexpected EOF at ${offset}/${size}; partial destination retained`); if (bytes.byteLength > len) throw new Error('Read exceeded requested length'); await write(offset, bytes); offset += bytes.byteLength; progress(offset); }
  check(); return offset;
}
