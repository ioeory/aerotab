export interface WebviewDrawTicket { generation: string; revision: number; outputSeq: bigint }

/** Evidence is renderer.render returning successfully, never RPC delivery or rAF scheduling. */
export function createWebviewDrawBarrier(generation: string) {
  let revision = 0;
  let disposed = false;
  let applied: WebviewDrawTicket | null = null;
  let resolve: ((seq: bigint) => void) | null = null;
  let reject: ((error: Error) => void) | null = null;
  const fail = (error: Error) => { reject?.(error); resolve = null; reject = null; applied = null; };
  return {
    begin(next: number): Promise<bigint> {
      if (disposed || next <= revision) return Promise.reject(new Error('invalid draw revision or disposed'));
      fail(new Error('draw revision superseded'));
      revision = next;
      return new Promise((ok, bad) => { resolve = ok; reject = bad; });
    },
    apply(epoch: string, rev: number, value: unknown): WebviewDrawTicket {
      if (disposed || !resolve || epoch !== generation || rev !== revision) throw new Error('stale draw generation/revision');
      if (typeof value !== 'string' || !/^(0|[1-9][0-9]{0,19})$/.test(value)) throw new Error('invalid output watermark');
      const outputSeq = BigInt(value);
      if (outputSeq > 18446744073709551615n) throw new Error('output watermark exceeds u64');
      applied = Object.freeze({ generation, revision, outputSeq });
      return applied;
    },
    complete(ticket: WebviewDrawTicket): boolean {
      if (disposed || !applied || !resolve || ticket.generation !== generation || ticket.revision !== revision || ticket.outputSeq !== applied.outputSeq) return false;
      resolve(ticket.outputSeq); resolve = null; reject = null; applied = null; return true;
    },
    fail,
    dispose() { disposed = true; fail(new Error('draw barrier disposed')); },
  };
}
