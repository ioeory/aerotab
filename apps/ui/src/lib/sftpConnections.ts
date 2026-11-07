export interface SftpLease { ready: Promise<string>; release(): void }
interface Connection { key: string; refs: number; id?: string; ready: Promise<string> }
/** Reference-counted backend ownership. A task retains synchronously at enqueue time. */
export class SftpConnections<T = unknown> {
  private entries = new Map<string, Connection>();
  private bySession = new Map<string, Connection>();
  constructor(private open: (target: T) => Promise<string>, private close: (id: string) => Promise<unknown>) {}
  acquire(key: string, target: T): SftpLease {
    let entry = this.entries.get(key);
    if (!entry) {
      entry = { key, refs: 0, ready: Promise.resolve('') };
      const current = entry;
      current.ready = this.open(target).then(async id => {
        if (current.refs === 0) { await this.close(id); throw new Error('SFTP lease released'); }
        current.id = id; this.bySession.set(id, current); return id;
      }).catch(error => { if (this.entries.get(key) === current) this.entries.delete(key); throw error; });
      this.entries.set(key, current);
    }
    return this.lease(entry);
  }
  retainSession(id: string): SftpLease {
    const entry = this.bySession.get(id);
    if (!entry) throw new Error('SFTP session unavailable; reconnect before retrying');
    return this.lease(entry);
  }
  private lease(entry: Connection): SftpLease {
    entry.refs++; let released = false;
    return { ready: entry.ready, release: () => {
      if (released) return; released = true;
      if (--entry.refs !== 0) return;
      if (this.entries.get(entry.key) === entry) this.entries.delete(entry.key);
      if (entry.id) { this.bySession.delete(entry.id); void this.close(entry.id).catch(() => {}); }
    } };
  }
}
