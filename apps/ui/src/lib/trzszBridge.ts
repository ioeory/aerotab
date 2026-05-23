import { b64decode, b64encode, tauriInvoke } from './rpc';

export type TrzszTerminalOutput = string | ArrayBuffer | Uint8Array | Blob;
export type TrzszTerminalInput = string | ArrayBuffer | Uint8Array | Blob;

export interface TrzszFilterInstance {
  processServerOutput(output: TrzszTerminalOutput): void;
  processTerminalInput(input: string): void;
  processBinaryInput(input: string): void;
  setTerminalColumns(columns: number): void;
  isTransferringFiles(): boolean;
  stopTransferringFiles(): void;
  uploadFiles(items: string[] | DataTransferItemList): Promise<void>;
}

export interface TrzszFilterOptions {
  writeToTerminal(output: TrzszTerminalOutput): void;
  sendToServer(input: TrzszTerminalInput): void;
  terminalColumns?: number;
  isWindowsShell?: boolean;
  maxDataChunkSize?: number;
  dragInitTimeout?: number;
}

type TrzszFilterConstructor = new (options: TrzszFilterOptions & {
  chooseSendFiles?: (directory?: boolean) => Promise<string[] | undefined>;
  chooseSaveDirectory?: () => Promise<string | undefined>;
}) => TrzszFilterInstance;

type NodeCallback<T = unknown> = (error: unknown, value?: T, extra?: unknown) => void;

type RequireShim = ((name: string) => unknown) & { resolve?: (name: string) => string };

interface LocalPathInfo {
  kind: 'file' | 'dir' | 'other';
  size: number;
}

interface LocalReadChunk {
  data: string;
}

interface NodeStat {
  size: number;
  isDirectory(): boolean;
  isFile(): boolean;
}

interface OpenFile {
  path: string;
  offset: number;
  created: boolean;
}

const openFiles = new Map<number, OpenFile>();
let nextFileDescriptor = 3;
let filterConstructorPromise: Promise<TrzszFilterConstructor> | null = null;

function toError(value: unknown): Error {
  if (value instanceof Error) return value;
  if (typeof value === 'string') return new Error(value);
  return new Error(String(value));
}

async function invokeNative<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const promise = tauriInvoke<T>(cmd, args);
  if (!promise) throw new Error('Native file access is unavailable outside the Tauri app');
  return promise;
}

function makeStat(info: LocalPathInfo): NodeStat {
  return {
    size: info.size,
    isDirectory: () => info.kind === 'dir',
    isFile: () => info.kind === 'file',
  };
}

function asBytes(data: ArrayBuffer | Uint8Array): Uint8Array {
  return data instanceof Uint8Array ? data : new Uint8Array(data);
}

function access(path: string, modeOrCallback: number | NodeCallback<boolean>, callback?: NodeCallback<boolean>): void {
  const done = typeof modeOrCallback === 'function' ? modeOrCallback : callback;
  if (!done) return;
  invokeNative<LocalPathInfo>('local_stat', { path })
    .then(() => done(null, true))
    .catch((error: unknown) => done(toError(error)));
}

function stat(path: string, callback: NodeCallback<NodeStat>): void {
  invokeNative<LocalPathInfo>('local_stat', { path })
    .then((info) => callback(null, makeStat(info)))
    .catch((error: unknown) => callback(toError(error)));
}

function realpath(path: string, callback: NodeCallback<string>): void {
  invokeNative<string>('local_realpath', { path })
    .then((resolved) => callback(null, resolved))
    .catch((error: unknown) => callback(toError(error)));
}

function readdir(path: string, callback: NodeCallback<string[]>): void {
  invokeNative<string[]>('local_read_dir', { path })
    .then((names) => callback(null, names))
    .catch((error: unknown) => callback(toError(error)));
}

function mkdir(path: string, optionsOrCallback: unknown, callback?: NodeCallback<boolean>): void {
  const done = typeof optionsOrCallback === 'function' ? optionsOrCallback as NodeCallback<boolean> : callback;
  if (!done) return;
  invokeNative<void>('local_mkdir', { path })
    .then(() => done(null, true))
    .catch((error: unknown) => done(toError(error)));
}

function open(path: string, mode: string, callback: NodeCallback<number>): void {
  const fileDescriptor = nextFileDescriptor++;
  openFiles.set(fileDescriptor, { path, offset: 0, created: false });
  if (!mode.includes('w')) {
    callback(null, fileDescriptor);
    return;
  }
  invokeNative<void>('local_write_chunk', { path, offset: 0, data: '', create: true })
    .then(() => {
      const file = openFiles.get(fileDescriptor);
      if (file) file.created = true;
      callback(null, fileDescriptor);
    })
    .catch((error: unknown) => {
      openFiles.delete(fileDescriptor);
      callback(toError(error));
    });
}

function read(
  fileDescriptor: number,
  buffer: Uint8Array,
  bufferOffset: number,
  length: number,
  position: number | null,
  callback: NodeCallback<number>,
): void {
  const file = openFiles.get(fileDescriptor);
  if (!file) {
    callback(new Error(`Bad file descriptor: ${fileDescriptor}`));
    return;
  }
  const readOffset = position ?? file.offset;
  invokeNative<LocalReadChunk>('local_read_chunk', { path: file.path, offset: readOffset, len: length })
    .then((chunk) => {
      const bytes = b64decode(chunk.data);
      buffer.set(bytes, bufferOffset);
      if (position === null) file.offset = readOffset + bytes.length;
      callback(null, bytes.length, buffer);
    })
    .catch((error: unknown) => callback(toError(error)));
}

function write(fileDescriptor: number, data: ArrayBuffer | Uint8Array, callback: NodeCallback<number>): void {
  const file = openFiles.get(fileDescriptor);
  if (!file) {
    callback(new Error(`Bad file descriptor: ${fileDescriptor}`));
    return;
  }
  const bytes = asBytes(data);
  invokeNative<void>('local_write_chunk', {
    path: file.path,
    offset: file.offset,
    data: b64encode(bytes),
    create: !file.created,
  })
    .then(() => {
      file.offset += bytes.length;
      file.created = true;
      callback(null, bytes.length);
    })
    .catch((error: unknown) => callback(toError(error)));
}

function close(fileDescriptor: number, callback: NodeCallback<boolean>): void {
  openFiles.delete(fileDescriptor);
  callback(null, true);
}

function remove(path: string, optionsOrCallback: unknown, callback?: NodeCallback<boolean>): void {
  const done = typeof optionsOrCallback === 'function' ? optionsOrCallback as NodeCallback<boolean> : callback;
  if (!done) return;
  const recursive = typeof optionsOrCallback === 'object' && !!optionsOrCallback && 'recursive' in optionsOrCallback
    ? Boolean((optionsOrCallback as { recursive?: unknown }).recursive)
    : false;
  invokeNative<boolean>('local_remove', { path, recursive })
    .then((removed) => done(null, removed))
    .catch((error: unknown) => done(toError(error)));
}

function unlink(path: string, callback: NodeCallback<boolean>): void {
  invokeNative<boolean>('local_remove', { path, recursive: false })
    .then((removed) => callback(null, removed))
    .catch((error: unknown) => callback(toError(error)));
}

function preferredSeparator(path: string): string {
  return path.includes('\\') && !path.includes('/') ? '\\' : '/';
}

function joinPath(base: string, ...parts: string[]): string {
  const separator = preferredSeparator(base);
  let out = base;
  for (const part of parts.filter(Boolean)) {
    if (!out) {
      out = part;
    } else if (out.endsWith('/') || out.endsWith('\\')) {
      out += part;
    } else {
      out += `${separator}${part}`;
    }
  }
  return out;
}

function basename(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, '');
  const index = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  return index >= 0 ? trimmed.slice(index + 1) : trimmed;
}

function installNodeRuntimeShim(): void {
  const globalObject = globalThis as unknown as { require?: RequireShim };
  const previousRequire = globalObject.require;
  const fsShim = {
    constants: { R_OK: 4, W_OK: 2 },
    access,
    stat,
    mkdir,
    readdir,
    close,
    open,
    realpath,
    write,
    read,
    rm: remove,
    rmdir: remove,
    unlink,
  };
  const pathShim = {
    join: joinPath,
    resolve: (path: string) => path,
    basename,
  };
  const shim = ((name: string) => {
    if (name === 'fs') return fsShim;
    if (name === 'path') return pathShim;
    if (previousRequire) return previousRequire(name);
    throw new Error(`Cannot require ${name}`);
  }) as RequireShim;
  shim.resolve = (name: string) => {
    if (name === 'fs' || name === 'path') return name;
    if (previousRequire?.resolve) return previousRequire.resolve(name);
    throw new Error(`Cannot resolve ${name}`);
  };
  globalObject.require = shim;
}

async function chooseSendFiles(directory?: boolean): Promise<string[] | undefined> {
  const selected = await invokeNative<string[] | null>('pick_open_files', { directory: !!directory });
  return selected ?? undefined;
}

async function chooseSaveDirectory(): Promise<string | undefined> {
  const selected = await invokeNative<string | null>('pick_directory');
  return selected ?? undefined;
}

export async function createTrzszFilter(options: TrzszFilterOptions): Promise<TrzszFilterInstance> {
  installNodeRuntimeShim();
  filterConstructorPromise ??= import('trzsz').then((module) => {
    const imported = module as unknown as { TrzszFilter: TrzszFilterConstructor };
    return imported.TrzszFilter;
  });
  const TrzszFilter = await filterConstructorPromise;
  return new TrzszFilter({
    ...options,
    chooseSendFiles,
    chooseSaveDirectory,
  });
}
