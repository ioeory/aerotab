// @ts-ignore -- explicit extension supports Node strip-types regression tests.
import { tracePaste } from './nativePasteTrace.ts';
/** A native view of an already attached Rust terminal engine; never owns a PTY. */
export type TerminalViewBackend = 'webview' | 'native-surface';
export const NATIVE_VIEW_OCCLUSION_EVENT = 'aerotab:native-view-occlusion';
export interface NativeViewGeometry { x: number; y: number; width: number; height: number }
export interface NativeViewCapabilities {
  available: boolean;
  backend?: string;
  geometryUnit?: string;
  reason?: string;
}
/** Exact camelCase serde ViewConfig; bold weight/cursor/ligatures are NOT host fields. */
export interface NativeViewConfig {
  fontFamily: string;
  fontSize: number;
  fontWeight: number;
  linePadding: number;
  foreground: number[];
  background: number[];
  palette: number[][];
  expectedGeneration?: string;
  revision?: number;
  inputGeneration?: string;
  inputReady?: boolean;
  inputFrozen?: boolean;
  inputAck?: number;
}
export function nativeOutputSeq(value: unknown): bigint {
  if (typeof value !== 'string' || !/^(0|[1-9][0-9]{0,19})$/.test(value)) throw new Error('invalid native output watermark');
  const parsed = BigInt(value);
  if (parsed > 18446744073709551615n) throw new Error('native output watermark exceeds u64');
  return parsed;
}

export type NativeViewInvoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

/** GTK allocation uses logical pixels. DPR belongs to the GL backing buffer ONLY.
 * Refuse partial clipping rather than display an input surface over adjacent UI. */
export function nativeViewGeometry(
  rect: { left: number; top: number; width: number; height: number },
  viewport: { width: number; height: number },
  _devicePixelRatio = 1,
): NativeViewGeometry | null {
  const { left: x, top: y, width, height } = rect;
  if (![x, y, width, height].every(v => Number.isFinite(v) && v >= 0 && v <= 16384)
    || width < 1 || height < 1 || x + width > viewport.width || y + height > viewport.height) return null;
  return { x, y, width, height };
}

export function nativeSurfaceError(error: unknown): Error {
  const e = error as { code?: string; message?: string } | null;
  return new Error(`native_surface_unavailable: ${e?.code ? `${e.code}: ` : ''}${e?.message ?? String(error)}`);
}

export function createNativeViewHost(options: {
  sessionId: string;
  invoke: NativeViewInvoke;
  cfg?: NativeViewConfig;
}) {
  const { invoke, sessionId } = options;
  let disposed = false;
  let attached = false;
  let generation: string | null = null;
  let requestedRevision = 0;
  let handoffOutputTarget: bigint | null = null;
  let inputReady = false;
  let inputFrozen = false;
  let inputRecovery = false;
  let inputAck = 0;
  let lastConfig = options.cfg;
  let desiredGeometry: NativeViewGeometry | null = null;
  const inputConfig = (cfg: NativeViewConfig) => inputReady && generation
    ? { ...cfg, inputGeneration: generation, inputReady: !inputFrozen && !inputRecovery, inputFrozen, inputAck } : cfg;
  let starting: Promise<void> | null = null;
  let updating: Promise<void> | null = null;
  let stopping: Promise<void> | null = null;
  let desired: { geometry: NativeViewGeometry; visible: boolean; cfg?: NativeViewConfig; revision: number } | null = null;
  let pendingConfig: NativeViewConfig | undefined;
  let focusing: Promise<void> | null = null;
  let focusAllowed = false;
  let visible = false;
  let suspendedForWebview = false;
  let drawVisible = true;
  let cancelDraw: (() => void) | null = null;
  let drawing: Promise<boolean> | null = null;
  const pasteWaiters = new Set<() => void>();
  function cancelPasteWaiters() { for (const cancel of pasteWaiters) cancel(); }

  function setDrawVisible(shown: boolean) {
    drawVisible = shown;
    if (!shown) cancelDraw?.();
  }

  function checkState(state: unknown) {
    const error = (state as { error?: unknown } | null)?.error;
    if (error) throw error;
  }
  async function detach() {
    if (!attached) return;
    attached = false;
    await invoke('native_view_detach', { sessionId });
  }
  return {
    generation() { return disposed || !attached ? null : generation; },
    async quiesceInput(): Promise<number> {
      if (disposed || !attached || !generation || !desiredGeometry || !lastConfig) throw new Error('native input freeze unavailable');
      inputFrozen = true;
      cancelPasteWaiters();
      inputReady = true;
      focusAllowed = false;
      await this.update(desiredGeometry, visible, lastConfig);
      const state = await this.state() as { generation?: string; inputAccepted?: number; inputFrozen?: boolean };
      if (state.generation !== generation || !state.inputFrozen || !Number.isSafeInteger(state.inputAccepted)) {
        throw new Error('native input freeze watermark unavailable');
      }
      return state.inputAccepted!;
    },
    async setInputRecovery(pending: boolean) {
      inputRecovery = pending;
      if (pending) { focusAllowed = false; cancelPasteWaiters(); }
      if (!disposed && attached && desiredGeometry && lastConfig) await this.update(desiredGeometry, visible, lastConfig);
    },
    async acknowledgeInput(seq: number) {
      if (disposed || !attached || !generation || !desiredGeometry || !lastConfig) throw new Error('native input handshake unavailable');
      inputReady = true; inputAck = Math.max(inputAck, seq);
      await this.update(desiredGeometry, visible, lastConfig);
    },
    /** Reversible input cut: hide the same widget, then capture its final reservation. */
    async suspendForWebview(): Promise<number> {
      if (disposed || !attached || !desiredGeometry || !lastConfig || inputFrozen) throw new Error('native suspension unavailable');
      focusAllowed = false;
      suspendedForWebview = true;
      cancelPasteWaiters();
      await this.update(desiredGeometry, false, lastConfig);
      const state = await this.state() as { generation?: string; inputAccepted?: number };
      if (disposed || !attached || state.generation !== generation || !Number.isSafeInteger(state.inputAccepted)) throw new Error('native suspension superseded');
      return state.inputAccepted!;
    },
    async restoreAfterWebview(): Promise<void> {
      if (disposed || !attached || !desiredGeometry || !lastConfig || inputFrozen) throw new Error('native restore unavailable');
      suspendedForWebview = false;
      await this.update(desiredGeometry, true, lastConfig);
    },
    async canPaste(expected: string, eligible: () => boolean = () => true, timeoutMs = 3000): Promise<boolean> {
      // A confirmation may close before the queued show RPC completes. Await
      // that actual update, not a timer/frame guess, then re-check authorization.
      let timer: ReturnType<typeof setTimeout> | undefined;
      let cancel: (() => void) | undefined;
      try {
        if (updating) {
          tracePaste('host-wait', 'start');
          const completed = await Promise.race([
            updating.then(() => true, () => false),
            new Promise<boolean>(resolve => { cancel = () => resolve(false); pasteWaiters.add(cancel); }),
            new Promise<boolean>(resolve => { timer = setTimeout(() => resolve(false), timeoutMs); }),
          ]);
          tracePaste('host-wait', completed ? 'end' : 'cancelled-or-timeout');
          if (!completed) return false;
        }
      } finally { clearTimeout(timer); if (cancel) pasteWaiters.delete(cancel); }
      if (!eligible() || suspendedForWebview || inputRecovery) { tracePaste('host-check', suspendedForWebview ? 'suspended' : 'ineligible'); return false; }
      if (disposed || !attached || generation !== expected || !inputReady || inputFrozen || !visible) {
        tracePaste('host-check', disposed ? 'disposed' : !attached ? 'detached' : generation !== expected ? 'generation'
          : !inputReady ? 'not-ready' : inputFrozen ? 'frozen' : 'hidden'); return false;
      }
      const state = await invoke('native_view_state', { sessionId }) as { generation?: string; inputFrozen?: boolean; error?: unknown };
      const allowed = eligible() && !disposed && attached && generation === expected && inputReady && !inputFrozen && visible && !updating && !suspendedForWebview && !inputRecovery
        && state.generation === expected && state.inputFrozen === false && !state.error;
      tracePaste('host-check', allowed ? 'accepted' : state.error ? 'host-error' : state.generation !== expected ? 'host-generation'
        : state.inputFrozen !== false ? 'host-frozen' : disposed ? 'disposed' : !attached ? 'detached'
        : generation !== expected ? 'generation' : !inputReady ? 'not-ready' : inputFrozen ? 'frozen'
        : !visible ? 'hidden' : updating ? 'update-during-state' : suspendedForWebview ? 'suspended' : 'ineligible');
      return allowed;
    },
    async isCurrentGeneration(expected: string): Promise<boolean> {
      if (disposed || !attached || generation !== expected) return false;
      const state = await invoke('native_view_state', { sessionId }) as { generation?: string };
      return !disposed && attached && state.generation === expected;
    },
    start(geometry: NativeViewGeometry): Promise<void> {
      if (starting) return starting;
      starting = (async () => {
        try {
          const caps = await invoke('native_view_capabilities') as NativeViewCapabilities;
          if (!caps?.available) throw new Error(caps?.reason ?? 'host capability unavailable');
          if (caps.geometryUnit !== 'css-px') throw new Error('unsupported host geometry unit');
          if (disposed) return;
          const state = await invoke('native_view_attach', {
            sessionId, geometry,
            cfg: options.cfg ?? {
              fontFamily: 'monospace', fontSize: 13, fontWeight: 400, linePadding: 0,
              foreground: [226, 237, 247], background: [24, 28, 36],
              palette: [0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5,
                0x7f7f7f, 0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff]
                .map(c => [(c >> 16) & 255, (c >> 8) & 255, c & 255]),
            },
          });
          lastConfig ??= {
            fontFamily: 'monospace', fontSize: 13, fontWeight: 400, linePadding: 0,
            foreground: [226, 237, 247], background: [24, 28, 36],
            palette: [0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5,
              0x7f7f7f, 0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff]
              .map(c => [(c >> 16) & 255, (c >> 8) & 255, c & 255]),
          };
          attached = true;
          generation = (state as { generation?: string })?.generation ?? null;
          checkState(state);
          handoffOutputTarget = nativeOutputSeq((state as { handoffOutputSeq?: unknown })?.handoffOutputSeq);
          if (disposed) await detach();
        } catch (error) { throw nativeSurfaceError(error); }
      })();
      return starting;
    },
    update(geometry: NativeViewGeometry, shown: boolean, cfg?: NativeViewConfig): Promise<void> {
      if (disposed || !attached) return Promise.resolve();
      desiredGeometry = geometry;
      if (cfg) { lastConfig = cfg; pendingConfig = cfg; }
      if (requestedRevision >= 0xffffffff) return Promise.reject(new Error('native configuration revision exhausted'));
      shown = shown && !suspendedForWebview;
      desired = { geometry, visible: shown, revision: ++requestedRevision, ...(pendingConfig ? { cfg: pendingConfig } : {}) };
      tracePaste('visibility-update', 'received', undefined, { revision: requestedRevision, shown });
      setDrawVisible(shown);
      if (!shown) visible = false; // Suppress focus immediately, before IPC completes.
      if (!updating) {
        updating = (async () => {
          while (desired && !disposed && attached) {
            const next = desired;
            desired = null;
            pendingConfig = undefined;
            try {
              tracePaste('visibility-update', 'start', undefined, { revision: next.revision, shown: next.visible });
              checkState(await invoke('native_view_update', { sessionId, geometry: next.geometry, visible: next.visible,
                cfg: { ...inputConfig(next.cfg ?? lastConfig!), expectedGeneration: generation, revision: next.revision },
              }));
              tracePaste('visibility-update', 'end', undefined, { revision: next.revision, shown: next.visible });
              visible = next.visible; // focus() waits for the whole drain, including a queued hide.
            } catch (error) { throw nativeSurfaceError(error); }
          }
        })().finally(() => { updating = null; });
      }
      return updating;
    },
    setDrawVisible,
    /** Configuration/geometry draw barrier, NOT a canonical output-sequence handoff. */
    waitForDraw({ timeoutMs = 5000, pollMs = 32 } = {}): Promise<boolean> {
      if (disposed || !attached || !drawVisible) return Promise.resolve(false);
      if (drawing) return drawing;
      const wait = new Promise<boolean>((resolve, reject) => {
        let done = false;
        let pollTimer: ReturnType<typeof setTimeout> | undefined;
        let timeout: ReturnType<typeof setTimeout>;
        const finish = (drawn: boolean, error?: unknown) => {
          if (done) return;
          done = true;
          clearTimeout(pollTimer);
          clearTimeout(timeout);
          cancelDraw = null;
          if (error) reject(nativeSurfaceError(error)); else resolve(drawn);
        };
        cancelDraw = () => finish(false);
        timeout = setTimeout(() => finish(false, new Error('first_draw_timeout: native view did not draw within deadline')), timeoutMs);
        const poll = async () => {
          try {
            const state = await invoke('native_view_state', { sessionId });
            if (done) return;
            checkState(state);
            const drawn = state as { generation?: string; drawnRevision?: number | null; drawnOutputSeq?: unknown } | null;
            if (generation && drawn?.generation === generation && drawn.drawnRevision != null
              && drawn.drawnRevision >= requestedRevision && handoffOutputTarget !== null
              && drawn.drawnOutputSeq != null && nativeOutputSeq(drawn.drawnOutputSeq) >= handoffOutputTarget) {
              finish(true); // Repeated/wrapping wire seq has no readiness authority.
            } else if (disposed || !drawVisible) finish(false);
            else pollTimer = setTimeout(() => { void poll(); }, pollMs);
          } catch (error) { finish(false, error); }
        };
        void poll();
      });
      drawing = wait.finally(() => { drawing = null; });
      return drawing;
    },
    setFocusAllowed(allowed: boolean) { focusAllowed = allowed; },
    focus(): Promise<void> {
      if (disposed || !attached || !focusAllowed) return Promise.resolve();
      if (!focusing) {
        focusing = (async () => {
          await updating;
          if (disposed || !attached || !focusAllowed || !visible) return;
          try { await invoke('native_view_focus', { sessionId }); }
          catch (error) { throw nativeSurfaceError(error); }
        })().finally(() => { focusing = null; });
      }
      return focusing;
    },
    async state() {
      try {
        const state = await invoke('native_view_state', { sessionId });
        checkState(state);
        return state;
      } catch (error) { throw nativeSurfaceError(error); }
    },
    dispose(): Promise<void> {
      if (stopping) return stopping;
      disposed = true;
      cancelPasteWaiters();
      setDrawVisible(false);
      focusAllowed = false;
      visible = false;
      desired = null;
      stopping = (async () => {
        await starting?.catch(() => {});
        await updating?.catch(() => {});
        await focusing?.catch(() => {});
        try { await detach(); } catch (error) { throw nativeSurfaceError(error); }
      })();
      return stopping;
    },
  };
}
