<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import { i18n } from '../lib/i18n.svelte';
  import { settlePaneLayout } from '../lib/paneLayoutBarrier';
  import { traceView, traceLayout } from '../lib/viewCausalTrace';
  import { runNativeViewTransaction, recoverNativeInputRollback } from '../lib/nativeViewTransaction';
  import type { RpcClient } from '../lib/rpc';
  import {
    createNativeViewHost, nativeViewGeometry, nativeSurfaceError,
    NATIVE_VIEW_OCCLUSION_EVENT, type TerminalViewBackend, type NativeViewConfig,
  } from '../lib/nativeViewHost';
  import { tracePaste } from '../lib/nativePasteTrace';
  import {
    createPasteRequestGate, listenNativePaste, listenNativeEvent,
    NATIVE_FONT_DELTA_EVENT, type FontDeltaRequest,
  } from '../lib/nativePasteBridge';
  import { createNativeInputQueue, quiesceAndDrainNativeInput, NATIVE_INPUT_EVENT, type NativeInputEvent } from '../lib/nativeInputBridge';
  import { tauriInvoke } from '../lib/termgrid/tauriChannel';
  import { Palette } from '../lib/termgrid/palette';
  import {
    NativeTerminalController,
    type LinkModifier,
    type NativeRendererPref,
    type ThemeColors,
  } from '../lib/termgrid';

  interface Props {
    rpc: RpcClient;
    sessionId: string;
    /** Existing native renderer is explicitly WebView compatibility, not an OS surface. */
    backend?: TerminalViewBackend;
    occluded?: boolean;
    fontFamily?: string;
    fontSize?: number;
    fontWeight?: number;
    fontWeightBold?: number;
    linePadding?: number;
    scrollback?: number;
    cursorBlink?: boolean;
    altIsMeta?: boolean;
    ligatures?: boolean;
    theme?: ThemeColors;
    renderer?: NativeRendererPref;
    translucent?: boolean;
    transferDetect?: boolean;
    linkModifier?: LinkModifier;
    copyOnSelect?: boolean;
    writeInput?: (data: string) => void | Promise<void>;
    visible?: boolean;
    active?: boolean;
    onBell?: () => void;
    onTitle?: (title: string) => void;
    onExit?: () => void;
    onActivity?: () => void;
    onRaw?: (bytes: Uint8Array) => void;
    onTrzszDetected?: () => void;
    onZmodemHint?: () => void;
    onReady?: (ctl: NativeTerminalController) => void;
    onError?: (msg: string) => void;
    onRecovered?: (ctl: NativeTerminalController, backend: TerminalViewBackend, request: number) => void;
    writeNativeBytes?: (bytes: Uint8Array, valid: () => boolean) => Promise<void>;
    onNativePaste?: (text: string, valid: () => Promise<boolean>) => Promise<void>;
  }

  let {
    rpc,
    sessionId,
    backend = 'webview',
    occluded = false,
    fontFamily = 'JetBrains Mono, Menlo, monospace',
    fontSize = 15,
    fontWeight = 400,
    fontWeightBold = 700,
    linePadding = 0,
    scrollback = 10_000,
    cursorBlink = true,
    altIsMeta = false,
    ligatures = false,
    theme,
    renderer = 'auto',
    translucent = false,
    transferDetect = false,
    linkModifier = 'none',
    copyOnSelect = false,
    writeInput,
    visible = true,
    active = true,
    onBell,
    onTitle,
    onExit,
    onActivity,
    onRaw,
    onTrzszDetected,
    onZmodemHint,
    onReady,
    onError,
    onRecovered,
    onNativePaste,
    writeNativeBytes,
  }: Props = $props();

  let canvas: HTMLCanvasElement | null = $state(null);
  let input: HTMLTextAreaElement | null = $state(null);
  let controller: NativeTerminalController | null = $state(null);
  let surface: HTMLDivElement | null = $state(null);
  let hostAdapter: ReturnType<typeof createNativeViewHost> | null = null;
  let surfaceReady = $state(false);
  let surfaceError = $state('');
  let globalOccluded = $state(false);
  let disposed = false;
  let stagingNative = false;
  let committedNative = $state(false);
  let committedWebview = $state(false);
  let stagingWebview = $state(false);
  let webviewHandoff = $state(false);
  let reverseEpoch = 0;
  let transitionClosing = false;
  let stagingError: unknown = null;
  let releasingInput = false;
  let ackRecovering = $state(false);
  let inputBackpressure = $state(false);
  let rollbackPending = $state(false);
  let pasteGate = createPasteRequestGate();
  let unlistenPaste: (() => void) | null = null;
  let unlistenInput: (() => void) | null = null;
  let unlistenFontDelta: (() => void) | null = null;
  let inputQueue: ReturnType<typeof createNativeInputQueue> | null = null;
  function disposePaste() {
    pasteGate.dispose(); unlistenPaste?.(); unlistenPaste = null;
    unlistenFontDelta?.(); unlistenFontDelta = null;
    inputQueue?.dispose(); unlistenInput?.(); unlistenInput = null;
  }
  let startup: Promise<void> | null = null;
  let syncSurface: (() => void) | null = null;
  let cleanupSurface: (() => void) | null = null;

  function fail(error: unknown) {
    if (stagingNative) { stagingError = error; return; }
    const message = ((backend === 'native-surface' && !committedWebview) || committedNative)
      ? nativeSurfaceError(error).message : (error as Error)?.message ?? String(error);
    surfaceError = message;
    surfaceReady = false;
    if (!releasingInput && !inputQueue?.hasPending()) void hostAdapter?.dispose().catch(console.warn);
    onError?.(message);
  }

  function viewConfig(): NativeViewConfig {
    const palette = new Palette(theme);
    const rgb = (c: number) => [(c >> 16) & 255, (c >> 8) & 255, c & 255];
    return {
      fontFamily, fontSize, fontWeight, linePadding,
      foreground: rgb(palette.foreground), background: rgb(palette.background),
      palette: Array.from({ length: 16 }, (_, i) => rgb(palette.resolve(i, true))),
    };
  }

  function canFocusSurface() {
    return active && visible && !occluded && !globalOccluded && !document.hidden && !disposed && !surfaceError && !ackRecovering && !rollbackPending;
  }

  /** Commit this request's post-modal visibility, even when sync's dirty loop is busy. */
  function requestPasteVisibility(): boolean {
    if (!hostAdapter || !canFocusSurface() || releasingInput || stagingWebview) return false;
    const geometry = surface && nativeViewGeometry(surface.getBoundingClientRect(), {
      width: window.innerWidth, height: window.innerHeight,
    }, window.devicePixelRatio);
    if (!geometry) return false;
    // Submit synchronously: canPaste waits this concrete queue/revision, never a future show.
    void hostAdapter.update(geometry, true, viewConfig()).catch(fail);
    return true;
  }

  async function mountSurface() {
    pasteGate = createPasteRequestGate();
    const invoke = tauriInvoke();
    if (!invoke || !surface) throw new Error('Tauri native view bridge unavailable');
    hostAdapter = createNativeViewHost({ sessionId, invoke, cfg: viewConfig() });
    if (!writeNativeBytes) throw new Error('Native input writer unavailable');
    inputQueue = createNativeInputQueue({
      sessionId, generation: () => hostAdapter?.generation() ?? null,
      write: (bytes, valid) => writeNativeBytes!(bytes, valid),
      ack: (seq) => hostAdapter!.acknowledgeInput(seq),
      onAckRecovery: pending => {
        ackRecovering = pending;
        // Deliberately not fatal: keep the same attachment/listener and retry
        // ACK only. Host readiness gates new input, not accepted reservations.
        void hostAdapter?.setInputRecovery(pending).catch(() => {});
        if (!pending && !disposed) syncSurface?.();
      },
      fail: () => { if (!disposed) fail(new Error('Native input bridge failed or overflowed')); },
    });
    const stopInput = await listenNativeEvent<NativeInputEvent>(NATIVE_INPUT_EVENT, event => inputQueue?.accept(event));
    if (disposed) { stopInput(); return; }
    unlistenInput = stopInput;
    // Register before attach/focus; absent listener always fails closed.
    const unlisten = await listenNativePaste((request) => {
      const generation = hostAdapter?.generation() ?? null;
      const token = pasteGate.accept(request, { sessionId, generation,
        eligible: canFocusSurface() && surfaceReady && !!onNativePaste });
      if (!token || !generation) return;
      const valid = async () => {
        await tick(); // Flush logical modal close/occlusion effects before inspecting its host update.
        return token.valid(hostAdapter?.generation() ?? null)
          && requestPasteVisibility()
          && await hostAdapter!.canPaste(generation, () => token.valid(hostAdapter?.generation() ?? null)
            && canFocusSurface() && !releasingInput && !stagingWebview).catch(() => { tracePaste('authorization', 'exception', request.requestId); return false; });
      };
      void onNativePaste!(request.text, valid).catch(() => {
        // Clipboard text/error payload must not enter diagnostic logs.
        onError?.('Native paste failed');
      }).finally(token.finish);
    });
    if (disposed) { unlisten(); return; }
    unlistenPaste = unlisten;
    const stopFont = await listenNativeEvent<FontDeltaRequest>(NATIVE_FONT_DELTA_EVENT, (request) => {
      if (request?.sessionId !== sessionId) return;
      if (hostAdapter?.generation() !== request.generation) return;
      if (!Number.isFinite(request.delta) || !request.delta) return;
      // Reuse the pane-level font zoom path (Ctrl+= / Ctrl+- / Ctrl+wheel).
      document.dispatchEvent(new CustomEvent('aerotab:terminal-font-delta', { detail: request.delta }));
    });
    if (disposed) { stopFont(); return; }
    unlistenFontDelta = stopFont;
    let started = false;
    let inputAcknowledged = false;
    let busy = false;
    let dirty = false;
    let lastGeometry = { x: 0, y: 0, width: 1, height: 1 };
    let lastConfig = '';
    let focusRestored = false;
    const sync = async () => {
      // Cancel first-draw polling even while the main sync is awaiting a frame.
      const rectNow = surface && nativeViewGeometry(surface.getBoundingClientRect(), {
        width: window.innerWidth, height: window.innerHeight,
      }, window.devicePixelRatio);
      hostAdapter?.setDrawVisible(!!rectNow && visible && !occluded && !globalOccluded && !document.hidden);
      if (busy) { dirty = true; return; }
      if (disposed || surfaceError || !surface || stagingWebview || (committedWebview && !stagingNative)) return;
      busy = true;
      try {
        do {
          dirty = false;
          const rect = nativeViewGeometry(surface.getBoundingClientRect(), {
            width: window.innerWidth, height: window.innerHeight,
          }, window.devicePixelRatio);
          if (stagingWebview || (committedWebview && !stagingNative)) return;
          const shown = visible && !occluded && !globalOccluded && !document.hidden && !!rect;
          if (!started && !shown) continue;
          if (rect) lastGeometry = rect;
          if (!started) {
            // terminal.attach + frame Channel must have completed before native view attach.
            await hostAdapter!.start(lastGeometry);
            started = true;
          }
          if (disposed) return;
          const cfg = viewConfig();
          const configKey = JSON.stringify(cfg);
          hostAdapter!.setFocusAllowed(canFocusSurface() && shown);
          if (!canFocusSurface() || !shown) focusRestored = false;
          await hostAdapter!.update(lastGeometry, shown, configKey === lastConfig ? undefined : cfg);
          lastConfig = configKey;
          if (!inputAcknowledged && !stagingNative) {
            await hostAdapter!.acknowledgeInput(0);
            inputAcknowledged = true;
          }
          // Every visible configuration/geometry update must draw its own revision;
          // an earlier frame or repeated wire sequence cannot authorize readiness/focus.
          if (shown && !(await hostAdapter!.waitForDraw())) continue;
          if (stagingNative) { hostAdapter!.setFocusAllowed(false); continue; }
          if (!surfaceReady) {
            if (!shown) continue;
            if (disposed || !visible || occluded || globalOccluded || document.hidden) continue;
            surfaceReady = true;
            if (controller) onReady?.(controller);
          }
          // Re-check latest props after async show: another pane/modal may now own focus.
          hostAdapter!.setFocusAllowed(canFocusSurface() && shown);
          if (!focusRestored && canFocusSurface() && shown) {
            await hostAdapter!.focus();
            focusRestored = true;
          }
        } while (dirty && !disposed);
      } catch (error) { if (!disposed && !ackRecovering) fail(error); }
      finally { busy = false; }
    };
    syncSurface = () => { void sync(); };
    const ro = new ResizeObserver(() => { traceLayout('resize', surface); syncSurface?.(); });
    ro.observe(surface);
    // Changes to ancestor layout/class can move a pane without changing its size.
    const mo = new MutationObserver(() => { traceLayout('mutation', surface); syncSurface?.(); });
    for (let node: HTMLElement | null = surface; node; node = node.parentElement) {
      mo.observe(node, { attributes: true, attributeFilter: ['style', 'class', 'hidden'] });
    }
    const onOcclusion = (event: Event) => {
      globalOccluded = (event as CustomEvent<{ occluded: boolean }>).detail?.occluded === true;
      syncSurface?.(); // Do not wait for a paint when a modal is about to open.
    };
    window.addEventListener('resize', syncSurface);
    window.addEventListener('scroll', syncSurface, true);
    document.addEventListener('visibilitychange', syncSurface);
    document.addEventListener(NATIVE_VIEW_OCCLUSION_EVENT, onOcclusion);
    // Native GL failures can arise after attach. Never leave a silent blank surface.
    const health = window.setInterval(() => {
      if (started && surfaceReady && !disposed && visible && !occluded && !globalOccluded && !document.hidden) {
        void hostAdapter?.state().catch(fail);
      }
    }, 1000);
    cleanupSurface = () => {
      ro.disconnect(); mo.disconnect(); window.clearInterval(health);
      window.removeEventListener('resize', syncSurface!);
      window.removeEventListener('scroll', syncSurface!, true);
      document.removeEventListener('visibilitychange', syncSurface!);
      document.removeEventListener(NATIVE_VIEW_OCCLUSION_EVENT, onOcclusion);
    };
    await sync();
  }

  /** WebView -> native only. Reuse the sole frame owner; never attach another Channel. */
  export async function stageNativeSurface(request = 0) {
    if ((backend !== 'webview' && !committedWebview) || committedNative || stagingNative || !controller) throw new Error('native staging unavailable');
    const source = controller;
    await settlePaneLayout(surface);
    if (disposed || controller !== source) throw new Error('native staging superseded during layout barrier');
    const sourceConfig = JSON.stringify(viewConfig());
    const sourceBounds = surface?.getBoundingClientRect();
    const sourceGeometry = sourceBounds && [sourceBounds.x, sourceBounds.y, sourceBounds.width, sourceBounds.height].join(',');
    traceLayout('capture', surface);
    stagingNative = true;
    stagingError = null;
    const current = () => {
      const bounds = surface?.getBoundingClientRect();
      const status = disposed || transitionClosing ? 'disposed' : stagingError ? 'target-error' : controller !== source ? 'controller'
        : sourceConfig !== JSON.stringify(viewConfig()) ? 'config'
        : !bounds || sourceGeometry !== [bounds.x, bounds.y, bounds.width, bounds.height].join(',') ? 'geometry'
        : !visible || occluded || globalOccluded || document.hidden ? 'visibility' : 'current';
      traceView({ stage: 'transaction.current', status });
      traceLayout('current', surface);
      return status === 'current';
    };
    await runNativeViewTransaction({
      current,
      prepare: async () => {
        // Host attach captures the actual canonical target before source release.
        // It never changes the existing terminal_frames sink.
        await mountSurface();
        if (stagingError) throw stagingError;
        if (!hostAdapter || !(await hostAdapter.waitForDraw())) throw new Error('native staged draw unavailable');
        return hostAdapter.generation();
      },
      validate: receipt => !!receipt && receipt === hostAdapter?.generation(),
      drain: async () => {
        let timer: ReturnType<typeof setTimeout> | undefined;
        try {
          await Promise.race([source.quiesceWebviewInput(), new Promise<never>((_, reject) => {
            timer = setTimeout(() => reject(new Error('WebView input drain timeout')), 5000);
          })]);
        } finally { clearTimeout(timer); }
      },
      commit: async () => {
        if (!(await hostAdapter!.waitForDraw())) throw new Error('staged native draw superseded');
        // Last await before the synchronous ownership flip. No source freeze is
        // needed during target preparation, so rollback keeps its input usable.
        if (!current()) throw new Error('native staging superseded');
        await hostAdapter!.acknowledgeInput(0);
        if (!(await hostAdapter!.waitForDraw())) throw new Error('native commit revision not drawn');
        if (!current()) throw new Error('native staging superseded');
        committedNative = true;
        committedWebview = false;
        stagingNative = false;
        surfaceReady = true;
        source.commitNativeSurface();
        hostAdapter!.setFocusAllowed(canFocusSurface());
        syncSurface?.();
      },
      rollback: async () => {
        const host = hostAdapter, queue = inputQueue;
        const retained = () => !disposed && !transitionClosing && controller === source && hostAdapter === host;
        const restore = async () => {
          if (!retained()) return;
          cleanupSurface?.(); cleanupSurface = null; syncSurface = null;
          try { await host?.dispose(); }
          finally {
            hostAdapter = null;
            inputQueue?.dispose(); inputQueue = null;
            unlistenInput?.(); unlistenInput = null;
            unlistenPaste?.(); unlistenPaste = null;
            unlistenFontDelta?.(); unlistenFontDelta = null;
            stagingNative = false; rollbackPending = false;
            if (!disposed && !transitionClosing) {
              source.resumeWebviewInput(); source.focus(); onRecovered?.(source, 'webview', request);
            }
          }
        };
        // ACK(0) may have enabled the target even if its RPC response was lost.
        // Freeze/capture it before detach; late delivered reservations still own
        // their bytes and must drain before source input can resume.
        if (host?.generation() && queue && retained()) {
          const drain = () => quiesceAndDrainNativeInput(host, queue);
          try { await drain(); }
          catch (error) {
            if (!retained()) return;
            rollbackPending = true;
            void recoverNativeInputRollback({current: retained, drain, rollback: restore})
              .catch(error => { if (retained()) onError?.(nativeSurfaceError(error).message); });
            throw error;
          }
        }
        await restore();
      },
    });
  }

  /** Native -> WebView uses the SAME controller and canonical snapshot draw. */
  export async function stageWebviewSurface(request = 0) {
    if (disposed || stagingWebview || !controller || !hostAdapter || !inputQueue) throw new Error('WebView staging unavailable');
    const source = controller, host = hostAdapter, queue = inputQueue;
    await settlePaneLayout(surface);
    if (disposed || controller !== source || hostAdapter !== host) throw new Error('WebView staging superseded during layout barrier');
    const generation = host.generation(), epoch = ++reverseEpoch;
    const config = JSON.stringify(viewConfig());
    const bounds = () => { const r = surface?.getBoundingClientRect(); return r && [r.x,r.y,r.width,r.height].join(','); };
    const geometry = bounds();
    const current = () => !disposed && reverseEpoch === epoch && controller === source && hostAdapter === host
      && host.generation() === generation && JSON.stringify(viewConfig()) === config && bounds() === geometry
      && visible && !occluded && !globalOccluded && !document.hidden;
    stagingWebview = true;
    let suspended = false;
    let drainTarget: number | null = null;
    const cut = { quiesceInput: async () => {
      if (drainTarget === null) drainTarget = await host.suspendForWebview();
      return drainTarget;
    } };
    const drainSource = () => quiesceAndDrainNativeInput(cut, queue);
    await runNativeViewTransaction({
      current,
      prepare: async () => {
        await source.prepareWebviewSurface();
        await source.waitForDraw();
        return { generation, epoch };
      },
      validate: receipt => receipt.generation === generation && receipt.epoch === epoch,
      drain: async () => {
        // Start buffering target input before hiding source. The existing widget
        // keeps its generation and reservation counter; hiding is reversible.
        source.beginWebviewInputHandoff();
        webviewHandoff = true;
        await tick();
        source.focusWebviewHandoff();
        suspended = true;
        await drainSource();
      },
      commit: async () => {
        if (!current()) throw new Error('WebView staging superseded');
        committedWebview = true; committedNative = false;
        stagingWebview = false; webviewHandoff = false;
        source.commitWebviewSurface();
        cleanupSurface?.(); cleanupSurface = null; syncSurface = null;
        // No await before ownership flip; late native callbacks remain scoped to
        // the old host generation and are disposed only after accepted ACK drain.
        disposePaste();
        hostAdapter = null;
        // Ownership is already committed; detach failure must not run source rollback.
        void host.dispose().catch(error => onError?.(nativeSurfaceError(error).message));
        if (canFocusSurface()) source.focus();
      },
      rollback: async () => {
        const retained = () => !disposed && !transitionClosing && controller === source && hostAdapter === host && host.generation() === generation;
        const restore = async () => {
          if (!retained()) return;
          await source.rollbackWebviewSurface();
          if (!retained()) return;
          if (suspended) await host.restoreAfterWebview();
          if (!retained()) return;
          stagingWebview = false; webviewHandoff = false; rollbackPending = false;
          host.setFocusAllowed(canFocusSurface());
          syncSurface?.();
          onRecovered?.(source, 'native-surface', request);
        };
        // Return the failed transaction promptly, but retain ownership and target
        // buffering until the background ACK-only recovery permits rollback.
        try {
          if (suspended) await drainSource();
          await restore();
        }
        catch (error) {
          if (!retained()) return;
          rollbackPending = true;
          void recoverNativeInputRollback({ current: retained,
            drain: drainSource, rollback: restore,
          }).catch(error => { if (retained()) onError?.(nativeSurfaceError(error).message); });
          throw error;
        }
      },
    });
  }

  export function cancelViewTransition() { transitionClosing = true; reverseEpoch++; inputQueue?.dispose(); }

  export function isRetainedNativeReady(expected: NativeTerminalController): boolean {
    return !disposed && !stagingWebview && !committedWebview && controller === expected && !!hostAdapter && surfaceReady && !surfaceError && !ackRecovering && !rollbackPending;
  }

  export function isRetainedWebviewReady(expected: NativeTerminalController): boolean {
    return !disposed && !releasingInput && !stagingNative && !committedNative && !surfaceError
      && backend === 'webview' && controller === expected && !expected.isDisposed();
  }

  export function getController(): NativeTerminalController | null {
    return controller;
  }

  export function focus() {
    if (((backend === 'native-surface' && !committedWebview) || committedNative)) {
      hostAdapter?.setFocusAllowed(!!canFocusSurface() && surfaceReady);
      if (canFocusSurface() && surfaceReady) void hostAdapter?.focus().catch(fail);
    } else if (canFocusSurface()) controller?.focus();
  }

  /** Await OS view detach before mounting another view of this same engine. */
  export async function release() {
    releasingInput = true;
    // A view switch is not session cancellation: keep generation and listener alive
    // until the frozen host's final reserved sequence has reached writer + ACK.
    cleanupSurface?.();
    cleanupSurface = null;
    syncSurface = null;
    hostAdapter?.setDrawVisible(false);
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([
        (async () => {
          await startup;
          if (hostAdapter && inputQueue) {
            await quiesceAndDrainNativeInput(hostAdapter, inputQueue);
          }
        })(),
        new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error('native input handoff timeout')), 5000); }),
      ]);
    } catch (error) {
      // Do not detach, cancel accepted writes, or claim the replacement is ready.
      const message = nativeSurfaceError(error).message;
      surfaceError = message;
      onError?.(message);
      throw error;
    } finally { clearTimeout(timer); }
    disposed = true;
    disposePaste();
    await hostAdapter?.dispose();
    await controller?.dispose();
  }

  onMount(() => {
    globalOccluded = document.body.dataset.nativeViewOccluded === 'true';
    if (!canvas) return;
    const ctl = new NativeTerminalController({
      sessionId,
      // Capability installed only by the backend's debug-assertions + E2E-env harness.
      // Release builds install no sink; no terminal content or input is exposed.
      onDrawReceipt: receipt => {
        const observer = (window as unknown as {
          __aerotabDebugDrawObserver?: {
            receipt: (r: import('../lib/termgrid/controller').DrawReceipt) => void;
            identity?: (session: string, controller: object, canvas: object) => void;
          };
        }).__aerotabDebugDrawObserver;
        if (canvas) observer?.identity?.(sessionId, ctl, canvas);
        observer?.receipt(receipt);
      },
      nativeSurface: ((backend === 'native-surface' && !committedWebview) || committedNative),
      rpc,
      canvas,
      input,
      fontFamily,
      fontSize,
      fontWeight,
      fontWeightBold,
      linePadding,
      scrollback,
      cursorBlink,
      altIsMeta,
      ligatures,
      theme,
      renderer,
      translucent,
      transferDetect,
      linkModifier,
      copyOnSelect,
      writeInput,
      onInputBackpressure: blocked => { inputBackpressure = blocked; },
      onBell,
      onTitle,
      onExit,
      onActivity,
      onRaw,
      onTrzszDetected,
      onZmodemHint,
      onClipboard: (bytes) => {
        const text = new TextDecoder().decode(bytes);
        void navigator.clipboard.writeText(text).catch(() => {});
      },
    });
    controller = ctl;
    startup = ctl.start().then(async () => {
      if (disposed) return;
      if (((backend === 'native-surface' && !committedWebview) || committedNative)) await mountSurface();
      if (disposed || surfaceError) return;
      if (backend === 'webview') {
        await ctl.waitForDraw();
        if (disposed || surfaceError) return;
        onReady?.(ctl);
        if (visible && active && !occluded && !globalOccluded) ctl.focus();
      }
    }).catch((error: unknown) => { if (!disposed) fail(error); });
  });

  $effect(() => {
    controller?.setVisible(visible && !occluded && !globalOccluded);
    void visible; void occluded; void globalOccluded; void active;
    hostAdapter?.setFocusAllowed(!!canFocusSurface());
    syncSurface?.();
  });

  $effect(() => {
    if (!active || !visible || occluded || globalOccluded) controller?.blur();
  });

  $effect(() => {
    void theme; void fontFamily; void fontSize; void fontWeight; void linePadding;
    if (((backend === 'native-surface' && !committedWebview) || committedNative)) syncSurface?.();
  });

  $effect(() => {
    if (theme) controller?.setTheme(theme);
  });

  $effect(() => {
    controller?.setFont({
      family: fontFamily,
      size: fontSize,
      weight: fontWeight,
      weightBold: fontWeightBold,
      linePadding,
    });
  });

  $effect(() => {
    controller?.setLinkModifier(linkModifier);
  });

  $effect(() => {
    controller?.setCopyOnSelect(copyOnSelect);
  });

  $effect(() => {
    controller?.setWriteInput(writeInput);
  });

  $effect(() => {
    controller?.setTransferDetect(transferDetect);
  });

  onDestroy(() => {
    disposed = true;
    disposePaste();
    cleanupSurface?.();
    syncSurface = null;
    void hostAdapter?.dispose().catch(console.warn);
    void controller?.dispose();
    controller = null;
  });
</script>

<div bind:this={surface} data-native-session={sessionId} data-native-input-active={active && visible && !occluded && !globalOccluded && !surfaceError && !rollbackPending} data-terminal-view-backend={backend} class="native-term absolute inset-0 min-h-0 min-w-0 overflow-hidden">
  {#if inputBackpressure}
    <div role="alert" class="absolute bottom-2 left-2 right-2 text-xs text-[var(--color-warning)]">{i18n.t('terminal.inputBackpressure')}</div>
  {/if}
  {#if ackRecovering || rollbackPending}
    <div role="status" class="absolute top-2 left-2 right-2 text-xs text-[var(--color-warning)] break-words">{i18n.t('terminal.nativeAckRecovery')}</div>
  {/if}
  {#if surfaceError}
    <div role="alert" class="absolute top-2 left-2 right-2 text-xs text-[var(--color-danger)] break-words">{surfaceError}</div>
  {/if}
  <canvas
    hidden={((backend === 'native-surface' && !committedWebview) || committedNative) && !webviewHandoff}
    aria-hidden={((backend === 'native-surface' && !committedWebview) || committedNative)}
    bind:this={canvas}
    class="block w-full h-full outline-none"
    tabindex={((backend === 'native-surface' && !committedWebview) || committedNative) ? -1 : 0}
    style:display={((backend === 'native-surface' && !committedWebview) || committedNative) && !webviewHandoff ? 'none' : undefined}
  ></canvas>
  <!-- IME / composition target; visually hidden but focusable via controller.focus() -->
  <textarea
    bind:this={input}
    hidden={((backend === 'native-surface' && !committedWebview) || committedNative) && !webviewHandoff}
    disabled={((backend === 'native-surface' && !committedWebview) || committedNative) && !webviewHandoff}
    class="native-term-ime"
    autocomplete="off"
    autocapitalize="off"
    spellcheck="false"
    tabindex="0"
    aria-label="Terminal input"
  ></textarea>
</div>

<style>
  /*
   * Mirror xterm's helper textarea: focusable, in-flow stacking, no z-index:-1.
   * WebView2 drops key events to elements painted under the canvas (z-index < 0).
   * pointer-events:none so clicks hit the canvas; controller.focus() still works.
   */
  .native-term-ime {
    position: absolute;
    left: 0;
    top: 0;
    width: 20px;
    height: 20px;
    opacity: 0;
    border: 0;
    padding: 0;
    margin: 0;
    resize: none;
    overflow: hidden;
    caret-color: transparent;
    color: transparent;
    background: transparent;
    z-index: 1;
    pointer-events: none;
    outline: none;
  }
</style>
