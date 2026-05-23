<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    X, Folder, FileText, RefreshCw, ChevronRight, Home, ArrowUp,
    Upload, Download, Trash2, FolderPlus, Pencil, PanelRightClose, FolderUp,
    FolderDown, ExternalLink, CircleX, CheckCircle2, Clock3, Loader2,
  } from '@lucide/svelte';
  import type { RpcClient } from '../lib/rpc';
  import { b64decode, b64encode, tauriInvoke } from '../lib/rpc';
  import { i18n } from '../lib/i18n.svelte';
  import type { SftpEntry, SshProfileSpec, StoredProfile } from '../lib/types';

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
    onClose: () => void;
    onCollapse?: () => void;
    onPopOut?: (sudo: boolean) => void;
    onError: (msg: string) => void;
  }
  let { rpc, profile, source, mode = 'modal', onClose, onCollapse, onPopOut, onError }: Props = $props();
  const target = $derived(source ?? profile);

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
  const uploadFiles = new Map<string, File>();
  const downloadEntries = new Map<string, SftpEntry>();
  const knownRemoteDirs = new Set<string>();
  const CHUNK_SIZE = 256 * 1024;

  type TransferKind = 'upload' | 'download';
  type TransferStatus = 'queued' | 'running' | 'done' | 'error' | 'canceled';
  interface TransferTask {
    id: string;
    kind: TransferKind;
    name: string;
    path: string;
    size: number;
    transferred: number;
    status: TransferStatus;
    localPath?: string;
    localBaseDir?: string;
    relativePath?: string[];
    message?: string;
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

  async function connect() {
    if (!target) return;
    loading = true;
    listError = null;
    try {
      const r = await rpc.call<{ id: string }>('sftp.open', { profile: target.ssh, sudo: sudoMode });
      sessionId = r.id;
      knownRemoteDirs.clear();
      // Resolve home (".") to an absolute path so navigation is predictable.
      const real = await rpc.call<{ path: string }>('sftp.realpath', { id: r.id, path: '.' });
      cwd = real.path || '.';
      await refresh();
    } catch (e) {
      sessionId = null;
      entries = [];
      listError = (e as Error).message;
      onError(`sftp: ${(e as Error).message}`);
    } finally {
      loading = false;
    }
  }

  async function reconnect(nextSudo = sudoMode) {
    const current = sessionId;
    const prevCwd = cwd;
    sessionId = null;
    entries = [];
    listError = null;
    knownRemoteDirs.clear();
    if (current) await rpc.call('sftp.close', { id: current }).catch(() => {});
    sudoMode = nextSudo;
    loading = true;
    try {
      const r = await rpc.call<{ id: string }>('sftp.open', { profile: target!.ssh, sudo: sudoMode });
      sessionId = r.id;
      knownRemoteDirs.clear();
      let nextCwd = prevCwd;
      try {
        const real = await rpc.call<{ path: string }>('sftp.realpath', { id: r.id, path: prevCwd });
        nextCwd = real.path || prevCwd;
        await rpc.call<SftpEntry[]>('sftp.list', { id: r.id, path: nextCwd });
        cwd = nextCwd;
      } catch {
        const home = await rpc.call<{ path: string }>('sftp.realpath', { id: r.id, path: '.' });
        cwd = home.path || '/';
        onError(i18n.t('sftp.cwdFallback'));
      }
      await refresh();
    } catch (e) {
      sessionId = null;
      entries = [];
      listError = (e as Error).message;
      onError(`sftp: ${(e as Error).message}`);
    } finally {
      loading = false;
    }
  }

  async function refresh() {
    if (!sessionId) return;
    const seq = ++listSeq;
    loading = true;
    listError = null;
    try {
      const list = await rpc.call<SftpEntry[]>('sftp.list', { id: sessionId, path: cwd });
      if (seq !== listSeq) return;
      entries = sortEntries(list);
    } catch (e) {
      if (seq !== listSeq) return;
      listError = (e as Error).message;
      entries = [];
      onError(`list: ${(e as Error).message}`);
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
      await write;
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
      await write;
    }
  }

  async function mkdirLocalRelative(baseDir: string, relative: string[]) {
    const mkdir = tauriInvoke<string>('local_mkdir_relative', { baseDir, relative });
    if (!mkdir) throw new Error('desktop directory writer is not available');
    await mkdir;
  }

  async function enter(e: SftpEntry) {
    if (e.kind !== 'Dir') return;
    cwd = joinPath(cwd, e.name);
    await refresh();
  }

  async function goUp() {
    cwd = parentPath(cwd);
    await refresh();
  }

  async function goHome() {
    if (!sessionId) return;
    const real = await rpc.call<{ path: string }>('sftp.realpath', { id: sessionId, path: '.' });
    cwd = real.path || '.';
    await refresh();
  }

  async function removeEntry(e: SftpEntry) {
    if (!sessionId) return;
    if (!confirm(`Delete ${e.name}?`)) return;
    const path = joinPath(cwd, e.name);
    try {
      if (e.kind === 'Dir') {
        await rpc.call('sftp.removeDir', { id: sessionId, path });
      } else {
        await rpc.call('sftp.removeFile', { id: sessionId, path });
      }
      await refresh();
    } catch (err) {
      onError((err as Error).message);
    }
  }

  function nextTransferId(): string {
    transferSeq += 1;
    return `sftp-transfer-${Date.now()}-${transferSeq}`;
  }

  function updateTransfer(id: string, patch: Partial<TransferTask>) {
    const transfer = transfers.find((candidate) => candidate.id === id);
    if (!transfer) return;
    Object.assign(transfer, patch);
    transfers = [...transfers];
  }

  function isCanceled(id: string): boolean {
    return transfers.find((transfer) => transfer.id === id)?.status === 'canceled';
  }

  function enqueueTransfer(task: TransferTask) {
    transfers = [...transfers, task];
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
        updateTransfer(task.id, { status: 'running', message: undefined });
        try {
          if (task.kind === 'upload') {
            await runUpload(task);
          } else {
            await runDownload(task);
          }
          if (isCanceled(task.id)) continue;
          updateTransfer(task.id, { status: 'done', transferred: task.size });
          if (task.kind === 'download') {
            if (task.localPath) lastDownloadPath = task.localPath;
            else if (task.localBaseDir) lastDownloadPath = task.localBaseDir;
          }
        } catch (e) {
          if (isCanceled(task.id)) continue;
          updateTransfer(task.id, { status: 'error', message: (e as Error).message });
          onError(`transfer ${task.name}: ${(e as Error).message}`);
        }
      }
    } finally {
      processingTransfers = false;
    }
  }

  async function runUpload(task: TransferTask) {
    if (!sessionId) throw new Error('SFTP session is not open');
    const file = uploadFiles.get(task.id);
    if (!file) throw new Error('local file is not available');
    const destinationDir = parentPath(task.path);
    if (destinationDir !== '/' && destinationDir !== '.') {
      updateTransfer(task.id, { message: 'Preparing directories' });
      await ensureRemoteDir(destinationDir);
    }
    updateTransfer(task.id, { message: undefined });
    if (file.size === 0) {
      await rpc.call('sftp.writeChunk', {
        id: sessionId,
        path: task.path,
        offset: 0,
        data: '',
        create: true,
      });
      updateTransfer(task.id, { transferred: 0 });
      return;
    }
    let offset = 0;
    while (offset < file.size) {
      if (isCanceled(task.id)) return;
      const chunk = file.slice(offset, Math.min(file.size, offset + CHUNK_SIZE));
      const bytes = new Uint8Array(await chunk.arrayBuffer());
      await rpc.call('sftp.writeChunk', {
        id: sessionId,
        path: task.path,
        offset,
        data: b64encode(bytes),
        create: offset === 0,
      });
      offset += bytes.byteLength;
      updateTransfer(task.id, { transferred: offset });
    }
    needsRefreshAfterTransfers = true;
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
        const entry = await rpc.call<SftpEntry>('sftp.stat', { id: sessionId, path: cursor });
        if (entry.kind !== 'Dir') throw new Error(`${cursor} exists and is not a directory`);
      } catch (e) {
        try {
          await rpc.call('sftp.mkdir', { id: sessionId, path: cursor });
        } catch (mkdirError) {
          const entry = await rpc.call<SftpEntry>('sftp.stat', { id: sessionId, path: cursor })
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
      if (isCanceled(task.id)) return;
      const len = Math.min(CHUNK_SIZE, entry.size - offset);
      const r = await rpc.call<{ data: string }>('sftp.readChunk', {
        id: sessionId,
        path: task.path,
        offset,
        len,
      });
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

  function clearFinishedTransfers() {
    const finished = new Set(
      transfers
        .filter((transfer) => transfer.status === 'done' || transfer.status === 'error' || transfer.status === 'canceled')
        .map((transfer) => transfer.id),
    );
    for (const id of finished) {
      uploadFiles.delete(id);
      downloadEntries.delete(id);
    }
    transfers = transfers.filter((transfer) => !finished.has(transfer.id));
  }

  async function renameEntry(e: SftpEntry) {
    if (!sessionId) return;
    const nextName = prompt('Rename to', e.name)?.trim();
    if (!nextName || nextName === e.name) return;
    if (nextName.includes('/')) {
      onError('rename: name must not contain /');
      return;
    }
    try {
      await rpc.call('sftp.rename', {
        id: sessionId,
        from: joinPath(cwd, e.name),
        to: joinPath(cwd, nextName),
      });
      await refresh();
    } catch (err) {
      onError(`rename: ${(err as Error).message}`);
    }
  }

  async function mkdirHere() {
    if (!sessionId) return;
    const name = prompt('New folder name');
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
    const list = sortEntries(await rpc.call<SftpEntry[]>('sftp.list', { id: sessionId, path: remotePath }));
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
    if (task.status === 'queued') return 'Queued';
    if (task.status === 'running') return task.message ?? `${formatSize(task.transferred)} / ${formatSize(task.size)}`;
    if (task.status === 'done') return 'Done';
    if (task.status === 'canceled') return 'Canceled';
    return task.message ?? 'Failed';
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
    cwd = p;
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
    void loadSftpSettings();
    void connect();
  });

  onDestroy(() => {
    if (sessionId) {
      void rpc.call('sftp.close', { id: sessionId }).catch(() => {});
    }
  });
</script>

<div
  class={mode === 'modal'
    ? 'fixed inset-0 bg-black/60 z-50 grid place-items-center p-6'
    : 'h-full w-full min-w-0 bg-[var(--color-panel)]'}
  role={mode === 'modal' ? 'dialog' : 'complementary'}
  aria-modal={mode === 'modal'}
  aria-label={i18n.t('sftp.aria')}
>
  <div
    class={mode === 'modal'
      ? 'bg-[var(--color-panel)] border border-[var(--color-border)] rounded-lg shadow-2xl w-full max-w-[900px] h-full max-h-[640px] flex flex-col overflow-hidden'
      : 'h-full w-full flex flex-col overflow-hidden'}
  >
    <header class="flex items-center gap-2 px-4 py-2.5 border-b border-[var(--color-border-soft)]">
      <div class="text-[var(--color-accent)] font-semibold text-[13px]">SFTP</div>
      <div class="text-[12px] text-[var(--color-fg-muted)]">·</div>
      <div class="text-[12px] text-[var(--color-fg)] truncate">{target?.name ?? i18n.t('sftp.sshSession')}</div>
      <div class="text-[11px] text-[var(--color-fg-muted)] truncate">
        ({target?.ssh.user}@{target?.ssh.host}:{target?.ssh.port})
      </div>
      <button
        type="button"
        class="px-2 py-0.5 rounded text-[10.5px] border {sudoMode ? 'border-[var(--color-accent)] text-[var(--color-accent)] bg-[var(--color-accent-soft)]' : 'border-[var(--color-border-soft)] text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]'}"
        onclick={toggleSudoMode}
        title={i18n.t('sftp.toggleSudo')}
        aria-pressed={sudoMode}
      >sudo</button>
      <div class="ml-auto flex items-center gap-1">
        {#if onPopOut}
          <button
            type="button"
            class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]"
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
            class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]"
            onclick={onCollapse}
            title={i18n.t('sftp.collapseDock')}
            aria-label={i18n.t('sftp.collapseDock')}
          >
            <PanelRightClose size={14} />
          </button>
        {/if}
        <button
          type="button"
          class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]"
          onclick={onClose}
          aria-label={i18n.t('common.close')}
        >
          <X size={14} />
        </button>
      </div>
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
        <button type="button" class="btn-secondary shrink-0 text-[11px] px-2 py-0.5" onclick={() => { void refresh(); }}>
          {i18n.t('sftp.retryList')}
        </button>
      </div>
    {/if}

    <div class="flex-1 min-h-0 overflow-y-auto">
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
              <th class="w-[80px]"></th>
            </tr>
          </thead>
          <tbody>
            {#each entries as e (e.name)}
              <tr class="hover:bg-[var(--color-panel-2)] group">
                <td class="px-3 py-1 truncate">
                  <button
                    type="button"
                    class="flex items-center gap-2 w-full text-left"
                    ondblclick={() => enter(e)}
                    onclick={() => e.kind === 'Dir' && enter(e)}
                  >
                    {#if e.kind === 'Dir'}
                      <Folder size={13} class="text-[var(--color-accent)]" />
                    {:else}
                      <FileText size={13} class="text-[var(--color-fg-muted)]" />
                    {/if}
                    <span class="truncate text-[var(--color-fg)]">{e.name}</span>
                  </button>
                </td>
                <td class="px-3 py-1 text-right text-[var(--color-fg-muted)]">
                  {e.kind === 'File' ? formatSize(e.size) : ''}
                </td>
                <td class="px-3 py-1 text-[var(--color-fg-muted)] font-mono text-[11px]">
                  {(e.mode & 0o777).toString(8).padStart(3, '0')}
                </td>
                <td class="px-2 py-1 text-right whitespace-nowrap">
                  {#if e.kind === 'File' || e.kind === 'Dir'}
                    <button
                      type="button"
                      class="opacity-0 group-hover:opacity-100 p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-accent)]"
                      onclick={() => downloadEntry(e)}
                      title={e.kind === 'Dir' ? i18n.t('common.downloadFolder') : i18n.t('common.download')}
                      aria-label={e.kind === 'Dir' ? i18n.t('common.downloadFolder') : i18n.t('common.download')}
                    >
                      {#if e.kind === 'Dir'}<FolderDown size={12} />{:else}<Download size={12} />{/if}
                    </button>
                  {/if}
                  <button
                    type="button"
                    class="opacity-0 group-hover:opacity-100 p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-accent)]"
                    onclick={() => renameEntry(e)}
                    title={i18n.t('common.rename')}
                    aria-label={i18n.t('common.rename')}
                  >
                    <Pencil size={12} />
                  </button>
                  <button
                    type="button"
                    class="opacity-0 group-hover:opacity-100 p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-danger)]"
                    onclick={() => removeEntry(e)}
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

    <div class="border-t border-[var(--color-border-soft)] px-3 py-1.5 text-[11px] text-[var(--color-fg-muted)]
                flex items-center gap-2 min-h-[28px]">
      {#if lastDownloadPath}
        <span class="truncate min-w-0" title={lastDownloadPath}>
          {i18n.t('sftp.downloadDir', { path: lastDownloadPath })}
        </span>
      {/if}
      <button
        type="button"
        class="ml-auto shrink-0 text-[var(--color-accent)] hover:underline"
        onclick={() => { void chooseDownloadDir(); }}
      >
        {i18n.t('sftp.pickDownloadDir')}
      </button>
    </div>

    {#if transfers.length > 0}
      <div class="border-t border-[var(--color-border-soft)] bg-[var(--color-panel)] max-h-[180px] overflow-y-auto">
        <div class="sticky top-0 z-10 flex items-center gap-2 px-3 py-1.5 bg-[var(--color-panel)] border-b border-[var(--color-border-soft)]">
          <div class="text-[11px] uppercase tracking-[0.12em] text-[var(--color-fg-muted)]">{i18n.t('sftp.transfers')}</div>
          <div class="text-[11px] text-[var(--color-fg-muted)]">{transfers.length}</div>
          <button
            type="button"
            class="ml-auto text-[11px] text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]"
            onclick={clearFinishedTransfers}
          >{i18n.t('common.clearFinished')}</button>
        </div>
        <div class="divide-y divide-[var(--color-border-soft)]">
          {#each transfers as task (task.id)}
            {@const pct = transferPercent(task)}
            <div class="px-3 py-2 text-[11.5px]">
              <div class="flex items-center gap-2 min-w-0">
                {#if task.status === 'queued'}
                  <Clock3 size={13} class="text-[var(--color-fg-muted)] shrink-0" />
                {:else if task.status === 'running'}
                  <Loader2 size={13} class="text-[var(--color-accent)] shrink-0 animate-spin" />
                {:else if task.status === 'done'}
                  <CheckCircle2 size={13} class="text-[var(--color-success)] shrink-0" />
                {:else}
                  <CircleX size={13} class="text-[var(--color-danger)] shrink-0" />
                {/if}
                <div class="min-w-0 flex-1">
                  <div class="flex items-center gap-2 min-w-0">
                    <span class="uppercase text-[9.5px] text-[var(--color-fg-muted)] shrink-0">{task.kind}</span>
                    <span class="truncate text-[var(--color-fg)]">{task.name}</span>
                    <span class="ml-auto text-[10.5px] text-[var(--color-fg-muted)] shrink-0">{pct}%</span>
                  </div>
                  <div class="mt-1 h-1 rounded bg-[var(--color-panel-2)] overflow-hidden">
                    <div class="h-full bg-[var(--color-accent)]" style="width: {pct}%"></div>
                  </div>
                  <div class="mt-1 truncate text-[10.5px] text-[var(--color-fg-muted)]">{transferSummary(task)}</div>
                </div>
                {#if task.status === 'queued' || task.status === 'running'}
                  <button
                    type="button"
                    class="p-1 text-[var(--color-fg-muted)] hover:text-[var(--color-danger)]"
                    title={i18n.t('sftp.cancelTransfer')}
                    aria-label={i18n.t('sftp.cancelTransfer')}
                    onclick={() => cancelTransfer(task.id)}
                  >
                    <X size={12} />
                  </button>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .toolbtn {
    display: inline-grid;
    place-items: center;
    width: 26px; height: 26px;
    color: var(--color-fg-muted);
    border-radius: var(--radius-sm);
    background: transparent;
  }
  .toolbtn:hover {
    color: var(--color-fg);
    background: var(--color-panel-2);
  }
</style>
