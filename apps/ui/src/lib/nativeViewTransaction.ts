// @ts-ignore Node strip-types tests require an explicit extension; Vite resolves this source directly.
import { traceView } from './viewCausalTrace.ts';

/** Restore only a still-live retained owner, never a stale async catch. */
export function restoreRetainedSourceStatus<T>(options: {
  source: T; generation: number;
  current: () => { source: T | null; generation: number; ready: boolean; closed: boolean };
  report: () => void;
}): boolean {
  const state = options.current();
  if (state.closed || !state.ready || state.source !== options.source || state.generation !== options.generation) return false;
  traceView({ stage: 'rollback.restore-status', epoch: options.generation });
  options.report();
  return true;
}

/** Background rollback owns the buffered target until accepted native input is
 * acknowledged. Drain deadlines and host update rejection are retryable; cancellation/write failure
 * must never be converted into permission to replay target input. */
export async function recoverNativeInputRollback(options: {
  current: () => boolean; drain: () => Promise<void>; rollback: () => Promise<void>; retryMs?: number;
}): Promise<void> {
  while (options.current()) {
    try {
      await options.drain();
      if (options.current()) await options.rollback();
      return;
    }
    catch (error) {
      const message = (error as Error)?.message ?? '';
      if (['native input drain timeout', 'native input handoff timeout', 'native input freeze watermark unavailable'].includes(message) || message.startsWith('native_surface_unavailable:')) {
        await new Promise<void>(resolve => setTimeout(resolve, options.retryMs ?? 250));
        continue;
      }
      throw error;
    }
  }
}

/** One frame owner survives staging. Source input is quiesced only after target draw. */
export async function runNativeViewTransaction<T>(options: {
  current: () => boolean;
  prepare: () => Promise<T>;
  validate: (receipt: T) => boolean;
  drain: () => Promise<void>;
  commit: () => Promise<void>;
  rollback: () => Promise<void>;
}): Promise<void> {
  const check = () => { if (!options.current()) throw new Error('view transaction superseded or session closed'); };
  try {
    check();
    traceView({ stage: 'transaction.prepare' });
    const receipt = await options.prepare();
    traceView({ stage: 'transaction.prepared' });
    check();
    if (!options.validate(receipt)) throw new Error('stale target draw receipt');
    traceView({ stage: 'transaction.drain' });
    await options.drain();
    traceView({ stage: 'transaction.drained' });
    check();
    traceView({ stage: 'transaction.commit' });
    await options.commit();
    traceView({ stage: 'transaction.committed' });
  } catch (error) {
    traceView({ stage: 'transaction.rollback' });
    await options.rollback();
    traceView({ stage: 'transaction.rolled-back' });
    throw error;
  }
}
