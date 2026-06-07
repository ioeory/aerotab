<script lang="ts">
  import {
    X, ArrowLeftRight, RefreshCw, Clock3, Loader2, CheckCircle2, CircleX,
    Pause, RotateCw, Trash2, History, ListChecks,
  } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import { b64decode, b64encode, tauriInvoke } from '../lib/rpc';
  import type { LocalEntry, SftpEntry, SshProfileSpec, StoredProfile } from '../lib/types';
  import { i18n } from '../lib/i18n.svelte';
  import SftpBrowser from './SftpBrowser.svelte';
  import SftpLocalPane from './SftpLocalPane.svelte';
  import type { LocalUploadTransferRequest, RemoteCrossTransferRequest } from '../lib/sftpTransferTypes';
  import {
    joinLocalPath,
    parentLocalPath,
    parseRemoteDrag,
    readSftpDragData,
    SFTP_DRAG_REMOTE,
  } from '../lib/sftpLocal';
  import { onMount } from 'svelte';

  interface TransferTarget {
    name: string;
    ssh: SshProfileSpec;
  }

  interface Props {
    rpc: RpcClient;
    initialTarget?: TransferTarget | null;
    initialProfileId?: string | null;
    standalone?: boolean;
    embedded?: boolean;
    onClose: () => void;
    onError: (msg: string) => void;
  }

  type Side = 'left' | 'right';
  type TransferStatus = 'queued' | 'running' | 'paused' | 'done' | 'error' | 'canceled';
  type TransferDirection = 'remote-remote' | 'local-remote' | 'remote-local';
  type TransferEndpoint = 'remote' | 'local';
  type TransferEntryKind = 'File' | 'Dir';
  type TransferMode = 'auto' | 'direct' | 'relay';

  interface PlannedFile {
    sourcePath: string;
    destPath: string;
    size: number;
  }

  interface TransferTask {
    id: string;
    direction: TransferDirection;
    name: string;
    sourceEndpoint: TransferEndpoint;
    destEndpoint: TransferEndpoint;
    sourceLabel: string;
    destLabel: string;
    sourceProfile?: SshProfileSpec;
    destProfile?: SshProfileSpec;
    mode: TransferMode;
    method?: 'direct' | 'relay';
    sourceSessionId?: string;
    sourcePath: string;
    sourceKind: TransferEntryKind;
    sourceSize: number;
    destSessionId?: string;
    destDir: string;
    destPath: string;
    size: number;
    transferred: number;
    status: TransferStatus;
    attempts: number;
    createdAt: number;
    startedAt?: number;
    finishedAt?: number;
    message?: string;
    files?: PlannedFile[];
    dirs?: string[];
  }

  interface PersistedTransferState {
    version: 1;
    tasks: TransferTask[];
  }

  let {
    rpc,
    initialTarget = null,
    initialProfileId = null,
    standalone = false,
    embedded = false,
    onClose,
    onError,
  }: Props = $props();

  const LOCAL_ENDPOINT_ID = '__local__';
  const INITIAL_ENDPOINT_ID = '__initial__';
  const STORAGE_KEY = 'aerotab.fileTransfer.tasks.v1';
  const MAX_PERSISTED_HISTORY = 200;
  const CHUNK_SIZE = 256 * 1024;

  let profiles = $state<StoredProfile[]>([]);
  let loadingProfiles = $state(false);
  let leftId = $state(LOCAL_ENDPOINT_ID);
  let rightId = $state('');
  let tasks = $state<TransferTask[]>([]);
  let transferMode = $state<TransferMode>('auto');
  let processing = false;
  let transferSeq = 0;
  let refreshToken = $state(0);
  let selectedTaskIds = $state<Set<string>>(new Set());
  let lastInitialKey = $state('');
  let tasksHydrated = $state(false);

  let leftLocalCwd = $state('');
  let leftLocalEntries = $state<LocalEntry[]>([]);
  let leftLocalLoading = $state(false);
  let leftLocalListError = $state<string | null>(null);
  let leftLocalListSeq = 0;

  let rightLocalCwd = $state('');
  let rightLocalEntries = $state<LocalEntry[]>([]);
  let rightLocalLoading = $state(false);
  let rightLocalListError = $state<string | null>(null);
  let rightLocalListSeq = 0;

  const windowId = `transfer-${Date.now()}-${Math.round(Math.random() * 100000)}`;
  const sshProfiles = $derived(profiles.filter((p) => p.kind === 'ssh'));
  const leftTarget = $derived(targetForId(leftId));
  const rightTarget = $derived(targetForId(rightId));
  const effectiveLeftTarget = $derived(leftTarget);
  const activeTasks = $derived(tasks.filter((t) => t.status === 'queued' || t.status === 'running' || t.status === 'paused'));
  const historyTasks = $derived(tasks.filter((t) => t.status === 'done' || t.status === 'error' || t.status === 'canceled'));
  const aggregate = $derived.by(() => {
    const running = activeTasks;
    const total = running.reduce((sum, t) => sum + Math.max(0, t.size), 0);
    const done = running.reduce((sum, t) => sum + Math.min(Math.max(0, t.transferred), Math.max(0, t.size)), 0);
    const percent = total > 0 ? Math.min(100, Math.round((done / total) * 100)) : 0;
    return { count: running.length, total, done, percent };
  });

  function targetForId(id: string): TransferTarget | null {
    if (id === INITIAL_ENDPOINT_ID) return initialTarget;
    const profile = sshProfiles.find((p) => p.id === id);
    if (!profile || profile.kind !== 'ssh') return null;
    return { name: profile.name, ssh: profile.ssh };
  }

  function isLocalEndpoint(id: string): boolean {
    return id === LOCAL_ENDPOINT_ID;
  }

  function normalizeEndpointSelection(id: string): string {
    if (id === LOCAL_ENDPOINT_ID || id === INITIAL_ENDPOINT_ID) return id;
    return sshProfiles.some((p) => p.id === id) ? id : '';
  }

  function pickInitialIds(list: StoredProfile[]) {
    const ssh = list.filter((p) => p.kind === 'ssh');
    if (initialProfileId && ssh.some((p) => p.id === initialProfileId)) {
      leftId = initialProfileId;
    } else if (initialTarget) {
      const match = ssh.find((p) => p.name === initialTarget?.name);
      leftId = match?.id ?? INITIAL_ENDPOINT_ID;
    } else if (!leftId || (!isLocalEndpoint(leftId) && !ssh.some((p) => p.id === leftId))) {
      leftId = LOCAL_ENDPOINT_ID;
    }

    rightId = normalizeEndpointSelection(rightId);
    if (!rightId || rightId === leftId) {
      rightId = isLocalEndpoint(leftId) ? (ssh[0]?.id ?? '') : LOCAL_ENDPOINT_ID;
    }
  }

  async function refreshProfiles() {
    loadingProfiles = true;
    try {
      const list = await rpc.call<StoredProfile[]>('profile.list');
      profiles = list;
      pickInitialIds(list);
    } catch (e) {
      onError(`profile.list: ${(e as Error).message}`);
      profiles = [];
    } finally {
      loadingProfiles = false;
    }
  }

  async function initLocalSide(side: Side) {
    const cwd = side === 'left' ? leftLocalCwd : rightLocalCwd;
    if (cwd) return;
    try {
      const home = await tauriInvoke<string>('local_home_dir');
      if (!home) throw new Error('home directory is not available');
      if (side === 'left') leftLocalCwd = home;
      else rightLocalCwd = home;
      await refreshLocalSide(side);
    } catch (e) {
      if (side === 'left') leftLocalListError = (e as Error).message;
      else rightLocalListError = (e as Error).message;
    }
  }

  async function refreshLocalSide(side: Side) {
    const cwd = side === 'left' ? leftLocalCwd : rightLocalCwd;
    if (!cwd) return;
    const seq = side === 'left' ? ++leftLocalListSeq : ++rightLocalListSeq;
    if (side === 'left') {
      leftLocalLoading = true;
      leftLocalListError = null;
    } else {
      rightLocalLoading = true;
      rightLocalListError = null;
    }
    try {
      const list = await tauriInvoke<LocalEntry[]>('local_list_dir', { path: cwd });
      if (!list) throw new Error('local file browser is not available');
      if (side === 'left') {
        if (seq !== leftLocalListSeq) return;
        leftLocalEntries = list;
      } else {
        if (seq !== rightLocalListSeq) return;
        rightLocalEntries = list;
      }
    } catch (e) {
      if (side === 'left') {
        if (seq !== leftLocalListSeq) return;
        leftLocalListError = (e as Error).message;
        leftLocalEntries = [];
      } else {
        if (seq !== rightLocalListSeq) return;
        rightLocalListError = (e as Error).message;
        rightLocalEntries = [];
      }
    } finally {
      if (side === 'left') {
        if (seq === leftLocalListSeq) leftLocalLoading = false;
      } else if (seq === rightLocalListSeq) {
        rightLocalLoading = false;
      }
    }
  }

  async function navigateLocalSide(side: Side, path: string) {
    if (side === 'left') leftLocalCwd = path;
    else rightLocalCwd = path;
    await refreshLocalSide(side);
  }

  async function localSideGoUp(side: Side) {
    await navigateLocalSide(side, parentLocalPath(side === 'left' ? leftLocalCwd : rightLocalCwd));
  }

  async function localSideGoHome(side: Side) {
    const home = await tauriInvoke<string>('local_home_dir');
    if (home) await navigateLocalSide(side, home);
  }

  function preventDragDefaults(e: DragEvent) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
  }

  function ignoreFileDrop(e: DragEvent) {
    e.preventDefault();
  }

  function swapTargets() {
    const left = leftId;
    leftId = rightId;
    rightId = left;
  }

  async function closeWindow() {
    if (standalone) {
      const closed = await tauriInvoke<void>('close_current_window')?.then(() => true).catch(() => false);
      if (closed) return;
    }
    onClose();
  }

  function nextTaskId(): string {
    transferSeq += 1;
    return `transfer-${Date.now()}-${transferSeq}`;
  }

  function enqueueRemoteTransfer(request: RemoteCrossTransferRequest) {
    const task: TransferTask = {
      id: nextTaskId(),
      direction: 'remote-remote',
      name: request.name,
      sourceEndpoint: 'remote',
      destEndpoint: 'remote',
      sourceLabel: request.sourceLabel ?? i18n.t('sftp.sshSession'),
      destLabel: request.destLabel,
      sourceProfile: request.sourceProfile,
      destProfile: request.destProfile,
      mode: transferMode,
      sourceSessionId: request.sourceSessionId,
      sourcePath: request.sourcePath,
      sourceKind: request.sourceKind,
      sourceSize: request.sourceSize,
      destSessionId: request.destSessionId,
      destDir: request.destDir,
      destPath: request.destPath,
      size: request.sourceKind === 'File' ? request.sourceSize : 0,
      transferred: 0,
      status: 'queued',
      attempts: 1,
      createdAt: Date.now(),
    };
    tasks = [...tasks, task];
    void processQueue();
  }

  function enqueueLocalUpload(request: LocalUploadTransferRequest) {
    const task: TransferTask = {
      id: nextTaskId(),
      direction: 'local-remote',
      name: request.name,
      sourceEndpoint: 'local',
      destEndpoint: 'remote',
      sourceLabel: i18n.t('transfer.localComputer'),
      destLabel: request.destLabel,
      mode: 'relay',
      sourcePath: request.sourcePath,
      sourceKind: request.sourceKind,
      sourceSize: request.sourceSize,
      destSessionId: request.destSessionId,
      destDir: request.destDir,
      destPath: request.destPath,
      size: request.sourceKind === 'File' ? request.sourceSize : 0,
      transferred: 0,
      status: 'queued',
      attempts: 1,
      createdAt: Date.now(),
    };
    tasks = [...tasks, task];
    void processQueue();
  }

  function enqueueRemoteDownload(payload: { remote: NonNullable<ReturnType<typeof parseRemoteDrag>>; destDir: string; destLabel: string }) {
    if (!payload.remote.sourceSessionId) return;
    const task: TransferTask = {
      id: nextTaskId(),
      direction: 'remote-local',
      name: payload.remote.name,
      sourceEndpoint: 'remote',
      destEndpoint: 'local',
      sourceLabel: payload.remote.sourceLabel ?? i18n.t('sftp.sshSession'),
      destLabel: payload.destLabel,
      mode: 'relay',
      sourceSessionId: payload.remote.sourceSessionId,
      sourcePath: payload.remote.path,
      sourceKind: payload.remote.kind,
      sourceSize: payload.remote.size,
      destDir: payload.destDir,
      destPath: joinLocalPath(payload.destDir, payload.remote.name),
      size: payload.remote.kind === 'File' ? payload.remote.size : 0,
      transferred: 0,
      status: 'queued',
      attempts: 1,
      createdAt: Date.now(),
    };
    tasks = [...tasks, task];
    void processQueue();
  }

  async function handleLocalPaneDrop(side: Side, e: DragEvent) {
    e.preventDefault();
    const remoteRaw = readSftpDragData(e.dataTransfer, SFTP_DRAG_REMOTE);
    if (!remoteRaw) return;
    const remote = parseRemoteDrag(remoteRaw);
    if (!remote?.sourceSessionId) return;
    const destDir = side === 'left' ? leftLocalCwd : rightLocalCwd;
    if (!destDir) return;
    enqueueRemoteDownload({ remote, destDir, destLabel: i18n.t('transfer.localComputer') });
  }

  function updateTask(id: string, patch: Partial<TransferTask>) {
    tasks = tasks.map((task) => task.id === id ? { ...task, ...patch } : task);
  }

  function currentTask(id: string): TransferTask | undefined {
    return tasks.find((task) => task.id === id);
  }

  function isCanceled(id: string): boolean {
    return currentTask(id)?.status === 'canceled';
  }

  function isPaused(id: string): boolean {
    return currentTask(id)?.status === 'paused';
  }

  async function waitWhilePaused(id: string) {
    while (isPaused(id) && !isCanceled(id)) {
      await new Promise((resolve) => setTimeout(resolve, 150));
    }
  }

  function pauseTask(id: string) {
    const status = currentTask(id)?.status;
    if (status === 'queued' || status === 'running') {
      updateTask(id, { status: 'paused', message: i18n.t('sftp.paused') });
    }
  }

  function resumeTask(id: string) {
    if (currentTask(id)?.status !== 'paused') return;
    updateTask(id, { status: 'queued', message: undefined });
    void processQueue();
  }

  function cancelTask(id: string) {
    const status = currentTask(id)?.status;
    if (!status || status === 'done' || status === 'error' || status === 'canceled') return;
    updateTask(id, { status: 'canceled', message: i18n.t('sftp.transferCanceled'), finishedAt: Date.now() });
  }

  function retryTask(id: string) {
    const task = currentTask(id);
    if (!task || (task.status !== 'error' && task.status !== 'canceled')) return;
    updateTask(id, {
      status: 'queued',
      transferred: 0,
      size: task.sourceKind === 'File' ? task.sourceSize : 0,
      message: undefined,
      files: undefined,
      dirs: undefined,
      method: undefined,
      attempts: task.attempts + 1,
      startedAt: undefined,
      finishedAt: undefined,
    });
    void processQueue();
  }

  function cancelActiveTasks() {
    for (const task of activeTasks) cancelTask(task.id);
  }

  function clearHistory() {
    const history = new Set(historyTasks.map((t) => t.id));
    tasks = tasks.filter((task) => !history.has(task.id));
    selectedTaskIds = new Set([...selectedTaskIds].filter((id) => !history.has(id)));
  }

  function clearAllTasks() {
    cancelActiveTasks();
    tasks = [];
    selectedTaskIds = new Set();
  }

  function retryFailedTasks() {
    for (const task of tasks) {
      if (task.status === 'error') retryTask(task.id);
    }
  }

  function removeSelectedTasks() {
    const selected = selectedTaskIds;
    for (const id of selected) cancelTask(id);
    tasks = tasks.filter((task) => !selected.has(task.id));
    selectedTaskIds = new Set();
  }

  function toggleTaskSelection(id: string) {
    const next = new Set(selectedTaskIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selectedTaskIds = next;
  }

  function normalizeRestoredTask(task: TransferTask): TransferTask {
    const restored = { ...task, mode: task.mode ?? 'auto', sourceProfile: undefined, destProfile: undefined };
    if (restored.status === 'running' || restored.status === 'queued') {
      return {
        ...restored,
        status: 'paused',
        message: i18n.t('transfer.restoredPaused'),
        startedAt: undefined,
      };
    }
    return restored;
  }

  function serializableTask(task: TransferTask): TransferTask {
    const { sourceProfile, destProfile, ...rest } = task;
    void sourceProfile;
    void destProfile;
    return rest as TransferTask;
  }

  function restorePersistedTasks() {
    try {
      const raw = window.localStorage.getItem(STORAGE_KEY);
      if (!raw) return;
      const state = JSON.parse(raw) as PersistedTransferState;
      if (state.version !== 1 || !Array.isArray(state.tasks)) return;
      tasks = state.tasks.map(normalizeRestoredTask);
      const maxSeq = tasks
        .map((task) => Number(task.id.split('-').pop()))
        .filter((n) => Number.isFinite(n))
        .reduce((max, n) => Math.max(max, n), 0);
      transferSeq = maxSeq;
    } catch {
      tasks = [];
    }
  }

  function persistTasks() {
    const history = historyTasks.slice(-MAX_PERSISTED_HISTORY);
    const active = activeTasks;
    const state: PersistedTransferState = { version: 1, tasks: [...active, ...history].map(serializableTask) };
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  }

  function whenCanceled(id: string): Promise<'canceled'> {
    return new Promise((resolve) => {
      const check = () => {
        const t = currentTask(id);
        if (!t || t.status === 'canceled') resolve('canceled');
        else setTimeout(check, 300);
      };
      check();
    });
  }

  async function processQueue() {
    if (processing) return;
    processing = true;
    try {
      while (true) {
        const task = tasks.find((candidate) => candidate.status === 'queued');
        if (!task) return;
        updateTask(task.id, { status: 'running', startedAt: Date.now(), message: undefined });
        try {
          const result = await Promise.race([
            runTask(task.id).then(() => 'done' as const, () => 'error' as const),
            whenCanceled(task.id),
          ]);
          if (result === 'canceled') continue;
          if (isCanceled(task.id)) continue;
          if (result === 'error') continue;
          const latest = currentTask(task.id);
          updateTask(task.id, {
            status: 'done',
            transferred: latest?.size ?? task.size,
            message: undefined,
            finishedAt: Date.now(),
          });
          refreshToken += 1;
          if (leftId === LOCAL_ENDPOINT_ID) void refreshLocalSide('left');
          if (rightId === LOCAL_ENDPOINT_ID) void refreshLocalSide('right');
        } catch (e) {
          if (isCanceled(task.id)) continue;
          updateTask(task.id, { status: 'error', message: (e as Error).message, finishedAt: Date.now() });
          onError(i18n.t('sftp.crossTransferFailed', { message: (e as Error).message }));
        }
      }
    } finally {
      processing = false;
    }
  }

  async function runTask(id: string) {
    const task = currentTask(id);
    if (!task || task.status === 'canceled') return;
    if (task.direction === 'remote-remote') {
      await runRemoteToRemoteTask(id, task);
      return;
    }
    if (task.direction === 'local-remote') {
      await runLocalToRemoteTask(id, task);
      return;
    }
    await runRemoteToLocalTask(id, task);
  }

  async function tryDirectRemoteTransfer(id: string, task: TransferTask): Promise<boolean> {
    if (isCanceled(id)) return false;
    if (!task.sourceSessionId || !task.destSessionId) return false;
    updateTask(id, { method: 'direct', message: i18n.t('transfer.directRunning') });
    try {
      await rpc.call('sftp.directTransfer', {
        source_session_id: task.sourceSessionId,
        dest_session_id: task.destSessionId,
        source_path: task.sourcePath,
        kind: task.sourceKind,
        dest_path: task.destPath,
      });
      await verifyDirectTransferTarget(task);
      updateTask(id, {
        method: 'direct',
        transferred: Math.max(task.size, task.sourceKind === 'File' ? task.sourceSize : task.size),
        message: i18n.t('transfer.directDone'),
      });
      return true;
    } catch (e) {
      updateTask(id, { message: `${i18n.t('transfer.directFallback', { message: (e as Error).message })}` });
      return false;
    }
  }

  async function verifyDirectTransferTarget(task: TransferTask) {
    if (!task.destSessionId) return;
    const entry = await rpc.call<SftpEntry>('sftp.stat', { id: task.destSessionId, path: task.destPath });
    if (task.sourceKind === 'Dir') {
      if (entry.kind !== 'Dir') throw new Error(i18n.t('transfer.directVerifyFailed'));
      return;
    }
    if (entry.kind !== 'File' || entry.size < task.sourceSize) {
      throw new Error(i18n.t('transfer.directVerifyFailed'));
    }
  }

  async function tryBackendRelayTransfer(id: string, task: TransferTask): Promise<boolean> {
    if (isCanceled(id)) return false;
    if (!task.sourceSessionId || !task.destSessionId) return false;
    updateTask(id, { method: 'relay', message: i18n.t('transfer.relayRunning') });

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const w = window as any;
    const eventListen: any = w.__TAURI__?.event?.listen
      ?? w.__TAURI_INTERNALS__?.event?.listen;
    let unlistenProgress: (() => void) | null = null;
    let unlistenFile: (() => void) | null = null;

    if (eventListen) {
      unlistenProgress = await eventListen('transfer:relay-progress', (event: { payload: { transfer_id: string; path: string; offset: number; total: number } }) => {
        const p = event.payload;
        if (p.transfer_id === id) {
          updateTask(id, { transferred: p.offset, size: Math.max(task.size, p.total), message: `${p.path}: ${formatSize(p.offset)} / ${formatSize(p.total)}` });
        }
      });
      unlistenFile = await eventListen('transfer:relay-file', (event: { payload: { transfer_id: string; path: string; total: number } }) => {
        const p = event.payload;
        if (p.transfer_id === id) {
          updateTask(id, { transferred: 0, size: Math.max(task.size, p.total), message: `${i18n.t('transfer.relayRunning')} — ${p.path}` });
        }
      });
    }

    try {
      await rpc.call('sftp.relayTransfer', {
        transfer_id: id,
        source_session_id: task.sourceSessionId,
        dest_session_id: task.destSessionId,
        source_path: task.sourcePath,
        source_kind: task.sourceKind,
        dest_path: task.destPath,
      });
    } catch (e) {
      updateTask(id, { message: `Relay error: ${(e as Error).message}` });
      return false;
    } finally {
      unlistenProgress?.();
      unlistenFile?.();
    }

    updateTask(id, {
      method: 'relay',
      transferred: Math.max(task.size, task.sourceKind === 'File' ? task.sourceSize : task.size),
      message: i18n.t('transfer.relayDone'),
    });
    return true;
  }

  async function runRemoteToRemoteTask(id: string, task: TransferTask) {
    if (task.mode !== 'relay') {
      const directDone = await tryDirectRemoteTransfer(id, task);
      if (directDone) return;
      if (isCanceled(id)) return;
      if (task.mode === 'direct') {
        const latest = currentTask(id);
        throw new Error(latest?.message || i18n.t('transfer.directUnavailable'));
      }
    }
    const backendRelayDone = await tryBackendRelayTransfer(id, task);
    if (backendRelayDone) return;
    if (isCanceled(id)) return;
    if (!task.sourceSessionId || !task.destSessionId) throw new Error('remote session is not available');
    updateTask(id, { method: 'relay', message: undefined });
    const dirCache = new Set<string>();
    if (task.sourceKind === 'File') {
      await ensureRemoteDirOn(task.destSessionId, parentRemotePath(task.destPath), dirCache);
      await copyRemoteFile(id, task.sourceSessionId, task.sourcePath, task.destSessionId, task.destPath, task.sourceSize, 0);
      return;
    }
    updateTask(id, { message: i18n.t('transfer.planning') });
    const dirs: string[] = [];
    const files = await planRemoteDir(task.sourceSessionId, task.sourcePath, task.destPath, dirs);
    const size = files.reduce((sum, file) => sum + file.size, 0);
    updateTask(id, { files, dirs, size, transferred: 0, message: undefined });
    await ensureRemoteDirOn(task.destSessionId, task.destPath, dirCache);
    for (const dir of dirs) await ensureRemoteDirOn(task.destSessionId, dir, dirCache);
    let transferred = 0;
    for (const file of files) {
      await waitWhilePaused(id);
      if (isCanceled(id)) return;
      await ensureRemoteDirOn(task.destSessionId, parentRemotePath(file.destPath), dirCache);
      await copyRemoteFile(id, task.sourceSessionId, file.sourcePath, task.destSessionId, file.destPath, file.size, transferred);
      transferred += file.size;
      updateTask(id, { transferred });
    }
  }

  async function runLocalToRemoteTask(id: string, task: TransferTask) {
    if (isCanceled(id)) return;
    if (!task.destSessionId) throw new Error('remote destination is not available');
    const dirCache = new Set<string>();
    if (task.sourceKind === 'File') {
      await ensureRemoteDirOn(task.destSessionId, parentRemotePath(task.destPath), dirCache);
      await copyLocalFileToRemote(id, task.sourcePath, task.destSessionId, task.destPath, task.sourceSize, 0);
      return;
    }
    updateTask(id, { message: i18n.t('transfer.planning') });
    const dirs: string[] = [];
    const files = await planLocalDir(task.sourcePath, task.destPath, dirs);
    const size = files.reduce((sum, file) => sum + file.size, 0);
    updateTask(id, { files, dirs, size, transferred: 0, message: undefined });
    await ensureRemoteDirOn(task.destSessionId, task.destPath, dirCache);
    for (const dir of dirs) await ensureRemoteDirOn(task.destSessionId, dir, dirCache);
    let transferred = 0;
    for (const file of files) {
      await waitWhilePaused(id);
      if (isCanceled(id)) return;
      await ensureRemoteDirOn(task.destSessionId, parentRemotePath(file.destPath), dirCache);
      await copyLocalFileToRemote(id, file.sourcePath, task.destSessionId, file.destPath, file.size, transferred);
      transferred += file.size;
      updateTask(id, { transferred });
    }
  }

  async function runRemoteToLocalTask(id: string, task: TransferTask) {
    if (isCanceled(id)) return;
    if (!task.sourceSessionId) throw new Error('remote source is not available');
    if (task.sourceKind === 'File') {
      await copyRemoteFileToLocal(id, task.sourceSessionId, task.sourcePath, task.destPath, task.sourceSize, 0);
      return;
    }
    updateTask(id, { message: i18n.t('transfer.planning') });
    const dirs: string[] = [];
    const files = await planRemoteDir(task.sourceSessionId, task.sourcePath, task.destPath, dirs);
    const size = files.reduce((sum, file) => sum + file.size, 0);
    updateTask(id, { files, dirs, size, transferred: 0, message: undefined });
    await ensureLocalDir(task.destPath);
    for (const dir of dirs) await ensureLocalDir(dir);
    let transferred = 0;
    for (const file of files) {
      await waitWhilePaused(id);
      if (isCanceled(id)) return;
      await copyRemoteFileToLocal(id, task.sourceSessionId, file.sourcePath, file.destPath, file.size, transferred);
      transferred += file.size;
      updateTask(id, { transferred });
    }
  }

  async function planRemoteDir(srcSid: string, srcPath: string, destPath: string, dirs: string[] = []): Promise<PlannedFile[]> {
    const out: PlannedFile[] = [];
    const list = sortEntries(await rpc.call<SftpEntry[]>('sftp.list', { id: srcSid, path: srcPath }));
    for (const entry of list) {
      const childSrc = joinRemotePath(srcPath, entry.name);
      const childDest = taskDestJoin(destPath, entry.name);
      if (entry.kind === 'Dir') {
        dirs.push(childDest);
        out.push(...await planRemoteDir(srcSid, childSrc, childDest, dirs));
      } else if (entry.kind === 'File') {
        out.push({ sourcePath: childSrc, destPath: childDest, size: entry.size });
      }
    }
    return out;
  }

  async function planLocalDir(srcPath: string, destPath: string, dirs: string[] = []): Promise<PlannedFile[]> {
    const out: PlannedFile[] = [];
    const list = await tauriInvoke<LocalEntry[]>('local_list_dir', { path: srcPath });
    if (!list) throw new Error('local file browser is not available');
    for (const entry of list) {
      const childSrc = joinLocalPath(srcPath, entry.name);
      const childDest = joinRemotePath(destPath, entry.name);
      if (entry.kind === 'dir') {
        dirs.push(childDest);
        out.push(...await planLocalDir(childSrc, childDest, dirs));
      } else if (entry.kind === 'file') {
        out.push({ sourcePath: childSrc, destPath: childDest, size: entry.size });
      }
    }
    return out;
  }

  async function copyRemoteFile(
    taskId: string,
    srcSid: string,
    srcPath: string,
    destSid: string,
    destPath: string,
    size: number,
    baseTransferred: number,
  ) {
    if (size === 0) {
      await rpc.call('sftp.writeChunk', { id: destSid, path: destPath, offset: 0, data: '', create: true });
      updateTask(taskId, { transferred: baseTransferred });
      return;
    }
    let offset = 0;
    while (offset < size) {
      await waitWhilePaused(taskId);
      if (isCanceled(taskId)) return;
      const len = Math.min(CHUNK_SIZE, size - offset);
      const r = await rpc.call<{ data: string }>('sftp.readChunk', { id: srcSid, path: srcPath, offset, len });
      const bytes = b64decode(r.data);
      if (bytes.byteLength === 0) break;
      await rpc.call('sftp.writeChunk', {
        id: destSid,
        path: destPath,
        offset,
        data: b64encode(bytes),
        create: offset === 0,
      });
      offset += bytes.byteLength;
      updateTask(taskId, { transferred: baseTransferred + offset });
    }
  }

  async function copyLocalFileToRemote(
    taskId: string,
    srcPath: string,
    destSid: string,
    destPath: string,
    size: number,
    baseTransferred: number,
  ) {
    if (size === 0) {
      await rpc.call('sftp.writeChunk', { id: destSid, path: destPath, offset: 0, data: '', create: true });
      updateTask(taskId, { transferred: baseTransferred });
      return;
    }
    let offset = 0;
    while (offset < size) {
      await waitWhilePaused(taskId);
      if (isCanceled(taskId)) return;
      const len = Math.min(CHUNK_SIZE, size - offset);
      const r = await tauriInvoke<{ data: string }>('local_read_chunk', { path: srcPath, offset, len });
      if (!r) throw new Error('desktop file reader is not available');
      const bytes = b64decode(r.data);
      if (bytes.byteLength === 0) break;
      await rpc.call('sftp.writeChunk', {
        id: destSid,
        path: destPath,
        offset,
        data: b64encode(bytes),
        create: offset === 0,
      });
      offset += bytes.byteLength;
      updateTask(taskId, { transferred: baseTransferred + offset });
    }
  }

  async function copyRemoteFileToLocal(
    taskId: string,
    srcSid: string,
    srcPath: string,
    destPath: string,
    size: number,
    baseTransferred: number,
  ) {
    await ensureLocalDir(parentLocalPath(destPath));
    if (size === 0) {
      await writeLocalChunk(destPath, 0, new Uint8Array(), true);
      updateTask(taskId, { transferred: baseTransferred });
      return;
    }
    let offset = 0;
    while (offset < size) {
      await waitWhilePaused(taskId);
      if (isCanceled(taskId)) return;
      const len = Math.min(CHUNK_SIZE, size - offset);
      const r = await rpc.call<{ data: string }>('sftp.readChunk', { id: srcSid, path: srcPath, offset, len });
      const bytes = b64decode(r.data);
      if (bytes.byteLength === 0) break;
      await writeLocalChunk(destPath, offset, bytes, offset === 0);
      offset += bytes.byteLength;
      updateTask(taskId, { transferred: baseTransferred + offset });
    }
  }

  async function writeLocalChunk(path: string, offset: number, bytes: Uint8Array, create: boolean) {
    const write = tauriInvoke<void>('local_write_chunk', {
      path,
      offset,
      data: b64encode(bytes),
      create,
    });
    if (!write) throw new Error('desktop file writer is not available');
    await write;
  }

  async function ensureLocalDir(path: string) {
    const mkdir = tauriInvoke<void>('local_mkdir', { path });
    if (!mkdir) throw new Error('desktop directory writer is not available');
    await mkdir;
  }

  async function ensureRemoteDirOn(sid: string, path: string, dirCache: Set<string>) {
    const normalized = normalizeRemotePath(path);
    if (normalized === '/' || normalized === '.') return;
    const absolute = normalized.startsWith('/');
    const segments = normalized.split('/').filter(Boolean);
    let cursor = absolute ? '/' : '.';
    for (const segment of segments) {
      cursor = joinRemotePath(cursor, segment);
      if (dirCache.has(cursor)) continue;
      try {
        const entry = await rpc.call<SftpEntry>('sftp.stat', { id: sid, path: cursor });
        if (entry.kind !== 'Dir') throw new Error(`${cursor} exists and is not a directory`);
      } catch (e) {
        try {
          await rpc.call('sftp.mkdir', { id: sid, path: cursor });
        } catch (mkdirError) {
          const entry = await rpc.call<SftpEntry>('sftp.stat', { id: sid, path: cursor }).catch(() => null);
          if (entry?.kind !== 'Dir') throw mkdirError;
        }
      }
      dirCache.add(cursor);
    }
  }

  function normalizeRemotePath(path: string): string {
    const trimmed = path.trim();
    if (!trimmed || trimmed === '.') return '.';
    const absolute = trimmed.startsWith('/');
    const parts: string[] = [];
    for (const part of trimmed.split('/')) {
      if (!part || part === '.') continue;
      if (part === '..') parts.pop();
      else parts.push(part);
    }
    return `${absolute ? '/' : ''}${parts.join('/')}` || (absolute ? '/' : '.');
  }

  function joinRemotePath(base: string, name: string): string {
    const b = base || '.';
    if (b === '/' || b.endsWith('/')) return `${b}${name}`.replace(/\/+/g, '/');
    return `${b}/${name}`.replace(/\/+/g, '/');
  }

  function taskDestJoin(base: string, name: string): string {
    return base.includes('\\') || /^[A-Za-z]:/.test(base) ? joinLocalPath(base, name) : joinRemotePath(base, name);
  }

  function parentRemotePath(path: string): string {
    const normalized = normalizeRemotePath(path);
    if (normalized === '/' || normalized === '.') return normalized;
    const idx = normalized.lastIndexOf('/');
    if (idx <= 0) return normalized.startsWith('/') ? '/' : '.';
    return normalized.slice(0, idx);
  }

  function sortEntries(list: SftpEntry[]): SftpEntry[] {
    return [...list].sort((a, b) => {
      if (a.kind !== b.kind) return a.kind === 'Dir' ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
  }

  function formatSize(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
    return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }

  function percent(task: TransferTask): number {
    if (task.size <= 0) return task.status === 'done' ? 100 : 0;
    return Math.min(100, Math.round((task.transferred / task.size) * 100));
  }

  function directionLabel(task: TransferTask): string {
    if (task.direction === 'local-remote') return i18n.t('transfer.directionUpload');
    if (task.direction === 'remote-local') return i18n.t('transfer.directionDownload');
    return i18n.t('transfer.directionRemote');
  }

  function modeLabel(mode: TransferMode): string {
    if (mode === 'direct') return i18n.t('transfer.modeDirect');
    if (mode === 'relay') return i18n.t('transfer.modeRelay');
    return i18n.t('transfer.modeAuto');
  }

  function taskModeLabel(task: TransferTask): string {
    if (task.method === 'direct') return i18n.t('transfer.modeDirectAgent');
    if (task.method === 'relay') return i18n.t('transfer.modeRelay');
    return modeLabel(task.mode ?? 'auto');
  }

  function summary(task: TransferTask): string {
    if (task.status === 'queued') return i18n.t('sftp.transferQueued');
    if (task.status === 'paused') return task.message ?? i18n.t('sftp.paused');
    if (task.status === 'running') return task.message ?? `${formatSize(task.transferred)} / ${formatSize(task.size)}`;
    if (task.status === 'done') return i18n.t('sftp.transferDone');
    if (task.status === 'canceled') return i18n.t('sftp.transferCanceled');
    return task.message ?? i18n.t('sftp.transferFailed');
  }

  function statusIcon(task: TransferTask) {
    if (task.status === 'queued') return Clock3;
    if (task.status === 'paused') return Clock3;
    if (task.status === 'running') return Loader2;
    if (task.status === 'done') return CheckCircle2;
    return CircleX;
  }

  function statusClass(task: TransferTask): string {
    if (task.status === 'running') return 'text-[var(--color-accent)] animate-spin';
    if (task.status === 'paused') return 'text-[var(--color-warning)]';
    if (task.status === 'done') return 'text-[var(--color-success)]';
    if (task.status === 'error' || task.status === 'canceled') return 'text-[var(--color-danger)]';
    return 'text-[var(--color-fg-muted)]';
  }

  onMount(() => {
    restorePersistedTasks();
    tasksHydrated = true;
    void refreshProfiles();
  });

  $effect(() => {
    if (leftId === LOCAL_ENDPOINT_ID) void initLocalSide('left');
    if (rightId === LOCAL_ENDPOINT_ID) void initLocalSide('right');
  });

  $effect(() => {
    const key = `${initialProfileId ?? ''}|${initialTarget?.name ?? ''}`;
    if (key === lastInitialKey || profiles.length === 0) return;
    lastInitialKey = key;
    pickInitialIds(profiles);
  });

  $effect(() => {
    if (!tasksHydrated) return;
    void tasks;
    persistTasks();
  });
</script>

<div
  class={embedded
    ? 'h-full w-full bg-[var(--color-bg)] text-[var(--color-fg)] flex flex-col overflow-hidden'
    : standalone
      ? 'h-screen w-screen bg-[var(--color-bg)] text-[var(--color-fg)] flex flex-col overflow-hidden'
      : 'fixed inset-0 z-50 bg-black/60 grid place-items-center p-5'}
  role={embedded ? 'region' : 'dialog'}
  aria-modal={!embedded && !standalone}
  aria-label={i18n.t('transfer.windowTitle')}
  data-aerotab-modal={!embedded && !standalone ? '' : undefined}
>
  <div class={embedded || standalone ? 'w-full h-full flex flex-col overflow-hidden bg-[var(--color-bg)]' : 'panel w-full max-w-[min(1500px,98vw)] h-full max-h-[900px] flex flex-col overflow-hidden'}>
    <header class="flex items-center gap-2 px-4 py-2.5 border-b border-[var(--color-border-soft)] bg-[var(--color-panel)]">
      <div class="font-semibold text-[13px] text-[var(--color-accent)]">{i18n.t('transfer.windowTitle')}</div>
      <div class="text-[11px] text-[var(--color-fg-muted)] truncate">{i18n.t('transfer.windowHint')}</div>
      <div class="ml-auto flex items-center gap-2 text-[11px] text-[var(--color-fg-muted)]">
        {#if aggregate.count > 0}
          <span>{i18n.t('transfer.aggregate', { count: aggregate.count, percent: aggregate.percent })}</span>
        {/if}
        <button type="button" class="btn-ghost p-1" onclick={() => { void refreshProfiles(); }} title={i18n.t('common.refresh')} aria-label={i18n.t('common.refresh')}>
          <RefreshCw size={14} class={loadingProfiles ? 'animate-spin' : ''} />
        </button>
        <button type="button" class="btn-ghost p-1" onclick={() => { void closeWindow(); }} aria-label={i18n.t('common.close')}>
          <X size={14} />
        </button>
      </div>
    </header>

    <div class="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)_minmax(132px,180px)] gap-2 items-end px-4 py-2 border-b border-[var(--color-border-soft)] bg-[var(--color-panel)]">
      <label class="min-w-0">
        <span class="block text-[10.5px] text-[var(--color-fg-muted)] mb-1">{i18n.t('transfer.leftServer')}</span>
        <select class="input py-1 text-[12px]" bind:value={leftId}>
          <option value={LOCAL_ENDPOINT_ID}>{i18n.t('transfer.localComputer')}</option>
          {#if initialTarget}
            <option value={INITIAL_ENDPOINT_ID}>{initialTarget.name}</option>
          {/if}
          {#each sshProfiles as profile (profile.id)}
            <option value={profile.id}>{profile.name}</option>
          {/each}
        </select>
      </label>
      <button type="button" class="btn-secondary px-2 py-1 mb-[1px]" onclick={swapTargets} title={i18n.t('transfer.swap')} aria-label={i18n.t('transfer.swap')}>
        <ArrowLeftRight size={14} />
      </button>
      <label class="min-w-0">
        <span class="block text-[10.5px] text-[var(--color-fg-muted)] mb-1">{i18n.t('transfer.rightServer')}</span>
        <select class="input py-1 text-[12px]" bind:value={rightId}>
          <option value="">{i18n.t('transfer.pickServer')}</option>
          <option value={LOCAL_ENDPOINT_ID}>{i18n.t('transfer.localComputer')}</option>
          {#each sshProfiles as profile (profile.id)}
            <option value={profile.id}>{profile.name}</option>
          {/each}
        </select>
      </label>
      <label class="min-w-0">
        <span class="block text-[10.5px] text-[var(--color-fg-muted)] mb-1">{i18n.t('transfer.mode')}</span>
        <select class="input py-1 text-[12px]" bind:value={transferMode}>
          <option value="auto">{i18n.t('transfer.modeAuto')}</option>
          <option value="direct">{i18n.t('transfer.modeDirect')}</option>
          <option value="relay">{i18n.t('transfer.modeRelay')}</option>
        </select>
      </label>
    </div>

    <div class="flex-1 min-h-0 grid grid-cols-2 divide-x divide-[var(--color-border-soft)]">
      <div class="min-w-0 min-h-0">
        {#if leftId === LOCAL_ENDPOINT_ID}
          <SftpLocalPane
            cwd={leftLocalCwd}
            entries={leftLocalEntries}
            loading={leftLocalLoading}
            listError={leftLocalListError}
            onRefresh={() => { void refreshLocalSide('left'); }}
            onNavigate={(path) => { void navigateLocalSide('left', path); }}
            onGoUp={() => { void localSideGoUp('left'); }}
            onGoHome={() => { void localSideGoHome('left'); }}
            onDropRemote={(e) => { void handleLocalPaneDrop('left', e); }}
            onDropFiles={ignoreFileDrop}
            onDragOverPane={preventDragDefaults}
          />
        {:else if effectiveLeftTarget}
          <SftpBrowser
            {rpc}
            registryId={`${windowId}-left`}
            source={effectiveLeftTarget}
            mode="dock"
            showLocalPane={false}
            {refreshToken}
            onRemoteCrossTransfer={enqueueRemoteTransfer}
            onLocalUploadTransfer={enqueueLocalUpload}
            onClose={() => { leftId = ''; }}
            {onError}
          />
        {:else}
          <div class="h-full grid place-items-center text-[12px] text-[var(--color-fg-muted)]">{i18n.t('transfer.pickServer')}</div>
        {/if}
      </div>
      <div class="min-w-0 min-h-0">
        {#if rightId === LOCAL_ENDPOINT_ID}
          <SftpLocalPane
            cwd={rightLocalCwd}
            entries={rightLocalEntries}
            loading={rightLocalLoading}
            listError={rightLocalListError}
            onRefresh={() => { void refreshLocalSide('right'); }}
            onNavigate={(path) => { void navigateLocalSide('right', path); }}
            onGoUp={() => { void localSideGoUp('right'); }}
            onGoHome={() => { void localSideGoHome('right'); }}
            onDropRemote={(e) => { void handleLocalPaneDrop('right', e); }}
            onDropFiles={ignoreFileDrop}
            onDragOverPane={preventDragDefaults}
          />
        {:else if rightTarget}
          <SftpBrowser
            {rpc}
            registryId={`${windowId}-right`}
            source={rightTarget}
            mode="dock"
            showLocalPane={false}
            {refreshToken}
            onRemoteCrossTransfer={enqueueRemoteTransfer}
            onLocalUploadTransfer={enqueueLocalUpload}
            onClose={() => { rightId = ''; }}
            {onError}
          />
        {:else}
          <div class="h-full grid place-items-center text-[12px] text-[var(--color-fg-muted)]">{i18n.t('transfer.pickSecondServer')}</div>
        {/if}
      </div>
    </div>

    <section class="border-t border-[var(--color-border-soft)] bg-[var(--color-panel)] min-h-[220px] max-h-[260px] flex flex-col">
      <div class="flex flex-wrap items-center gap-2 px-3 py-1.5 border-b border-[var(--color-border-soft)] text-[11px]">
        <ListChecks size={13} class="text-[var(--color-accent)]" />
        <span class="uppercase tracking-[0.12em] text-[var(--color-fg-muted)]">{i18n.t('transfer.queue')}</span>
        <span class="text-[var(--color-fg-muted)]">{activeTasks.length}</span>
        {#if aggregate.count > 0}
          <span class="text-[var(--color-fg-muted)]">{formatSize(aggregate.done)} / {formatSize(aggregate.total)} · {aggregate.percent}%</span>
        {/if}
        <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={cancelActiveTasks}>{i18n.t('sftp.cancelAll')}</button>
        <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={retryFailedTasks}>{i18n.t('transfer.retryFailed')}</button>
        {#if selectedTaskIds.size > 0}
          <button type="button" class="text-[var(--color-danger)] hover:underline" onclick={removeSelectedTasks}>{i18n.t('sftp.transferRemoveSelected')}</button>
        {/if}
        <button type="button" class="ml-auto text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={clearHistory}>{i18n.t('transfer.clearHistory')}</button>
        <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={clearAllTasks}>{i18n.t('sftp.transferClearAll')}</button>
      </div>
      <div class="flex-1 min-h-0 overflow-y-auto divide-y divide-[var(--color-border-soft)]">
        {#if tasks.length === 0}
          <div class="h-full grid place-items-center text-[12px] text-[var(--color-fg-muted)]">{i18n.t('transfer.queueEmpty')}</div>
        {:else}
          {#each activeTasks as task (task.id)}
            {@const Icon = statusIcon(task)}
            {@const pct = percent(task)}
            <div class="px-3 py-2 text-[11.5px] {selectedTaskIds.has(task.id) ? 'bg-[var(--color-panel-2)]' : ''}">
              <div class="flex items-center gap-2 min-w-0">
                <input type="checkbox" class="shrink-0" checked={selectedTaskIds.has(task.id)} onchange={() => toggleTaskSelection(task.id)} aria-label={task.name} />
                <Icon size={13} class={`shrink-0 ${statusClass(task)}`} />
                <div class="min-w-0 flex-1">
                  <div class="flex items-center gap-2 min-w-0">
                    <span class="uppercase text-[9.5px] text-[var(--color-fg-muted)] shrink-0">{directionLabel(task)}</span>
                    <span class="uppercase text-[9.5px] text-[var(--color-accent)] shrink-0">{taskModeLabel(task)}</span>
                    <span class="truncate text-[var(--color-fg)]">{task.name}</span>
                    <span class="text-[10.5px] text-[var(--color-fg-muted)] truncate">{task.sourceLabel} → {task.destLabel}</span>
                    <span class="ml-auto text-[10.5px] text-[var(--color-fg-muted)] shrink-0">{pct}%</span>
                  </div>
                  <div class="mt-1 h-1 rounded bg-[var(--color-panel-2)] overflow-hidden">
                    <div class="h-full bg-[var(--color-accent)]" style="width: {pct}%"></div>
                  </div>
                  <div class="mt-1 truncate text-[10.5px] text-[var(--color-fg-muted)]">{summary(task)}</div>
                </div>
                {#if task.status === 'paused'}
                  <button type="button" class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-accent)]" title={i18n.t('sftp.resumeTransfer')} aria-label={i18n.t('sftp.resumeTransfer')} onclick={() => resumeTask(task.id)}><RefreshCw size={12} /></button>
                {:else}
                  <button type="button" class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-warning)]" title={i18n.t('sftp.pauseTransfer')} aria-label={i18n.t('sftp.pauseTransfer')} onclick={() => pauseTask(task.id)}><Pause size={12} /></button>
                {/if}
                <button type="button" class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-danger)]" title={i18n.t('sftp.cancelTransfer')} aria-label={i18n.t('sftp.cancelTransfer')} onclick={() => cancelTask(task.id)}><X size={12} /></button>
              </div>
            </div>
          {/each}
          {#if historyTasks.length > 0}
            <div class="sticky top-0 z-10 flex items-center gap-2 px-3 py-1 bg-[var(--color-panel)] text-[11px] text-[var(--color-fg-muted)]">
              <History size={12} />
              <span class="uppercase tracking-[0.12em]">{i18n.t('transfer.history')}</span>
              <span>{historyTasks.length}</span>
            </div>
            {#each historyTasks as task (task.id)}
              {@const Icon = statusIcon(task)}
              {@const pct = percent(task)}
              <div class="px-3 py-2 text-[11.5px] opacity-90 {selectedTaskIds.has(task.id) ? 'bg-[var(--color-panel-2)]' : ''}">
                <div class="flex items-center gap-2 min-w-0">
                  <input type="checkbox" class="shrink-0" checked={selectedTaskIds.has(task.id)} onchange={() => toggleTaskSelection(task.id)} aria-label={task.name} />
                  <Icon size={13} class={`shrink-0 ${statusClass(task)}`} />
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2 min-w-0">
                      <span class="uppercase text-[9.5px] text-[var(--color-fg-muted)] shrink-0">{task.status}</span>
                      <span class="truncate text-[var(--color-fg)]">{task.name}</span>
                      <span class="text-[10.5px] text-[var(--color-fg-muted)] truncate">{task.sourceLabel} → {task.destLabel}</span>
                      <span class="ml-auto text-[10.5px] text-[var(--color-fg-muted)] shrink-0">{pct}%</span>
                    </div>
                    <div class="mt-1 truncate text-[10.5px] text-[var(--color-fg-muted)]">{summary(task)} · {i18n.t('transfer.attempts', { count: task.attempts })}</div>
                  </div>
                  {#if task.status === 'error' || task.status === 'canceled'}
                    <button type="button" class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-accent)]" title={i18n.t('transfer.retry')} aria-label={i18n.t('transfer.retry')} onclick={() => retryTask(task.id)}><RotateCw size={12} /></button>
                  {/if}
                  <button type="button" class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-danger)]" title={i18n.t('common.delete')} aria-label={i18n.t('common.delete')} onclick={() => { selectedTaskIds = new Set([task.id]); removeSelectedTasks(); }}><Trash2 size={12} /></button>
                </div>
              </div>
            {/each}
          {/if}
        {/if}
      </div>
    </section>
  </div>
</div>
