<script module lang="ts">
  const HIDDEN_OUTPUT_REPLAY_LIMIT = 256 * 1024;
  const VISIBLE_OUTPUT_MAX_CHUNKS = 256;
  const INTERRUPT_OUTPUT_MAX_CHUNKS = 32;
  const HIDDEN_OUTPUT_MAX_CHUNKS = 32;
  const OUTPUT_CATCHUP_DELAY_MS = 8;
  const INTERRUPT_CATCHUP_DELAY_MS = 16;
  const INTERRUPT_OUTPUT_COOLDOWN_MS = 700;
  interface ReplayChunk { seq: number; bytes: Uint8Array }

  const sessionReplay = new Map<string, ReplayChunk[]>();
  const sessionReplayBytes = new Map<string, number>();
  const sessionReplaySeq = new Map<string, number>();

  function appendReplayChunk(sessionId: string, bytes: Uint8Array): number {
    if (bytes.byteLength === 0) return sessionReplaySeq.get(sessionId) ?? 0;
    const chunks = sessionReplay.get(sessionId) ?? [];
    const seq = (sessionReplaySeq.get(sessionId) ?? 0) + 1;
    sessionReplaySeq.set(sessionId, seq);
    chunks.push({ seq, bytes });
    let total = (sessionReplayBytes.get(sessionId) ?? 0) + bytes.byteLength;
    while (total > HIDDEN_OUTPUT_REPLAY_LIMIT && chunks.length > 1) {
      total -= chunks.shift()?.bytes.byteLength ?? 0;
    }
    sessionReplay.set(sessionId, chunks);
    sessionReplayBytes.set(sessionId, total);
    return seq;
  }

  function cachedReplayChunks(sessionId: string): ReplayChunk[] {
    return sessionReplay.get(sessionId) ?? [];
  }

  function replayChunksAfter(sessionId: string, seq: number): ReplayChunk[] {
    return cachedReplayChunks(sessionId).filter((chunk) => chunk.seq > seq);
  }

  function dropReplayChunks(sessionId: string) {
    sessionReplay.delete(sessionId);
    sessionReplayBytes.delete(sessionId);
    sessionReplaySeq.delete(sessionId);
  }

  /** Replace the replay ring with plain text (native → xterm handoff). */
  function replaceReplayWithText(sessionId: string, text: string) {
    dropReplayChunks(sessionId);
    if (!text) return;
    const normalized = text.replace(/\n/g, '\r\n');
    appendReplayChunk(sessionId, new TextEncoder().encode(normalized));
  }
</script>

<script lang="ts">
  import { workspaceMenu } from '../lib/workspaceMenu';
  import { onDestroy, onMount, tick, untrack } from 'svelte';
  import { SESSIONS_CLOSING, type SessionsClosingDetail } from '../lib/sessionLifecycle';
  import { scheduleTerminalTeardown } from '../lib/terminalTeardown';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { WebLinksAddon } from '@xterm/addon-web-links';
  import { SearchAddon } from '@xterm/addon-search';
  import { X, ChevronUp, ChevronDown, CaseSensitive, Regex, Upload, Download, Info, FolderOpen, Copy, ClipboardPaste, ScanText, Search, Eraser, Columns2, Rows2, Maximize2, RotateCw, ExternalLink } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import { b64decode, b64encode } from '../lib/rpc';
  import type { SessionMeta } from '../lib/types';
  import { TOKYO_NIGHT, xtermPalette } from '../lib/theme';
  import { resolveTerminalPaletteName } from '../lib/terminalPalettePreview';
  import { colorSchemeByName, toXtermTheme } from '../lib/colorSchemes';
  import { normalizeAppearancePreferences, resolveAppearanceTheme } from '../lib/appearancePreferences';
  import { applyLigatures } from '../lib/customCss';
  import { tabs } from '../lib/tabs.svelte';
  import { terminalPollIntervalMs } from '../lib/terminalPoll';
  import { i18n } from '../lib/i18n.svelte';
  import { appConfirm } from '../lib/confirm.svelte';
  import { hotkeys } from '../lib/hotkeys';
  import { isPaneDragActive } from '../lib/paneDrag';
  import {
    allowTerminalFileDragOver,
    installTerminalFileDrag,
    resolveDropFilePaths,
  } from '../lib/terminalFileDrag';
  import { clampMenuToViewport } from '../lib/contextMenuPosition';
  import { portal } from '../lib/portal';
  import { TerminalTransferDetector, type TerminalTransferDetection } from '../lib/terminalTransfer';
  import {
    createTrzszFilter,
    notifyTrzszDragPaths,
    queueTrzszDragPaths,
    type TrzszFilterInstance,
    type TrzszTerminalInput,
    type TrzszTerminalOutput,
  } from '../lib/trzszBridge';
  import { createRawBypassGate } from '../lib/trzszRawBypass';
  import {
    installWindowsTerminalCaretFix,
    refreshCursorBlinkIfFocused,
  } from '../lib/terminalInputWindows';
  import {
    installMacTextareaInputGuard,
    shouldSuppressMacSpuriousInput,
    trackMacBackspaceKeydown,
  } from '../lib/terminalInputMac';
  import { focusTerminalAuto, focusTerminalIfAllowed, shouldAutoFocusTerminal } from '../lib/modalFocus';
  import { scheduleTerminalFit } from '../lib/terminalFit';
  import { getTerminalSettings, invalidateTerminalSettingsCache } from '../lib/terminalSettingsCache';
  import { createTerminalSettingsReload } from '../lib/terminalRendererSwitch';
  import { traceView, traceController } from '../lib/viewCausalTrace';
  import { restoreRetainedSourceStatus } from '../lib/nativeViewTransaction';
  import { spawnDetachedNativeTerminal } from '../lib/nativeTerminal';
  import {
    NativeTerminalController,
    type ThemeColors as NativeThemeColors,
  } from '../lib/termgrid';
  import NativeTerminalView from './NativeTerminalView.svelte';
  import { preparePasteText } from '../lib/nativePasteBridge';
  import { sessionEndedGate } from '../lib/sessionEndedGate.svelte';
  import { terminalUi, type TerminalFrontendKind } from '../lib/terminalUi.svelte';

  interface Props {
    rpc: RpcClient;
    session: SessionMeta;
    active: boolean;
    /** False while pane is hidden by maximize (display:none); avoids fit() at zero size. */
    layoutVisible?: boolean;
    /** False when this pane's tab is not the active tab (drains output without rendering). */
    tabVisible?: boolean;
    /** Bumped by parent whenever persisted settings change. */
    settingsRev?: number;
    /** Explicit per-pane choice; `native` remains the legacy WebView shared-core view. */
    rendererOverride?: 'native-surface' | 'native' | 'dom' | 'canvas' | 'webgl';
    /** App must set this BEFORE opening WebView modals/popovers above OS surfaces. */
    nativeSurfaceOccluded?: boolean;
    /** Invoked when the user clicks the pane's close button from inside (e.g. exited overlay). */
    onClosePane?: () => void;
    onActivate?: () => void;
    /** Invoked when transfer detection should open the active SSH pane's SFTP browser. */
    onOpenSftp?: () => void;
    onSplitRight?: () => void;
    onSplitDown?: () => void;
    onMaximize?: () => void;
    /** When true, keystrokes are sent to every SSH pane in the tab. */
    broadcastEnabled?: boolean;
    broadcastTargetIds?: string[];
    onError?: (msg: string) => void;
  }
  let {
    rpc,
    session,
    active,
    layoutVisible = true,
    tabVisible = true,
    settingsRev = 0,
    rendererOverride,
    nativeSurfaceOccluded = false,
    onClosePane,
    onActivate,
    onOpenSftp,
    onSplitRight,
    onSplitDown,
    onMaximize,
    broadcastEnabled = false,
    broadcastTargetIds = [],
    onError,
  }: Props = $props();

  let host: HTMLDivElement | null = null;
  let nativeHost: HTMLDivElement | null = $state(null);
  let term: Terminal | null = null;
  let fit: FitAddon | null = null;
  let search: SearchAddon | null = null;
  let rendererAddon: { dispose: () => void } | null = null;
  let activeRenderer = $state<'dom' | 'canvas' | 'webgl'>('dom');
  /** When true, Rust VT engine + termgrid renderer replace xterm.js. */
  let useNative = $state(false);
  let nativeBackend = $state<'webview' | 'native-surface'>('webview');
  /** True once onMount finished choosing a frontend (footer leaves "pending"). */
  let frontendReady = $state(false);
  let frontendError = $state(false);
  let switchingFrontend = false;
  let nativeCtl: NativeTerminalController | null = $state(null);
  let nativeView: NativeTerminalView | null = $state(null);
  let nativeViewEpoch = $state(0);
  let nativeTheme: NativeThemeColors | undefined = $state(undefined);
  let nativeFontFamily = $state('JetBrains Mono, Menlo, monospace');
  let nativeFontSize = $state(15);
  let nativeFontWeight = $state(400);
  let nativeFontWeightBold = $state(700);
  let nativeLinePadding = $state(0);
  let nativeScrollback = $state(10_000);
  let nativeCursorBlink = $state(true);
  let nativeAltIsMeta = $state(false);
  let nativeLigatures = $state(false);
  let nativeTransferDetect = $state(false);
  let pollHandle: number | null = null;
  let pollIntervalMs = 0;
  let pollInFlight = false;
  let lastTabVisibleForReplay = false;
  let replayCursorSeq = 0;
  let interruptOutputCooldownUntil = 0;
  /** Bumped to drop in-flight `session.poll` results when the pane or tab is closing. */
  let pollEpoch = 0;
  let documentHidden = $state(false);
  const encoder = new TextEncoder();
  const decoder = new TextDecoder('utf-8');

  // Search overlay state.
  let searchOpen = $state(false);
  let searchQuery = $state('');
  let searchCase = $state(false);
  let searchRegex = $state(false);

  // Context menu state.
  let menuOpen = $state(false);
  let menuX = $state(0);
  let menuY = $state(0);
  let menuEl = $state<HTMLDivElement | null>(null);
  let exitedOverlayEl = $state<HTMLDivElement | null>(null);
  let sessionFontDelta = $state(0);

  // Behavior toggles loaded from settings.
  let copyOnSelect = false;
  let rmbPaste = false;
  let rightClickAction: 'menu' | 'paste' | 'select-word' = 'menu';
  let middleClickPaste = false;
  // Terminal-section behaviour toggles (M3).
  let bellMode: 'off' | 'visual' | 'audible' = 'off';
  let linkModifier: 'none' | 'ctrl' | 'alt' | 'meta' = 'none';
  let copyWithFormatting = false;
  let bracketedPaste = true;
  let pasteMultilineWarn = true;
  let pasteFlattenNewlines = false;
  let pasteTrimWhitespace = false;
  let bellFlashHandle: number | null = null;
  /** Must be $state: shared settings cache only runs Fresh side-effects on one pane. */
  let transferDetectionEnabled = $state(false);
  let transferNotice = $state<TerminalTransferDetection | null>(null);
  let transferNoticeHandle: number | null = null;
  let transferFilter = $state<TrzszFilterInstance | null>(null);
  let transferFilterGeneration = 0;
  let cleanupFileDrag: (() => void) | null = null;
  let sessionWriteChain: Promise<void> = Promise.resolve();
  const transferDetector = new TerminalTransferDetector();
  const rawBypassGate = createRawBypassGate({
    isTransferring: () => !!transferFilter?.isTransferringFiles(),
    release: () => { void nativeCtl?.setRawBypass(false); },
  });
  const canOpenSftp = $derived(session.kind === 'Ssh' && !!onOpenSftp);
  const canOpenExternalNative = $derived(
    session.kind === 'Ssh' || session.kind === 'ssh' || session.kind === 'LocalShell' || session.kind === 'local',
  );

  function isLocalWindowsShell(): boolean {
    if (session.kind !== 'LocalShell') return false;
    return typeof navigator !== 'undefined' && /Win/i.test(navigator.userAgent);
  }

  function isSshSession(): boolean {
    return session.kind === 'Ssh' || session.kind === 'ssh';
  }

  function trzszDragInitTimeoutMs(): number {
    // Remote SSH needs more time for trz handshake over higher-latency links.
    return isSshSession() ? 30_000 : 8_000;
  }

  // Liveness state. Flips to true when backend reports session ended.
  let exited = $state(false);
  let reconnecting = $state(false);
  const canReconnect = $derived(
    exited && session.kind === 'Ssh' && !!(session.profileId || session.sshProfile),
  );
  const canUseTrzszTransfer = $derived(
    transferDetectionEnabled && !!transferFilter && active && !exited,
  );

  interface PollResult { chunks: string[]; alive: boolean }

  function pollMaxChunks(): number {
    if (!tabVisible) return HIDDEN_OUTPUT_MAX_CHUNKS;
    if (performance.now() < interruptOutputCooldownUntil) return INTERRUPT_OUTPUT_MAX_CHUNKS;
    return VISIBLE_OUTPUT_MAX_CHUNKS;
  }

  function pollCatchupDelay(): number {
    if (performance.now() < interruptOutputCooldownUntil) return INTERRUPT_CATCHUP_DELAY_MS;
    return OUTPUT_CATCHUP_DELAY_MS;
  }

  function replayCachedOutput(sessionId: string, replace = false) {
    if (!term) return;
    const chunks = replace ? cachedReplayChunks(sessionId) : replayChunksAfter(sessionId, replayCursorSeq);
    if (!chunks || chunks.length === 0) return;
    if (replace) term.clear();
    for (const chunk of chunks) {
      term.write(chunk.bytes);
      replayCursorSeq = chunk.seq;
    }
  }

  function markExited() {
    if (exited) return;
    exited = true;
    term?.write('\r\n\x1b[31m[session ended]\x1b[0m The process exited or the connection was closed.\r\n');
    tabs.markActivity(session.id, 'bell');
    if (active) focusExitedOverlay(true);
  }

  $effect(() => {
    if (active) sessionEndedGate.setActiveExited(exited);
    return () => {
      if (active) sessionEndedGate.setActiveExited(false);
    };
  });

  function focusExitedOverlay(auto = false) {
    requestAnimationFrame(() => {
      if (auto && !shouldAutoFocusTerminal(host)) return;
      exitedOverlayEl?.focus();
    });
  }

  function bindingLabel(actionId: string): string {
    return hotkeys.getBindings(actionId)[0] ?? '';
  }

  function hostHasLayout(): boolean {
    return !!host && host.offsetWidth >= 2 && host.offsetHeight >= 2;
  }

  /** Skip fit while display:none (maximize hide) — zero-size fit clears canvas terminals. */
  function safeFit(redraw = false) {
    if (useNative) {
      if (!layoutVisible || !tabVisible) return;
      nativeCtl?.fit();
      return;
    }
    if (!fit || !term || !layoutVisible || !tabVisible || !hostHasLayout()) return;
    try {
      fit.fit();
    } catch {
      /* renderer not ready */
    }
    if (redraw) {
      try {
        term.refresh(0, term.rows - 1);
      } catch {
        /* ignore */
      }
    }
  }

  function scheduleSafeFit(redraw = false) {
    scheduleTerminalFit(() => safeFit(redraw));
  }

  function adjustFontSize(delta: number) {
    if (!delta) return;
    if (useNative) {
      sessionFontDelta += delta;
      nativeFontSize = Math.min(32, Math.max(8, nativeFontSize + delta));
      nativeCtl?.setFont({ size: nativeFontSize });
      // nativeFontSize prop → NativeTerminalView $effect → hostAdapter.update(viewConfig).
      scheduleSafeFit();
      return;
    }
    if (!term) return;
    sessionFontDelta += delta;
    const next = Math.min(32, Math.max(8, (term.options.fontSize ?? 15) + delta));
    term.options.fontSize = next;
    scheduleSafeFit();
  }

  function onExitedKeydown(ev: KeyboardEvent) {
    if (!exited || !active) return;
    if (hotkeys.matchesAction(ev, 'session-ended-close')) {
      ev.preventDefault();
      ev.stopPropagation();
      onClosePane?.();
      return;
    }
    if (canReconnect && hotkeys.matchesAction(ev, 'session-ended-reconnect')) {
      ev.preventDefault();
      ev.stopPropagation();
      void reconnectSession();
    }
  }

  function reconnectTargetLabel(): string {
    if (!session.sshProfile) return session.title;
    const port = session.sshProfile.port === 22 ? '' : `:${session.sshProfile.port}`;
    return `${session.title} (${session.sshProfile.user}@${session.sshProfile.host}${port})`;
  }

  async function reconnectSession() {
    if (!canReconnect || reconnecting) return;
    reconnecting = true;
    const oldId = session.id;
    const tab = tabs.tabOf(oldId);
    try {
      let meta: { id: string; kind: string; title: string };
      let next: SessionMeta;
      if (session.sshProfile && !session.profileId) {
        meta = await rpc.call('session.openSsh', {
          title: session.title,
          profile: session.sshProfile,
        });
        next = {
          id: meta.id,
          kind: meta.kind,
          title: meta.title,
          profileId: session.profileId,
          sshProfile: session.sshProfile,
        };
      } else if (session.profileId) {
        meta = await rpc.call('session.openSshProfile', { profile_id: session.profileId });
        next = {
          id: meta.id,
          kind: meta.kind,
          title: meta.title,
          profileId: session.profileId,
        };
      } else {
        return;
      }
      if (tab) tabs.replacePaneSession(tab.id, oldId, next);
      document.dispatchEvent(
        new CustomEvent('aerotab:session-replaced', { detail: { oldId, session: next } }),
      );
      try { await rpc.call('session.close', { id: oldId }); } catch { /* old session may already be gone */ }
    } catch (err) {
      const message = `reconnect ${reconnectTargetLabel()}: ${(err as Error).message}`;
      onError?.(message);
      console.warn('reconnect failed', err);
    } finally {
      reconnecting = false;
    }
  }

  function cancelPolling() {
    pollEpoch += 1;
    stopPolling();
  }

  async function pollOnce(): Promise<boolean> {
    if (!term) return false;
    const epoch = pollEpoch;
    const maxChunks = pollMaxChunks();
    try {
      const r = await rpc.call<PollResult>('session.poll', {
        id: session.id,
        max_chunks: maxChunks,
      });
      if (epoch !== pollEpoch || !term) return false;
      for (const bytes of concatPollChunks(r.chunks)) {
        const text = decoder.decode(bytes);
        inspectTransferOutput(text);
        const replaySeq = appendReplayChunk(session.id, bytes);
        if (tabVisible) {
          processSessionOutput(bytes, text);
          replayCursorSeq = replaySeq;
        }
      }
      if (r.chunks.length > 0 && !active) tabs.markActivity(session.id, 'output');
      if (!r.alive && !exited) {
        markExited();
        stopPolling();
      }
      return tabVisible && r.alive && r.chunks.length >= maxChunks;
    } catch (e) {
      const msg = (e as Error).message ?? String(e);
      if (msg.toLowerCase().includes('not found')) {
        if (!exited) { markExited(); stopPolling(); }
      } else {
        console.warn('poll', session.id, e);
      }
      return false;
    }
  }

  function schedulePoll(delayMs: number) {
    if (pollIntervalMs <= 0 || pollHandle != null || pollInFlight) return;
    pollHandle = window.setTimeout(async () => {
      pollHandle = null;
      pollInFlight = true;
      const caughtUp = await pollOnce();
      pollInFlight = false;
      if (pollIntervalMs <= 0) return;
      schedulePoll(caughtUp ? pollCatchupDelay() : pollIntervalMs);
    }, delayMs);
  }

  function startPolling(intervalMs: number) {
    if (intervalMs <= 0) {
      stopPolling();
      return;
    }
    if ((pollHandle != null || pollInFlight) && pollIntervalMs === intervalMs) return;
    stopPolling();
    pollIntervalMs = intervalMs;
    schedulePoll(0);
  }

  function stopPolling() {
    if (pollHandle != null) {
      window.clearTimeout(pollHandle);
      pollHandle = null;
    }
    pollIntervalMs = 0;
  }

  function syncPolling() {
    if (useNative) {
      stopPolling();
      nativeCtl?.setVisible(tabVisible && layoutVisible && !documentHidden);
      return;
    }
    const ms = terminalPollIntervalMs({
      active,
      tabVisible,
      documentHidden,
    });
    if (ms <= 0) stopPolling();
    else startPolling(ms);
  }

  // If the user selected a terminal color scheme (M4), use it as the final
  // xterm palette; otherwise fall back to the appearance theme.
  function isTranslucent(): boolean {
    return typeof document !== 'undefined' && document.body.dataset.translucent === 'true';
  }

  function paletteFromCfg(cfg: { theme: typeof TOKYO_NIGHT; colorSchemeName: string }) {
    const cs = colorSchemeByName(resolveTerminalPaletteName(cfg.colorSchemeName));
    const theme = cs ? toXtermTheme(cs) : xtermPalette(cfg.theme);
    if (isTranslucent()) {
      return { ...theme, background: 'transparent' };
    }
    return theme;
  }

  async function loadTermSettingsFresh() {
    const out = {
      fontFamily: 'JetBrains Mono, Menlo, monospace',
      fontSize: 15,
      scrollback: 2000,
      cursorBlink: true,
      theme: TOKYO_NIGHT,
      // appearance group
      ligatures: false,
      fontWeight: 400 as number,
      fontWeightBold: 700 as number,
      fallbackFont: '',
      cursorStyle: 'block' as 'block' | 'bar' | 'underline',
      minContrastRatio: 1,
      linePadding: 0,
      // terminal group (M3)
      renderer: 'native' as 'native' | 'dom' | 'canvas' | 'webgl',
      altIsMeta: false,
      scrollOnInput: true,
      wordSeparator: ' ()[]{}\'",;:',
      // M4 — palette override
      colorSchemeName: '' as string,
      experimentalTransferDetection: false,
    };
    try {
      const f = await rpc.call<{ value: unknown }>('settings.get', { key: 'font' });
      if (f.value && typeof f.value === 'object') {
        const v = f.value as Record<string, unknown>;
        if (typeof v.family === 'string') out.fontFamily = v.family;
        if (typeof v.size === 'number') out.fontSize = v.size;
      }
      const t = await rpc.call<{ value: unknown }>('settings.get', { key: 'terminal' });
      if (t.value && typeof t.value === 'object') {
        const v = t.value as Record<string, unknown>;
        if (typeof v.scrollback === 'number') out.scrollback = v.scrollback;
        if (typeof v.cursorBlink === 'boolean') out.cursorBlink = v.cursorBlink;
        if (v.renderer === 'native' || v.renderer === 'dom' || v.renderer === 'canvas' || v.renderer === 'webgl') {
          out.renderer = v.renderer;
        }
        if (typeof v.altIsMeta === 'boolean') out.altIsMeta = v.altIsMeta;
        if (typeof v.scrollOnInput === 'boolean') out.scrollOnInput = v.scrollOnInput;
        if (typeof v.wordSeparator === 'string') out.wordSeparator = v.wordSeparator;
        if (v.bell === 'off' || v.bell === 'visual' || v.bell === 'audible') bellMode = v.bell;
        if (v.linkModifier === 'none' || v.linkModifier === 'ctrl'
            || v.linkModifier === 'alt' || v.linkModifier === 'meta') linkModifier = v.linkModifier;
        if (typeof v.copyWithFormatting === 'boolean') copyWithFormatting = v.copyWithFormatting;
        if (typeof v.bracketedPaste === 'boolean') bracketedPaste = v.bracketedPaste;
        if (typeof v.pasteMultilineWarn === 'boolean') pasteMultilineWarn = v.pasteMultilineWarn;
        if (typeof v.pasteFlattenNewlines === 'boolean') pasteFlattenNewlines = v.pasteFlattenNewlines;
        if (typeof v.pasteTrimWhitespace === 'boolean') pasteTrimWhitespace = v.pasteTrimWhitespace;
        if (typeof v.experimentalTransferDetection === 'boolean') {
          out.experimentalTransferDetection = v.experimentalTransferDetection;
        }
      }
      const th = await rpc.call<{ value: unknown }>('settings.get', { key: 'theme' });
      const ui = await rpc.call<{ value: unknown }>('settings.get', { key: 'uiAppearance' });
      out.theme = resolveAppearanceTheme(normalizeAppearancePreferences(ui.value, th.value), window.matchMedia('(prefers-color-scheme: dark)').matches);
      const cs = await rpc.call<{ value: unknown }>('settings.get', { key: 'terminalColorScheme' });
      if (typeof cs.value === 'string') out.colorSchemeName = cs.value;
      const beh = await rpc.call<{ value: unknown }>('settings.get', { key: 'behavior' });
      if (beh.value && typeof beh.value === 'object') {
        const v = beh.value as Record<string, unknown>;
        if (typeof v.copyOnSelect === 'boolean') copyOnSelect = v.copyOnSelect;
        if (v.rightClickAction === 'menu' || v.rightClickAction === 'paste'
            || v.rightClickAction === 'select-word') {
          rightClickAction = v.rightClickAction;
          rmbPaste = rightClickAction === 'paste';
        } else if (typeof v.rmbPaste === 'boolean') {
          rmbPaste = v.rmbPaste;
          rightClickAction = v.rmbPaste ? 'paste' : 'menu';
        }
        if (typeof v.middleClickPaste === 'boolean') middleClickPaste = v.middleClickPaste;
      }
      const ap = await rpc.call<{ value: unknown }>('settings.get', { key: 'appearance' });
      if (ap.value && typeof ap.value === 'object') {
        const v = ap.value as Record<string, unknown>;
        if (typeof v.ligatures === 'boolean') out.ligatures = v.ligatures;
        if (typeof v.fontWeight === 'number') out.fontWeight = v.fontWeight;
        if (typeof v.fontWeightBold === 'number') out.fontWeightBold = v.fontWeightBold;
        if (typeof v.fallbackFont === 'string') out.fallbackFont = v.fallbackFont;
        if (v.cursorStyle === 'block' || v.cursorStyle === 'bar' || v.cursorStyle === 'underline') {
          out.cursorStyle = v.cursorStyle;
        }
        if (typeof v.minContrastRatio === 'number') out.minContrastRatio = v.minContrastRatio;
        if (typeof v.linePadding === 'number') out.linePadding = v.linePadding;
      }
    } catch {
      /* settings store may be unconfigured — fall back to defaults. */
    }
    // Compose final family string with fallback appended.
    if (out.fallbackFont && !out.fontFamily.includes(out.fallbackFont)) {
      out.fontFamily = `${out.fontFamily}, ${out.fallbackFont}`;
    }
    // xterm canvas/webgl paint an opaque glyph surface; under window opacity
    // <100% that becomes a solid black slab. Demote those to xterm DOM only.
    // Never override `native` — termgrid already falls back to Canvas2D when
    // translucent and honors transparent backgrounds.
    if (isTranslucent() && (out.renderer === 'canvas' || out.renderer === 'webgl')) {
      out.renderer = 'dom';
    }
    // Do not mutate per-pane state here — getTerminalSettings() caches this
    // result and skips Fresh for other panes. Callers must applyTransferDetectionFromCfg.
    return out;
  }

  async function loadTermSettings() {
    const cfg = await getTerminalSettings(() => loadTermSettingsFresh());
    // Never put per-pane overrides in the globally shared settings cache/store.
    return { ...cfg, renderer: rendererOverride ?? cfg.renderer };
  }

  function applyTransferDetectionFromCfg(enabled: boolean) {
    if (!enabled) {
      transferDetector.reset();
      clearTransferNotice();
    }
    transferDetectionEnabled = enabled;
  }

  function inspectTransferOutput(text: string) {
    if (!transferDetectionEnabled) return;
    const detected = transferDetector.push(text);
    if (!detected) return;
    // trzsz is handled by the active filter; keep the banner for ZMODEM/lrzsz only.
    if (transferFilter && detected.protocol === 'trzsz') return;
    transferNotice = detected;
    if (transferNoticeHandle != null) window.clearTimeout(transferNoticeHandle);
    transferNoticeHandle = window.setTimeout(() => {
      transferNotice = null;
      transferNoticeHandle = null;
    }, 12000);
  }

  function clearTransferNotice() {
    if (transferNoticeHandle != null) window.clearTimeout(transferNoticeHandle);
    transferNoticeHandle = null;
    transferNotice = null;
  }

  function transferProtocolLabel(detected: TerminalTransferDetection): string {
    return detected.protocol === 'trzsz' ? 'trzsz' : 'ZMODEM / lrzsz';
  }

  function transferTitle(detected: TerminalTransferDetection): string {
    const protocol = transferProtocolLabel(detected);
    if (detected.direction === 'upload') return i18n.t('terminal.transferUploadDetected', { protocol });
    if (detected.direction === 'download') return i18n.t('terminal.transferDownloadDetected', { protocol });
    return i18n.t('terminal.transferDetected', { protocol });
  }

  function transferHint(detected: TerminalTransferDetection): string {
    return i18n.t(detected.protocol === 'trzsz' ? 'terminal.transferTrzszHint' : 'terminal.transferHint');
  }

  async function terminalInputBytes(input: TrzszTerminalInput): Promise<Uint8Array> {
    if (typeof input === 'string') return encoder.encode(input);
    if (input instanceof Uint8Array) return input;
    if (input instanceof ArrayBuffer) return new Uint8Array(input);
    if (input instanceof Blob) return new Uint8Array(await input.arrayBuffer());
    return encoder.encode(String(input));
  }

  async function sendSessionBytes(bytes: Uint8Array): Promise<void> {
    const data = b64encode(bytes);
    const targets =
      broadcastEnabled && broadcastTargetIds.length > 1
        ? broadcastTargetIds
        : [session.id];
    if (targets.length > 1) {
      await rpc.call('session.writeMany', { ids: targets, data });
    } else {
      await rpc.call('session.write', { id: session.id, data });
    }
  }

  async function sendSessionInputAsync(input: TrzszTerminalInput): Promise<void> {
    if (typeof input === 'string') {
      // Both the WebView grid and HWND input bridge use this ordered writer.
      // Enter must reveal the live cursor after browsing scrollback; mouse
      // reports and unrelated control sequences must retain the viewport.
      if (useNative && (input.includes('\r') || input.includes('\n'))) {
        await rpc.call('terminal.scroll', { id: session.id, cmd: 'bottom' });
      }
      if (input.includes('\x03')) {
        interruptOutputCooldownUntil = performance.now() + INTERRUPT_OUTPUT_COOLDOWN_MS;
      }
      await sendSessionBytes(encoder.encode(input));
      return;
    }
    await sendSessionBytes(await terminalInputBytes(input));
  }

  function sendSessionInput(input: TrzszTerminalInput, valid?: () => boolean): Promise<void> {
    const write = sessionWriteChain
      .then(() => !valid || valid() ? sendSessionInputAsync(input) : undefined);
    sessionWriteChain = write.catch((err: unknown) => console.warn('terminal write failed', err));
    return write;
  }

  function concatPollChunks(chunks: string[]): Uint8Array[] {
    if (chunks.length <= 1) {
      return chunks.map((c) => b64decode(c));
    }
    const parts = chunks.map((c) => b64decode(c));
    const total = parts.reduce((sum, part) => sum + part.length, 0);
    const merged = new Uint8Array(total);
    let offset = 0;
    for (const part of parts) {
      merged.set(part, offset);
      offset += part.length;
    }
    return [merged];
  }

  function writeTerminalOutput(output: TrzszTerminalOutput): void {
    const writeBytes = (bytes: Uint8Array) => {
      if (useNative) {
        void nativeCtl?.feed(bytes);
        return;
      }
      term?.write(bytes);
    };
    const writeText = (text: string) => {
      if (useNative) {
        void nativeCtl?.feed(encoder.encode(text));
        return;
      }
      term?.write(text);
    };
    if (typeof output === 'string') {
      writeText(output);
      return;
    }
    if (output instanceof Uint8Array) {
      writeBytes(output);
      return;
    }
    if (output instanceof ArrayBuffer) {
      writeBytes(new Uint8Array(output));
      return;
    }
    if (output instanceof Blob) {
      void output.arrayBuffer()
        .then((buffer) => writeBytes(new Uint8Array(buffer)))
        .catch((err: unknown) => console.warn('terminal blob write failed', err));
    }
  }

  function processSessionOutput(bytes: Uint8Array, textFallback: string): void {
    if (!transferFilter) {
      term?.write(textFallback);
      return;
    }
    try {
      transferFilter.processServerOutput(bytes);
    } catch (err) {
      console.warn('trzsz output processing failed', err);
      term?.write(textFallback);
    }
  }

  function terminalColumnsForTransfer(): number {
    return term?.cols ?? 80;
  }

  async function configureTransferFilter(enabled: boolean, forceRecreate = false): Promise<void> {
    const generation = ++transferFilterGeneration;
    if (!enabled) {
      if (transferFilter?.isTransferringFiles()) transferFilter.stopTransferringFiles();
      transferFilter = null;
      rawBypassGate.dispose();
      void nativeCtl?.setRawBypass(false);
      return;
    }
    if (transferFilter && !forceRecreate) {
      transferFilter.setTerminalColumns(terminalColumnsForTransfer());
      return;
    }
    if (transferFilter) {
      if (transferFilter.isTransferringFiles()) transferFilter.stopTransferringFiles();
      transferFilter = null;
    }
    sessionWriteChain = Promise.resolve();
    try {
      const filter = await createTrzszFilter({
        writeToTerminal: writeTerminalOutput,
        sendToServer: sendSessionInput,
        terminalColumns: terminalColumnsForTransfer(),
        isWindowsShell: isLocalWindowsShell(),
        maxDataChunkSize: 1024 * 1024,
        dragInitTimeout: trzszDragInitTimeoutMs(),
        sshSlowLink: isSshSession(),
      });
      if (generation !== transferFilterGeneration || !transferDetectionEnabled) {
        if (filter.isTransferringFiles()) filter.stopTransferringFiles();
        return;
      }
      transferFilter = filter;
    } catch (err) {
      console.warn('trzsz filter unavailable', err);
    }
  }

  /** trzsz has no drain API; only suspend an idle filter around view handoff. */
  async function suspendTransferFilterForViewSwitch(): Promise<() => Promise<void>> {
    if (transferFilter?.isTransferringFiles()) {
      throw new Error('Native view switch unavailable while a trzsz transfer is in progress');
    }
    const reenable = transferDetectionEnabled && !!transferFilter;
    if (transferFilter) {
      await configureTransferFilter(false);
      await sessionWriteChain;
    }
    return async () => {
      if (reenable && transferDetectionEnabled && !destroyed) {
        await configureTransferFilter(true);
      }
    };
  }

  function nativeWriteInput(data: string, valid?: () => boolean): void {
    if (valid && !valid()) return;
    if (transferFilter) transferFilter.processTerminalInput(data);
    else sendSessionInput(data, valid);
  }

  async function nativeWriteBytes(bytes: Uint8Array, valid: () => boolean): Promise<void> {
    // Valid UTF-8 key/IME data takes exactly the existing trzsz/text path.
    // Legacy mouse bytes must not be decoded with replacement and re-encoded.
    let text: string;
    try { text = new TextDecoder('utf-8', { fatal: true }).decode(bytes); }
    catch {
      if (transferFilter) throw new Error('Raw mouse input unavailable during terminal transfer');
      await sendSessionInput(bytes, valid);
      return;
    }
    if (!transferFilter) {
      await sendSessionInput(text, valid);
      return;
    }
    // trzsz may retain input internally and has no flush API. Never pretend
    // that awaiting our writer also drains its private async protocol state.
    nativeWriteInput(text, valid);
    await sessionWriteChain;
  }

  function onNativeRaw(bytes: Uint8Array): void {
    if (!transferFilter) {
      void nativeCtl?.feed(bytes);
      void nativeCtl?.setRawBypass(false);
      return;
    }
    try {
      transferFilter.processServerOutput(bytes);
    } catch (err) {
      console.warn('trzsz output processing failed', err);
      void nativeCtl?.feed(bytes);
    }
    rawBypassGate.onServerOutput();
  }

  function onNativeTrzszDetected(): void {
    if (!transferDetectionEnabled) return;
    void configureTransferFilter(true);
  }

  async function applyRenderer(renderer: 'native-surface' | 'native' | 'dom' | 'canvas' | 'webgl') {
    if (!term || renderer === 'native' || renderer === 'native-surface') return;
    const target = isTranslucent() ? 'dom' : renderer;
    if (target === activeRenderer) return;
    try { rendererAddon?.dispose(); } catch { /* ignore renderer teardown */ }
    rendererAddon = null;
    activeRenderer = 'dom';
    if (target === 'dom') return;

    const loadCanvas = async () => {
      if (!term || isTranslucent()) return;
      try {
        const { CanvasAddon } = await import('@xterm/addon-canvas');
        const addon = new CanvasAddon();
        term.loadAddon(addon);
        rendererAddon = addon;
        activeRenderer = 'canvas';
      } catch (err) {
        console.warn('canvas renderer failed; using DOM renderer:', err);
      }
    };

    if (target === 'canvas') {
      await loadCanvas();
      refreshCursorBlinkIfFocused(term);
      return;
    }

    try {
      const { WebglAddon } = await import('@xterm/addon-webgl');
      const addon = new WebglAddon();
      addon.onContextLoss(() => {
        console.warn('WebGL context lost — falling back to canvas renderer');
        try { addon.dispose(); } catch { /* ignore */ }
        if (rendererAddon === addon) rendererAddon = null;
        activeRenderer = 'dom';
        void loadCanvas();
      });
      term.loadAddon(addon);
      rendererAddon = addon;
      activeRenderer = 'webgl';
    } catch (err) {
      console.warn('webgl renderer failed; falling back to canvas:', err);
      await loadCanvas();
    }
    refreshCursorBlinkIfFocused(term);
  }

  let destroyed = false;
  let frontendInitialized = false;
  let resolveNativeReady: (() => void) | null = null;
  let rejectNativeReady: ((error: Error) => void) | null = null;

  function beginNativeReadiness(): Promise<void> {
    return new Promise((resolve, reject) => {
      resolveNativeReady = resolve;
      rejectNativeReady = reject;
    });
  }

  function onNativeError(message: string) {
    frontendReady = false;
    frontendError = true;
    reportFrontendKind();
    rejectNativeReady?.(new Error(message));
    resolveNativeReady = null;
    rejectNativeReady = null;
    onError?.(message);
  }

  /** After xterm → native, feed the replay ring into the new engine once. */
  let seedNativeFromReplay = false;

  function applyNativeCfg(cfg: Awaited<ReturnType<typeof loadTermSettings>>) {
    nativeFontFamily = cfg.fontFamily;
    nativeFontSize = cfg.fontSize + sessionFontDelta;
    nativeFontWeight = cfg.fontWeight;
    nativeFontWeightBold = cfg.fontWeightBold;
    nativeLinePadding = cfg.linePadding;
    nativeScrollback = cfg.scrollback;
    nativeCursorBlink = cfg.cursorBlink;
    nativeAltIsMeta = cfg.altIsMeta;
    nativeLigatures = cfg.ligatures;
    nativeTransferDetect = cfg.experimentalTransferDetection;
    nativeTheme = paletteFromCfg(cfg) as NativeThemeColors;
  }

  function bindSharedTerminalListeners(opts: { xtermSettingsGate?: boolean } = {}) {
    const settingsListener = () => {
      invalidateTerminalSettingsCache();
      if (opts.xtermSettingsGate && !term && !useNative) return;
      void reloadSettingsLive();
    };
    const searchListener = () => { if (active) openSearch(); };
    const copyListener = () => { if (active) void doCopy(); };
    const fontListener = (ev: Event) => {
      if (!active) return;
      const delta = (ev as CustomEvent<number>).detail;
      if (typeof delta === 'number') adjustFontSize(delta);
    };
    const focusListener = (ev: Event) => {
      const detail = (ev as CustomEvent<{ sessionId?: string }>).detail;
      if (detail?.sessionId && detail.sessionId !== session.id) return;
      // Do not gate on `active` here: dispatchFocusPane can race Svelte's prop flush
      // after tabs.focusPane; sessionId match is the authoritative target.
      if (exited) {
        focusExitedOverlay();
        return;
      }
      requestAnimationFrame(() => {
        // force: active prop may not have flushed yet after tabs.focusPane.
        if (useNative) nativeView?.focus({ force: true });
        else focusTerminalIfAllowed(term);
      });
    };
    const fitListener = (ev: Event) => {
      const detail = (ev as CustomEvent<{ sessionId?: string }>).detail;
      if (detail?.sessionId && detail.sessionId !== session.id) return;
      scheduleSafeFit(true);
    };
    const endedListener = (ev: Event) => {
      if (!active || !exited) return;
      const action = (ev as CustomEvent<'close' | 'reconnect'>).detail;
      if (action === 'close') onClosePane?.();
      else if (action === 'reconnect' && canReconnect) void reconnectSession();
    };
    const appearanceMedia = window.matchMedia('(prefers-color-scheme: dark)');
    appearanceMedia.addEventListener('change', settingsListener);
    document.addEventListener('aerotab:terminal-palette-preview', settingsListener);
    document.addEventListener('aerotab:settings-changed', settingsListener);
    document.addEventListener('aerotab:search', searchListener);
    document.addEventListener('aerotab:terminal-copy', copyListener);
    document.addEventListener('aerotab:terminal-font-delta', fontListener);
    document.addEventListener('aerotab:session-ended-action', endedListener);
    document.addEventListener('aerotab:focus-pane', focusListener);
    document.addEventListener('aerotab:fit-pane', fitListener);
    return () => {
      appearanceMedia.removeEventListener('change', settingsListener);
      document.removeEventListener('aerotab:terminal-palette-preview', settingsListener);
      document.removeEventListener('aerotab:settings-changed', settingsListener);
      document.removeEventListener('aerotab:search', searchListener);
      document.removeEventListener('aerotab:terminal-copy', copyListener);
      document.removeEventListener('aerotab:terminal-font-delta', fontListener);
      document.removeEventListener('aerotab:session-ended-action', endedListener);
      document.removeEventListener('aerotab:focus-pane', focusListener);
      document.removeEventListener('aerotab:fit-pane', fitListener);
    };
  }

  async function bootXterm(
    cfg: Awaited<ReturnType<typeof loadTermSettings>>,
    opts: { bindSharedListeners?: boolean } = {},
  ) {
    if (!host) {
      frontendReady = true;
      reportFrontendKind();
      return;
    }
    const bindShared = opts.bindSharedListeners !== false;
    term = new Terminal({
      fontFamily: cfg.fontFamily,
      fontSize: cfg.fontSize + sessionFontDelta,
      fontWeight: cfg.fontWeight,
      fontWeightBold: cfg.fontWeightBold,
      theme: paletteFromCfg(cfg),
      cursorBlink: cfg.cursorBlink,
      cursorStyle: cfg.cursorStyle,
      scrollback: cfg.scrollback,
      lineHeight: 1 + cfg.linePadding / Math.max(cfg.fontSize, 1),
      minimumContrastRatio: cfg.minContrastRatio,
      macOptionIsMeta: cfg.altIsMeta,
      scrollOnUserInput: cfg.scrollOnInput,
      wordSeparator: cfg.wordSeparator,
      allowProposedApi: true,
      allowTransparency: isTranslucent(),
    });
    applyLigatures(cfg.ligatures);
    fit = new FitAddon();
    search = new SearchAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    const linkHandler = linkModifier === 'none'
      ? undefined
      : (ev: MouseEvent, uri: string) => {
          const ok = (linkModifier === 'ctrl' && ev.ctrlKey)
            || (linkModifier === 'alt' && ev.altKey)
            || (linkModifier === 'meta' && ev.metaKey);
          if (ok) window.open(uri, '_blank', 'noopener,noreferrer');
        };
    term.loadAddon(new WebLinksAddon(linkHandler));
    term.open(host);
    cleanupFileDrag?.();
    cleanupFileDrag = installTerminalFileDrag(host, {
      onDragOver: onTransferDragOver,
      onDrop: (ev) => { void onTransferDrop(ev); },
    });
    replayCursorSeq = 0;
    replayCachedOutput(session.id, true);
    await applyRenderer(cfg.renderer === 'native' ? 'dom' : cfg.renderer);
    frontendReady = true;
    reportFrontendKind();
    scheduleSafeFit();

    const activeTerm = term;
    windowsCaretFix = installWindowsTerminalCaretFix(activeTerm.textarea);
    const textarea = activeTerm.textarea;
    const onTextareaFocus = () => refreshCursorBlinkIfFocused(activeTerm);
    textarea?.addEventListener('focus', onTextareaFocus);

    activeTerm.attachCustomKeyEventHandler((ev) => {
      if (ev.type === 'keydown') trackMacBackspaceKeydown(ev);
      if (ev.type !== 'keydown') return true;
      if ((ev.ctrlKey || ev.metaKey) && !ev.shiftKey && (ev.key === 'f' || ev.key === 'F')) {
        openSearch();
        return false;
      }
      return true;
    });
    macTextareaGuard = installMacTextareaInputGuard(activeTerm.textarea);

    const onVis = () => {
      documentHidden = document.hidden;
      syncPolling();
    };
    documentHidden = document.hidden;
    document.addEventListener('visibilitychange', onVis);

    term.onData((data) => {
      if (shouldSuppressMacSpuriousInput(data)) return;
      if (transferFilter) transferFilter.processTerminalInput(data);
      else sendSessionInput(data);
    });
    term.onBinary((data) => {
      if (shouldSuppressMacSpuriousInput(data)) return;
      if (transferFilter) transferFilter.processBinaryInput(data);
      else sendSessionInput(data);
    });
    term.onResize(({ cols, rows }) => {
      transferFilter?.setTerminalColumns(cols);
      void rpc.call('session.resize', { id: session.id, cols, rows });
    });
    term.onSelectionChange(() => {
      if (!copyOnSelect || !term) return;
      const sel = term.getSelection();
      if (sel) void navigator.clipboard.writeText(sel).catch(() => {});
    });
    term.onBell(() => {
      tabs.markActivity(session.id, 'bell');
      triggerBell();
    });

    let resizeFrame = 0;
    const ro = new ResizeObserver((entries) => {
      let ok = false;
      for (const entry of entries) {
        const { width, height } = entry.contentRect;
        if (width >= 2 && height >= 2) ok = true;
      }
      if (!ok) return;
      if (resizeFrame) cancelAnimationFrame(resizeFrame);
      resizeFrame = requestAnimationFrame(() => {
        resizeFrame = 0;
        safeFit();
      });
    });
    ro.observe(host);

    applyTransferDetectionFromCfg(cfg.experimentalTransferDetection);
    await configureTransferFilter(cfg.experimentalTransferDetection);
    syncPolling();
    frontendReady = true;
    reportFrontendKind();

    const onSessionsClosing = (ev: Event) => {
      const ids = (ev as CustomEvent<SessionsClosingDetail>).detail?.sessionIds;
      if (ids?.includes(session.id)) {
        dropReplayChunks(session.id);
        cancelPolling();
      }
    };
    document.addEventListener(SESSIONS_CLOSING, onSessionsClosing);

    cleanupHost = () => {
      ro.disconnect();
      document.removeEventListener(SESSIONS_CLOSING, onSessionsClosing);
      textarea?.removeEventListener('focus', onTextareaFocus);
      document.removeEventListener('visibilitychange', onVis);
    };

    if (bindShared) {
      const unbindShared = bindSharedTerminalListeners({ xtermSettingsGate: true });
      cleanupSearchListener = () => {
        cleanupHost?.();
        cleanupHost = null;
        unbindShared();
      };
    }
  }

  onMount(async () => {
    const cfg = await loadTermSettings();
    if (cfg.renderer === 'native' || cfg.renderer === 'native-surface') {
      nativeBackend = cfg.renderer === 'native-surface' ? 'native-surface' : 'webview';
      useNative = true;
      applyNativeCfg(cfg);
      applyTransferDetectionFromCfg(cfg.experimentalTransferDetection);
      void configureTransferFilter(cfg.experimentalTransferDetection);
      frontendReady = false;
      reportFrontendKind();
      // Native path: no session.poll — the engine owns the output receiver.
      cleanupSearchListener = bindSharedTerminalListeners();
      return;
    }
    await bootXterm(cfg);
  });

  let cleanupHost: (() => void) | null = null;
  let cleanupSearchListener: (() => void) | null = null;
  let macTextareaGuard: (() => void) | null = null;
  let windowsCaretFix: (() => void) | null = null;
  let lastSessionId: string | null = null;

  $effect(() => {
    const currentSessionId = session.id;
    if (lastSessionId === null) {
      lastSessionId = currentSessionId;
      return;
    }
    if (currentSessionId === lastSessionId) return;
    lastSessionId = currentSessionId;
    exited = false;
    reconnecting = false;
    exitedOverlayEl = null;
    lastTabVisibleForReplay = tabVisible;
    replayCursorSeq = 0;
    sessionWriteChain = Promise.resolve();
    if (term) void configureTransferFilter(transferDetectionEnabled, true);
  });

  $effect(() => {
    void active;
    void tabVisible;
    void documentHidden;
    syncPolling();
  });

  $effect(() => {
    void layoutVisible;
    if (!layoutVisible) return;
    scheduleSafeFit(true);
  });

  $effect(() => {
    void tabVisible;
    void active;
    const becameVisible = tabVisible && !lastTabVisibleForReplay;
    lastTabVisibleForReplay = tabVisible;
    if (!tabVisible || !layoutVisible || !term) return;
    if (becameVisible) replayCachedOutput(session.id);
    scheduleSafeFit(true);
    scheduleTerminalFit(() => {
      if (!term) return;
      try {
        term.refresh(0, term.rows - 1);
      } catch {
        /* renderer not ready */
      }
    });
  });

  let frontendRequest: number | undefined;

  function reportFrontendKind(finishEpoch?: number) {
    const kind: TerminalFrontendKind = frontendError
      ? 'unavailable'
      : !frontendReady
      ? 'pending'
      : useNative
        ? (nativeBackend === 'native-surface' ? 'native-surface' : 'native')
        : (`xterm-${activeRenderer}` as TerminalFrontendKind);
    traceView({ stage: 'pane.report', epoch: nativeViewEpoch, controller: traceController(nativeCtl), status: kind, cancelled: destroyed });
    if (finishEpoch !== undefined) terminalUi.finishRequest(session.id, finishEpoch, kind);
    else terminalUi.report(session.id, kind);
  }

  $effect(() => {
    void useNative;
    void activeRenderer;
    void frontendReady;
    reportFrontendKind();
  });

  let wasActive = false;
  $effect(() => {
    const isActive = active;
    const becameActive = isActive && !wasActive;
    wasActive = isActive;
    if (!becameActive) return;
    untrack(() => tabs.clearActivity(session.id));
    // Immediate OS/DOM focus on activate — scheduleTerminalFit retries after layout.
    if (!searchOpen) {
      if (useNative) nativeView?.focus({ force: true });
      else focusTerminalAuto(term, host);
    }
    scheduleTerminalFit(() => {
      safeFit(true);
      if (searchOpen) return;
      if (useNative) nativeView?.focus({ force: true });
      else focusTerminalAuto(term, host);
    });
  });

  async function teardownXtermInstance() {
    cancelPolling();
    try { rendererAddon?.dispose(); } catch { /* ignore */ }
    rendererAddon = null;
    activeRenderer = 'dom';
    windowsCaretFix?.();
    windowsCaretFix = null;
    macTextareaGuard?.();
    macTextareaGuard = null;
    try { search?.dispose(); } catch { /* ignore */ }
    search = null;
    try { fit?.dispose(); } catch { /* ignore */ }
    fit = null;
    try { term?.dispose(); } catch { /* ignore */ }
    term = null;
    cleanupHost?.();
    cleanupHost = null;
  }

  async function switchToNativeFrontend(
    cfg: Awaited<ReturnType<typeof loadTermSettings>>,
  ) {
    if (switchingFrontend) return;
    switchingFrontend = true;
    try {
      seedNativeFromReplay = cachedReplayChunks(session.id).length > 0;
      await teardownXtermInstance();
      applyNativeCfg(cfg);
      applyTransferDetectionFromCfg(cfg.experimentalTransferDetection);
      await configureTransferFilter(cfg.experimentalTransferDetection);
      nativeBackend = cfg.renderer === 'native-surface' ? 'native-surface' : 'webview';
      const ready = beginNativeReadiness();
      useNative = true;
      frontendReady = false;
      reportFrontendKind();
      await tick();
      // NativeTerminalView mounts and calls onNativeReady after attach/snapshot.
      await ready;
    } finally {
      switchingFrontend = false;
    }
  }

  async function switchToXtermFrontend(
    cfg: Awaited<ReturnType<typeof loadTermSettings>>,
  ) {
    if (switchingFrontend) return;
    switchingFrontend = true;
    try {
      try {
        const exported = await rpc.call<{ text?: string }>('terminal.exportText', {
          id: session.id,
        });
        if (typeof exported?.text === 'string') {
          replaceReplayWithText(session.id, exported.text);
        }
      } catch (err) {
        console.warn('native exportText failed', err);
      }
      const resumeTransfer = await suspendTransferFilterForViewSwitch();
      try {
        // Release OS overlay first; only the explicit xterm handoff relinquishes the engine.
        await nativeView?.release();
        // Must detach so session.poll can feed xterm (engine owns output while attached).
        await rpc.call('terminal.detach', { id: session.id });
        nativeCtl = null;
        useNative = false;
        frontendReady = false;
        reportFrontendKind();
        await tick();
        if (!host) {
          frontendReady = true;
          reportFrontendKind();
          return;
        }
        // Shared document listeners already bound from the previous frontend.
        await bootXterm(cfg, { bindSharedListeners: false });
      } finally {
        await resumeTransfer();
      }
    } finally {
      switchingFrontend = false;
    }
  }

  // Live-apply settings changes (font / theme / scrollback) without re-mounting
  // the terminal. xterm's options object accepts partial updates.
  const settingsReload = createTerminalSettingsReload({
    load: loadTermSettings,
    apply: applyTermSettingsLive,
  });

  function reloadSettingsLive() {
    return settingsReload.request().catch((error: unknown) => onError?.(String(error)));
  }

  async function applyTermSettingsLive(cfg: Awaited<ReturnType<typeof loadTermSettings>>) {
    const request = terminalUi.requestEpoch(session.id);
    frontendRequest = request;
    const wantNative = cfg.renderer === 'native' || cfg.renderer === 'native-surface';
    if (wantNative !== useNative) {
      if (wantNative) await switchToNativeFrontend(cfg);
      else await switchToXtermFrontend(cfg);
      return;
    }
    if (useNative) {
      const nextBackend = cfg.renderer === 'native-surface' ? 'native-surface' : 'webview';
      if (nextBackend !== nativeBackend) {
        const resumeTransfer = await suspendTransferFilterForViewSwitch();
        try {
          if (nativeBackend === 'webview' && nextBackend === 'native-surface' && nativeView) {
            applyNativeCfg(cfg);
            await tick(); // stage the requested font/theme, not the previous settings
            const sourceView = nativeView;
            const sourceController = nativeCtl;
            const sourceEpoch = nativeViewEpoch;
            let sourceClosing = false;
            const onSourceClosing = (event: Event) => {
              if ((event as CustomEvent<SessionsClosingDetail>).detail?.sessionIds?.includes(session.id)) {
                sourceClosing = true; sourceView.cancelViewTransition();
              }
            };
            document.addEventListener(SESSIONS_CLOSING, onSourceClosing);
            traceView({ stage: 'pane.stage.begin', epoch: sourceEpoch, controller: traceController(sourceController) });
            try {
              await sourceView.stageNativeSurface(request ?? 0);
              traceView({ stage: 'pane.stage.success', epoch: sourceEpoch, controller: traceController(sourceController) });
            } catch (error) {
              traceView({ stage: 'pane.stage.catch', epoch: sourceEpoch, controller: traceController(sourceController), cancelled: destroyed || sourceClosing });
              // App marked pending before dispatching the requested override. A
              // retained source needs its actual state re-reported after rollback,
              // but an obsolete callback must not resurrect a closed/new view.
              if (sourceController && nativeView === sourceView && useNative && nativeBackend === 'webview') {
                restoreRetainedSourceStatus({
                  source: sourceController, generation: sourceEpoch,
                  current: () => ({ source: nativeCtl, generation: nativeViewEpoch,
                    ready: frontendReady && !frontendError && sourceView.isRetainedWebviewReady(sourceController),
                    closed: destroyed || exited || sourceClosing }),
                  report: () => reportFrontendKind(request),
                });
              }
              throw error; // preserve the original diagnostic
            } finally {
              document.removeEventListener(SESSIONS_CLOSING, onSourceClosing);
            }
            nativeBackend = nextBackend;
            reportFrontendKind(request);
            return;
          }
          if (nativeBackend === 'native-surface' && nextBackend === 'webview' && nativeView && nativeCtl) {
            const sourceView = nativeView, sourceController = nativeCtl, sourceEpoch = nativeViewEpoch;
            let closing = false;
            const cancel = (event: Event) => {
              if ((event as CustomEvent<SessionsClosingDetail>).detail?.sessionIds?.includes(session.id)) {
                closing = true; sourceView.cancelViewTransition();
              }
            };
            document.addEventListener(SESSIONS_CLOSING, cancel);
            applyNativeCfg(cfg);
            await tick();
            try { await sourceView.stageWebviewSurface(request ?? 0); }
            catch (error) {
              restoreRetainedSourceStatus({ source: sourceController, generation: sourceEpoch,
                current: () => ({ source: nativeCtl, generation: nativeViewEpoch,
                  ready: sourceView.isRetainedNativeReady(sourceController), closed: destroyed || exited || closing }),
                report: () => reportFrontendKind(request) });
              throw error;
            } finally { document.removeEventListener(SESSIONS_CLOSING, cancel); }
            nativeBackend = nextBackend;
            reportFrontendKind(request);
            return;
          }
          await nativeView?.release();
          nativeCtl = null;
          frontendReady = false;
          frontendError = false;
          nativeBackend = nextBackend;
          nativeViewEpoch++;
        } finally {
          await resumeTransfer();
        }
      }
      applyNativeCfg(cfg);
      nativeCtl?.setFont({
        family: nativeFontFamily,
        size: nativeFontSize,
        weight: nativeFontWeight,
        weightBold: nativeFontWeightBold,
        linePadding: nativeLinePadding,
      });
      if (nativeTheme) nativeCtl?.setTheme(nativeTheme);
      applyTransferDetectionFromCfg(cfg.experimentalTransferDetection);
      await configureTransferFilter(cfg.experimentalTransferDetection);
      return;
    }
    if (!term) return;
    term.options.fontFamily = cfg.fontFamily;
    term.options.fontSize = cfg.fontSize + sessionFontDelta;
    term.options.fontWeight = cfg.fontWeight;
    term.options.fontWeightBold = cfg.fontWeightBold;
    term.options.theme = paletteFromCfg(cfg);
    term.options.cursorBlink = cfg.cursorBlink;
    term.options.cursorStyle = cfg.cursorStyle;
    term.options.scrollback = cfg.scrollback;
    term.options.lineHeight = 1 + cfg.linePadding / Math.max(cfg.fontSize, 1);
    term.options.minimumContrastRatio = cfg.minContrastRatio;
    term.options.macOptionIsMeta = cfg.altIsMeta;
    term.options.scrollOnUserInput = cfg.scrollOnInput;
    term.options.wordSeparator = cfg.wordSeparator;
    term.options.allowTransparency = isTranslucent();
    applyTransferDetectionFromCfg(cfg.experimentalTransferDetection);
    await configureTransferFilter(cfg.experimentalTransferDetection);
    await applyRenderer(cfg.renderer);
    applyLigatures(cfg.ligatures);
    refreshCursorBlinkIfFocused(term);
    // Force a full redraw so canvas/webgl renderers invalidate their glyph
    // atlas and pick up the new palette / font metrics. xterm only repaints
    // changed cells otherwise, which leaves stale colors on screen.
    try { term.refresh(0, term.rows - 1); } catch { /* renderer not ready */ }
    scheduleSafeFit();
  }

  $effect(() => {
    void settingsRev;
    void rendererOverride;
    if (useNative) {
      void reloadSettingsLive();
      return;
    }
    if (!term) return;
    void reloadSettingsLive();
  });

  onDestroy(() => {
    destroyed = true;
    settingsReload.dispose();
    resolveNativeReady?.();
    resolveNativeReady = null;
    rejectNativeReady = null;
    terminalUi.clear(session.id);
    void nativeCtl?.dispose();
    nativeCtl = null;
    const termToDispose = term;
    const searchToDispose = search;
    const rendererToDispose = rendererAddon;
    const filterToStop = transferFilter;
    cleanupHost?.();
    cleanupHost = null;
    cleanupFileDrag?.();
    cleanupFileDrag = null;
    rawBypassGate.dispose();
    cleanupSearchListener?.();
    cleanupSearchListener = null;
    macTextareaGuard?.();
    macTextareaGuard = null;
    windowsCaretFix?.();
    windowsCaretFix = null;
    cancelPolling();
    clearTransferNotice();
    transferFilter = null;
    search = null;
    term = null;
    rendererAddon = null;
    if (termToDispose) {
      scheduleTerminalTeardown({
        term: termToDispose,
        search: searchToDispose,
        rendererAddon: rendererToDispose,
        beforeDispose: () => {
          if (filterToStop?.isTransferringFiles()) filterToStop.stopTransferringFiles();
        },
      });
    }
  });

  // --- search ---
  export function openSearch() {
    searchOpen = true;
    requestAnimationFrame(() => {
      const el = document.getElementById(`search-input-${session.id}`);
      (el as HTMLInputElement | null)?.focus();
    });
  }
  function runSearch(direction: 'next' | 'prev') {
    if (useNative) {
      if (!searchQuery || !nativeCtl) return;
      void nativeCtl.search(searchQuery, {
        caseSensitive: searchCase,
        regex: searchRegex,
        direction,
      });
      return;
    }
    if (!search || !searchQuery) return;
    const opts = { caseSensitive: searchCase, regex: searchRegex };
    if (direction === 'next') search.findNext(searchQuery, opts);
    else search.findPrevious(searchQuery, opts);
  }
  function closeSearch() {
    searchOpen = false;
    search?.clearDecorations();
    requestAnimationFrame(() => {
      if (useNative) nativeView?.focus();
      else focusTerminalIfAllowed(term);
    });
  }

  // --- context menu ---
  async function onContextMenu(ev: MouseEvent) {
    if (rightClickAction === 'paste') { void doPasteFromClipboard(); ev.preventDefault(); return; }
    if (rightClickAction === 'select-word') {
      // xterm doesn't expose select-word; trigger a synthetic double-click.
      const cell = host?.querySelector('.xterm-rows');
      cell?.dispatchEvent(new MouseEvent('dblclick', {
        bubbles: true,
        clientX: ev.clientX,
        clientY: ev.clientY,
      }));
      ev.preventDefault();
      return;
    }
    ev.preventDefault();
    focusTerminalIfAllowed(term);
    menuX = ev.clientX;
    menuY = ev.clientY;
    menuOpen = true;
    await tick();
    const clamped = clampMenuToViewport(menuX, menuY, menuEl);
    menuX = clamped.x;
    menuY = clamped.y;
  }
  async function doCopy() {
    menuOpen = false;
    const sel = useNative
      ? await nativeCtl?.selectionText()
      : term?.getSelection();
    if (!sel) return;
    if (copyWithFormatting && navigator.clipboard && 'write' in navigator.clipboard) {
      try {
        const html = `<pre style="font-family: ${term?.options.fontFamily ?? 'monospace'}">${escapeHtml(sel)}</pre>`;
        const item = new ClipboardItem({
          'text/plain': new Blob([sel], { type: 'text/plain' }),
          'text/html': new Blob([html], { type: 'text/html' }),
        });
        await navigator.clipboard.write([item]);
        return;
      } catch {
        // fall through to plain text
      }
    }
    await navigator.clipboard.writeText(sel).catch(() => {});
  }
  function escapeHtml(s: string): string {
    return s.replace(/[&<>"']/g, (c) => ({
      '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
    } as Record<string, string>)[c]!);
  }
  async function doPasteFromClipboard() {
    menuOpen = false;
    const text = await navigator.clipboard.readText().catch(() => '');
    if (!text) return;
    await pasteText(text);
  }
  async function pasteText(raw: string, nativeValid?: () => Promise<boolean>) {
    const pasteSession = session.id;
    const text = await preparePasteText(raw, {
      trim: pasteTrimWhitespace, flatten: pasteFlattenNewlines, warn: pasteMultilineWarn,
      confirm: (count) => appConfirm(i18n.t('terminal.pasteConfirm', { count })),
    }, nativeValid ? async () => !exited && session.id === pasteSession && await nativeValid() : undefined);
    if (text === null) return;
    if (useNative) {
      if (nativeValid) await nativeCtl?.pasteFromNative(text, nativeValid);
      else await nativeCtl?.paste(text);
      return;
    }
    if (bracketedPaste) {
      term?.paste(text);
    } else {
      // Bypass bracketed-paste wrapping by writing through onData path.
      sendSessionInput(text);
    }
  }
  async function doPaste() { await doPasteFromClipboard(); }
  function triggerBell() {
    if (bellMode === 'off') return;
    const surface = host ?? document.querySelector(
      `.terminal-host`,
    ) as HTMLElement | null;
    if (bellMode === 'visual') {
      if (!surface) return;
      surface.classList.add('bell-flash');
      if (bellFlashHandle != null) window.clearTimeout(bellFlashHandle);
      bellFlashHandle = window.setTimeout(() => {
        surface.classList.remove('bell-flash');
        bellFlashHandle = null;
      }, 180);
    } else if (bellMode === 'audible') {
      try {
        const AC = (window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext);
        const ctx = new AC();
        const osc = ctx.createOscillator();
        const gain = ctx.createGain();
        osc.type = 'sine';
        osc.frequency.value = 880;
        gain.gain.value = 0.05;
        osc.connect(gain).connect(ctx.destination);
        osc.start();
        osc.stop(ctx.currentTime + 0.12);
        osc.onended = () => ctx.close();
      } catch { /* audio context may be unavailable */ }
    }
  }
  function doClear() {
    menuOpen = false;
    if (useNative) {
      // Clear viewport by scrolling to bottom + feeding form feed; keep scrollback.
      void nativeCtl?.clearSelection();
      return;
    }
    term?.clear();
  }
  function doSearchAction() {
    menuOpen = false;
    openSearch();
  }
  function doSelectAll() {
    menuOpen = false;
    if (useNative) {
      void nativeCtl?.selectAll();
      return;
    }
    term?.selectAll();
  }

  function onTransferDragOver(ev: DragEvent) {
    if (isPaneDragActive()) {
      ev.preventDefault();
      if (ev.dataTransfer) ev.dataTransfer.dropEffect = 'move';
      return;
    }
    // Accept drops on the focused pane even when detection is off, so we can
    // surface a clear "enable in Settings" error instead of a silent no-op.
    allowTerminalFileDragOver(ev, active && !exited);
  }

  async function onTransferDrop(ev: DragEvent) {
    if (isPaneDragActive()) return;
    if (!active || exited) return;
    ev.preventDefault();
    ev.stopPropagation();
    const dt = ev.dataTransfer;
    if (!dt) return;
    clearTransferNotice();

    if (!transferDetectionEnabled) {
      onError?.(i18n.t('terminal.transferDisabled'));
      return;
    }
    if (!transferFilter) {
      onError?.(i18n.t('terminal.transferNotReady'));
      return;
    }

    try {
      const paths = await resolveDropFilePaths(dt);
      if (transferFilter.isTransferringFiles()) {
        // Handshake already running — feed paths into chooseSendFiles waiter/queue.
        if (paths.length > 0) {
          if (!notifyTrzszDragPaths(paths)) queueTrzszDragPaths(paths);
        }
        return;
      }

      if (paths.length > 0) {
        // Prefer an in-flight remote `trz` waiter; otherwise start upload ourselves.
        if (notifyTrzszDragPaths(paths)) return;
        await transferFilter.uploadFiles(paths);
        return;
      }
      const items = dt.items;
      if (items && items.length > 0) {
        await transferFilter.uploadFiles(items);
        return;
      }
      onError?.(i18n.t('terminal.transferNoPaths'));
    } catch (err) {
      console.warn('trzsz upload failed', err);
      onError?.(i18n.t('terminal.transferFailed', { message: (err as Error).message }));
    }
  }

  async function onPointerUp(ev: PointerEvent) {
    // Middle-click paste.
    if (middleClickPaste && ev.button === 1) {
      ev.preventDefault();
      const text = await navigator.clipboard.readText().catch(() => '');
      if (text) await pasteText(text);
      return;
    }
    // Legacy right-click paste path (when rightClickAction='paste' the
    // contextmenu handler already pastes; this is a no-op safeguard).
    if (rmbPaste && ev.button === 2) {
      ev.preventDefault();
      const text = await navigator.clipboard.readText().catch(() => '');
      if (text) await pasteText(text);
    }
  }

  async function openExternalNativeTerminal() {
    menuOpen = false;
    try {
      await spawnDetachedNativeTerminal(rpc, session);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      onError?.(msg);
    }
  }

  async function onNativeReady(ctl: NativeTerminalController) {
    if (destroyed) return;
    nativeCtl = ctl;
    useNative = true;
    ctl.setVisible(tabVisible && layoutVisible);
    ctl.setWriteInput(nativeWriteInput);
    ctl.setLinkModifier(linkModifier);
    ctl.setCopyOnSelect(copyOnSelect);
    ctl.setTransferDetect(nativeTransferDetect);
    cleanupFileDrag?.();
    if (nativeHost) {
      cleanupFileDrag = installTerminalFileDrag(nativeHost, {
        onDragOver: onTransferDragOver,
        onDrop: (ev) => { void onTransferDrop(ev); },
      });
    }
    if (seedNativeFromReplay) {
      seedNativeFromReplay = false;
      const chunks = cachedReplayChunks(session.id);
      for (const chunk of chunks) {
        try {
          await ctl.feed(chunk.bytes);
        } catch (err) {
          onNativeError(`native replay feed failed: ${String(err)}`);
          return;
        }
      }
    }
    frontendError = false;
    frontendReady = true;
    reportFrontendKind(frontendRequest);
    resolveNativeReady?.();
    resolveNativeReady = null;
    rejectNativeReady = null;
    // OS surface focuses after its latest geometry/show update; do not duplicate that IPC here.
    if (nativeBackend === 'webview' && active && !nativeSurfaceOccluded && !searchOpen && !menuOpen) nativeView?.focus();
  }

  function onNativeExit() {
    if (!exited) markExited();
  }
</script>

{#if useNative}
  <div
    bind:this={nativeHost}
    role="application"
    data-aerotab-context-menu=""
    class="terminal-surface terminal-host h-full w-full min-h-0 min-w-0 relative overflow-hidden
           {active ? '' : 'opacity-[0.92]'}
           {active || nativeBackend !== 'native-surface' ? '' : 'pointer-events-none'}"
    oncontextmenu={onContextMenu}
    onpointerup={onPointerUp}
    ondragover={onTransferDragOver}
    ondrop={onTransferDrop}
  >
    {#key `${session.id}:${nativeViewEpoch}`}
    <NativeTerminalView
      bind:this={nativeView}
      backend={nativeBackend}
      {onActivate}
      {active}
      occluded={nativeSurfaceOccluded || searchOpen || menuOpen || (exited && active) || !!transferNotice}
      {rpc}
      sessionId={session.id}
      fontFamily={nativeFontFamily}
      fontSize={nativeFontSize}
      fontWeight={nativeFontWeight}
      fontWeightBold={nativeFontWeightBold}
      linePadding={nativeLinePadding}
      scrollback={nativeScrollback}
      cursorBlink={nativeCursorBlink}
      altIsMeta={nativeAltIsMeta}
      ligatures={nativeLigatures}
      theme={nativeTheme}
      translucent={isTranslucent()}
      transferDetect={nativeTransferDetect}
      linkModifier={linkModifier}
      copyOnSelect={copyOnSelect}
      writeInput={nativeWriteInput}
      writeNativeBytes={nativeWriteBytes}
      onNativePaste={pasteText}
      visible={tabVisible && layoutVisible}
      onBell={triggerBell}
      onExit={onNativeExit}
      onActivity={() => { if (!active) tabs.markActivity(session.id, 'output'); }}
      onRaw={onNativeRaw}
      onTrzszDetected={onNativeTrzszDetected}
      onRecovered={(ctl, backend, request) => {
        if (destroyed || exited || ctl !== nativeCtl || !nativeView || nativeBackend !== backend) return;
        const ready = backend === 'native-surface' ? nativeView.isRetainedNativeReady(ctl) : nativeView.isRetainedWebviewReady(ctl);
        if (!ready) return;
        frontendError = false; frontendReady = true;
        reportFrontendKind(request || undefined);
      }}
      onReady={onNativeReady}
      onError={onNativeError}
    />
    {/key}
  </div>
{:else}
  <div
    bind:this={host}
    role="application"
    data-aerotab-context-menu=""
    class="terminal-surface terminal-host h-full w-full min-h-0 min-w-0 relative overflow-hidden
           {active ? '' : 'opacity-[0.92]'}"
    oncontextmenu={onContextMenu}
    onpointerdown={() => { if (!active) onActivate?.(); }}
    onpointerup={onPointerUp}
    ondragover={onTransferDragOver}
    ondrop={onTransferDrop}
  ></div>
{/if}

{#if exited}
  <div class="absolute bottom-3 right-3 z-20 max-w-[min(320px,calc(100%-24px))] pointer-events-none">
    <div
      bind:this={exitedOverlayEl}
      tabindex="0"
      role="toolbar"
      aria-label={i18n.t('terminal.sessionEnded')}
      class="terminal-status-chip pointer-events-auto flex items-center gap-3 bg-[var(--color-panel)]/96 border border-[var(--color-border)]
                rounded shadow-xl px-3 py-2 text-[12px] text-[var(--color-fg)] backdrop-blur outline-none"
      onkeydown={onExitedKeydown}
    >
      <div class="min-w-0">
        <div class="text-[var(--color-danger)] font-semibold leading-tight">{i18n.t('terminal.sessionEnded')}</div>
        <div class="text-[var(--color-fg-muted)] leading-tight truncate">{i18n.t('terminal.historyVisible')}</div>
      </div>
      {#if canReconnect}
        <button
          type="button"
          class="btn-primary shrink-0 text-[12px] px-2 py-1"
          disabled={reconnecting}
          title={bindingLabel('session-ended-reconnect')}
          onclick={() => { void reconnectSession(); }}
        >
          {reconnecting ? i18n.t('terminal.reconnecting') : i18n.t('terminal.reconnect')}
          {#if bindingLabel('session-ended-reconnect')}
            <span class="opacity-60 ml-1">({bindingLabel('session-ended-reconnect')})</span>
          {/if}
        </button>
      {/if}
      {#if onClosePane}
        <button
          type="button"
          class="btn-secondary shrink-0 text-[12px] px-2 py-1"
          title={bindingLabel('session-ended-close')}
          onclick={() => onClosePane?.()}
        >
          {i18n.t('common.close')}
          {#if bindingLabel('session-ended-close')}
            <span class="opacity-60 ml-1">({bindingLabel('session-ended-close')})</span>
          {/if}
        </button>
      {/if}
    </div>
  </div>
{/if}

{#if transferNotice && active}
  <div class="absolute bottom-3 left-3 z-20 max-w-[min(380px,calc(100%-24px))] pointer-events-none">
    <div class="terminal-status-chip pointer-events-auto flex items-center gap-3 bg-[var(--color-panel)]/96 border border-[var(--color-border)]
                rounded shadow-xl px-3 py-2 text-[12px] text-[var(--color-fg)] backdrop-blur">
      <div class="shrink-0 text-[var(--color-accent)]">
        {#if transferNotice.direction === 'upload'}
          <Upload size={15} />
        {:else if transferNotice.direction === 'download'}
          <Download size={15} />
        {:else}
          <Info size={15} />
        {/if}
      </div>
      <div class="min-w-0">
        <div class="text-[var(--color-accent)] font-semibold leading-tight truncate">{transferTitle(transferNotice)}</div>
        <div class="text-[var(--color-fg-muted)] leading-tight">{transferHint(transferNotice)}</div>
      </div>
      {#if canOpenSftp}
        <button type="button" class="btn-secondary shrink-0 text-[12px] px-2 py-1 inline-flex items-center gap-1.5" onclick={() => onOpenSftp?.()}>
          <FolderOpen size={12} /> {i18n.t('terminal.transferOpenSftp')}
        </button>
      {/if}
      <button
        type="button"
        class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]"
        title={i18n.t('terminal.transferDismiss')}
        aria-label={i18n.t('terminal.transferDismiss')}
        onclick={clearTransferNotice}
      >
        <X size={12} />
      </button>
    </div>
  </div>
{/if}

{#if searchOpen}
  <div data-aerotab-focus-zone="" class="terminal-search absolute top-2 right-2 z-30 flex items-center gap-1 bg-[var(--color-panel)]/95
              border border-[var(--color-border)] rounded shadow-lg backdrop-blur p-1"
       class:pointer-events-none={!active}
       class:opacity-0={!active}>
    <input id="search-input-{session.id}"
           type="search" aria-label={i18n.t('common.search')} placeholder={i18n.t('common.search')} bind:value={searchQuery}
           onkeydown={(e) => {
             if (e.key === 'Enter') { runSearch(e.shiftKey ? 'prev' : 'next'); }
             else if (e.key === 'Escape') { closeSearch(); }
           }}
           class="bg-[var(--color-bg)] text-[var(--color-fg)] text-[12px] px-2 py-1 rounded
                  border border-[var(--color-border)] outline-none w-[200px]" />
    <button type="button" aria-label={i18n.t('terminal.searchCaseSensitive')} aria-pressed={searchCase} title={i18n.t('terminal.searchCaseSensitive')}
            class="p-1 rounded {searchCase ? 'text-[var(--color-accent)] bg-[var(--color-bg)]' : 'text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]'}"
            onclick={() => (searchCase = !searchCase)}>
      <CaseSensitive size={13} />
    </button>
    <button type="button" aria-label={i18n.t('terminal.searchRegex')} aria-pressed={searchRegex} title={i18n.t('terminal.searchRegex')}
            class="p-1 rounded {searchRegex ? 'text-[var(--color-accent)] bg-[var(--color-bg)]' : 'text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]'}"
            onclick={() => (searchRegex = !searchRegex)}>
      <Regex size={13} />
    </button>
    <button type="button" aria-label={i18n.t('common.previous')} title={i18n.t('common.previous')}
            class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]"
            onclick={() => runSearch('prev')}><ChevronUp size={13} /></button>
    <button type="button" aria-label={i18n.t('common.next')} title={i18n.t('common.next')}
            class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]"
            onclick={() => runSearch('next')}><ChevronDown size={13} /></button>
    <button type="button" title={i18n.t('common.close')} aria-label={i18n.t('terminal.closeSearch')}
            class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]"
            onclick={closeSearch}><X size={12} /></button>
  </div>
{/if}

{#if menuOpen && active}
  <div use:portal class="contents">
    <div
      role="presentation"
      data-aerotab-menu-open=""
      class="fixed inset-0 z-40"
      onclick={() => (menuOpen = false)}
      oncontextmenu={(e) => { e.preventDefault(); menuOpen = false; }}
    ></div>
    <div
      bind:this={menuEl}
      use:workspaceMenu={() => { menuOpen = false; }}
      role="menu"
      tabindex="-1"
      data-aerotab-context-menu=""
      data-aerotab-menu-open=""
      class="panel fixed z-[41] min-w-[200px] py-1 text-[12.5px] text-[var(--color-fg)]"
      style="left: {menuX}px; top: {menuY}px;"
      onkeydown={(e) => e.stopPropagation()}
      onclick={(e) => e.stopPropagation()}
    >
      <button type="button" class="menu-item menu-item--shortcut" onclick={doCopy}>
        <Copy size={15} /><span>{i18n.t('common.copy')}</span>
        {#if bindingLabel('terminal-copy')}<kbd class="kbd">{bindingLabel('terminal-copy')}</kbd>{/if}
      </button>
      <button type="button" class="menu-item" onclick={doPaste}><ClipboardPaste size={15} />{i18n.t('common.paste')}</button>
      <button type="button" class="menu-item" onclick={doSelectAll}><ScanText size={15} />{i18n.t('common.selectAll')}</button>
      <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
      {#if exited && canReconnect}
        <button type="button" class="menu-item" onclick={() => { menuOpen = false; void reconnectSession(); }}>
          <RotateCw size={15} />{i18n.t('terminal.reconnect')}
        </button>
        {#if onClosePane}
          <button type="button" class="menu-item" onclick={() => { menuOpen = false; onClosePane?.(); }}>
            <X size={15} />{i18n.t('common.close')}
          </button>
        {/if}
        <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
      {/if}
      {#if onSplitRight}
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => { menuOpen = false; onSplitRight?.(); }}>
          <Columns2 size={15} /><span>{i18n.t('tabbar.splitRight')}</span>
          <kbd class="kbd">{bindingLabel('split-right')}</kbd>
        </button>
      {/if}
      {#if onSplitDown}
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => { menuOpen = false; onSplitDown?.(); }}>
          <Rows2 size={15} /><span>{i18n.t('tabbar.splitDown')}</span>
          <kbd class="kbd">{bindingLabel('split-down')}</kbd>
        </button>
      {/if}
      {#if onMaximize}
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => { menuOpen = false; onMaximize?.(); }}>
          <Maximize2 size={15} /><span>{i18n.t('pane.maximizePane')}</span>
          <kbd class="kbd">{bindingLabel('maximize-pane')}</kbd>
        </button>
      {/if}
      {#if canOpenSftp}
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => { menuOpen = false; onOpenSftp?.(); }}>
          <FolderOpen size={15} /><span>{i18n.t('terminal.transferOpenSftp')}</span>
          <kbd class="kbd">{bindingLabel('open-sftp')}</kbd>
        </button>
      {/if}
      <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
      <button type="button" class="menu-item" onclick={doSearchAction}><Search size={15} />{i18n.t('terminal.searchAction')}</button>
      <button type="button" class="menu-item" onclick={doClear}><Eraser size={15} />{i18n.t('common.clearScreen')}</button>
      <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
      {#if canOpenExternalNative}
        <button type="button" class="menu-item" onclick={() => { void openExternalNativeTerminal(); }}>
          <ExternalLink size={15} />{i18n.t('terminal.openExternalNative')}
        </button>
      {/if}
    </div>
  </div>
{/if}

<style>
  .terminal-search {
    max-width: calc(100% - 16px);
    gap: 4px;
    padding: 6px;
    border-radius: 9px;
    background: var(--color-surface-raised);
    box-shadow: var(--shadow-panel);
  }
  .terminal-search input {
    flex: 1 1 180px;
    min-width: 60px;
    width: 220px;
    min-height: 32px;
    font-size: var(--text-compact, 14px);
    background: var(--color-bg-soft);
  }
  .terminal-search button { flex: 0 0 auto; min-width: 30px; min-height: 32px; border-radius: 5px; }
  .terminal-search button[aria-pressed='true'] { background: var(--color-accent-soft); color: var(--color-accent); }
  .terminal-search button:focus-visible, .terminal-search input:focus-visible { outline: 2px solid var(--color-accent); outline-offset: -2px; }
  .terminal-status-chip {
    flex-wrap: wrap;
    background: var(--color-surface-raised);
    border-radius: 8px;
    box-shadow: var(--shadow-panel);
    font-size: var(--text-compact, 14px);
    line-height: 1.5;
  }
  .terminal-status-chip button { min-height: 30px; font-size: var(--text-compact, 14px); }
  .terminal-status-chip button span { font-size: var(--text-meta, 12px); }
  .menu-item { width: 100%; text-align: left; color: inherit; background: transparent; border: 0; cursor: pointer; }
  .menu-item--shortcut { display: flex; align-items: center; gap: 10px; }
  .menu-item--shortcut > span { flex: 1; }
  .menu-item--shortcut > .kbd { margin-left: auto; }
</style>
