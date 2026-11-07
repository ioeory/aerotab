export const NATIVE_ACTIVATION_EVENT = 'aerotab:native-activation';
export interface NativeActivation { sessionId: string; generation: string; seq: number }
/** Shared across mounted panes so a late A state response cannot undo a newer B click. */
export function createNativeActivationRouter() {
  let newest = 0;
  let epoch = 0;
  return {
    invalidate() { epoch++; },
    async accept(event: NativeActivation, owner: {
      sessionId: string; generation: () => string | null; eligible: () => boolean;
      state: () => Promise<{ generation?: string; focused?: boolean }>;
      activate: () => void;
    }) {
      if (!event || event.sessionId !== owner.sessionId || event.generation !== owner.generation()
        || !Number.isSafeInteger(event.seq) || event.seq <= newest) return;
      newest = event.seq;
      const request = ++epoch;
      if (!owner.eligible()) return;
      try {
        const state = await owner.state();
        if (epoch === request && newest === event.seq && owner.eligible() && owner.generation() === event.generation
          && state.generation === event.generation && state.focused === true) owner.activate();
      } catch { /* Closed/replaced attachments cannot activate a pane. */ }
    },
  };
}
export const nativeActivationRouter = createNativeActivationRouter();
