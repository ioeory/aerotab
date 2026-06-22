<script lang="ts">
  import { X, ArrowLeftRight, RefreshCw, Pause, ListChecks } from '@lucide/svelte';
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
  import { onDestroy, onMount } from 'svelte';
  import { tabs, type TransferBootstrap } from '../lib/tabs.svelte';
  import { transferTabBridge } from '../lib/transferTabBridge.svelte';
  import EndpointSelector from './EndpointSelector.svelte';
  import SftpTransferQueue, { type TransferQueueItem } from './SftpTransferQueue.svelte';

  interface TransferTarget {
    name: string;
    ssh: SshProfileSpec;
  }

  interface Props {
    rpc: RpcClient;
    tabId?: string;
    transferBootstrap?: TransferBootstrap;
    onOpenSftpDock?: () => void;
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
    lastProgressAt?: number;
    files?: PlannedFile[];
    dirs?: string[];
  }

  interface PersistedTransferState {
    version: 1;
    tasks: TransferTask[];
  }

  let {
    rpc,
    tabId = '',
    transferBootstrap,
    onOpenSftpDock,
    onError,
  }: Props = $props();

  const LOCAL_ENDPOINT_ID = '__local__';
  const STORAGE_KEY = $derived(`aerotab.fileTransfer.tasks.v1.${tabId}`);
  const MAX_PERSISTED_HISTORY = 200;
  const CHUNK_SIZE = 256 * 1024;
  const TRANSFER_STALL_MS = 45_000;
  const MAX_AUTO_ATTEMPTS = 3;
  const RETRY_BACKOFF_MS = [1000, 3000];
  const TRANSFER_RPC_TIMEOUT_MS = 45_000;
  const RELAY_CANCEL_GRACE_MS = 3000;

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
  let tasksHydrated = $state(false);
  let progressTick = $state(0);
  let queueView = $state<'active' | 'history'>('active');

  const STALE_PROGRESS_MS = TRANSFER_STALL_MS;

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
  const pausedTasksCount = $derived(tasks.filter((t) => t.status === 'paused').length);
  const aggregate = $derived.by(() => {
    const running = activeTasks;
    const total = running.reduce((sum, t) => sum + Math.max(0, t.size), 0);
    const done = running.reduce((sum, t) => sum + Math.min(Math.max(0, t.transferred), Math.max(0, t.size)), 0);
    const percent = total > 0 ? Math.min(100, Math.round((done / total) * 100)) : 0;
    return { count: running.length, total, done, percent };
  });

  function targetForId(id: string): TransferTarget | null {
    const profile = sshProfiles.find((p) => p.id === id);
    if (!profile || profile.kind !== 'ssh') return null;
    return { name: profile.name, ssh: profile.ssh };
  }

  function isLocalEndpoint(id: string): boolean {
    return id === LOCAL_ENDPOINT_ID;
  }

  function normalizeEndpointSelection(id: string): string {
    if (id === LOCAL_ENDPOINT_ID) return id;
    return sshProfiles.some((p) => p.id === id) ? id : '';
  }

  function pickInitialIds(list: StoredProfile[]) {
    const ssh = list.filter((p) => p.kind === 'ssh');
    if (!leftId || (!isLocalEndpoint(leftId) && !ssh.some((p) => p.id === leftId))) {
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

  function closeWindow() {
    if (tabId) {
      tabs.remove(tabId);
    }
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
    tasks = tasks.map((task) => {
      if (task.id !== id) return task;
      const next = { ...task, ...patch };
      if (patch.transferred !== undefined && patch.transferred !== task.transferred) {
        next.lastProgressAt = Date.now();
      }
      return next;
    });
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

  async function withRelayStallCancel<T>(id: string, promise: Promise<T>): Promise<T> {
    let stalled = false;
    let stopMonitor = false;
    const monitor = (async () => {
      while (!stopMonitor && !stalled) {
        const task = currentTask(id);
        if (!task || task.status !== 'running' || isCanceled(id)) return;
        if (isPaused(id)) {
          await sleep(500);
          continue;
        }
        const last = task.lastProgressAt ?? task.startedAt ?? task.createdAt;
        const remaining = TRANSFER_STALL_MS - (Date.now() - last);
        if (remaining <= 0) {
          stalled = true;
          updateTask(id, { message: i18n.t('transfer.cancelingStalled') });
          await rpc.call('sftp.cancelRelayTransfer', { transfer_id: id }).catch(() => {});
          await sleep(RELAY_CANCEL_GRACE_MS);
          throw new Error(i18n.t('transfer.timeout', { seconds: Math.round(TRANSFER_STALL_MS / 1000) }));
        }
        await sleep(Math.min(remaining, 500));
      }
    })();
    try {
      const value = await Promise.race([promise, monitor.then(() => new Promise<T>(() => {}))]);
      if (stalled) throw new Error(i18n.t('transfer.timeout', { seconds: Math.round(TRANSFER_STALL_MS / 1000) }));
      return value;
    } catch (error) {
      if (stalled) throw new Error(i18n.t('transfer.timeout', { seconds: Math.round(TRANSFER_STALL_MS / 1000) }));
      throw error;
    } finally {
      stopMonitor = true;
      await monitor.catch(() => {});
    }
  }

  async function retryOrFailTask(task: TransferTask, err: unknown) {
    const message = (err as Error).message;
    if (task.attempts < MAX_AUTO_ATTEMPTS && !isCanceled(task.id)) {
      const nextAttempt = task.attempts + 1;
      updateTask(task.id, {
        status: 'queued',
        transferred: 0,
        size: task.sourceKind === 'File' ? task.sourceSize : 0,
        message: i18n.t('transfer.retrying', { attempt: nextAttempt, max: MAX_AUTO_ATTEMPTS }),
        files: undefined,
        dirs: undefined,
        method: undefined,
        attempts: nextAttempt,
        startedAt: undefined,
        lastProgressAt: undefined,
        finishedAt: undefined,
      });
      await sleep(RETRY_BACKOFF_MS[Math.min(nextAttempt - 2, RETRY_BACKOFF_MS.length - 1)] ?? 1000);
      return;
    }
    updateTask(task.id, {
      status: 'error',
      message: i18n.t('transfer.finalFailed', { attempts: task.attempts, message }),
      finishedAt: Date.now(),
    });
    onError(i18n.t('sftp.crossTransferFailed', { message }));
  }

  function pauseTask(id: string) {
    const status = currentTask(id)?.status;
    if (status === 'queued' || status === 'running') {
      updateTask(id, { status: 'paused', message: i18n.t('sftp.paused') });
    }
  }

  function resumeTask(id: string) {
    const task = currentTask(id);
    if (task?.status !== 'paused') return;
    if (task.startedAt) {
      updateTask(id, { status: 'running', message: undefined, lastProgressAt: Date.now() });
    } else {
      updateTask(id, { status: 'queued', message: undefined, lastProgressAt: undefined });
      void processQueue();
    }
  }

  function cancelTask(id: string) {
    const status = currentTask(id)?.status;
    if (!status || status === 'done' || status === 'error' || status === 'canceled') return;
    void rpc.call('sftp.cancelRelayTransfer', { transfer_id: id }).catch(() => {});
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

  function clearFinishedTasks() {
    tasks = tasks.filter((task) => task.status !== 'done' && task.status !== 'canceled');
    selectedTaskIds = new Set([...selectedTaskIds].filter((id) => tasks.some((task) => task.id === id)));
  }

  function resumeAllPaused() {
    for (const task of tasks) {
      if (task.status === 'paused') resumeTask(task.id);
    }
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
        updateTask(task.id, { status: 'running', startedAt: Date.now(), lastProgressAt: Date.now(), message: undefined });
        try {
          const result = await Promise.race([
            runTask(task.id).then(() => 'done' as const),
            whenCanceled(task.id),
          ]);
          if (result === 'canceled') continue;
          if (isCanceled(task.id)) continue;
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
          await retryOrFailTask(currentTask(task.id) ?? task, e);
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
      await withTransferTimeout('sftp.directTransfer', rpc.call('sftp.directTransfer', {
        source_session_id: task.sourceSessionId,
        dest_session_id: task.destSessionId,
        source_path: task.sourcePath,
        kind: task.sourceKind,
        dest_path: task.destPath,
        timeout_ms: TRANSFER_RPC_TIMEOUT_MS,
      }), TRANSFER_RPC_TIMEOUT_MS + 5000);
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
    const entry = await withTransferTimeout('sftp.stat', rpc.call<SftpEntry>('sftp.stat', { id: task.destSessionId, path: task.destPath }));
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
      await withRelayStallCancel(id, rpc.call('sftp.relayTransfer', {
        transfer_id: id,
        source_session_id: task.sourceSessionId,
        dest_session_id: task.destSessionId,
        source_path: task.sourcePath,
        source_kind: task.sourceKind,
        dest_path: task.destPath,
      }));
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
    const list = sortEntries(await withTransferTimeout('sftp.list', rpc.call<SftpEntry[]>('sftp.list', { id: srcSid, path: srcPath })));
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
    const list = await withTransferTimeout('local_list_dir', tauriInvoke<LocalEntry[]>('local_list_dir', { path: srcPath }) ?? Promise.reject(new Error('local file browser is not available')));
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
      await withTransferTimeout('sftp.writeChunk', rpc.call('sftp.writeChunk', { id: destSid, path: destPath, offset: 0, data: '', create: true }));
      updateTask(taskId, { transferred: baseTransferred });
      return;
    }
    let offset = 0;
    while (offset < size) {
      await waitWhilePaused(taskId);
      if (isCanceled(taskId)) return;
      const len = Math.min(CHUNK_SIZE, size - offset);
      const r = await withTransferTimeout('sftp.readChunk', rpc.call<{ data: string }>('sftp.readChunk', { id: srcSid, path: srcPath, offset, len }));
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
      await withTransferTimeout('sftp.writeChunk', rpc.call('sftp.writeChunk', { id: destSid, path: destPath, offset: 0, data: '', create: true }));
      updateTask(taskId, { transferred: baseTransferred });
      return;
    }
    let offset = 0;
    while (offset < size) {
      await waitWhilePaused(taskId);
      if (isCanceled(taskId)) return;
      const len = Math.min(CHUNK_SIZE, size - offset);
      const r = await withTransferTimeout('local_read_chunk', tauriInvoke<{ data: string }>('local_read_chunk', { path: srcPath, offset, len }) ?? Promise.reject(new Error('desktop file reader is not available')));
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
      const r = await withTransferTimeout('sftp.readChunk', rpc.call<{ data: string }>('sftp.readChunk', { id: srcSid, path: srcPath, offset, len }));
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
    await withTransferTimeout('local_write_chunk', write);
  }

  async function ensureLocalDir(path: string) {
    const mkdir = tauriInvoke<void>('local_mkdir', { path });
    if (!mkdir) throw new Error('desktop directory writer is not available');
    await withTransferTimeout('local_mkdir', mkdir);
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
    void progressTick;
    if (task.status === 'queued') return i18n.t('sftp.transferQueued');
    if (task.status === 'paused') return task.message ?? i18n.t('sftp.paused');
    if (task.status === 'running') {
      const last = task.lastProgressAt ?? task.startedAt ?? task.createdAt;
      const idleMs = Date.now() - last;
      if (idleMs > STALE_PROGRESS_MS) {
        return i18n.t('transfer.waitingServer', { seconds: Math.round(idleMs / 1000) })
          + ` · ${formatSize(task.transferred)} / ${formatSize(task.size)}`;
      }
      return task.message ?? `${formatSize(task.transferred)} / ${formatSize(task.size)}`;
    }
    if (task.status === 'done') return i18n.t('sftp.transferDone');
    if (task.status === 'canceled') return i18n.t('sftp.transferCanceled');
    return task.message ?? i18n.t('sftp.transferFailed');
  }

  function taskToQueueItem(task: TransferTask, view: 'active' | 'history'): TransferQueueItem {
    void progressTick;
    const pct = percent(task);
    const baseSummary = summary(task);
    return {
      id: task.id,
      name: task.name,
      status: task.status,
      percent: pct,
      summary: view === 'history'
        ? `${baseSummary} · ${i18n.t('transfer.attempts', { count: task.attempts })}`
        : baseSummary,
      directionLabel: view === 'active' ? directionLabel(task) : task.status,
      modeLabel: view === 'active' ? taskModeLabel(task) : undefined,
      routeLabel: `${task.sourceLabel} → ${task.destLabel}`,
    };
  }

  const activeQueueItems = $derived(activeTasks.map((task) => taskToQueueItem(task, 'active')));
  const historyQueueItems = $derived(historyTasks.map((task) => taskToQueueItem(task, 'history')));
  const visibleQueueItems = $derived(queueView === 'active' ? activeQueueItems : historyQueueItems);

  onMount(() => {
    restorePersistedTasks();
    tasksHydrated = true;
    void refreshProfiles();
    if (tabId) {
      transferTabBridge.register(tabId, {
        enqueueRemote: enqueueRemoteTransfer,
        enqueueLocal: enqueueLocalUpload,
      });
    }
    const tick = window.setInterval(() => {
      if (activeTasks.some((t) => t.status === 'running')) progressTick += 1;
    }, 5000);
    return () => window.clearInterval(tick);
  });

  onDestroy(() => {
    if (tabId) transferTabBridge.unregister(tabId);
  });

  $effect(() => {
    const bootstrap = transferBootstrap;
    if (!bootstrap || !tabId) return;
    if (bootstrap.leftId) leftId = bootstrap.leftId;
    const nextRight = bootstrap.rightId ?? bootstrap.rightProfileId;
    if (nextRight) rightId = nextRight;
    tabs.clearTransferBootstrap(tabId);
  });

  $effect(() => {
    if (leftId === LOCAL_ENDPOINT_ID) void initLocalSide('left');
    if (rightId === LOCAL_ENDPOINT_ID) void initLocalSide('right');
  });

  $effect(() => {
    if (profiles.length === 0) return;
    pickInitialIds(profiles);
  });

  $effect(() => {
    if (!tasksHydrated) return;
    void tasks;
    persistTasks();
  });
</script>

<div
  class="transfer-window-shell h-full w-full text-[var(--color-fg)] flex flex-col overflow-hidden"
  role="region"
  aria-label={i18n.t('transfer.windowTitle')}
>
  <div class="transfer-window-surface w-full h-full flex flex-col overflow-hidden">
    <header class="transfer-window-bar flex items-center gap-2 px-4 py-2.5 border-b border-[var(--color-border-soft)]">
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

    <div class="transfer-window-bar grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] gap-2 items-end px-4 py-2 border-b border-[var(--color-border-soft)]">
      <label class="min-w-0">
        <span class="block text-[10.5px] text-[var(--color-fg-muted)] mb-1">{i18n.t('transfer.leftServer')}</span>
        <EndpointSelector
          profiles={sshProfiles}
          value={leftId}
          localLabel={i18n.t('transfer.localComputer')}
          placeholder={i18n.t('transfer.pickServer')}
          onChange={(v) => { leftId = v; }}
        />
      </label>
      <button type="button" class="btn-secondary px-2 py-1 mb-[1px]" onclick={swapTargets} title={i18n.t('transfer.swap')} aria-label={i18n.t('transfer.swap')}>
        <ArrowLeftRight size={14} />
      </button>
      <label class="min-w-0">
        <span class="block text-[10.5px] text-[var(--color-fg-muted)] mb-1">{i18n.t('transfer.rightServer')}</span>
        <EndpointSelector
          profiles={sshProfiles}
          value={rightId}
          localLabel={i18n.t('transfer.localComputer')}
          placeholder={i18n.t('transfer.pickServer')}
          onChange={(v) => { rightId = v; }}
        />
      </label>
    </div>

    <details class="transfer-window-bar px-4 py-1.5 border-b border-[var(--color-border-soft)] text-[12px]">
      <summary class="cursor-pointer select-none text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]">{i18n.t('transfer.advanced')}</summary>
      <label class="mt-2 block max-w-[220px]">
        <span class="block text-[10.5px] text-[var(--color-fg-muted)] mb-1">{i18n.t('transfer.mode')}</span>
        <select class="input py-1 text-[12px]" bind:value={transferMode}>
          <option value="auto">{i18n.t('transfer.modeAuto')}</option>
          <option value="direct">{i18n.t('transfer.modeDirect')}</option>
          <option value="relay">{i18n.t('transfer.modeRelay')}</option>
        </select>
      </label>
    </details>

    <div class="transfer-workspace flex-1 min-h-0 grid grid-cols-2 divide-x divide-[var(--color-border-soft)]">
      <div class="transfer-pane min-w-0 min-h-0">
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
            {onError}
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
      <div class="transfer-pane min-w-0 min-h-0">
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
            {onError}
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

    <section class="transfer-queue border-t border-[var(--color-border-soft)] min-h-[220px] max-h-[260px] flex flex-col">
      <div class="flex flex-wrap items-center gap-2 px-3 py-1.5 border-b border-[var(--color-border-soft)] text-[11px]">
        <ListChecks size={13} class="text-[var(--color-accent)]" />
        <span class="uppercase tracking-[0.12em] text-[var(--color-fg-muted)]">{i18n.t('transfer.queue')}</span>
        <div class="inline-flex rounded border border-[var(--color-border-soft)] overflow-hidden text-[10.5px]">
          <button
            type="button"
            class="px-2 py-0.5 {queueView === 'active' ? 'bg-[var(--color-accent)] text-white' : 'text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]'}"
            aria-pressed={queueView === 'active'}
            onclick={() => { queueView = 'active'; }}
          >
            {i18n.t('transfer.queueActive')} ({activeTasks.length})
          </button>
          <button
            type="button"
            class="px-2 py-0.5 {queueView === 'history' ? 'bg-[var(--color-accent)] text-white' : 'text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]'}"
            aria-pressed={queueView === 'history'}
            onclick={() => { queueView = 'history'; }}
          >
            {i18n.t('transfer.history')} ({historyTasks.length})
          </button>
        </div>
        {#if queueView === 'active' && aggregate.count > 0}
          <span class="text-[var(--color-fg-muted)]">{formatSize(aggregate.done)} / {formatSize(aggregate.total)} · {aggregate.percent}%</span>
        {/if}
        {#if queueView === 'active'}
          <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={cancelActiveTasks}>{i18n.t('sftp.cancelAll')}</button>
          <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={clearFinishedTasks}>{i18n.t('transfer.clearCompleted')}</button>
          <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={retryFailedTasks}>{i18n.t('transfer.retryFailed')}</button>
          {#if selectedTaskIds.size > 0}
            <button type="button" class="text-[var(--color-danger)] hover:underline" onclick={removeSelectedTasks}>{i18n.t('sftp.transferRemoveSelected')}</button>
          {/if}
          <button type="button" class="ml-auto text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={clearAllTasks}>{i18n.t('sftp.transferClearAll')}</button>
        {:else}
          <button type="button" class="text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={retryFailedTasks}>{i18n.t('transfer.retryFailed')}</button>
          <button type="button" class="ml-auto text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]" onclick={clearHistory}>{i18n.t('transfer.clearHistory')}</button>
        {/if}
      </div>

      {#if pausedTasksCount > 0}
        <div class="flex items-center gap-2 px-3 py-1.5 bg-[var(--color-warning)]/10 border-b border-[var(--color-border-soft)] text-[11px]">
          <Pause size={12} class="text-[var(--color-warning)] shrink-0" />
          <span class="text-[var(--color-fg-muted)]">{i18n.t('transfer.pausedBanner', { count: pausedTasksCount })}</span>
          <button type="button" class="ml-auto text-[var(--color-accent)] hover:underline" onclick={resumeAllPaused}>{i18n.t('transfer.resumeAll')}</button>
        </div>
      {/if}

      {#if tasks.length === 0}
        <div class="flex-1 min-h-0 grid place-items-center px-4 py-6 text-center">
          <div class="max-w-sm space-y-3">
            <p class="text-[12px] text-[var(--color-fg-muted)]">{i18n.t('transfer.queueEmptyHint')}</p>
            {#if onOpenSftpDock}
              <button type="button" class="btn-secondary text-[12px]" onclick={onOpenSftpDock}>
                {i18n.t('transfer.useCurrentSshDock')}
              </button>
            {/if}
          </div>
        </div>
      {:else}
        <div class="flex-1 min-h-0 flex flex-col">
          <SftpTransferQueue
            variant="full"
            {queueView}
            tasks={visibleQueueItems}
            selectedIds={selectedTaskIds}
            showToolbar={false}
            onToggleSelect={toggleTaskSelection}
            onPause={pauseTask}
            onResume={resumeTask}
            onCancel={cancelTask}
            onRetry={retryTask}
            onDelete={(id) => { selectedTaskIds = new Set([id]); removeSelectedTasks(); }}
          />
        </div>
      {/if}
    </section>
  </div>
</div>


<style>
  .transfer-window-shell {
    background: var(--color-bg);
    isolation: isolate;
  }

  .transfer-window-surface {
    background: var(--color-bg);
  }

  .transfer-window-bar {
    background: var(--color-surface-raised);
  }

  .transfer-workspace {
    background: var(--color-bg-soft);
  }

  .transfer-pane {
    background: var(--color-bg-soft);
  }

  .transfer-queue {
    background: var(--color-surface-raised);
  }

  :global(.transfer-queue .sftp-transfer-queue) {
    border-top: 0;
    background: transparent;
    flex: 1;
    min-height: 0;
  }

  :global(.transfer-window-shell .panel),
  :global(.transfer-window-shell .input),
  :global(.transfer-window-shell select),
  :global(.transfer-window-shell option) {
    background-color: var(--color-panel);
  }
</style>
