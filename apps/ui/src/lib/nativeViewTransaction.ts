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

type ViewFailureStage = 'prepare' | 'drain' | 'commit';

/** Strict allowlists: never serialize caller objects, exception messages or identifiers. */
export function nativeViewFailureDiagnostic(direction: unknown, stage: unknown, reason: unknown) {
  return {
    direction: direction === 'webview-to-native' || direction === 'native-to-webview' ? direction : 'unknown',
    stage: stage === 'prepare' || stage === 'drain' || stage === 'commit' ? stage : 'unknown',
    reason: typeof reason === 'string' && ['disposed', 'target-error', 'controller', 'config', 'geometry', 'visibility',
      'epoch', 'host', 'generation', 'stale-receipt', 'operation-error'].includes(reason) ? reason : 'unknown',
  };
}

/** One frame owner survives staging. Source input is quiesced only after target draw. */
export async function runNativeViewTransaction<T>(options: {
  current: () => boolean;
  prepare: () => Promise<T>;
  validate: (receipt: T) => boolean;
  drain: () => Promise<void>;
  commit: () => Promise<void>;
  rollback: () => Promise<void>;
  diagnostic?: {
    direction: 'webview-to-native' | 'native-to-webview';
    reason: () => string;
    report: (event: ReturnType<typeof nativeViewFailureDiagnostic>) => void;
  };
}): Promise<void> {
  let stage: ViewFailureStage = 'prepare';
  let receiptInvalid = false;
  const check = () => { if (!options.current()) throw new Error('view transaction superseded or session closed'); };
  try {
    check();
    traceView({ stage: 'transaction.prepare' });
    const receipt = await options.prepare();
    traceView({ stage: 'transaction.prepared' });
    check();
    if (!options.validate(receipt)) { receiptInvalid = true; throw new Error('stale target draw receipt'); }
    stage = 'drain';
    traceView({ stage: 'transaction.drain' });
    await options.drain();
    traceView({ stage: 'transaction.drained' });
    check();
    stage = 'commit';
    traceView({ stage: 'transaction.commit' });
    await options.commit();
    traceView({ stage: 'transaction.committed' });
  } catch (error) {
    // Report once before rollback mutates the retained state. A diagnostic sink
    // must never change rollback, input ownership, or the propagated exception.
    try {
      if (options.diagnostic) {
        const reason = options.diagnostic.reason();
        options.diagnostic.report(nativeViewFailureDiagnostic(options.diagnostic.direction, stage,
          receiptInvalid ? 'stale-receipt' : reason === 'current' ? 'operation-error' : reason));
      }
    } catch { /* Best effort diagnostics only. */ }
    traceView({ stage: 'transaction.rollback' });
    await options.rollback();
    traceView({ stage: 'transaction.rolled-back' });
    throw error;
  }
}
