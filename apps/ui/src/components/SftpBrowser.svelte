<script lang="ts">
  import { onMount, onDestroy, tick, untrack } from 'svelte';
  import {
    X, Folder, FileText, RefreshCw, ChevronRight, Home, ArrowUp,
    Upload, Download, Trash2, FolderPlus, Pencil, PanelRightClose, FolderUp,
    FolderDown, ExternalLink, CircleX, CheckCircle2, Clock3, Loader2,
  } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import { b64decode, b64encode, tauriInvoke } from '../lib/rpc';
  import { i18n } from '../lib/i18n.svelte';
  import { appConfirm, appPrompt } from '../lib/confirm.svelte';
  import type { LocalEntry, SftpEntry, SshProfileSpec, StoredProfile } from '../lib/types';
  import SftpLocalPane from './SftpLocalPane.svelte';
  import SftpTransferQueue from './SftpTransferQueue.svelte';
  import { profileEndpointLabel } from '../lib/profileMeta';
  import { startVerticalPanelResize } from '../lib/panelResize';
  import { sftpSessionRegistry, type RegisteredSftpSession } from '../lib/sftpSessionRegistry.svelte';
  import { portal } from '../lib/portal';
  import type { LocalUploadTransferRequest, RemoteCrossTransferRequest } from '../lib/sftpTransferTypes';
  import {
    SFTP_DRAG_LOCAL,
    SFTP_DRAG_REMOTE,
    joinLocalPath,
    parentLocalPath,
    parseLocalDrag,
    parseRemoteDrag,
    readSftpDragData,
    setSftpDragData,
    type LocalDragPayload,
    type RemoteDragPayload,
  } from '../lib/sftpLocal';

  interface SftpSource {
    name: string;
    ssh: SshProfileSpec;
    sudo?: boolean;
  }

  interface Props {
    rpc: RpcClient;
    profile?: StoredProfile;
    source?: SftpSource;
    mode?: 'modal' | 'dock';
    registryId?: string;
    /** Reserved for future terminal session reuse; currently always opens a dedicated SFTP connection. */
    terminalSessionId?: string | null;
    dockWidthPx?: number;
    transferPanelHeightPx?: number;
    onTransferPanelHeightChange?: (heightPx: number) => void;
    onActiveTransferCountChange?: (count: number) => void;
    onOpenTransferCenter?: () => void;
    onClose: () => void;
    onCollapse?: () => void;
    onPopOut?: (sudo: boolean) => void;
    showLocalPane?: boolean;
    onRemoteCrossTransfer?: (request: RemoteCrossTransferRequest) => void;
    onLocalUploadTransfer?: (request: LocalUploadTransferRequest) => void;
    refreshToken?: number;
    onError: (msg: string) => void;
  }
  let {
    rpc,
    profile,
    source,
    mode = 'modal',
    registryId: registryIdProp,
    terminalSessionId = null,
    dockWidthPx = 400,
    transferPanelHeightPx = 200,
    onTransferPanelHeightChange,
    onActiveTransferCountChange,
    onOpenTransferCenter,
    onClose,
    onCollapse,
    onPopOut,
    showLocalPane = true,
    onRemoteCrossTransfer,
    onLocalUploadTransfer,
    refreshToken = 0,
    onError,
  }: Props = $props();
  const target = $derived.by((): SftpSource | null => {
    if (source) return source;
    if (profile?.kind === 'ssh') return { name: profile.name, ssh: profile.ssh };
    return null;
  });
  const registryId = $derived(registryIdProp ?? `sftp-${target?.name ?? 'session'}-${mode}`);

  function initialSudoMode(): boolean {
    return Boolean(source?.sudo);
  }

  let sessionId = $state<string | null>(null);
  let sudoMode = $state(initialSudoMode());
  let cwd = $state('.');
  let entries = $state<SftpEntry[]>([]);
  let loading = $state(false);
  let listError = $state<string | null>(null);
  let listSeq = 0;
  let defaultDownloadDir = $state<string | null>(null);
  let lastDownloadPath = $state<string | null>(null);
  let preparingTransfers = $state(false);
  let transfers = $state<TransferTask[]>([]);
  let processingTransfers = false;
  let needsRefreshAfterTransfers = false;
  let transferSeq = 0;
  let appliedRefreshToken: number | null = null;
  const uploadFiles = new Map<string, File>();
  const downloadEntries = new Map<string, SftpEntry>();
  const knownRemoteDirs = new Set<string>();
  const CHUNK_SIZE = 256 * 1024;
  const TEXT_EDIT_MAX_BYTES = 512 * 1024;
  const TRANSFER_STALL_MS = 45_000;
  const MAX_AUTO_ATTEMPTS = 3;
  const RETRY_BACKOFF_MS = [1000, 3000];
  const TRANSFER_RPC_TIMEOUT_MS = 45_000;

  let localCwd = $state('');
  let localEntries = $state<LocalEntry[]>([]);
  let localLoading = $state(false);
  let localListError = $state<string | null>(null);
  let localListSeq = 0;

  let editOpen = $state(false);
  let editName = $state('');
  let editRemotePath = $state('');
  let editContent = $state('');
  let editSaving = $state(false);

  let remoteMenuOpen = $state(false);
  let remoteMenuX = $state(0);
  let remoteMenuY = $state(0);
  let remoteMenuEntry = $state<SftpEntry | null>(null);
  let remoteMenuEl = $state<HTMLDivElement | null>(null);
  let remotePaneEl = $state<HTMLDivElement | null>(null);
  let lastRemoteActionPosition = $state<{ x: number; y: number } | null>(null);
  let selectedRemoteNames = $state<Set<string>>(new Set());
  let focusedRemoteName = $state<string | null>(null);
  let lastSelectedRemoteName = $state<string | null>(null);
  let renamingName = $state<string | null>(null);
  let renameDraft = $state('');
  let localPaneCollapsed = $state(false);
  let downloadBannerDismissed = $state(false);

  const activeTransferCount = $derived(
    transfers.filter((t) => t.status === 'queued' || t.status === 'running' || t.status === 'paused').length,
  );
  const transferQueueItems = $derived(
    transfers.map((task) => ({
      id: task.id,
      name: task.name,
      status: task.status,
      percent: transferPercent(task),
      summary: transferSummary(task),
      kindLabel: task.kind,
    })),
  );
  const showLocalPaneEffective = $derived(showLocalPane && localCwd && !localPaneCollapsed);

  let reportedTransferCount = -1;

  $effect(() => {
    const count = activeTransferCount;
    if (count === reportedTransferCount) return;
    reportedTransferCount = count;
    const report = untrack(() => onActiveTransferCountChange);
    report?.(count);
  });

  $effect(() => {
    if (mode === 'dock' && dockWidthPx < 420) {
      localPaneCollapsed = true;
    }
  });

  let selectedTransferIds = $state<Set<string>>(new Set());
  let transferUiFlushScheduled = false;
  let transferProgressTick: number | null = null;
  let registrySnapshot = '';
  let connectSeq = 0;
  /** Last connect key we attempted; blocks auto-reconnect loops on failure. */
  let appliedConnectKey = '';
  let connectInFlight = false;

  function isStaleSftpError(message: string): boolean {
    const m = message.toLowerCase();
    return (
      m.includes('session closed')
      || m.includes('sessionnotfound')
      || m.includes('channel closed')
      || m.includes('connection reset')
      || m.includes('broken pipe')
      || m.includes('not connected')
      || m.includes('timed out')
      || m.includes('timeout')
    );
  }

  async function invalidateSftpSession() {
    const prev = sessionId;
    sessionId = null;
    registrySnapshot = '';
    appliedConnectKey = '';
    if (prev) {
      await rpc.call('sftp.close', { id: prev }).catch(() => {});
    }
    sftpSessionRegistry.unregister(registryId);
  }

  function markConnectSuccess(key: string) {
    appliedConnectKey = key;
  }

  type TransferKind = 'upload' | 'download';
  type TransferStatus = 'queued' | 'running' | 'paused' | 'done' | 'error' | 'canceled';
  interface TransferTask {
    id: string;
    kind: TransferKind;
    name: string;
    path: string;
    size: number;
    transferred: number;
    status: TransferStatus;
    localPath?: string;
    localFilePath?: string;
    localBaseDir?: string;
    relativePath?: string[];
    message?: string;
    attempts?: number;
    startedAt?: number;
    lastProgressAt?: number;
    finishedAt?: number;
  }

  async function initLocalPane() {
    if (defaultDownloadDir) {
      localCwd = defaultDownloadDir;
    } else {
      const home = await tauriInvoke<string>('local_home_dir');
      if (home) localCwd = home;
    }
    if (localCwd) await refreshLocal();
  }

  async function refreshLocal() {
    if (!localCwd) return;
    const seq = ++localListSeq;
    localLoading = true;
    localListError = null;
    try {
      const list = await tauriInvoke<LocalEntry[]>('local_list_dir', { path: localCwd });
      if (!list) throw new Error('local file browser is not available');
      if (seq !== localListSeq) return;
      localEntries = list;
    } catch (e) {
      if (seq !== localListSeq) return;
      localListError = (e as Error).message;
      localEntries = [];
    } finally {
      if (seq === localListSeq) localLoading = false;
    }
  }

  async function navigateLocal(path: string) {
    localCwd = path;
    await refreshLocal();
  }

  async function localGoUp() {
    localCwd = parentLocalPath(localCwd);
    await refreshLocal();
  }

  async function localGoHome() {
    const home = await tauriInvoke<string>('local_home_dir');
    if (home) {
      localCwd = home;
      await refreshLocal();
    }
  }

  function preventDragDefaults(e: DragEvent) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
  }

  async function handleRemotePaneDrop(e: DragEvent) {
    e.preventDefault();
    const localRaw = readSftpDragData(e.dataTransfer, SFTP_DRAG_LOCAL);
    if (localRaw) {
      const payload = parseLocalDrag(localRaw);
      if (payload && onLocalUploadTransfer && sessionId) {
        onLocalUploadTransfer({
          sourcePath: payload.path,
          sourceKind: payload.kind === 'dir' ? 'Dir' : 'File',
          sourceSize: payload.size,
          destSessionId: sessionId,
          destLabel: target?.name ?? i18n.t('sftp.sshSession'),
          destDir: cwd,
          destPath: joinPath(cwd, payload.name),
          name: payload.name,
        });
      } else if (payload) {
        await enqueueUploadFromLocal(payload);
      }
      return;
    }
    const remoteRaw = readSftpDragData(e.dataTransfer, SFTP_DRAG_REMOTE);
    if (remoteRaw && sessionId) {
      const payload = parseRemoteDrag(remoteRaw);
      if (payload?.sourceSessionId && payload.sourceSessionId !== sessionId) {
        await receiveRemoteDrop(payload);
        return;
      }
    }
    const files = Array.from(e.dataTransfer?.files ?? []);
    if (files.length > 0) {
      for (const f of files) {
        const id = nextTransferId();
        uploadFiles.set(id, f);
        enqueueTransfer({
          id,
          kind: 'upload',
          name: f.name,
          path: joinPath(cwd, f.name),
          size: f.size,
          transferred: 0,
          status: 'queued',
        });
      }
    }
  }

  async function handleLocalPaneDrop(e: DragEvent) {
    e.preventDefault();
    const remoteRaw = readSftpDragData(e.dataTransfer, SFTP_DRAG_REMOTE);
    if (!remoteRaw || !sessionId) return;
    const payload = parseRemoteDrag(remoteRaw);
    if (!payload) return;
    if (payload.kind === 'Dir') {
      await downloadDirectoryToLocal(payload.path, [payload.name], localCwd);
      return;
    }
    const dest = joinLocalPath(localCwd, payload.name);
    const id = nextTransferId();
    downloadEntries.set(id, {
      name: payload.name,
      kind: 'File',
      size: payload.size,
      mode: 0,
      mtime: null,
    });
    enqueueDownloadTransfer(
      { name: payload.name, kind: 'File', size: payload.size, mode: 0, mtime: null },
      payload.path,
      payload.name,
      { id, localPath: dest },
    );
  }

  async function enqueueUploadFromLocal(payload: LocalDragPayload) {
    if (!sessionId) return;
    if (payload.kind === 'dir') {
      await collectLocalDirectoryUploads(payload.path, [payload.name]);
      return;
    }
    const id = nextTransferId();
    enqueueTransfer({
      id,
      kind: 'upload',
      name: payload.name,
      path: joinPath(cwd, payload.name),
      size: payload.size,
      transferred: 0,
      status: 'queued',
      localFilePath: payload.path,
    });
  }

  async function collectLocalDirectoryUploads(dirPath: string, relative: string[]) {
    const list = await withTransferTimeout('local_list_dir', tauriInvoke<LocalEntry[]>('local_list_dir', { path: dirPath }) ?? Promise.reject(new Error('local file browser is not available')));
    if (!list) throw new Error('local file browser is not available');
    for (const entry of list) {
      const childPath = joinLocalPath(dirPath, entry.name);
      const childRelative = [...relative, entry.name];
      if (entry.kind === 'dir') {
        await collectLocalDirectoryUploads(childPath, childRelative);
      } else if (entry.kind === 'file') {
        const id = nextTransferId();
        enqueueTransfer({
          id,
          kind: 'upload',
          name: childRelative.join('/'),
          path: joinPathSegments(cwd, childRelative),
          size: entry.size,
          transferred: 0,
          status: 'queued',
          localFilePath: childPath,
        });
      }
    }
  }

  async function downloadDirectoryToLocal(remotePath: string, relative: string[], baseDir: string) {
    if (!sessionId) return;
    await mkdirLocalRelative(baseDir, relative);
    const list = sortEntries(await withTransferTimeout('sftp.list', rpc.call<SftpEntry[]>('sftp.list', { id: sessionId, path: remotePath })));
    for (const entry of list) {
      const childPath = joinPath(remotePath, entry.name);
      const childRelative = [...relative, entry.name];
      if (entry.kind === 'Dir') {
        await downloadDirectoryToLocal(childPath, childRelative, baseDir);
      } else if (entry.kind === 'File') {
        const id = nextTransferId();
        enqueueDownloadTransfer(entry, childPath, childRelative.join('/'), {
          id,
          localBaseDir: baseDir,
          relativePath: childRelative,
        });
      }
    }
  }

  function onRemoteDragStart(e: DragEvent, entry: SftpEntry) {
    const path = joinPath(cwd, entry.name);
    const payload: RemoteDragPayload = {
      path,
      name: entry.name,
      kind: entry.kind === 'Dir' ? 'Dir' : 'File',
      size: entry.size,
      sourceSessionId: sessionId ?? undefined,
      sourceRegistryId: registryId,
      sourceLabel: target?.name ?? i18n.t('sftp.sshSession'),
      sourceProfile: target?.ssh,
    };
    setSftpDragData(e.dataTransfer, SFTP_DRAG_REMOTE, JSON.stringify(payload));
  }

  async function openTextEditor(e: SftpEntry) {
    if (!sessionId || e.kind !== 'File') return;
    if (e.size > TEXT_EDIT_MAX_BYTES) {
      onError(i18n.t('sftp.editTooLarge', { max: formatSize(TEXT_EDIT_MAX_BYTES) }));
      return;
    }
    try {
      const path = joinPath(cwd, e.name);
      const r = await rpc.call<{ data: string }>('sftp.read', { id: sessionId, path });
      editRemotePath = path;
      editName = e.name;
      editContent = new TextDecoder().decode(b64decode(r.data));
      editOpen = true;
    } catch (err) {
      onError(`read: ${(err as Error).message}`);
    }
  }

  async function saveTextEditor() {
    if (!sessionId || !editRemotePath) return;
    editSaving = true;
    try {
      await rpc.call('sftp.write', {
        id: sessionId,
        path: editRemotePath,
        data: b64encode(new TextEncoder().encode(editContent)),
      });
      editOpen = false;
      await refresh();
    } catch (err) {
      onError(`save: ${(err as Error).message}`);
    } finally {
      editSaving = false;
    }
  }

  async function loadSftpSettings() {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'sftp' });
      if (r.value && typeof r.value === 'object') {
        const v = r.value as Record<string, unknown>;
        if (typeof v.defaultDownloadDir === 'string' && v.defaultDownloadDir.trim()) {
          defaultDownloadDir = v.defaultDownloadDir.trim();
          lastDownloadPath = defaultDownloadDir;
        }
      }
    } catch {
      /* optional settings */
    }
  }

  function sftpConnectKey(): string {
    if (!target) return '';
    const h = target.ssh;
    return `${h.host}|${h.port}|${h.user}|${sudoMode ? 1 : 0}`;
  }

  async function openSftpBackend(): Promise<string> {
    if (!target) throw new Error('SFTP target is not set');
    // Always use a dedicated SFTP SSH connection. Reusing the interactive shell
    // session (openForSession) can reset the link on some servers (Windows 10054).
    const r = await rpc.call<{ id: string }>('sftp.open', { profile: target.ssh, sudo: sudoMode });
    return r.id;
  }

  async function connect(): Promise<boolean> {
    if (!target) return false;
    await invalidateSftpSession();
    loading = true;
    listError = null;
    try {
      const id = await openSftpBackend();
      sessionId = id;
      knownRemoteDirs.clear();
      // Resolve home (".") to an absolute path so navigation is predictable.
      const real = await rpc.call<{ path: string }>('sftp.realpath', { id, path: '.' });
      const nextCwd = real.path || '.';
      const list = await rpc.call<SftpEntry[]>('sftp.list', { id, path: nextCwd });
      cwd = nextCwd;
      entries = sortEntries(list);
      markConnectSuccess(sftpConnectKey());
      return true;
    } catch (e) {
      await invalidateSftpSession();
      entries = [];
      listError = (e as Error).message;
      onError(`sftp: ${(e as Error).message}`);
      return false;
    } finally {
      loading = false;
    }
  }

  async function forceReconnect() {
    const key = sftpConnectKey();
    appliedConnectKey = '';
    connectSeq += 1;
    const seq = connectSeq;
    connectInFlight = true;
    try {
      if (seq === connectSeq) await connect();
    } finally {
      if (seq === connectSeq) connectInFlight = false;
    }
  }

  async function reconnect(nextSudo = sudoMode) {
    sudoMode = nextSudo;
    await forceReconnect();
  }

  function retryRemoteList() {
    void forceReconnect();
  }

  async function refresh() {
    if (!sessionId) {
      void forceReconnect();
      return;
    }
    const seq = ++listSeq;
    loading = true;
    listError = null;
    try {
      const list = await rpc.call<SftpEntry[]>('sftp.list', { id: sessionId, path: cwd });
      if (seq !== listSeq) return;
      entries = sortEntries(list);
      pruneRemoteSelection();
    } catch (e) {
      if (seq !== listSeq) return;
      const msg = (e as Error).message;
      listError = msg;
      entries = [];
      if (isStaleSftpError(msg)) {
        await invalidateSftpSession();
        void forceReconnect();
      }
    } finally {
      if (seq === listSeq) loading = false;
    }
  }

  function sortEntries(list: SftpEntry[]): SftpEntry[] {
    return [...list].sort((a, b) => {
      if (a.kind !== b.kind) return a.kind === 'Dir' ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
  }

  function joinPath(base: string, name: string): string {
    if (base === '/' || base === '') return '/' + name;
    return base.replace(/\/+$/, '') + '/' + name;
  }

  function parentPath(p: string): string {
    if (p === '/' || p === '') return '/';
    const trimmed = p.replace(/\/+$/, '');
    const i = trimmed.lastIndexOf('/');
    if (i <= 0) return '/';
    return trimmed.slice(0, i);
  }

  function normalizeRemotePath(path: string): string {
    if (path === '') return '.';
    if (path === '/') return '/';
    const absolute = path.startsWith('/');
    const segments = path.split('/').filter((segment) => segment.length > 0 && segment !== '.');
    const normalized = segments.join('/');
    if (absolute) return '/' + normalized;
    return normalized || '.';
  }

  function joinPathSegments(base: string, segments: string[]): string {
    return segments.reduce((path, segment) => joinPath(path, segment), base);
  }

  function safeRelativeSegments(path: string): string[] {
    return path
      .replace(/\\/g, '/')
      .split('/')
      .map((segment) => segment.trim())
      .filter((segment) => segment.length > 0 && segment !== '.' && segment !== '..');
  }

  async function pickSavePath(defaultName: string): Promise<string | null | undefined> {
    const pick = tauriInvoke<string | null>('pick_save_file', { defaultName });
    if (!pick) return undefined;
    return pick;
  }

  async function pickDirectoryPath(): Promise<string | null | undefined> {
    const pick = tauriInvoke<string | null>('pick_directory');
    if (!pick) return undefined;
    return pick;
  }

  async function writeLocalDownloadChunk(
    task: TransferTask,
    offset: number,
    bytes: Uint8Array,
    create: boolean,
  ) {
    const data = b64encode(bytes);
    if (task.localPath) {
      const write = tauriInvoke<void>('local_write_chunk', {
        path: task.localPath,
        offset,
        data,
        create,
      });
      if (!write) throw new Error('desktop file writer is not available');
      await withTransferTimeout('local_write_chunk', write);
      return;
    }
    if (task.localBaseDir && task.relativePath) {
      const write = tauriInvoke<void>('local_write_relative_chunk', {
        baseDir: task.localBaseDir,
        relative: task.relativePath,
        offset,
        data,
        create,
      });
      if (!write) throw new Error('desktop file writer is not available');
      await withTransferTimeout('local_write_relative_chunk', write);
    }
  }

  async function mkdirLocalRelative(baseDir: string, relative: string[]) {
    const mkdir = tauriInvoke<string>('local_mkdir_relative', { baseDir, relative });
    if (!mkdir) throw new Error('desktop directory writer is not available');
    await withTransferTimeout('local_mkdir_relative', mkdir);
  }

  async function enter(e: SftpEntry) {
    if (loading || e.kind !== 'Dir') return;
    cwd = joinPath(cwd, e.name);
    clearRemoteSelection();
    await refresh();
  }

  async function goUp() {
    if (loading) return;
    cwd = parentPath(cwd);
    clearRemoteSelection();
    await refresh();
  }

  async function goHome() {
    if (loading || !sessionId) return;
    const real = await rpc.call<{ path: string }>('sftp.realpath', { id: sessionId, path: '.' });
    cwd = real.path || '.';
    clearRemoteSelection();
    await refresh();
  }

  async function removeEntry(e: SftpEntry) {
    await removeEntries([e]);
  }

  async function removeEntries(items: SftpEntry[]) {
    if (!sessionId || items.length === 0) return;
    const label = items.length === 1 ? items[0]?.name ?? '' : i18n.t('sftp.deleteManyLabel', { count: items.length });
    if (!(await appConfirm(i18n.t('sftp.deleteConfirm', { name: label }), { danger: true, confirmLabel: i18n.t('common.delete'), position: focusedDialogPosition() }))) return;
    try {
      for (const entry of items) {
        const path = joinPath(cwd, entry.name);
        if (entry.kind === 'Dir') {
          await rpc.call('sftp.removeDir', { id: sessionId, path });
        } else {
          await rpc.call('sftp.removeFile', { id: sessionId, path });
        }
      }
      clearRemoteSelection();
      await refresh();
    } catch (err) {
      onError((err as Error).message);
    }
  }

  function nextTransferId(): string {
    transferSeq += 1;
    return `sftp-transfer-${Date.now()}-${transferSeq}`;
  }

  function flushTransfersUi() {
    if (transferUiFlushScheduled) return;
    transferUiFlushScheduled = true;
    requestAnimationFrame(() => {
      transferUiFlushScheduled = false;
      transfers = [...transfers];
    });
  }

  function updateTransfer(id: string, patch: Partial<TransferTask>) {
    const transfer = transfers.find((candidate) => candidate.id === id);
    if (!transfer) return;
    const prevTransferred = transfer.transferred;
    Object.assign(transfer, patch);
    if (patch.transferred !== undefined && patch.transferred !== prevTransferred) {
      transfer.lastProgressAt = Date.now();
    }
    flushTransfersUi();
  }

  function isCanceled(id: string): boolean {
    return transfers.find((transfer) => transfer.id === id)?.status === 'canceled';
  }

  function isPaused(id: string): boolean {
    return transfers.find((transfer) => transfer.id === id)?.status === 'paused';
  }

  async function waitWhilePaused(id: string) {
    while (isPaused(id) && !isCanceled(id)) {
      await new Promise((resolve) => setTimeout(resolve, 150));
    }
  }

  function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  async function withTransferTimeout<T>(label: string, promise: Promise<T>, timeoutMs = TRANSFER_RPC_TIMEOUT_MS): Promise<T> {
    let timer: ReturnType<typeof setTimeout> | null = null;
    try {
      return await Promise.race([
        promise,
        new Promise<T>((_, reject) => {
          timer = setTimeout(() => reject(new Error(`${label}: ${i18n.t('transfer.timeout', { seconds: Math.round(timeoutMs / 1000) })}`)), timeoutMs);
        }),
      ]);
    } finally {
      if (timer) clearTimeout(timer);
    }
  }

  async function runTransferWithStallWatch(task: TransferTask): Promise<void> {
    if (task.kind === 'upload') {
      await runUpload(task);
    } else {
      await runDownload(task);
    }
  }

  async function retryOrFailTransfer(task: TransferTask, err: unknown) {
    const attempts = task.attempts ?? 1;
    const message = (err as Error).message;
    if (attempts < MAX_AUTO_ATTEMPTS && !isCanceled(task.id)) {
      const nextAttempt = attempts + 1;
      updateTransfer(task.id, {
        status: 'queued',
        transferred: 0,
        message: i18n.t('transfer.retrying', { attempt: nextAttempt, max: MAX_AUTO_ATTEMPTS }),
        attempts: nextAttempt,
        startedAt: undefined,
        lastProgressAt: undefined,
        finishedAt: undefined,
      });
      await sleep(RETRY_BACKOFF_MS[Math.min(nextAttempt - 2, RETRY_BACKOFF_MS.length - 1)] ?? 1000);
      return;
    }
    updateTransfer(task.id, {
      status: 'error',
      message: i18n.t('transfer.finalFailed', { attempts, message }),
      finishedAt: Date.now(),
    });
    onError(`transfer ${task.name}: ${message}`);
  }

  function pauseTransfer(id: string) {
    const status = transfers.find((transfer) => transfer.id === id)?.status;
    if (status === 'queued' || status === 'running') {
      updateTransfer(id, { status: 'paused', message: i18n.t('sftp.paused') });
    }
  }

  function resumeTransfer(id: string) {
    const task = transfers.find((transfer) => transfer.id === id);
    if (task?.status !== 'paused') return;
    updateTransfer(id, {
      status: task.startedAt ? 'running' : 'queued',
      message: undefined,
    });
    void processTransfers();
  }

  function enqueueTransfer(task: TransferTask) {
    transfers = [...transfers, task];
    flushTransfersUi();
    void processTransfers();
  }

  async function processTransfers() {
    if (processingTransfers) return;
    processingTransfers = true;
    try {
      while (true) {
        const task = transfers.find((candidate) => candidate.status === 'queued');
        if (!task) {
          if (needsRefreshAfterTransfers) {
            needsRefreshAfterTransfers = false;
            await refresh();
          }
          return;
        }
        updateTransfer(task.id, { status: 'running', message: undefined, startedAt: Date.now(), lastProgressAt: Date.now(), attempts: task.attempts ?? 1 });
        try {
          await runTransferWithStallWatch(task);
          if (isCanceled(task.id)) continue;
          updateTransfer(task.id, { status: 'done', transferred: task.size, finishedAt: Date.now() });
          if (task.kind === 'download') {
            if (task.localPath) lastDownloadPath = task.localPath;
            else if (task.localBaseDir) lastDownloadPath = task.localBaseDir;
          }
        } catch (e) {
          if (isCanceled(task.id)) continue;
          await retryOrFailTransfer(transfers.find((candidate) => candidate.id === task.id) ?? task, e);
        }
      }
    } finally {
      processingTransfers = false;
    }
  }

  async function runUpload(task: TransferTask) {
    if (!sessionId) throw new Error('SFTP session is not open');
    const file = uploadFiles.get(task.id);
    const localPath = task.localFilePath;
    if (!file && !localPath) throw new Error('local file is not available');
    const totalSize = file?.size ?? task.size;
    const destinationDir = parentPath(task.path);
    if (destinationDir !== '/' && destinationDir !== '.') {
      updateTransfer(task.id, { message: 'Preparing directories' });
      await ensureRemoteDir(destinationDir);
    }
    updateTransfer(task.id, { message: undefined });
    if (totalSize === 0) {
      await withTransferTimeout('sftp.writeChunk', rpc.call('sftp.writeChunk', {
        id: sessionId,
        path: task.path,
        offset: 0,
        data: '',
        create: true,
      }));
      updateTransfer(task.id, { transferred: 0 });
      return;
    }
    let offset = 0;
    while (offset < totalSize) {
      await waitWhilePaused(task.id);
      if (isCanceled(task.id)) return;
      let bytes: Uint8Array;
      if (file) {
        const chunk = file.slice(offset, Math.min(totalSize, offset + CHUNK_SIZE));
        bytes = new Uint8Array(await chunk.arrayBuffer());
      } else if (localPath) {
        const len = Math.min(CHUNK_SIZE, totalSize - offset);
        const r = await withTransferTimeout('local_read_chunk', tauriInvoke<{ data: string }>('local_read_chunk', {
          path: localPath,
          offset,
          len,
        }) ?? Promise.reject(new Error('desktop file reader is not available')));
        if (!r) throw new Error('desktop file reader is not available');
        bytes = b64decode(r.data);
      } else {
        throw new Error('local file is not available');
      }
      await withTransferTimeout('sftp.writeChunk', rpc.call('sftp.writeChunk', {
        id: sessionId,
        path: task.path,
        offset,
        data: b64encode(bytes),
        create: offset === 0,
      }));
      offset += bytes.byteLength;
      updateTransfer(task.id, { transferred: offset });
    }
    needsRefreshAfterTransfers = true;
  }

  $effect(() => {
    const sid = sessionId;
    const path = cwd;
    const label = target?.name ?? i18n.t('sftp.sshSession');
    const snap = `${sid ?? ''}:${path}:${label}:${target?.ssh.host ?? ''}:${target?.ssh.user ?? ''}:${target?.ssh.port ?? ''}`;
    if (sid && snap !== registrySnapshot) {
      registrySnapshot = snap;
      sftpSessionRegistry.register({ registryId, label, sessionId: sid, cwd: path, profile: target?.ssh });
    }
  });

  $effect(() => {
    void target?.name;
    void target?.ssh.host;
    void target?.ssh.user;
    void target?.ssh.port;
    void sudoMode;
    if (!target) {
      appliedConnectKey = '';
      return;
    }
    const key = sftpConnectKey();
    if (!key || key === appliedConnectKey) return;
    if (untrack(() => connectInFlight)) return;
    const seq = ++connectSeq;
    connectInFlight = true;
    void (async () => {
      try {
        if (seq !== connectSeq) return;
        await connect();
      } finally {
        if (seq === connectSeq) connectInFlight = false;
      }
    })();
  });

  $effect(() => {
    const token = refreshToken;
    if (appliedRefreshToken === null) {
      appliedRefreshToken = token;
      return;
    }
    if (token === appliedRefreshToken) return;
    appliedRefreshToken = token;
    if (sessionId) void refresh();
  });

  function clampMenuToViewport(x: number, y: number, el: HTMLDivElement | null): { x: number; y: number } {
    if (!el) return { x, y };
    const pad = 8;
    const maxX = Math.max(pad, window.innerWidth - el.offsetWidth - pad);
    const maxY = Math.max(pad, window.innerHeight - el.offsetHeight - pad);
    return {
      x: Math.min(Math.max(pad, x), maxX),
      y: Math.min(Math.max(pad, y), maxY),
    };
  }

  function clearRemoteSelection() {
    selectedRemoteNames = new Set();
    focusedRemoteName = null;
    lastSelectedRemoteName = null;
  }

  function pruneRemoteSelection() {
    const names = new Set(entries.map((entry) => entry.name));
    selectedRemoteNames = new Set([...selectedRemoteNames].filter((name) => names.has(name)));
    if (focusedRemoteName && !names.has(focusedRemoteName)) focusedRemoteName = null;
    if (lastSelectedRemoteName && !names.has(lastSelectedRemoteName)) lastSelectedRemoteName = null;
  }

  function remoteEntryByName(name: string | null): SftpEntry | null {
    if (!name) return null;
    return entries.find((entry) => entry.name === name) ?? null;
  }

  function selectedRemoteEntries(): SftpEntry[] {
    return entries.filter((entry) => selectedRemoteNames.has(entry.name));
  }

  function rememberRemoteActionPosition(ev?: MouseEvent) {
    if (ev) lastRemoteActionPosition = { x: Math.round(ev.clientX), y: Math.round(ev.clientY) };
  }

  function selectRemoteEntry(entry: SftpEntry, ev?: MouseEvent) {
    if (loading) return;
    remotePaneEl?.focus();
    rememberRemoteActionPosition(ev);
    focusedRemoteName = entry.name;
    const next = new Set(selectedRemoteNames);
    if (ev?.shiftKey && lastSelectedRemoteName) {
      const from = entries.findIndex((item) => item.name === lastSelectedRemoteName);
      const to = entries.findIndex((item) => item.name === entry.name);
      if (from >= 0 && to >= 0) {
        next.clear();
        const [start, end] = from <= to ? [from, to] : [to, from];
        for (const item of entries.slice(start, end + 1)) next.add(item.name);
      }
    } else if (ev?.ctrlKey || ev?.metaKey) {
      if (next.has(entry.name)) next.delete(entry.name);
      else next.add(entry.name);
      lastSelectedRemoteName = entry.name;
    } else {
      next.clear();
      next.add(entry.name);
      lastSelectedRemoteName = entry.name;
    }
    selectedRemoteNames = next;
  }

  function focusRelativeRemote(delta: number) {
    if (loading || entries.length === 0) return;
    const current = focusedRemoteName ? entries.findIndex((entry) => entry.name === focusedRemoteName) : -1;
    const nextIdx = Math.min(entries.length - 1, Math.max(0, current + delta));
    selectRemoteEntry(entries[nextIdx]!);
  }

  function focusedDialogPosition(): { x: number; y: number } | undefined {
    if (remoteMenuOpen) return { x: remoteMenuX, y: remoteMenuY };
    if (lastRemoteActionPosition) return lastRemoteActionPosition;
    const focused = remotePaneEl?.querySelector<HTMLElement>('tr[data-remote-row].focused');
    if (focused) {
      const rect = focused.getBoundingClientRect();
      return { x: Math.round(rect.left + rect.width / 2), y: Math.round(rect.top + rect.height / 2) };
    }
    const active = document.activeElement as HTMLElement | null;
    const rect = active?.getBoundingClientRect();
    if (!rect || rect.width === 0 || rect.height === 0) return undefined;
    return { x: Math.round(rect.left + Math.min(rect.width, 24)), y: Math.round(rect.bottom + 6) };
  }

  function onRemotePaneKeydown(ev: KeyboardEvent) {
    if (renamingName) return;
    if (ev.key === 'F5') {
      ev.preventDefault();
      if (!loading) void refresh();
      return;
    }
    if (ev.key === 'Delete' || ev.key === 'Backspace') {
      const selected = selectedRemoteEntries();
      if (selected.length > 0) {
        ev.preventDefault();
        void removeEntries(selected);
      }
      return;
    }
    if (ev.key === 'F2') {
      const selected = selectedRemoteEntries();
      if (selected.length === 1) {
        ev.preventDefault();
        startInlineRename(selected[0]!);
      }
      return;
    }
    if (ev.key === 'Enter') {
      const focused = remoteEntryByName(focusedRemoteName);
      if (focused?.kind === 'Dir') {
        ev.preventDefault();
        void enter(focused);
      }
      return;
    }
    if (ev.key === 'ArrowDown') {
      ev.preventDefault();
      focusRelativeRemote(1);
    } else if (ev.key === 'ArrowUp') {
      ev.preventDefault();
      focusRelativeRemote(-1);
    }
  }

  async function openRemoteMenu(entry: SftpEntry, ev: MouseEvent) {
    ev.preventDefault();
    ev.stopPropagation();
    remotePaneEl?.focus({ preventScroll: true });
    rememberRemoteActionPosition(ev);
    if (!selectedRemoteNames.has(entry.name)) selectRemoteEntry(entry);
    focusedRemoteName = entry.name;
    remoteMenuEntry = entry;
    remoteMenuX = ev.clientX;
    remoteMenuY = ev.clientY;
    remoteMenuOpen = true;
    await tick();
    const clamped = clampMenuToViewport(remoteMenuX, remoteMenuY, remoteMenuEl);
    remoteMenuX = clamped.x;
    remoteMenuY = clamped.y;
    lastRemoteActionPosition = { x: remoteMenuX, y: remoteMenuY };
  }

  function closeRemoteMenu() {
    remoteMenuOpen = false;
    remoteMenuEntry = null;
  }

  const otherSftpSessions = $derived(sftpSessionRegistry.others(registryId));

  async function ensureRemoteDirOn(sid: string, path: string, dirCache: Set<string>) {
    const normalized = normalizeRemotePath(path);
    if (normalized === '/' || normalized === '.') return;
    const absolute = normalized.startsWith('/');
    const segments = normalized.split('/').filter(Boolean);
    let cursor = absolute ? '/' : '.';
    for (const segment of segments) {
      cursor = joinPath(cursor, segment);
      if (dirCache.has(cursor)) continue;
      try {
        const entry = await withTransferTimeout('sftp.stat', rpc.call<SftpEntry>('sftp.stat', { id: sid, path: cursor }));
        if (entry.kind !== 'Dir') throw new Error(`${cursor} exists and is not a directory`);
      } catch (e) {
        try {
          await withTransferTimeout('sftp.mkdir', rpc.call('sftp.mkdir', { id: sid, path: cursor }));
        } catch (mkdirError) {
          const entry = await withTransferTimeout('sftp.stat', rpc.call<SftpEntry>('sftp.stat', { id: sid, path: cursor })).catch(() => null);
          if (entry?.kind !== 'Dir') throw mkdirError;
        }
      }
      dirCache.add(cursor);
    }
  }

  async function copyRemoteFileBetweenSessions(
    srcSid: string,
    srcPath: string,
    destSid: string,
    destPath: string,
    size: number,
  ) {
    if (size === 0) {
      await withTransferTimeout('sftp.writeChunk', rpc.call('sftp.writeChunk', { id: destSid, path: destPath, offset: 0, data: '', create: true }));
      return;
    }
    let offset = 0;
    while (offset < size) {
      const len = Math.min(CHUNK_SIZE, size - offset);
      const r = await withTransferTimeout('sftp.readChunk', rpc.call<{ data: string }>('sftp.readChunk', {
        id: srcSid,
        path: srcPath,
        offset,
        len,
      }));
      const bytes = b64decode(r.data);
      if (bytes.byteLength === 0) break;
      await withTransferTimeout('sftp.writeChunk', rpc.call('sftp.writeChunk', {
        id: destSid,
        path: destPath,
        offset,
        data: b64encode(bytes),
        create: offset === 0,
      }));
      offset += bytes.byteLength;
    }
  }

  async function copyRemoteDirBetweenSessions(
    srcSid: string,
    srcPath: string,
    destSid: string,
    destPath: string,
    dirCache: Set<string>,
  ) {
    await ensureRemoteDirOn(destSid, destPath, dirCache);
    const list = sortEntries(await withTransferTimeout('sftp.list', rpc.call<SftpEntry[]>('sftp.list', { id: srcSid, path: srcPath })));
    for (const entry of list) {
      const childSrc = joinPath(srcPath, entry.name);
      const childDest = joinPath(destPath, entry.name);
      if (entry.kind === 'Dir') {
        await copyRemoteDirBetweenSessions(srcSid, childSrc, destSid, childDest, dirCache);
      } else if (entry.kind === 'File') {
        await copyRemoteFileBetweenSessions(srcSid, childSrc, destSid, childDest, entry.size);
      }
    }
  }

  async function receiveRemoteDrop(payload: RemoteDragPayload) {
    if (!sessionId || !payload.sourceSessionId) return;
    const destPath = joinPath(cwd, payload.name);
    if (onRemoteCrossTransfer) {
      onRemoteCrossTransfer({
        sourceSessionId: payload.sourceSessionId,
        sourceLabel: payload.sourceLabel,
        sourceProfile: payload.sourceProfile,
        sourcePath: payload.path,
        sourceKind: payload.kind,
        sourceSize: payload.size,
        destSessionId: sessionId,
        destLabel: target?.name ?? i18n.t('sftp.sshSession'),
        destProfile: target?.ssh,
        destDir: cwd,
        destPath,
        name: payload.name,
      });
      return;
    }
    const dirCache = new Set<string>();
    try {
      if (payload.kind === 'Dir') {
        await copyRemoteDirBetweenSessions(payload.sourceSessionId, payload.path, sessionId, destPath, dirCache);
      } else {
        const destDir = parentPath(destPath);
        if (destDir !== '/' && destDir !== '.') {
          await ensureRemoteDirOn(sessionId, destDir, dirCache);
        }
        await copyRemoteFileBetweenSessions(payload.sourceSessionId, payload.path, sessionId, destPath, payload.size);
      }
      await refresh();
    } catch (e) {
      onError(i18n.t('sftp.crossTransferFailed', { message: (e as Error).message }));
    }
  }

  async function sendEntryToSession(entry: SftpEntry, dest: RegisteredSftpSession) {
    if (!sessionId) return;
    closeRemoteMenu();
    const srcPath = joinPath(cwd, entry.name);
    const destPath = joinPath(dest.cwd, entry.name);
    if (onRemoteCrossTransfer && (entry.kind === 'File' || entry.kind === 'Dir')) {
      onRemoteCrossTransfer({
        sourceSessionId: sessionId,
        sourceLabel: target?.name ?? i18n.t('sftp.sshSession'),
        sourceProfile: target?.ssh,
        sourcePath: srcPath,
        sourceKind: entry.kind,
        sourceSize: entry.size,
        destSessionId: dest.sessionId,
        destLabel: dest.label,
        destProfile: dest.profile,
        destDir: dest.cwd,
        destPath,
        name: entry.name,
      });
      return;
    }
    const dirCache = new Set<string>();
    try {
      if (entry.kind === 'Dir') {
        await copyRemoteDirBetweenSessions(sessionId, srcPath, dest.sessionId, destPath, dirCache);
      } else if (entry.kind === 'File') {
        const destDir = parentPath(destPath);
        if (destDir !== '/' && destDir !== '.') {
          await ensureRemoteDirOn(dest.sessionId, destDir, dirCache);
        }
        await copyRemoteFileBetweenSessions(sessionId, srcPath, dest.sessionId, destPath, entry.size);
      }
    } catch (e) {
      onError(i18n.t('sftp.crossTransferFailed', { message: (e as Error).message }));
    }
  }

  function toggleTransferSelection(id: string) {
    const next = new Set(selectedTransferIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selectedTransferIds = next;
  }

  function selectAllTransfers() {
    selectedTransferIds = new Set(transfers.map((t) => t.id));
  }

  function invertTransferSelection() {
    const next = new Set<string>();
    for (const t of transfers) {
      if (!selectedTransferIds.has(t.id)) next.add(t.id);
    }
    selectedTransferIds = next;
  }

  function clearTransferSelection() {
    selectedTransferIds = new Set();
  }

  function removeSelectedTransfers() {
    const remove = selectedTransferIds;
    for (const id of remove) {
      const task = transfers.find((t) => t.id === id);
      if (task && (task.status === 'queued' || task.status === 'running' || task.status === 'paused')) {
        cancelTransfer(id);
      }
      uploadFiles.delete(id);
      downloadEntries.delete(id);
    }
    transfers = transfers.filter((t) => !remove.has(t.id));
    clearTransferSelection();
  }

  function clearAllTransfers() {
    cancelActiveTransfers();
    uploadFiles.clear();
    downloadEntries.clear();
    transfers = [];
    clearTransferSelection();
  }

  async function ensureRemoteDir(path: string) {
    if (!sessionId) throw new Error('SFTP session is not open');
    const normalized = normalizeRemotePath(path);
    if (normalized === '/' || normalized === '.') return;
    const absolute = normalized.startsWith('/');
    const segments = normalized.split('/').filter(Boolean);
    let cursor = absolute ? '/' : '.';
    for (const segment of segments) {
      cursor = joinPath(cursor, segment);
      if (knownRemoteDirs.has(cursor)) continue;
      try {
        const entry = await withTransferTimeout('sftp.stat', rpc.call<SftpEntry>('sftp.stat', { id: sessionId, path: cursor }));
        if (entry.kind !== 'Dir') throw new Error(`${cursor} exists and is not a directory`);
      } catch (e) {
        try {
          await withTransferTimeout('sftp.mkdir', rpc.call('sftp.mkdir', { id: sessionId, path: cursor }));
        } catch (mkdirError) {
          const entry = await withTransferTimeout('sftp.stat', rpc.call<SftpEntry>('sftp.stat', { id: sessionId, path: cursor }))
            .catch(() => null);
          if (entry?.kind !== 'Dir') {
            throw mkdirError;
          }
        }
      }
      knownRemoteDirs.add(cursor);
    }
  }

  async function runDownload(task: TransferTask) {
    if (!sessionId) throw new Error('SFTP session is not open');
    const entry = downloadEntries.get(task.id);
    if (!entry) throw new Error('remote file is not available');
    const writesToDisk = Boolean(task.localPath || (task.localBaseDir && task.relativePath));
    const chunks: Uint8Array[] = [];
    if (writesToDisk && entry.size === 0) {
      await writeLocalDownloadChunk(task, 0, new Uint8Array(), true);
      updateTransfer(task.id, { transferred: 0 });
      return;
    }
    let offset = 0;
    while (offset < entry.size) {
      await waitWhilePaused(task.id);
      if (isCanceled(task.id)) return;
      const len = Math.min(CHUNK_SIZE, entry.size - offset);
      const r = await withTransferTimeout('sftp.readChunk', rpc.call<{ data: string }>('sftp.readChunk', {
        id: sessionId,
        path: task.path,
        offset,
        len,
      }));
      const bytes = b64decode(r.data);
      if (bytes.byteLength === 0) break;
      if (writesToDisk) {
        await writeLocalDownloadChunk(task, offset, bytes, offset === 0);
      } else {
        chunks.push(bytes);
      }
      offset += bytes.byteLength;
      updateTransfer(task.id, { transferred: offset });
    }
    if (isCanceled(task.id)) return;
    if (writesToDisk) return;
    const blob = new Blob(chunks.map((chunk) => new Uint8Array(chunk)), { type: 'application/octet-stream' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = entry.name;
    a.click();
    URL.revokeObjectURL(url);
  }

  function cancelTransfer(id: string) {
    const status = transfers.find((transfer) => transfer.id === id)?.status;
    if (status === 'done' || status === 'error') return;
    updateTransfer(id, { status: 'canceled', message: 'Canceled' });
  }

  function cancelActiveTransfers() {
    for (const task of transfers) {
      if (task.status === 'queued' || task.status === 'running' || task.status === 'paused') {
        updateTransfer(task.id, { status: 'canceled', message: 'Canceled' });
      }
    }
  }

  function clearFinishedTransfers() {
    const finished = new Set(
      transfers
        .filter((transfer) =>
          transfer.status === 'done' || transfer.status === 'error' || transfer.status === 'canceled',
        )
        .map((transfer) => transfer.id),
    );
    for (const id of finished) {
      uploadFiles.delete(id);
      downloadEntries.delete(id);
    }
    transfers = transfers.filter((transfer) => !finished.has(transfer.id));
  }

  function startInlineRename(e: SftpEntry) {
    if (loading) return;
    selectRemoteEntry(e);
    renamingName = e.name;
    renameDraft = e.name;
  }

  function cancelInlineRename() {
    renamingName = null;
    renameDraft = '';
  }

  async function commitInlineRename(prevName: string) {
    const nextName = renameDraft.trim();
    renamingName = null;
    if (!sessionId || !nextName || nextName === prevName) return;
    if (nextName.includes('/')) {
      onError('rename: name must not contain /');
      return;
    }
    try {
      await rpc.call('sftp.rename', {
        id: sessionId,
        from: joinPath(cwd, prevName),
        to: joinPath(cwd, nextName),
      });
      await refresh();
    } catch (err) {
      onError(`rename: ${(err as Error).message}`);
    }
  }

  function runRemoteMenuAction(fn: (entry: SftpEntry) => void) {
    return (ev: MouseEvent) => {
      ev.preventDefault();
      ev.stopPropagation();
      const entry = remoteMenuEntry;
      closeRemoteMenu();
      if (entry) fn(entry);
    };
  }

  async function mkdirHere() {
    if (!sessionId) return;
    const name = await appPrompt(i18n.t('sftp.mkdirPrompt'), { position: focusedDialogPosition() });
    if (!name) return;
    try {
      await rpc.call('sftp.mkdir', { id: sessionId, path: joinPath(cwd, name) });
      await refresh();
    } catch (e) {
      onError((e as Error).message);
    }
  }

  async function downloadEntry(e: SftpEntry) {
    if (!sessionId) return;
    if (e.kind === 'Dir') {
      await downloadDirectory(e);
      return;
    }
    if (e.kind !== 'File') return;
    let savePath = await pickSavePath(e.name);
    if (savePath === undefined && defaultDownloadDir) {
      const sep = defaultDownloadDir.includes('\\') ? '\\' : '/';
      savePath = `${defaultDownloadDir.replace(/[/\\]+$/, '')}${sep}${e.name}`;
    }
    if (savePath === null) return;
    if (savePath) lastDownloadPath = savePath;
    const id = nextTransferId();
    const path = joinPath(cwd, e.name);
    downloadEntries.set(id, e);
    enqueueDownloadTransfer(e, path, e.name, {
      id,
      localPath: savePath,
    });
  }

  function enqueueDownloadTransfer(
    entry: SftpEntry,
    path: string,
    name: string,
    extra: Pick<TransferTask, 'id'> & Partial<TransferTask>,
  ) {
    downloadEntries.set(extra.id, entry);
    enqueueTransfer({
      id: extra.id,
      kind: 'download',
      name,
      path,
      size: entry.size,
      transferred: 0,
      status: 'queued',
      localPath: extra.localPath,
      localBaseDir: extra.localBaseDir,
      relativePath: extra.relativePath,
    });
  }

  async function downloadDirectory(e: SftpEntry) {
    let baseDir = await pickDirectoryPath();
    if (baseDir === undefined && defaultDownloadDir) baseDir = defaultDownloadDir;
    if (baseDir === null) return;
    if (baseDir === undefined) {
      onError('download folder: desktop directory picker is not available');
      return;
    }
    lastDownloadPath = baseDir;
    preparingTransfers = true;
    try {
      await collectDirectoryDownloads(joinPath(cwd, e.name), [e.name], baseDir);
    } catch (err) {
      onError(`download folder ${e.name}: ${(err as Error).message}`);
    } finally {
      preparingTransfers = false;
    }
  }

  async function collectDirectoryDownloads(remotePath: string, relative: string[], baseDir: string) {
    if (!sessionId) return;
    await mkdirLocalRelative(baseDir, relative);
    const list = sortEntries(await withTransferTimeout('sftp.list', rpc.call<SftpEntry[]>('sftp.list', { id: sessionId, path: remotePath })));
    for (const entry of list) {
      const childPath = joinPath(remotePath, entry.name);
      const childRelative = [...relative, entry.name];
      if (entry.kind === 'Dir') {
        await collectDirectoryDownloads(childPath, childRelative, baseDir);
      } else if (entry.kind === 'File') {
        const id = nextTransferId();
        enqueueDownloadTransfer(entry, childPath, childRelative.join('/'), {
          id,
          localBaseDir: baseDir,
          relativePath: childRelative,
        });
      }
    }
  }

  async function uploadFile() {
    if (!sessionId) return;
    const input = document.createElement('input');
    input.type = 'file';
    input.multiple = true;
    input.onchange = async () => {
      const files = Array.from(input.files ?? []);
      for (const f of files) {
        const id = nextTransferId();
        uploadFiles.set(id, f);
        enqueueTransfer({
          id,
          kind: 'upload',
          name: f.name,
          path: joinPath(cwd, f.name),
          size: f.size,
          transferred: 0,
          status: 'queued',
        });
      }
    };
    input.click();
  }

  async function uploadFolder() {
    if (!sessionId) return;
    const input = document.createElement('input') as HTMLInputElement & { webkitdirectory?: boolean };
    input.type = 'file';
    input.multiple = true;
    input.webkitdirectory = true;
    input.setAttribute('webkitdirectory', '');
    input.onchange = async () => {
      const files = Array.from(input.files ?? []);
      for (const f of files) {
        const relative = safeRelativeSegments(f.webkitRelativePath || f.name);
        if (relative.length === 0) continue;
        const id = nextTransferId();
        uploadFiles.set(id, f);
        enqueueTransfer({
          id,
          kind: 'upload',
          name: relative.join('/'),
          path: joinPathSegments(cwd, relative),
          size: f.size,
          transferred: 0,
          status: 'queued',
        });
      }
    };
    input.click();
  }

  function formatSize(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
    return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }

  function transferPercent(task: TransferTask): number {
    if (task.size <= 0) return task.status === 'done' ? 100 : 0;
    return Math.min(100, Math.round((task.transferred / task.size) * 100));
  }

  function transferSummary(task: TransferTask): string {
    if (task.status === 'queued') return i18n.t('sftp.transferQueued');
    if (task.status === 'paused') return task.message ?? i18n.t('sftp.paused');
    if (task.status === 'running') {
      const last = task.lastProgressAt ?? task.startedAt ?? Date.now();
      const idleMs = Date.now() - last;
      if (idleMs > TRANSFER_STALL_MS) {
        return i18n.t('transfer.waitingServer', { seconds: Math.round(idleMs / 1000) })
          + ` · ${formatSize(task.transferred)} / ${formatSize(task.size)}`;
      }
      return task.message ?? `${formatSize(task.transferred)} / ${formatSize(task.size)}`;
    }
    if (task.status === 'done') return i18n.t('sftp.transferDone');
    if (task.status === 'canceled') return i18n.t('sftp.transferCanceled');
    return task.message ?? i18n.t('sftp.transferFailed');
  }

  let breadcrumbs = $derived.by(() => {
    const parts = cwd.split('/').filter((p) => p.length > 0);
    const out: { label: string; path: string }[] = [{ label: '/', path: '/' }];
    let acc = '';
    for (const p of parts) {
      acc += '/' + p;
      out.push({ label: p, path: acc });
    }
    return out;
  });

  async function jumpTo(p: string) {
    if (loading) return;
    cwd = p;
    clearRemoteSelection();
    await refresh();
  }

  function toggleSudoMode() {
    void reconnect(!sudoMode);
  }

  async function chooseDownloadDir() {
    const picked = await pickDirectoryPath();
    if (!picked) return;
    defaultDownloadDir = picked;
    lastDownloadPath = picked;
    try {
      await rpc.call('settings.set', {
        key: 'sftp',
        value: { defaultDownloadDir: picked },
      });
    } catch (e) {
      onError(`sftp settings: ${(e as Error).message}`);
    }
  }

  onMount(() => {
    transferProgressTick = window.setInterval(() => {
      if (transfers.some((task) => task.status === 'running')) flushTransfersUi();
    }, 5000);
    void (async () => {
      await loadSftpSettings();
      await initLocalPane();
    })();
  });

  onDestroy(() => {
    if (transferProgressTick !== null) window.clearInterval(transferProgressTick);
    sftpSessionRegistry.unregister(registryId);
    if (sessionId) {
      void rpc.call('sftp.close', { id: sessionId }).catch(() => {});
    }
  });
</script>

<div
  class={mode === 'modal'
    ? 'fixed inset-0 bg-black/60 z-50 grid place-items-center p-6'
    : 'h-full w-full min-w-0 bg-[var(--color-panel)]'}
  data-aerotab-modal={mode === 'modal' ? '' : undefined}
  role={mode === 'modal' ? 'dialog' : 'complementary'}
  aria-modal={mode === 'modal'}
  aria-label={i18n.t('sftp.aria')}
>
  <div
    class={mode === 'modal'
      ? 'panel w-full max-w-[min(1200px,96vw)] h-full max-h-[720px] flex flex-col overflow-hidden'
      : 'h-full w-full flex flex-col overflow-hidden bg-[var(--color-panel)]'}
  >
    <header class="flex flex-col gap-0.5 px-4 py-2 border-b border-[var(--color-border-soft)]">
      <div class="flex items-center gap-2 min-w-0">
        <div class="text-[var(--color-accent)] font-semibold text-[13px] shrink-0">SFTP</div>
        <div class="min-w-0 flex-1">
          <div class="text-[12px] text-[var(--color-fg)] truncate">{target?.name ?? i18n.t('sftp.sshSession')}</div>
          {#if target}
            <div class="text-[10.5px] font-mono text-[var(--color-fg-muted)] truncate">{profileEndpointLabel({ name: target.name, kind: 'ssh', ssh: target.ssh } as StoredProfile)}</div>
          {/if}
        </div>
        {#if mode === 'dock' && showLocalPane}
          <button
            type="button"
            class="px-2 py-0.5 rounded text-[10.5px] border {localPaneCollapsed ? 'border-[var(--color-border-soft)] text-[var(--color-fg-muted)]' : 'border-[var(--color-accent)] text-[var(--color-accent)]'}"
            onclick={() => { localPaneCollapsed = !localPaneCollapsed; }}
          >
            {i18n.t('sftp.toggleLocalPane')}
          </button>
        {/if}
        <button
          type="button"
          class="px-2 py-0.5 rounded text-[10.5px] border {sudoMode ? 'border-[var(--color-accent)] text-[var(--color-accent)] bg-[var(--color-accent-soft)]' : 'border-[var(--color-border-soft)] text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]'}"
          onclick={toggleSudoMode}
          title={i18n.t('sftp.toggleSudo')}
          aria-pressed={sudoMode}
        >sudo</button>
        <div class="ml-auto flex items-center gap-1 shrink-0">
        {#if onPopOut}
          <button
            type="button"
            class="btn-ghost p-1"
            onclick={() => onPopOut(sudoMode)}
            title={i18n.t('sftp.openWindow')}
            aria-label={i18n.t('sftp.openWindow')}
          >
            <ExternalLink size={14} />
          </button>
        {/if}
        {#if mode === 'dock' && onCollapse}
          <button
            type="button"
            class="btn-ghost p-1"
            onclick={onCollapse}
            title={i18n.t('sftp.collapseDock')}
            aria-label={i18n.t('sftp.collapseDock')}
          >
            <PanelRightClose size={14} />
          </button>
        {/if}
        <button
          type="button"
          class="btn-ghost p-1"
          onclick={onClose}
          aria-label={i18n.t('common.close')}
        >
          <X size={14} />
        </button>
      </div>
      </div>
      {#if mode === 'dock'}
        <div class="text-[10px] text-[var(--color-fg-muted)]">{i18n.t('sftp.independentConnection')}</div>
      {/if}
    </header>

    <div class="flex items-center gap-1 px-3 py-1.5 border-b border-[var(--color-border-soft)] text-[12px]">
      <button type="button" class="toolbtn" onclick={goHome} title={i18n.t('common.home')}><Home size={13} /></button>
      <button type="button" class="toolbtn" onclick={goUp} title={i18n.t('common.up')}><ArrowUp size={13} /></button>
      <button type="button" class="toolbtn" onclick={refresh} title={i18n.t('common.refresh')}><RefreshCw size={13} /></button>
      <div class="mx-2 flex items-center gap-0.5 flex-wrap text-[11.5px] text-[var(--color-fg-muted)] min-w-0">
        {#each breadcrumbs as bc, i (i)}
          {#if i > 0}<ChevronRight size={11} class="text-[var(--color-border)]" />{/if}
          <button
            type="button"
            class="hover:text-[var(--color-accent)] px-1"
            onclick={() => jumpTo(bc.path)}
          >{bc.label}</button>
        {/each}
      </div>
      <div class="ml-auto flex items-center gap-1">
        {#if preparingTransfers}
          <Loader2 size={13} class="text-[var(--color-accent)] animate-spin mr-1" />
        {/if}
        <button type="button" class="toolbtn" onclick={mkdirHere} title={i18n.t('sftp.newFolder')}>
          <FolderPlus size={13} />
        </button>
        <button type="button" class="toolbtn" onclick={uploadFile} title={i18n.t('common.upload')}>
          <Upload size={13} />
        </button>
        <button type="button" class="toolbtn" onclick={uploadFolder} title={i18n.t('common.uploadFolder')}>
          <FolderUp size={13} />
        </button>
      </div>
    </div>

    {#if listError}
      <div class="mx-3 mt-2 px-3 py-2 rounded border border-[var(--color-danger)]/40 bg-[var(--color-danger)]/10
                  text-[12px] text-[var(--color-fg)] flex items-center gap-2">
        <span class="min-w-0 truncate">{i18n.t('sftp.listFailed', { message: listError })}</span>
        <button type="button" class="btn-secondary shrink-0 text-[11px] px-2 py-0.5" onclick={retryRemoteList}>
          {i18n.t('sftp.retryList')}
        </button>
      </div>
    {/if}

    <div class="flex-1 min-h-0 flex flex-col">
    <div class="flex-1 min-h-0 flex">
      {#if showLocalPaneEffective}
        <div class="w-[42%] min-w-[200px] max-w-[50%] flex flex-col min-h-0">
          <SftpLocalPane
            cwd={localCwd}
            entries={localEntries}
            loading={localLoading}
            listError={localListError}
            onRefresh={() => { void refreshLocal(); }}
            onNavigate={(p) => { void navigateLocal(p); }}
            onGoUp={() => { void localGoUp(); }}
            onGoHome={() => { void localGoHome(); }}
            onDragOverPane={preventDragDefaults}
            onDropRemote={(e) => { void handleLocalPaneDrop(e); }}
            onDropFiles={() => {}}
            {onError}
          />
        </div>
      {/if}
      <div
        bind:this={remotePaneEl}
        class="flex-1 min-w-0 flex flex-col min-h-0 outline-none"
        role="listbox"
        tabindex="0"
        aria-busy={loading}
        aria-label={i18n.t('sftp.remotePane')}
        onkeydown={onRemotePaneKeydown}
        ondragover={preventDragDefaults}
        ondrop={(e) => { if (!loading) void handleRemotePaneDrop(e); }}
      >
        <div class="px-2 py-1 shell-section-title border-b border-[var(--color-border-soft)]">
          {i18n.t('sftp.remotePane')}
        </div>
        <div class="relative flex-1 min-h-0 overflow-y-auto {loading && entries.length > 0 ? 'pointer-events-none opacity-70' : ''}">
          {#if loading && entries.length === 0 && !listError}
            <div class="px-4 py-6 text-[12px] text-[var(--color-fg-muted)]">{i18n.t('common.loading')}</div>
          {:else if entries.length === 0 && !listError}
            <div class="px-4 py-6 text-[12px] text-[var(--color-fg-muted)] italic">{i18n.t('sftp.emptyDirectory')}</div>
          {:else}
            <table class="w-full text-[12px]">
              <thead class="sticky top-0 bg-[var(--color-panel)] text-[10.5px] uppercase tracking-[0.12em] text-[var(--color-fg-muted)]">
                <tr>
                  <th class="text-left px-3 py-1.5 font-normal">{i18n.t('sftp.name')}</th>
                  <th class="text-right px-3 py-1.5 font-normal w-[100px]">{i18n.t('sftp.size')}</th>
                  <th class="text-left px-3 py-1.5 font-normal w-[100px]">{i18n.t('sftp.mode')}</th>
                  <th class="w-[96px]"></th>
                </tr>
              </thead>
              <tbody
                ondragover={preventDragDefaults}
                ondrop={(e) => {
                  e.stopPropagation();
                  void handleRemotePaneDrop(e);
                }}
              >
                {#each entries as e (e.name)}
                  <tr
                    data-remote-row=""
                    class="hover:bg-[var(--color-panel-2)] group {selectedRemoteNames.has(e.name) ? 'bg-[var(--color-accent-soft)] selected-row' : ''} {focusedRemoteName === e.name ? 'outline outline-1 outline-[var(--color-accent)] outline-offset-[-1px] selected-row' : ''}"
                    class:focused={focusedRemoteName === e.name}
                    draggable={!loading && (e.kind === 'File' || e.kind === 'Dir')}
                    role="option"
                    aria-selected={selectedRemoteNames.has(e.name)}
                    ondragstart={(ev) => onRemoteDragStart(ev, e)}
                    oncontextmenu={(ev) => { void openRemoteMenu(e, ev); }}
                    onclick={(ev) => selectRemoteEntry(e, ev)}
                    ondblclick={() => { void enter(e); }}
                  >
                    <td class="px-3 py-1 truncate">
                      {#if renamingName === e.name}
                        <form
                          class="flex items-center gap-1"
                          onsubmit={(ev) => {
                            ev.preventDefault();
                            void commitInlineRename(e.name);
                          }}
                        >
                          {#if e.kind === 'Dir'}
                            <Folder size={13} class="text-[var(--color-accent)] shrink-0" />
                          {:else}
                            <FileText size={13} class="text-[var(--color-fg-muted)] shrink-0" />
                          {/if}
                          <input
                            class="input flex-1 min-w-0 py-0.5 text-[12px] font-mono"
                            bind:value={renameDraft}
                            onkeydown={(ev) => {
                              if (ev.key === 'Escape') cancelInlineRename();
                            }}
                            onblur={() => { void commitInlineRename(e.name); }}
                          />
                        </form>
                      {:else}
                        <button
                          type="button"
                          class="flex items-center gap-2 w-full text-left"
                          ondblclick={(ev) => { ev.stopPropagation(); void enter(e); }}
                          onclick={(ev) => { ev.stopPropagation(); selectRemoteEntry(e, ev); }}
                        >
                          {#if e.kind === 'Dir'}
                            <Folder size={13} class="text-[var(--color-accent)]" />
                          {:else}
                            <FileText size={13} class="text-[var(--color-fg-muted)]" />
                          {/if}
                          <span class="truncate text-[var(--color-fg)]">{e.name}</span>
                        </button>
                      {/if}
                    </td>
                    <td class="px-3 py-1 text-right text-[var(--color-fg-muted)]">
                      {e.kind === 'File' ? formatSize(e.size) : ''}
                    </td>
                    <td class="px-3 py-1 text-[var(--color-fg-muted)] font-mono text-[11px]">
                      {(e.mode & 0o777).toString(8).padStart(3, '0')}
                    </td>
                    <td class="px-2 py-1 text-right whitespace-nowrap">
                      {#if e.kind === 'File'}
                        <button
                          type="button"
                          class="opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 [.group.selected-row_&]:opacity-100 p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-accent)]"
                          onclick={(ev) => { ev.stopPropagation(); void openTextEditor(e); }}
                          title={i18n.t('sftp.editFile')}
                          aria-label={i18n.t('sftp.editFile')}
                        >
                          <FileText size={12} />
                        </button>
                      {/if}
                      {#if e.kind === 'File' || e.kind === 'Dir'}
                        <button
                          type="button"
                          class="opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 [.group.selected-row_&]:opacity-100 p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-accent)]"
                          onclick={(ev) => { ev.stopPropagation(); downloadEntry(e); }}
                          title={e.kind === 'Dir' ? i18n.t('common.downloadFolder') : i18n.t('common.download')}
                          aria-label={e.kind === 'Dir' ? i18n.t('common.downloadFolder') : i18n.t('common.download')}
                        >
                          {#if e.kind === 'Dir'}<FolderDown size={12} />{:else}<Download size={12} />{/if}
                        </button>
                      {/if}
                      <button
                        type="button"
                        class="opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 [.group.selected-row_&]:opacity-100 p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-accent)]"
                        onclick={(ev) => { ev.stopPropagation(); startInlineRename(e); }}
                        title={i18n.t('common.rename')}
                        aria-label={i18n.t('common.rename')}
                      >
                        <Pencil size={12} />
                      </button>
                      <button
                        type="button"
                        class="opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 [.group.selected-row_&]:opacity-100 p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-danger)]"
                        onclick={(ev) => { ev.stopPropagation(); void removeEntry(e); }}
                        title={i18n.t('common.delete')}
                        aria-label={i18n.t('common.delete')}
                      >
                        <Trash2 size={12} />
                      </button>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}
        </div>
      </div>
    </div>

    {#if !defaultDownloadDir && !downloadBannerDismissed && showLocalPane}
      <div class="mx-3 mb-1 px-3 py-2 rounded border border-[var(--color-border-soft)] bg-[var(--color-panel-2)] text-[11px] flex items-center gap-2">
        <span class="min-w-0 flex-1">{i18n.t('sftp.downloadDirBanner')}</span>
        <button type="button" class="btn-secondary text-[11px] px-2 py-0.5 shrink-0" onclick={() => { void chooseDownloadDir(); }}>
          {i18n.t('sftp.pickDownloadDir')}
        </button>
        <button type="button" class="btn-ghost p-1 shrink-0" aria-label={i18n.t('common.close')} onclick={() => { downloadBannerDismissed = true; }}>
          <X size={12} />
        </button>
      </div>
    {/if}

    <div class="border-t border-[var(--color-border-soft)] px-3 py-1.5 text-[11px] text-[var(--color-fg-muted)]
                flex items-center gap-2 min-h-[28px]">
      <span class="truncate min-w-0" title={lastDownloadPath ?? defaultDownloadDir ?? ''}>
        {#if lastDownloadPath}
          {i18n.t('sftp.downloadDir', { path: lastDownloadPath })}
        {:else if defaultDownloadDir}
          {i18n.t('sftp.defaultDownloadDir', { path: defaultDownloadDir })}
        {:else}
          {i18n.t('sftp.downloadDirUnset')}
        {/if}
      </span>
      <button
        type="button"
        class="ml-auto shrink-0 text-[var(--color-accent)] hover:underline"
        onclick={() => { void chooseDownloadDir(); }}
      >
        {i18n.t('sftp.pickDownloadDir')}
      </button>
    </div>

    {#if transfers.length > 0}
      <button
        type="button"
        aria-label={i18n.t('sftp.resizeTransferPanel')}
        class="h-[3px] shrink-0 cursor-row-resize bg-[var(--color-border-soft)] hover:bg-[var(--color-accent)] border-0 p-0 w-full"
        onpointerdown={(ev) => {
          startVerticalPanelResize(ev, {
            startHeightPx: transferPanelHeightPx,
            minPx: 120,
            maxPx: 360,
            onHeight: (h) => onTransferPanelHeightChange?.(h),
          });
        }}
      ></button>
      <SftpTransferQueue
        variant="compact"
        queueView="active"
        tasks={transferQueueItems}
        selectedIds={selectedTransferIds}
        maxHeight="{transferPanelHeightPx}px"
        onSelectAll={selectAllTransfers}
        onInvertSelection={invertTransferSelection}
        onClearSelection={clearTransferSelection}
        onRemoveSelected={removeSelectedTransfers}
        onCancelAll={cancelActiveTransfers}
        onClearFinished={clearFinishedTransfers}
        onClearAll={clearAllTransfers}
        onToggleSelect={toggleTransferSelection}
        onPause={pauseTransfer}
        onResume={resumeTransfer}
        onCancel={cancelTransfer}
        onOpenTransferCenter={onOpenTransferCenter}
      />
    {/if}
    </div>
  </div>
</div>

{#if remoteMenuOpen && remoteMenuEntry}
  {@const entry = remoteMenuEntry}
  <div use:portal class="contents">
    <div
      role="presentation"
      data-aerotab-menu-open=""
      class="fixed inset-0 z-[58]"
      onmousedown={closeRemoteMenu}
      oncontextmenu={(e) => {
        e.preventDefault();
        closeRemoteMenu();
      }}
    ></div>
    <div
      bind:this={remoteMenuEl}
      role="menu"
      tabindex="-1"
      data-aerotab-menu-open=""
      class="panel fixed z-[59] min-w-[200px] py-1 text-[12.5px]"
      style="left: {remoteMenuX}px; top: {remoteMenuY}px;"
      onmousedown={(e) => e.stopPropagation()}
    >
      {#if entry.kind === 'Dir'}
        <button type="button" class="menu-item" onmousedown={runRemoteMenuAction((en) => { void enter(en); })}>
          {i18n.t('sftp.contextOpen')}
        </button>
      {:else if entry.kind === 'File'}
        <button type="button" class="menu-item" onmousedown={runRemoteMenuAction((en) => { void openTextEditor(en); })}>
          {i18n.t('sftp.contextEdit')}
        </button>
      {/if}
      {#if entry.kind === 'File' || entry.kind === 'Dir'}
        <button type="button" class="menu-item" onmousedown={runRemoteMenuAction((en) => { downloadEntry(en); })}>
          {entry.kind === 'Dir' ? i18n.t('common.downloadFolder') : i18n.t('sftp.contextDownload')}
        </button>
      {/if}
      <button type="button" class="menu-item" onmousedown={runRemoteMenuAction((en) => { startInlineRename(en); })}>
        {i18n.t('sftp.contextRename')}
      </button>
      {#if otherSftpSessions.length > 0}
        <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
        <div class="px-3 py-1 text-[10px] uppercase tracking-wide text-[var(--color-fg-muted)]">
          {i18n.t('sftp.sendToSessionTitle')}
        </div>
        {#each otherSftpSessions as dest (dest.registryId)}
          <button type="button" class="menu-item" onmousedown={runRemoteMenuAction((en) => { void sendEntryToSession(en, dest); })}>
            {i18n.t('sftp.sendToSession', { label: dest.label })}
          </button>
        {/each}
      {/if}
      <div class="my-1 border-t border-[var(--color-border-soft)]"></div>
      <button type="button" class="menu-item text-[var(--color-danger)]" onmousedown={runRemoteMenuAction((en) => { const selected = selectedRemoteEntries(); void removeEntries(selected.length > 1 && selected.some((item) => item.name === en.name) ? selected : [en]); })}>
        {selectedRemoteNames.size > 1 ? i18n.t('sftp.contextDeleteSelected', { count: selectedRemoteNames.size }) : i18n.t('sftp.contextDelete')}
      </button>
    </div>
  </div>
{/if}

{#if editOpen}
  <div class="fixed inset-0 z-[60] bg-black/50 grid place-items-center p-6" role="dialog" aria-modal="true" data-aerotab-modal="">
    <div class="bg-[var(--color-panel)] border border-[var(--color-border)] rounded-lg shadow-2xl w-full max-w-[720px] max-h-[80vh] flex flex-col">
      <header class="flex items-center gap-2 px-4 py-2 border-b border-[var(--color-border-soft)]">
        <span class="font-semibold text-[13px] truncate">{editName}</span>
        <button type="button" class="ml-auto p-1" onclick={() => { editOpen = false; }} aria-label={i18n.t('common.close')}>
          <X size={14} />
        </button>
      </header>
      <textarea
        class="flex-1 min-h-[320px] m-3 p-2 font-mono text-[12px] bg-[var(--color-bg-soft)] text-[var(--color-fg)] border border-[var(--color-border)] rounded resize-y"
        bind:value={editContent}
        spellcheck="false"
      ></textarea>
      <footer class="flex justify-end gap-2 px-4 py-2 border-t border-[var(--color-border-soft)]">
        <button type="button" class="btn-secondary text-[12px] px-3 py-1" onclick={() => { editOpen = false; }}>
          {i18n.t('common.cancel')}
        </button>
        <button
          type="button"
          class="btn-secondary text-[12px] px-3 py-1 text-[var(--color-accent)]"
          disabled={editSaving}
          onclick={() => { void saveTextEditor(); }}
        >
          {editSaving ? i18n.t('common.saving') : i18n.t('common.save')}
        </button>
      </footer>
    </div>
  </div>
{/if}
