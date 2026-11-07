/**
 * Native terminal controller: attaches the Rust engine, receives binary
 * frames over a Tauri Channel, and drives a Canvas2D / WebGL grid renderer.
 */

import { tracePaste } from '../nativePasteTrace';
import type { RpcClient } from '../rpc';
import type { TerminalSnapshotEnvelope } from '../types';
import { createWebviewDrawBarrier, type WebviewDrawTicket } from './webviewDrawBarrier';
import { b64encode } from '../rpc';
import { Canvas2DRenderer } from './canvas2d';
import type { CursorState } from './cellStyle';
import {
  applyFrame,
  FLAG_BELL,
  FLAG_CLIPBOARD,
  FLAG_EXITED,
  FLAG_RAW_BYPASS,
  FLAG_TITLE,
  FLAG_TRZSZ_DETECTED,
  FLAG_ZMODEM_HINT,
  GridState,
  KIND_ACTIVITY,
  KIND_RAW,
  type FrameHeader,
} from './frame';
import { measureCell, type FontSpec } from './fontMetrics';
import { encodeKeyEvent, encodePaste } from './keyEncoder';
import {
  encodeMouseEvent,
  mouseReportingActive,
  wheelButton,
  type MouseButton,
} from './mouseEncoder';
import { Palette, type ThemeColors } from './palette';
import type { GridRenderer } from './renderer';
import { createFrameChannel, tauriInvoke } from './tauriChannel';
import { WebGLRenderer } from './webgl';

export type NativeRendererPref = 'auto' | 'webgl' | 'canvas2d';
export type LinkModifier = 'none' | 'ctrl' | 'alt' | 'meta';

export interface DrawReceipt {
  readonly sessionId: string;
  readonly generation: string;
  readonly revision: number;
  readonly appliedOutputSeq: string;
  readonly drawnOutputSeq: string;
}

export interface NativeTerminalOptions {
  /** Read-only diagnostic sink. No input, scene bytes, or readiness authority. */
  onDrawReceipt?: (receipt: Readonly<DrawReceipt>) => void;
  sessionId: string;
  /** Consume shared-core metadata/raw frames only; OS host owns paint/input/size. */
  nativeSurface?: boolean;
  rpc: RpcClient;
  canvas: HTMLCanvasElement;
  /** Hidden textarea for IME / focus. */
  input?: HTMLTextAreaElement | null;
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
  transferDetect?: boolean;
  /** Prefer Canvas2D when window opacity < 100 (WebGL + translucency issues). */
  translucent?: boolean;
  /** Match Settings → Terminal → link modifier (xterm WebLinksAddon). */
  linkModifier?: LinkModifier;
  /** Copy selection to clipboard when the pointer is released. */
  copyOnSelect?: boolean;
  /**
   * Override keyboard/paste PTY writes (e.g. route through trzsz filter).
   * Receives UTF-8 text; default is `session.write`.
   */
  writeInput?: (data: string) => void | Promise<void>;
  onInputBackpressure?: (blocked: boolean) => void;
  onBell?: () => void;
  onTitle?: (title: string) => void;
  onExit?: () => void;
  onActivity?: () => void;
  onRaw?: (bytes: Uint8Array) => void;
  onTrzszDetected?: () => void;
  onZmodemHint?: () => void;
  onClipboard?: (bytes: Uint8Array) => void;
}

export class NativeTerminalController {
  private opts: NativeTerminalOptions;
  private rpc: RpcClient;
  private sessionId: string;
  private grid = new GridState();
  private palette: Palette;
  private renderer: GridRenderer | null = null;
  private header: FrameHeader | null = null;
  private mode = 0;
  private viewGeneration = crypto.randomUUID();
  private configRevision = 0;
  private drawBarrier = createWebviewDrawBarrier(this.viewGeneration);
  private pendingDraw: WebviewDrawTicket | null = null;
  private captureInFlight = false;
  private awaitingFullBase = false;
  private inputPaused = false;
  private inputWrites = new Set<Promise<void>>();
  private pausedWrites: { data: string; resolve: () => void; reject: (error: unknown) => void }[] = [];

  private pausedBytes = 0;
  private inputBackpressure = false;
  private reportInputBackpressure(blocked: boolean) {
    if (this.inputBackpressure === blocked) return;
    this.inputBackpressure = blocked;
    this.opts.onInputBackpressure?.(blocked);
  }
  private flushPausedWrites() {
    const pending = this.pausedWrites.splice(0);
    this.pausedBytes = 0; this.reportInputBackpressure(false);
    this.inputPaused = false;
    for (const item of pending) void this.writePty(item.data).then(item.resolve, item.reject);
  }

  isDisposed() { return this.disposed; }

  async quiesceWebviewInput(): Promise<void> {
    if (this.composing) throw new Error('WebView handoff unavailable during IME composition');
    this.inputPaused = true;
    await Promise.all([...this.inputWrites]);
  }

  resumeWebviewInput() { this.flushPausedWrites(); }

  /** Keep the existing Channel/parser owner, but retire canvas input and paint. */
  commitNativeSurface() {
    // Flush keystrokes accepted during the final await through the same ordered
    // writer before the native event loop can deliver newly enabled input.
    this.flushPausedWrites();
    this.inputPaused = true;
    this.opts.nativeSurface = true;
    this.resizeObs?.disconnect();
    if (this.raf != null) cancelAnimationFrame(this.raf);
    this.raf = null;
    this.renderer?.dispose();
    this.renderer = null;
  }
  private stagingWebview = false;
  private webviewHandoff = false;
  private webviewInputBound = false;

  /** Reuse the sole frame sink; prepare paint without enabling retired DOM input. */
  async prepareWebviewSurface() {
    if (this.disposed || !this.opts.nativeSurface || this.stagingWebview) throw new Error('WebView staging unavailable');
    this.stagingWebview = true;
    this.createRenderer();
  }

  beginWebviewInputHandoff() {
    if (!this.stagingWebview || this.disposed) throw new Error('WebView target unavailable');
    this.inputPaused = true;
    this.webviewHandoff = true;
    if (!this.webviewInputBound) { this.bindInput(); this.webviewInputBound = true; }
  }

  focusWebviewHandoff() {
    if (!this.webviewHandoff || this.disposed) return;
    this.focused = true;
    (this.opts.input ?? this.opts.canvas).focus({ preventScroll: true });
  }

  commitWebviewSurface() {
    if (!this.stagingWebview || this.disposed) throw new Error('WebView target unavailable');
    this.opts.nativeSurface = false;
    this.stagingWebview = false;
    this.webviewHandoff = false;
    this.flushPausedWrites();
    this.requestedGeometry = '';
    this.observeResize();
    this.fit();
  }

  async rollbackWebviewSurface() {
    this.stagingWebview = false;
    this.webviewHandoff = false;
    // These inputs were accepted by the target during handoff, not stale DOM.
    const pending = this.pausedWrites.splice(0);
    this.pausedBytes = 0; this.reportInputBackpressure(false);
    await Promise.all(pending.map(async item => {
      try { await this.writeOrderedInput(item.data); item.resolve(); }
      catch (error) { item.reject(error); throw error; }
    }));
    this.renderer?.dispose(); this.renderer = null;
    if (this.raf != null) cancelAnimationFrame(this.raf);
    this.raf = null;
    this.inputPaused = true;
  }

  private alive = true;
  private focused = false;
  private cursorBlinkOn = true;
  private blinkTimer: number | null = null;
  private raf: number | null = null;
  private needsPaint = false;
  private selecting = false;
  private disposed = false;
  private composing = false;
  private mouseDown = false;
  private mouseButton: MouseButton = 3;
  private ptrMoved = false;
  private hoverUrl: string | null = null;
  private hoverTimer: number | null = null;
  private clickOpensLink = false;
  private boundCompStart = () => { this.composing = true; };
  private boundCompEnd = (e: CompositionEvent) => { void this.onCompositionEnd(e).catch(() => {}); };
  private boundInput = (e: Event) => { void this.onImeInput(e as InputEvent).catch(() => {}); };
  private cols = 80;
  private rows = 24;
  private metrics = measureCell({
    family: 'monospace',
    size: 13,
    weight: 400,
    weightBold: 700,
    linePadding: 0,
    dpr: 1,
  });
  private spec: FontSpec = {
    family: 'monospace',
    size: 13,
    weight: 400,
    weightBold: 700,
    linePadding: 0,
    dpr: 1,
  };
  private resizeObs: ResizeObserver | null = null;
  private requestedGeometry = '';
  private boundKey = (e: KeyboardEvent) => { void this.onKey(e).catch(() => {}); };
  private boundPaste = (e: ClipboardEvent) => this.onPaste(e);
  private boundWinKey = (e: KeyboardEvent) => {
    if (!this.shouldHandleGlobalInput(e.target)) return;
    void this.onKey(e).catch(() => {});
  };
  private boundWinPaste = (e: ClipboardEvent) => {
    if (!this.shouldHandleGlobalInput(e.target)) return;
    this.onPaste(e);
  };
  private boundWheel = (e: WheelEvent) => void this.onWheel(e);
  private boundPtrDown = (e: PointerEvent) => void this.onPtrDown(e);
  private boundPtrMove = (e: PointerEvent) => void this.onPtrMove(e);
  private boundPtrUp = (e: PointerEvent) => void this.onPtrUp(e);
  private boundPtrLeave = () => {
    this.hoverUrl = null;
    this.opts.canvas.style.cursor = '';
  };

  constructor(opts: NativeTerminalOptions) {
    this.opts = opts;
    this.rpc = opts.rpc;
    this.sessionId = opts.sessionId;
    this.palette = new Palette(opts.theme);
  }

  async start(): Promise<void> {
    this.applyFontFromOpts();
    if (!this.opts.nativeSurface) this.createRenderer();
    await this.rpc.call('terminal.attach', {
      id: this.sessionId,
      cols: this.cols,
      rows: this.rows,
      scrollback: this.opts.scrollback ?? 10_000,
      transferDetect: this.opts.transferDetect ?? false,
    });
    await this.connectFrames();
    await this.rpc.call('terminal.snapshot', { id: this.sessionId });
    if (!this.opts.nativeSurface) {
      this.bindInput();
      this.webviewInputBound = true;
      this.observeResize();
      this.startBlink();
      this.schedulePaint();
    }
  }

  /** Fixed capture target; stream frames cannot substitute for its actual draw. */
  async waitForDraw(timeoutMs = 3000): Promise<void> {
    if (this.opts.nativeSurface && !this.stagingWebview) throw new Error('WebView draw gate cannot acknowledge native surface');
    if (this.captureInFlight) throw new Error('WebView draw capture already in progress');
    this.captureInFlight = true;
    const revision = ++this.configRevision;
    const drawn = this.drawBarrier.begin(revision);
    // Attach rejection handlers immediately, including while RPC is pending.
    let timer: ReturnType<typeof setTimeout> | undefined;
    const deadline = new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error('WebView draw timeout')), timeoutMs); });
    const operation = (async () => {
      const envelope = await this.rpc.call<TerminalSnapshotEnvelope>('terminal.snapshotEnvelope', { id: this.sessionId });
      if (this.disposed || revision !== this.configRevision) throw new Error('WebView draw generation/revision superseded');
      if (envelope.version !== 1 || envelope.frameVersion !== 1 || !Array.isArray(envelope.bytes)
        || !envelope.bytes.every(n => Number.isInteger(n) && n >= 0 && n <= 255)) throw new Error('Invalid snapshot envelope');
      const grid = new GridState();
      const frame = applyFrame(new Uint8Array(envelope.bytes).buffer, grid);
      if (frame.header.kind !== 0) throw new Error('Snapshot envelope requires full frame');
      this.pendingDraw = this.drawBarrier.apply(this.viewGeneration, revision, envelope.outputSeq);
      this.grid = grid;
      this.header = frame.header;
      this.mode = frame.header.mode;
      this.cols = frame.header.cols;
      this.rows = frame.header.rows;
      this.schedulePaint();
    })();
    try {
      await Promise.race([Promise.all([operation, drawn]), deadline]);
    } catch (error) {
      ++this.configRevision; // revoke late RPC completion even after timeout
      this.drawBarrier.fail(error instanceof Error ? error : new Error(String(error)));
      throw error;
    } finally {
      clearTimeout(timer);
      this.pendingDraw = null;
      this.captureInFlight = false;
      this.awaitingFullBase = true;
      // Discarded stream damage needs a new full base; core/parser stays attached.
      if (!this.disposed) void this.rpc.call('terminal.snapshot', { id: this.sessionId });
    }
  }

  private invalidateDraw() {
    if (this.captureInFlight) {
      ++this.configRevision;
      this.pendingDraw = null;
      this.drawBarrier.fail(new Error('WebView draw configuration superseded'));
    }
  }

  setTheme(theme: ThemeColors) {
    this.invalidateDraw();
    this.palette.setTheme(theme);
    this.schedulePaint();
  }

  setFont(partial: Partial<FontSpec> & { family?: string; size?: number }) {
    this.invalidateDraw();
    Object.assign(this.opts, {
      fontFamily: partial.family ?? this.opts.fontFamily,
      fontSize: partial.size ?? this.opts.fontSize,
      fontWeight: partial.weight ?? this.opts.fontWeight,
      fontWeightBold: partial.weightBold ?? this.opts.fontWeightBold,
      linePadding: partial.linePadding ?? this.opts.linePadding,
    });
    this.applyFontFromOpts();
    this.renderer?.setFont(this.spec, this.metrics);
    this.fit();
  }

  setVisible(visible: boolean) {
    void this.rpc.call('terminal.setVisible', { id: this.sessionId, visible });
    if (visible) {
      void this.rpc.call('terminal.snapshot', { id: this.sessionId });
      this.schedulePaint();
    }
  }

  async selectionText(): Promise<string> {
    const r = await this.rpc.call<{ text: string }>('terminal.selectionText', {
      id: this.sessionId,
    });
    return r.text ?? '';
  }

  async selectAll() {
    await this.rpc.call('terminal.selectAll', { id: this.sessionId });
  }

  async clearSelection() {
    await this.rpc.call('terminal.selectionClear', { id: this.sessionId });
  }

  async search(query: string, opts: {
    regex?: boolean;
    caseSensitive?: boolean;
    direction?: 'next' | 'prev';
  } = {}) {
    return this.rpc.call('terminal.search', {
      id: this.sessionId,
      query,
      regex: opts.regex ?? false,
      caseSensitive: opts.caseSensitive ?? false,
      direction: opts.direction ?? 'next',
    });
  }

  /** Feed display bytes (trzsz filter output) into the engine. */
  async feed(bytes: Uint8Array) {
    await this.rpc.call('terminal.feed', {
      id: this.sessionId,
      data: b64encode(bytes),
    });
  }

  async setRawBypass(on: boolean) {
    await this.rpc.call('terminal.setRawBypass', { id: this.sessionId, on });
  }

  /** Paste text, honouring bracketed-paste mode from the last frame. */
  async paste(text: string) {
    await this.writePty(encodePaste(text, this.mode));
  }

  /** Only the host/policy-authorized paste route may write after DOM retirement. */
  async pasteFromNative(text: string, valid: () => Promise<boolean>) {
    if (!(await valid()) || this.disposed || !this.opts.nativeSurface) {
      tracePaste('authorization', this.disposed ? 'disposed' : !this.opts.nativeSurface ? 'not-native' : 'invalid'); return;
    }
    tracePaste('authorization', 'accepted');
    tracePaste('writer', 'start');
    await this.writeOrderedInput(encodePaste(text, this.mode));
    tracePaste('writer', 'completed');
  }

  setLinkModifier(mod: LinkModifier) {
    this.opts.linkModifier = mod;
  }

  setCopyOnSelect(on: boolean) {
    this.opts.copyOnSelect = on;
  }

  setWriteInput(fn: ((data: string) => void | Promise<void>) | undefined) {
    this.opts.writeInput = fn;
  }

  setTransferDetect(on: boolean) {
    this.opts.transferDetect = on;
    void this.rpc.call('terminal.setTransferDetect', { id: this.sessionId, on });
  }

  focus() {
    if (this.opts.nativeSurface) return; // Native focus remains click-to-focus.
    this.focused = true;
    // Prefer the IME textarea so CJK composition targets a real editable.
    // Fall back to the canvas if WebView refuses the hidden textarea.
    const ime = this.opts.input;
    const canvas = this.opts.canvas;
    try {
      ime?.focus({ preventScroll: true });
    } catch {
      /* older WebView */
      ime?.focus();
    }
    if (ime && document.activeElement !== ime) {
      try {
        canvas.focus({ preventScroll: true });
      } catch {
        canvas.focus();
      }
    }
    this.schedulePaint();
  }

  blur() {
    this.focused = false;
    this.schedulePaint();
  }

  /**
   * Tear down the frontend controller.
   * By default the Rust VT engine is kept so a layout remount (split/reparent)
   * can `terminal.attach` + snapshot and restore scrollback. Pass
   * `{ detachEngine: true }` when handing off to xterm (after exportText) or
   * when the session is intentionally leaving the native path.
   * Session close still removes the engine via `session.close`.
   */
  async dispose(opts: { detachEngine?: boolean } = {}) {
    if (this.disposed) return;
    this.disposed = true;
    for (const item of this.pausedWrites.splice(0)) item.reject(new Error('session view disposed'));
    this.pausedBytes = 0; this.reportInputBackpressure(false);
    this.drawBarrier.dispose();
    this.pendingDraw = null;
    this.resizeObs?.disconnect();
    this.resizeObs = null;
    if (this.blinkTimer != null) window.clearInterval(this.blinkTimer);
    if (this.raf != null) cancelAnimationFrame(this.raf);
    const canvas = this.opts.canvas;
    canvas.removeEventListener('keydown', this.boundKey as EventListener);
    canvas.removeEventListener('paste', this.boundPaste as EventListener);
    canvas.removeEventListener('wheel', this.boundWheel);
    canvas.removeEventListener('pointerdown', this.boundPtrDown);
    canvas.removeEventListener('pointermove', this.boundPtrMove);
    canvas.removeEventListener('pointerup', this.boundPtrUp);
    canvas.removeEventListener('pointerleave', this.boundPtrLeave);
    window.removeEventListener('keydown', this.boundWinKey, true);
    window.removeEventListener('paste', this.boundWinPaste, true);
    const ime = this.opts.input;
    if (ime) {
      ime.removeEventListener('keydown', this.boundKey as EventListener);
      ime.removeEventListener('paste', this.boundPaste as EventListener);
      ime.removeEventListener('compositionstart', this.boundCompStart);
      ime.removeEventListener('compositionend', this.boundCompEnd as EventListener);
      ime.removeEventListener('input', this.boundInput as EventListener);
    }
    if (this.hoverTimer != null) window.clearTimeout(this.hoverTimer);
    this.renderer?.dispose();
    this.renderer = null;
    if (!opts.detachEngine) return;
    try {
      await this.rpc.call('terminal.detach', { id: this.sessionId });
    } catch {
      /* session may already be gone */
    }
  }

  // ---- private ----

  private applyFontFromOpts() {
    const dpr = window.devicePixelRatio || 1;
    this.spec = {
      family: this.opts.fontFamily ?? 'JetBrains Mono, Menlo, monospace',
      size: this.opts.fontSize ?? 15,
      weight: this.opts.fontWeight ?? 400,
      weightBold: this.opts.fontWeightBold ?? 700,
      linePadding: this.opts.linePadding ?? 0,
      dpr,
    };
    this.metrics = measureCell(this.spec);
  }

  private createRenderer() {
    const pref = this.opts.renderer ?? 'auto';
    const force2d = pref === 'canvas2d' || !!this.opts.translucent || !!this.opts.ligatures;
    if (!force2d) {
      try {
        this.renderer = new WebGLRenderer(this.opts.canvas, this.spec, this.metrics);
        return;
      } catch (e) {
        console.warn('[termgrid] webgl unavailable, falling back to canvas2d', e);
      }
    }
    this.renderer = new Canvas2DRenderer(this.opts.canvas);
    this.renderer.setFont(this.spec, this.metrics);
  }

  private async connectFrames() {
    const invoke = tauriInvoke();
    const channel = createFrameChannel((buf) => this.onFrame(buf));
    if (invoke && channel) {
      await invoke('terminal_frames', {
        sessionId: this.sessionId,
        onFrame: channel,
      });
      return;
    }
    if (this.opts.nativeSurface) throw new Error('native_surface_unavailable: Tauri frame Channel unavailable');
    // Dev / HTTP fallback: poll snapshots are not available; warn once.
    console.warn('[termgrid] Tauri Channel unavailable — native frames will not stream');
  }

  private onFrame(buf: ArrayBuffer) {
    if (this.disposed) return;
    try {
      // During the finite snapshot draw, preserve its exact grid. Metadata/raw
      // still drain normally; subsequent full snapshot restores stream base.
      const waitingForBase = this.awaitingFullBase && new Uint8Array(buf)[1] !== 0;
      const isolated = this.captureInFlight || waitingForBase;
      const decoded = applyFrame(buf, isolated ? new GridState() : this.grid);
      if (!isolated) {
        this.awaitingFullBase = false;
        this.header = decoded.header;
        this.mode = decoded.header.mode;
        this.cols = decoded.header.cols || this.cols;
        this.rows = decoded.header.rows || this.rows;
      }
      const f = decoded.header.flags;
      if (f & FLAG_BELL) this.opts.onBell?.();
      if (f & FLAG_TITLE && decoded.header.title != null) this.opts.onTitle?.(decoded.header.title);
      if (f & FLAG_EXITED) {
        this.alive = false;
        this.opts.onExit?.();
      }
      if (f & FLAG_CLIPBOARD && decoded.header.clipboard) {
        this.opts.onClipboard?.(decoded.header.clipboard);
      }
      if (f & FLAG_TRZSZ_DETECTED) this.opts.onTrzszDetected?.();
      if (f & FLAG_ZMODEM_HINT) this.opts.onZmodemHint?.();
      if (decoded.header.kind === KIND_RAW && decoded.raw) {
        this.opts.onRaw?.(decoded.raw);
        return;
      }
      if (decoded.header.kind === KIND_ACTIVITY) {
        this.opts.onActivity?.();
        return;
      }
      if (f & FLAG_RAW_BYPASS) {
        // still paint if we have grid updates
      }
      this.schedulePaint();
    } catch (e) {
      console.warn('[termgrid] bad frame', e);
    }
  }

  private schedulePaint() {
    if ((this.opts.nativeSurface && !this.stagingWebview) || this.disposed) return;
    this.needsPaint = true;
    if (this.raf != null) return;
    this.raf = requestAnimationFrame(() => {
      this.raf = null;
      if (!this.needsPaint || !this.renderer) return;
      this.needsPaint = false;
      const ticket = this.pendingDraw;
      const cursor = this.cursorState();
      try {
        // Resize and draw the received grid in the same RAF. Never expose an
        // erased backing store while waiting for the backend's resized frame.
        this.renderer.resize(this.grid.cols, this.grid.rows, this.metrics, this.spec.dpr);
        this.renderer.render({
          grid: this.grid,
          palette: this.palette,
          metrics: this.metrics,
          spec: this.spec,
          cursor,
          dpr: this.spec.dpr,
          ligatures: !!this.opts.ligatures && this.renderer.kind === 'canvas2d',
        });
        if (ticket && !this.disposed && this.drawBarrier.complete(ticket)) {
          // Complete readiness first; observer failures cannot change draw success.
          try {
            this.opts.onDrawReceipt?.(Object.freeze({
              sessionId: this.sessionId, generation: ticket.generation, revision: ticket.revision,
              appliedOutputSeq: ticket.outputSeq.toString(), drawnOutputSeq: ticket.outputSeq.toString(),
            }));
          } catch { /* Diagnostics must never affect rendering or readiness. */ }
        }
      } catch (error) {
        this.drawBarrier.fail(error instanceof Error ? error : new Error(String(error)));
        console.warn('[termgrid] render failed', error);
      }
    });
  }

  private cursorState(): CursorState | null {
    if (!this.header) return null;
    return {
      col: this.header.cursorCol,
      line: this.header.cursorLine,
      shape: this.header.cursorShape,
      visible: this.cursorBlinkOn || !(this.opts.cursorBlink ?? true),
      focused: this.focused,
    };
  }

  private startBlink() {
    if (!(this.opts.cursorBlink ?? true)) return;
    this.blinkTimer = window.setInterval(() => {
      this.cursorBlinkOn = !this.cursorBlinkOn;
      this.schedulePaint();
    }, 530);
  }

  private bindInput() {
    const ime = this.opts.input;
    const canvas = this.opts.canvas;
    // Bind keys on both targets: IME may fail to hold focus on WebView2
    // (opacity/stacking), so the canvas must also accept keydown.
    canvas.tabIndex = 0;
    canvas.addEventListener('keydown', this.boundKey as EventListener);
    canvas.addEventListener('paste', this.boundPaste as EventListener);
    if (ime) {
      ime.tabIndex = 0;
      ime.addEventListener('keydown', this.boundKey as EventListener);
      ime.addEventListener('paste', this.boundPaste as EventListener);
      ime.addEventListener('compositionstart', this.boundCompStart);
      ime.addEventListener('compositionend', this.boundCompEnd as EventListener);
      ime.addEventListener('input', this.boundInput as EventListener);
    }
    // Capture-phase window listener: when we believe we are focused but the
    // activeElement drifted (common after opacity/z-index quirks), still type.
    window.addEventListener('keydown', this.boundWinKey, true);
    window.addEventListener('paste', this.boundWinPaste, true);
    canvas.addEventListener('wheel', this.boundWheel, { passive: false });
    canvas.addEventListener('pointerdown', this.boundPtrDown);
    canvas.addEventListener('pointermove', this.boundPtrMove);
    canvas.addEventListener('pointerup', this.boundPtrUp);
    canvas.addEventListener('pointerleave', this.boundPtrLeave);
  }

  private observeResize() {
    const host = this.opts.canvas.parentElement ?? this.opts.canvas;
    this.resizeObs = new ResizeObserver(() => this.fit());
    this.resizeObs.observe(host);
    this.fit();
  }

  fit() {
    if (this.disposed || this.opts.nativeSurface) return;
    const host = this.opts.canvas.parentElement ?? this.opts.canvas;
    if (host.clientWidth < 2 || host.clientHeight < 2) return;
    const dpr = window.devicePixelRatio || 1;
    if (dpr !== this.spec.dpr) {
      this.invalidateDraw();
      this.spec = { ...this.spec, dpr };
      this.metrics = measureCell(this.spec);
      this.renderer?.setFont(this.spec, this.metrics);
    }
    const cols = Math.max(2, Math.floor(host.clientWidth / this.metrics.width));
    const rows = Math.max(1, Math.floor(host.clientHeight / this.metrics.height));
    const geometry = `${cols}:${rows}:${this.metrics.deviceWidth}:${this.metrics.deviceHeight}`;
    if (geometry !== this.requestedGeometry) {
      this.invalidateDraw();
      this.requestedGeometry = geometry;
      // cols/rows and grid describe received output, not an in-flight request.
      // Keeping them intact avoids blank frames and stale-frame resize loops.
      void this.rpc.call('terminal.resize', {
        id: this.sessionId,
        cols,
        rows,
        cellWidth: this.metrics.deviceWidth,
        cellHeight: this.metrics.deviceHeight,
      });
    }
    this.schedulePaint();
  }

  /** True when this pane owns keyboard (focused) and the event isn't for another editable. */
  private shouldHandleGlobalInput(target: EventTarget | null): boolean {
    if (!this.focused || !this.alive || this.disposed || (this.opts.nativeSurface && !this.webviewHandoff)) return false;
    const el = target instanceof Element ? target : null;
    if (!el) return true;
    if (el === this.opts.canvas || el === this.opts.input) return true;
    // Another pane/input has real focus — don't steal.
    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
      return el === this.opts.input;
    }
    if (el instanceof HTMLElement && el.isContentEditable) return false;
    // Active element drifted to body/document while we still think we're focused.
    const active = document.activeElement;
    if (active === this.opts.canvas || active === this.opts.input) return true;
    if (active === document.body || active === document.documentElement || active == null) return true;
    return false;
  }

  private async onKey(ev: KeyboardEvent) {
    if (!this.alive || this.disposed || (this.opts.nativeSurface && !this.webviewHandoff)) return;
    // App hotkeys (window capture) run first and set defaultPrevented for
    // Ctrl+F / Ctrl+= / Ctrl+Shift+C / etc. Do not swallow bare Ctrl+C/D —
    // those must reach the PTY (SIGINT / EOF).
    if (ev.defaultPrevented) return;
    if (this.composing || ev.isComposing || ev.key === 'Process') return;
    const bytes = encodeKeyEvent(ev, {
      mode: this.mode,
      altIsMeta: this.opts.altIsMeta ?? false,
    });
    if (bytes == null) return;
    ev.preventDefault();
    ev.stopPropagation();
    await this.writePty(bytes);
  }

  private onPaste(ev: ClipboardEvent) {
    const text = ev.clipboardData?.getData('text');
    if (!text) return;
    ev.preventDefault();
    void this.writePty(encodePaste(text, this.mode)).catch(() => {});
  }

  private async onCompositionEnd(ev: CompositionEvent) {
    this.composing = false;
    const text = ev.data ?? '';
    const ime = this.opts.input;
    if (ime) ime.value = '';
    if (text) await this.writePty(text);
  }

  private async onImeInput(ev: InputEvent) {
    // Some engines fire `input` with insertText outside composition (e.g. emoji).
    if (this.composing || ev.isComposing) return;
    if (ev.inputType !== 'insertText' || !ev.data) return;
    const ime = this.opts.input;
    if (ime) ime.value = '';
    await this.writePty(ev.data);
  }

  private async onWheel(ev: WheelEvent) {
    ev.preventDefault();
    // Ctrl/Cmd+wheel → font zoom (same as Ctrl+= / Ctrl+- hotkeys).
    if (ev.ctrlKey || ev.metaKey) {
      const delta = ev.deltaY < 0 ? 1 : -1;
      document.dispatchEvent(new CustomEvent('aerotab:terminal-font-delta', { detail: delta }));
      return;
    }
    if (mouseReportingActive(this.mode)) {
      const { col, line } = this.cellAt(ev);
      const seq = encodeMouseEvent({
        mode: this.mode,
        button: wheelButton(ev.deltaY),
        col,
        row: line,
        pressed: true,
        shift: ev.shiftKey,
        meta: ev.metaKey,
        ctrl: ev.ctrlKey,
      });
      if (seq) await this.writePty(seq);
      return;
    }
    const lines = Math.max(1, Math.round(Math.abs(ev.deltaY) / 40));
    const delta = ev.deltaY < 0 ? -lines : lines;
    await this.rpc.call('terminal.scroll', {
      id: this.sessionId,
      cmd: { delta },
    });
  }

  private cellAt(ev: { clientX: number; clientY: number }): { col: number; line: number } {
    const rect = this.opts.canvas.getBoundingClientRect();
    const x = ev.clientX - rect.left;
    const y = ev.clientY - rect.top;
    const col = Math.max(0, Math.min(this.cols - 1, Math.floor(x / this.metrics.width)));
    const line = Math.max(0, Math.min(this.rows - 1, Math.floor(y / this.metrics.height)));
    return { col, line };
  }

  private async onPtrDown(ev: PointerEvent) {
    this.focus();
    this.ptrMoved = false;
    const { col, line } = this.cellAt(ev);

    if (mouseReportingActive(this.mode)) {
      if (ev.button > 2) return;
      this.mouseDown = true;
      this.mouseButton = ev.button as MouseButton;
      this.opts.canvas.setPointerCapture(ev.pointerId);
      const seq = encodeMouseEvent({
        mode: this.mode,
        button: this.mouseButton,
        col,
        row: line,
        pressed: true,
        shift: ev.shiftKey,
        meta: ev.metaKey,
        ctrl: ev.ctrlKey,
      });
      if (seq) {
        ev.preventDefault();
        await this.writePty(seq);
      }
      return;
    }

    if (ev.button !== 0) return;
    this.selecting = true;
    this.clickOpensLink = !ev.altKey && ev.detail === 1;
    this.opts.canvas.setPointerCapture(ev.pointerId);
    const kind = ev.altKey
      ? 'block'
      : ev.detail >= 3
        ? 'line'
        : ev.detail === 2
          ? 'word'
          : 'simple';
    await this.rpc.call('terminal.selectionStart', {
      id: this.sessionId,
      col,
      line,
      kind,
      side: 'left',
    });
  }

  private async onPtrMove(ev: PointerEvent) {
    const { col, line } = this.cellAt(ev);
    if (this.mouseDown && mouseReportingActive(this.mode)) {
      this.ptrMoved = true;
      const seq = encodeMouseEvent({
        mode: this.mode,
        button: this.mouseButton,
        col,
        row: line,
        pressed: true,
        motion: true,
        shift: ev.shiftKey,
        meta: ev.metaKey,
        ctrl: ev.ctrlKey,
      });
      if (seq) await this.writePty(seq);
      return;
    }

    if (this.selecting) {
      this.ptrMoved = true;
      await this.rpc.call('terminal.selectionUpdate', {
        id: this.sessionId,
        col,
        line,
        side: 'right',
      });
      return;
    }

    // Hover: pointer over OSC-8 / detected URL.
    if (this.hoverTimer != null) window.clearTimeout(this.hoverTimer);
    this.hoverTimer = window.setTimeout(() => { void this.refreshHover(col, line); }, 40);
  }

  private async onPtrUp(ev: PointerEvent) {
    const { col, line } = this.cellAt(ev);
    try {
      this.opts.canvas.releasePointerCapture(ev.pointerId);
    } catch {
      /* already released */
    }

    if (this.mouseDown && mouseReportingActive(this.mode)) {
      this.mouseDown = false;
      const seq = encodeMouseEvent({
        mode: this.mode,
        button: this.mouseButton,
        col,
        row: line,
        pressed: false,
        shift: ev.shiftKey,
        meta: ev.metaKey,
        ctrl: ev.ctrlKey,
      });
      this.mouseButton = 3;
      if (seq) await this.writePty(seq);
      return;
    }

    if (this.selecting) {
      const openLink = this.clickOpensLink && !this.ptrMoved && ev.button === 0;
      this.selecting = false;
      this.clickOpensLink = false;
      await this.rpc.call('terminal.selectionUpdate', {
        id: this.sessionId,
        col,
        line,
        side: 'right',
      });
      if (this.opts.copyOnSelect && this.ptrMoved) {
        const text = await this.selectionText();
        if (text) void navigator.clipboard.writeText(text).catch(() => {});
      }
      // Single click without drag → try open hyperlink.
      if (openLink) {
        await this.tryOpenLink(col, line, ev);
      }
      return;
    }
  }

  private async refreshHover(col: number, line: number) {
    if (this.disposed || this.selecting || this.mouseDown) return;
    try {
      const r = await this.rpc.call<{ url: string | null }>('terminal.hyperlinkAt', {
        id: this.sessionId,
        col,
        line,
      });
      this.hoverUrl = r.url ?? null;
      this.opts.canvas.style.cursor = this.hoverUrl ? 'pointer' : '';
    } catch {
      this.hoverUrl = null;
      this.opts.canvas.style.cursor = '';
    }
  }

  private linkModifierOk(ev: { ctrlKey: boolean; altKey: boolean; metaKey: boolean }): boolean {
    const mod = this.opts.linkModifier ?? 'none';
    if (mod === 'none') return true;
    if (mod === 'ctrl') return ev.ctrlKey;
    if (mod === 'alt') return ev.altKey;
    if (mod === 'meta') return ev.metaKey;
    return false;
  }

  private async tryOpenLink(
    col: number,
    line: number,
    ev: { ctrlKey: boolean; altKey: boolean; metaKey: boolean },
  ) {
    if (!this.linkModifierOk(ev)) return;
    try {
      const r = await this.rpc.call<{ url: string | null }>('terminal.hyperlinkAt', {
        id: this.sessionId,
        col,
        line,
      });
      const url = r.url;
      if (!url) return;
      if (!/^https?:\/\//i.test(url) && !/^mailto:/i.test(url)) return;
      window.open(url, '_blank', 'noopener,noreferrer');
    } catch {
      /* ignore */
    }
  }

  private writePty(data: string): Promise<void> {
    if (this.disposed || (this.opts.nativeSurface && !this.webviewHandoff)) return Promise.resolve();
    if (this.inputPaused) {
      // Reject before ownership; never evict accepted bytes to admit newer input.
      if (data.length > 262144 || this.pausedWrites.length >= 256) {
        this.reportInputBackpressure(true);
        return Promise.reject(new Error('terminal input backpressure'));
      }
      const size = new TextEncoder().encode(data).length;
      if (this.pausedBytes + size > 262144) {
        this.reportInputBackpressure(true);
        return Promise.reject(new Error('terminal input backpressure'));
      }
      this.pausedBytes += size;
      return new Promise<void>((resolve, reject) => this.pausedWrites.push({ data, resolve, reject }));
    }
    return this.writeOrderedInput(data);
  }

  /** Shared writer preserves Pane broadcast/trzsz/sessionWriteChain routing. */
  private writeOrderedInput(data: string): Promise<void> {
    const write = (async () => {
      if (this.opts.writeInput) await this.opts.writeInput(data);
      else await this.rpc.call('session.write', { id: this.sessionId, data: b64encode(new TextEncoder().encode(data)) });
    })();
    this.inputWrites.add(write);
    void write.finally(() => this.inputWrites.delete(write)).catch(() => {});
    return write;
  }
}
