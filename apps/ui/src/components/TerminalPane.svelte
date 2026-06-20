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
</script>

<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import { SESSIONS_CLOSING, type SessionsClosingDetail } from '../lib/sessionLifecycle';
  import { scheduleTerminalTeardown } from '../lib/terminalTeardown';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { WebLinksAddon } from '@xterm/addon-web-links';
  import { SearchAddon } from '@xterm/addon-search';
  import { X, ChevronUp, ChevronDown, CaseSensitive, Regex, Upload, Download, Info, FolderOpen } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import { b64decode, b64encode } from '../lib/rpc';
  import type { SessionMeta } from '../lib/types';
  import { BUILTIN_THEMES, TOKYO_NIGHT, xtermPalette } from '../lib/theme';
  import { colorSchemeByName, toXtermTheme } from '../lib/colorSchemes';
  import { applyLigatures } from '../lib/customCss';
  import { tabs } from '../lib/tabs.svelte';
  import { terminalPollIntervalMs } from '../lib/terminalPoll';
  import { i18n } from '../lib/i18n.svelte';
  import { appConfirm } from '../lib/confirm.svelte';
  import { hotkeys } from '../lib/hotkeys';
  import { isPaneDragActive } from '../lib/paneDrag';
  import { clampMenuToViewport } from '../lib/contextMenuPosition';
  import { portal } from '../lib/portal';
  import { TerminalTransferDetector, type TerminalTransferDetection } from '../lib/terminalTransfer';
  import {
    createTrzszFilter,
    type TrzszFilterInstance,
    type TrzszTerminalInput,
    type TrzszTerminalOutput,
  } from '../lib/trzszBridge';
  import {
    installMacTextareaInputGuard,
    shouldSuppressMacSpuriousInput,
    trackMacBackspaceKeydown,
  } from '../lib/terminalInputMac';
  import { focusTerminalIfAllowed } from '../lib/modalFocus';
  import { scheduleTerminalFit } from '../lib/terminalFit';
  import { getTerminalSettings, invalidateTerminalSettingsCache } from '../lib/terminalSettingsCache';
  import { NativeEngineController } from '../lib/nativeTerminal';

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
    /** Invoked when the user clicks the pane's close button from inside (e.g. exited overlay). */
    onClosePane?: () => void;
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
    onClosePane,
    onOpenSftp,
    onSplitRight,
    onSplitDown,
    onMaximize,
    broadcastEnabled = false,
    broadcastTargetIds = [],
    onError,
  }: Props = $props();

  let host: HTMLDivElement | null = null;
  let term: Terminal | null = null;
  let fit: FitAddon | null = null;
  let search: SearchAddon | null = null;
  let rendererAddon: { dispose: () => void } | null = null;
  let activeRenderer: 'dom' | 'canvas' | 'webgl' = 'dom';
  let engineController: NativeEngineController | null = null;
  let engineCanvas: HTMLCanvasElement | null = null;
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
  let transferDetectionEnabled = false;
  let transferNotice = $state<TerminalTransferDetection | null>(null);
  let transferNoticeHandle: number | null = null;
  let transferFilter = $state<TrzszFilterInstance | null>(null);
  let transferFilterGeneration = 0;
  const transferDetector = new TerminalTransferDetector();
  const canOpenSftp = $derived(session.kind === 'Ssh' && !!onOpenSftp);

  function isLocalWindowsShell(): boolean {
    if (session.kind !== 'LocalShell') return false;
    return typeof navigator !== 'undefined' && /Win/i.test(navigator.userAgent);
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
    focusExitedOverlay();
  }

  function focusExitedOverlay() {
    requestAnimationFrame(() => exitedOverlayEl?.focus());
  }

  function bindingLabel(actionId: string): string {
    return hotkeys.getBindings(actionId)[0] ?? '';
  }

  function hostHasLayout(): boolean {
    return !!host && host.offsetWidth >= 2 && host.offsetHeight >= 2;
  }

  /** Skip fit while display:none (maximize hide) — zero-size fit clears canvas terminals. */
  function safeFit(redraw = false) {
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
    if (!term || !delta) return;
    sessionFontDelta += delta;
    const next = Math.min(32, Math.max(8, (term.options.fontSize ?? 13) + delta));
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
      if (session.profileId) {
        meta = await rpc.call('session.openSshProfile', { profile_id: session.profileId });
        next = {
          id: meta.id,
          kind: meta.kind,
          title: meta.title,
          profileId: session.profileId,
          sshProfile: session.sshProfile,
        };
      } else if (session.sshProfile) {
        meta = await rpc.call('session.openSsh', {
          title: session.title,
          profile: session.sshProfile,
        });
        next = {
          id: meta.id,
          kind: meta.kind,
          title: meta.title,
          sshProfile: session.sshProfile,
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
      for (const c of r.chunks) {
        const bytes = b64decode(c);
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
    const cs = colorSchemeByName(cfg.colorSchemeName);
    const theme = cs ? toXtermTheme(cs) : xtermPalette(cfg.theme);
    if (isTranslucent()) {
      return { ...theme, background: 'transparent' };
    }
    return theme;
  }

  async function loadTermSettingsFresh() {
    const out = {
      fontFamily: 'JetBrains Mono, Menlo, monospace',
      fontSize: 13,
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
      renderer: 'canvas' as 'dom' | 'canvas' | 'webgl',
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
        if (v.renderer === 'dom' || v.renderer === 'canvas' || v.renderer === 'webgl') out.renderer = v.renderer;
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
      if (typeof th.value === 'string') {
        const found = BUILTIN_THEMES.find((x) => x.name === th.value);
        if (found) out.theme = found;
      }
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
    // Canvas/WebGL renderers clear the glyph surface with an opaque xterm
    // background. In translucent windows that creates a black terminal slab
    // even when the app chrome is transparent. DOM renderer keeps cell
    // backgrounds in CSS/DOM and can honor a transparent theme background.
    if (isTranslucent()) out.renderer = 'dom';
    if (!out.experimentalTransferDetection) {
      transferDetector.reset();
      clearTransferNotice();
    }
    transferDetectionEnabled = out.experimentalTransferDetection;
    return out;
  }

  async function loadTermSettings() {
    return getTerminalSettings(() => loadTermSettingsFresh());
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

  function sendSessionInput(input: TrzszTerminalInput): void {
    if (typeof input === 'string') {
      if (input.includes('\x03')) {
        interruptOutputCooldownUntil = performance.now() + INTERRUPT_OUTPUT_COOLDOWN_MS;
      }
      void sendSessionBytes(encoder.encode(input))
        .catch((err: unknown) => console.warn('terminal write failed', err));
      return;
    }
    void terminalInputBytes(input)
      .then(sendSessionBytes)
      .catch((err: unknown) => console.warn('terminal write failed', err));
  }

  function writeTerminalOutput(output: TrzszTerminalOutput): void {
    if (!term) return;
    if (typeof output === 'string' || output instanceof Uint8Array) {
      term.write(output);
      return;
    }
    if (output instanceof ArrayBuffer) {
      term.write(new Uint8Array(output));
      return;
    }
    if (output instanceof Blob) {
      void output.arrayBuffer()
        .then((buffer) => term?.write(new Uint8Array(buffer)))
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

  async function configureTransferFilter(enabled: boolean): Promise<void> {
    const generation = ++transferFilterGeneration;
    if (!enabled) {
      if (transferFilter?.isTransferringFiles()) transferFilter.stopTransferringFiles();
      transferFilter = null;
      return;
    }
    if (transferFilter) {
      transferFilter.setTerminalColumns(term?.cols ?? 80);
      return;
    }
    try {
      const filter = await createTrzszFilter({
        writeToTerminal: writeTerminalOutput,
        sendToServer: sendSessionInput,
        terminalColumns: term?.cols ?? 80,
        isWindowsShell: isLocalWindowsShell(),
        maxDataChunkSize: 1024 * 1024,
        dragInitTimeout: 8000,
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

  async function applyRenderer(renderer: 'dom' | 'canvas' | 'webgl') {
    if (!term) return;
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
  }

  onMount(async () => {
    if (!host) return;
    const cfg = await loadTermSettings();
    term = new Terminal({
      fontFamily: cfg.fontFamily,
      fontSize: cfg.fontSize,
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
    // Link modifier: if a modifier is required, intercept and only follow
    // when the matching key is held; otherwise default web-links behaviour.
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
    replayCachedOutput(session.id, true);
    await applyRenderer(cfg.renderer);
    scheduleSafeFit();

    // Intercept Ctrl+F so the browser doesn't fire its own find-in-page.
    // macOS WKWebView: after Backspace, filter stray space only — xterm must encode keys (vim/zsh).
    const activeTerm = term;
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

    // App-level Ctrl+F also routes here when this pane is active.
    const searchListener = () => { if (active) openSearch(); };
    document.addEventListener('aerotab:search', searchListener);
    // Settings-changed fallback (in addition to the settingsRev prop) — fires
    // when any settings section calls settingsCoord.bumpRev(). Forces a
    // palette/font reload directly so live-apply works even if the prop
    // chain is somehow stale.
    const settingsListener = () => {
      invalidateTerminalSettingsCache();
      if (!active || !tabVisible) return;
      void reloadSettingsLive();
    };
    document.addEventListener('aerotab:settings-changed', settingsListener);
    const focusListener = (ev: Event) => {
      const detail = (ev as CustomEvent<{ sessionId?: string }>).detail;
      if (detail?.sessionId && detail.sessionId !== session.id) return;
      if (!active) return;
      if (exited) {
        focusExitedOverlay();
        return;
      }
      requestAnimationFrame(() => focusTerminalIfAllowed(term));
    };
    const fitListener = (ev: Event) => {
      const detail = (ev as CustomEvent<{ sessionId?: string }>).detail;
      if (detail?.sessionId && detail.sessionId !== session.id) return;
      scheduleSafeFit(true);
    };
    const copyListener = () => { if (active) void doCopy(); };
    const fontListener = (ev: Event) => {
      if (!active) return;
      const delta = (ev as CustomEvent<number>).detail;
      if (typeof delta === 'number') adjustFontSize(delta);
    };
    const endedListener = (ev: Event) => {
      if (!active || !exited) return;
      const action = (ev as CustomEvent<'close' | 'reconnect'>).detail;
      if (action === 'close') onClosePane?.();
      else if (action === 'reconnect' && canReconnect) void reconnectSession();
    };
    document.addEventListener('aerotab:terminal-copy', copyListener);
    document.addEventListener('aerotab:terminal-font-delta', fontListener);
    document.addEventListener('aerotab:session-ended-action', endedListener);
    document.addEventListener('aerotab:focus-pane', focusListener);
    document.addEventListener('aerotab:fit-pane', fitListener);
    const onVis = () => {
      documentHidden = document.hidden;
      syncPolling();
    };
    documentHidden = document.hidden;
    document.addEventListener('visibilitychange', onVis);
    cleanupSearchListener = () => {
      document.removeEventListener('aerotab:search', searchListener);
      document.removeEventListener('aerotab:settings-changed', settingsListener);
      document.removeEventListener('aerotab:terminal-copy', copyListener);
      document.removeEventListener('aerotab:terminal-font-delta', fontListener);
      document.removeEventListener('aerotab:session-ended-action', endedListener);
      document.removeEventListener('aerotab:focus-pane', focusListener);
      document.removeEventListener('aerotab:fit-pane', fitListener);
      document.removeEventListener('visibilitychange', onVis);
    };

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
    if (host) ro.observe(host);

    await configureTransferFilter(cfg.experimentalTransferDetection);
    syncPolling();

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
    };
  });

  let cleanupHost: (() => void) | null = null;
  let cleanupSearchListener: (() => void) | null = null;
  let macTextareaGuard: (() => void) | null = null;
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

  $effect(() => {
    if (!active) return;
    tabs.clearActivity(session.id);
    scheduleTerminalFit(() => {
      safeFit(true);
      if (!searchOpen) focusTerminalIfAllowed(term);
    });
  });

  // Live-apply settings changes (font / theme / scrollback) without re-mounting
  // the terminal. xterm's options object accepts partial updates.
  async function reloadSettingsLive() {
    if (!term) return;
    const cfg = await loadTermSettings();
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
    await configureTransferFilter(cfg.experimentalTransferDetection);
    await applyRenderer(cfg.renderer);
    applyLigatures(cfg.ligatures);
    // Force a full redraw so canvas/webgl renderers invalidate their glyph
    // atlas and pick up the new palette / font metrics. xterm only repaints
    // changed cells otherwise, which leaves stale colors on screen.
    try { term.refresh(0, term.rows - 1); } catch { /* renderer not ready */ }
    scheduleSafeFit();
  }

  $effect(() => {
    void settingsRev;
    if (!term || !active) return;
    void reloadSettingsLive();
  });

  onDestroy(() => {
    const termToDispose = term;
    const searchToDispose = search;
    const rendererToDispose = rendererAddon;
    const filterToStop = transferFilter;
    cleanupHost?.();
    cleanupHost = null;
    cleanupSearchListener?.();
    cleanupSearchListener = null;
    macTextareaGuard?.();
    macTextareaGuard = null;
    cancelPolling();
    clearTransferNotice();
    transferFilter = null;
    search = null;
    term = null;
    rendererAddon = null;
    scheduleTerminalTeardown({
      term: termToDispose,
      search: searchToDispose,
      rendererAddon: rendererToDispose,
      beforeDispose: () => {
        if (filterToStop?.isTransferringFiles()) filterToStop.stopTransferringFiles();
      },
    });
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
    if (!search || !searchQuery) return;
    const opts = { caseSensitive: searchCase, regex: searchRegex };
    if (direction === 'next') search.findNext(searchQuery, opts);
    else search.findPrevious(searchQuery, opts);
  }
  function closeSearch() {
    searchOpen = false;
    search?.clearDecorations();
    requestAnimationFrame(() => focusTerminalIfAllowed(term));
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
    const sel = term?.getSelection();
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
  async function pasteText(raw: string) {
    let text = raw;
    if (pasteTrimWhitespace) text = text.replace(/[ \t]+$/gm, '');
    if (pasteFlattenNewlines) text = text.replace(/\r?\n/g, ' ');
    if (pasteMultilineWarn && text.includes('\n') && text.length > 200) {
      if (!(await appConfirm(i18n.t('terminal.pasteConfirm', { count: text.length })))) return;
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
    if (bellMode === 'off' || !host) return;
    if (bellMode === 'visual') {
      host.classList.add('bell-flash');
      if (bellFlashHandle != null) window.clearTimeout(bellFlashHandle);
      bellFlashHandle = window.setTimeout(() => {
        host?.classList.remove('bell-flash');
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
    term?.clear();
  }
  function doSearchAction() {
    menuOpen = false;
    openSearch();
  }
  function doSelectAll() {
    menuOpen = false;
    term?.selectAll();
  }

  function onTransferDragOver(ev: DragEvent) {
    if (isPaneDragActive()) {
      ev.preventDefault();
      if (ev.dataTransfer) ev.dataTransfer.dropEffect = 'move';
      return;
    }
    if (!canUseTrzszTransfer) return;
    if (!ev.dataTransfer?.types.includes('Files')) return;
    ev.preventDefault();
    ev.dataTransfer.dropEffect = 'copy';
  }

  async function onTransferDrop(ev: DragEvent) {
    if (!canUseTrzszTransfer || !transferFilter) return;
    const items = ev.dataTransfer?.items;
    if (!items?.length) return;
    ev.preventDefault();
    clearTransferNotice();
    try {
      await transferFilter.uploadFiles(items);
    } catch (err) {
      console.warn('trzsz upload failed', err);
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

  async function startNativeEngine() {
    if (!engineCanvas) return;
    try {
      engineController = new NativeEngineController(rpc);
      await engineController.start(engineCanvas, 80, 24);
      engineCanvas?.focus();
    } catch (err: any) {
      console.warn('[native-engine] failed:', err);
      engineController = null;
    }
  }

  function stopNativeEngine() {
    engineController?.close();
    engineController = null;
  }

  function onEngineKeydown(e: KeyboardEvent) {
    if (!engineController) return;
    let s = e.key;
    if (s === 'Enter') s = '\r';
    if (s === 'Backspace') s = '\x7f';
    if (s === 'Tab') s = '\t';
    if (s === 'Escape') s = '\x1b';
    if (s.length === 1 || s === '\r' || s === '\x7f' || s === '\t' || s === '\x1b') {
      e.preventDefault();
      engineController.send(s);
    }
  }
</script>

<div
  bind:this={host}
  role="application"
  data-aerotab-context-menu=""
  class="terminal-surface terminal-host h-full w-full min-h-0 min-w-0 relative overflow-hidden
         {active ? '' : 'pointer-events-none opacity-[0.92]'}"
  class:hidden={!!engineController}
  oncontextmenu={onContextMenu}
  onpointerup={onPointerUp}
  ondragover={onTransferDragOver}
  ondrop={onTransferDrop}
></div>

{#if engineController}
  <canvas
    bind:this={engineCanvas}
    class="absolute inset-0 w-full h-full outline-none"
    onkeydown={onEngineKeydown}
    tabindex="0"
  ></canvas>
{/if}

{#if exited}
  <div class="absolute bottom-3 right-3 z-20 max-w-[min(320px,calc(100%-24px))] pointer-events-none">
    <div
      bind:this={exitedOverlayEl}
      tabindex="0"
      role="toolbar"
      aria-label={i18n.t('terminal.sessionEnded')}
      class="pointer-events-auto flex items-center gap-3 bg-[var(--color-panel)]/96 border border-[var(--color-border)]
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
    <div class="pointer-events-auto flex items-center gap-3 bg-[var(--color-panel)]/96 border border-[var(--color-border)]
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
  <div class="absolute top-2 right-2 z-30 flex items-center gap-1 bg-[var(--color-panel)]/95
              border border-[var(--color-border)] rounded shadow-lg backdrop-blur p-1"
       class:pointer-events-none={!active}
       class:opacity-0={!active}>
    <input id="search-input-{session.id}"
           type="search" placeholder={i18n.t('common.search')} bind:value={searchQuery}
           onkeydown={(e) => {
             if (e.key === 'Enter') { runSearch(e.shiftKey ? 'prev' : 'next'); }
             else if (e.key === 'Escape') { closeSearch(); }
           }}
           class="bg-[var(--color-bg)] text-[var(--color-fg)] text-[12px] px-2 py-1 rounded
                  border border-[var(--color-border)] outline-none w-[200px]" />
    <button type="button" title={i18n.t('terminal.searchCaseSensitive')}
            class="p-1 rounded {searchCase ? 'text-[var(--color-accent)] bg-[var(--color-bg)]' : 'text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]'}"
            onclick={() => (searchCase = !searchCase)}>
      <CaseSensitive size={13} />
    </button>
    <button type="button" title={i18n.t('terminal.searchRegex')}
            class="p-1 rounded {searchRegex ? 'text-[var(--color-accent)] bg-[var(--color-bg)]' : 'text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]'}"
            onclick={() => (searchRegex = !searchRegex)}>
      <Regex size={13} />
    </button>
    <button type="button" title={i18n.t('common.previous')}
            class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]"
            onclick={() => runSearch('prev')}><ChevronUp size={13} /></button>
    <button type="button" title={i18n.t('common.next')}
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
      class="fixed inset-0 z-40"
      onclick={() => (menuOpen = false)}
      oncontextmenu={(e) => { e.preventDefault(); menuOpen = false; }}
    ></div>
    <div
      bind:this={menuEl}
      role="menu"
      tabindex="-1"
      data-aerotab-context-menu=""
      class="panel fixed z-[41] min-w-[200px] py-1 text-[12.5px] text-[var(--color-fg)]"
      style="left: {menuX}px; top: {menuY}px;"
      onkeydown={(e) => e.stopPropagation()}
      onclick={(e) => e.stopPropagation()}
    >
      <button type="button" class="menu-item menu-item--shortcut" onclick={doCopy}>
        <span>{i18n.t('common.copy')}</span>
        {#if bindingLabel('terminal-copy')}<kbd class="kbd">{bindingLabel('terminal-copy')}</kbd>{/if}
      </button>
      <button type="button" class="menu-item" onclick={doPaste}>{i18n.t('common.paste')}</button>
      <button type="button" class="menu-item" onclick={doSelectAll}>{i18n.t('common.selectAll')}</button>
      <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
      {#if exited && canReconnect}
        <button type="button" class="menu-item" onclick={() => { menuOpen = false; void reconnectSession(); }}>
          {i18n.t('terminal.reconnect')}
        </button>
        {#if onClosePane}
          <button type="button" class="menu-item" onclick={() => { menuOpen = false; onClosePane?.(); }}>
            {i18n.t('common.close')}
          </button>
        {/if}
        <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
      {/if}
      {#if onSplitRight}
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => { menuOpen = false; onSplitRight?.(); }}>
          <span>{i18n.t('tabbar.splitRight')}</span>
          <kbd class="kbd">{bindingLabel('split-right')}</kbd>
        </button>
      {/if}
      {#if onSplitDown}
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => { menuOpen = false; onSplitDown?.(); }}>
          <span>{i18n.t('tabbar.splitDown')}</span>
          <kbd class="kbd">{bindingLabel('split-down')}</kbd>
        </button>
      {/if}
      {#if onMaximize}
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => { menuOpen = false; onMaximize?.(); }}>
          <span>{i18n.t('pane.maximizePane')}</span>
          <kbd class="kbd">{bindingLabel('maximize-pane')}</kbd>
        </button>
      {/if}
      {#if canOpenSftp}
        <button type="button" class="menu-item menu-item--shortcut" onclick={() => { menuOpen = false; onOpenSftp?.(); }}>
          <span>{i18n.t('terminal.transferOpenSftp')}</span>
          <kbd class="kbd">{bindingLabel('open-sftp')}</kbd>
        </button>
      {/if}
      <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
      <button type="button" class="menu-item" onclick={doSearchAction}>{i18n.t('terminal.searchAction')}</button>
      <button type="button" class="menu-item" onclick={doClear}>{i18n.t('common.clearScreen')}</button>
      <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
      {#if !engineController}
        <button type="button" class="menu-item" onclick={() => { menuOpen = false; void startNativeEngine(); }}>Try Native Engine (Canvas)</button>
      {:else}
        <button type="button" class="menu-item" onclick={() => { menuOpen = false; stopNativeEngine(); }}>Stop Native Engine</button>
      {/if}
    </div>
  </div>
{/if}

<style>
  :global(.menu-item) {
    display: block;
    width: 100%;
    text-align: left;
    padding: 6px 12px;
    background: transparent;
    color: inherit;
    border: none;
    cursor: pointer;
  }
  :global(.menu-item:hover) {
    background: var(--color-panel-2);
  }
</style>
