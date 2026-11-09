/** Serialize temporary same-engine surface handoffs. No controller/session is replaced. */
export function createFloatFallback(options: {
  native: () => boolean;
  toWebview: () => Promise<void>;
  toNative: () => Promise<void>;
}) {
  let wanted = false, temporary = false, disposed = false;
  let pending: Promise<void> | null = null;
  let failed: { wanted: boolean; error: unknown } | null = null;
  const run = async () => {
    while (!disposed) {
      if (failed?.wanted === wanted) throw failed.error;
      const target = wanted;
      try {
        if (target && options.native()) {
          await options.toWebview(); temporary = true; continue;
        }
        if (!target && temporary) {
          await options.toNative(); temporary = false; continue;
        }
      } catch (error) {
        // Rollback retained the source. Observers/timers are not new requests:
        // remember the failure until the actual float state changes.
        failed = { wanted: target, error };
        throw error;
      }
      break;
    }
  };
  return {
    update(float: boolean) {
      if (float !== wanted) failed = null;
      wanted = float;
      if (!pending) pending = run().finally(() => { pending = null; });
      return pending;
    },
    dispose() { disposed = true; },
  };
}
