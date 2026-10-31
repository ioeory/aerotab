/**
 * Minimal Tauri 2 Channel shim (same wire format as `@tauri-apps/api/core`).
 * Prefer `window.__TAURI__.core.Channel` when withGlobalTauri injects it.
 */

const SERIALIZE = '__TAURI_TO_IPC_KEY__';

interface TauriInternals {
  transformCallback(cb: (data: unknown) => void, once?: boolean): number;
  unregisterCallback(id: number): void;
  invoke(cmd: string, args?: Record<string, unknown>): Promise<unknown>;
}

function internals(): TauriInternals | null {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const w = window as any;
  const i = w.__TAURI_INTERNALS__;
  if (i && typeof i.transformCallback === 'function' && typeof i.invoke === 'function') {
    return i as TauriInternals;
  }
  return null;
}

export function tauriInvoke(): ((cmd: string, args?: Record<string, unknown>) => Promise<unknown>) | null {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const w = window as any;
  const invoke = w.__TAURI__?.core?.invoke ?? w.__TAURI__?.invoke ?? w.__TAURI_INTERNALS__?.invoke;
  return typeof invoke === 'function' ? invoke.bind(w.__TAURI__?.core ?? w.__TAURI__ ?? w.__TAURI_INTERNALS__) : null;
}

export interface FrameChannel {
  id: number;
  onmessage: (data: ArrayBuffer | Uint8Array) => void;
  /** Serialized form expected by Rust `tauri::ipc::Channel`. */
  toJSON(): string;
}

/** Create a Channel that receives raw `ArrayBuffer` frames from `terminal_frames`. */
export function createFrameChannel(
  onFrame: (buf: ArrayBuffer) => void,
): FrameChannel | null {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const w = window as any;
  const GlobalChannel = w.__TAURI__?.core?.Channel;
  if (typeof GlobalChannel === 'function') {
    const ch = new GlobalChannel((message: ArrayBuffer | Uint8Array) => {
      onFrame(toArrayBuffer(message));
    });
    return ch as FrameChannel;
  }

  const i = internals();
  if (!i) return null;

  let nextIndex = 0;
  const pending: Array<ArrayBuffer | undefined> = [];
  let endIndex: number | undefined;
  let handler = onFrame;

  const id = i.transformCallback((raw: unknown) => {
    const msg = raw as { message?: ArrayBuffer | Uint8Array; index?: number; end?: boolean };
    const index = msg.index ?? 0;
    if (msg.end) {
      if (index === nextIndex) i.unregisterCallback(id);
      else endIndex = index;
      return;
    }
    const payload = msg.message;
    if (payload === undefined) return;
    if (index === nextIndex) {
      handler(toArrayBuffer(payload));
      nextIndex++;
      while (nextIndex in pending) {
        const p = pending[nextIndex];
        delete pending[nextIndex];
        if (p) handler(p);
        nextIndex++;
      }
      if (nextIndex === endIndex) i.unregisterCallback(id);
    } else {
      pending[index] = toArrayBuffer(payload);
    }
  });

  const ch: FrameChannel = {
    id,
    get onmessage() {
      return (data: ArrayBuffer | Uint8Array) => handler(toArrayBuffer(data));
    },
    set onmessage(h: (data: ArrayBuffer | Uint8Array) => void) {
      handler = (data) => h(toArrayBuffer(data));
    },
    toJSON() {
      return `__CHANNEL__:${id}`;
    },
  };
  // Tauri IPC serializer looks for this symbol/key.
  (ch as unknown as Record<string, unknown>)[SERIALIZE] = () => `__CHANNEL__:${id}`;
  return ch;
}

function toArrayBuffer(data: ArrayBuffer | Uint8Array): ArrayBuffer {
  if (data instanceof ArrayBuffer) return data;
  return data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) as ArrayBuffer;
}
