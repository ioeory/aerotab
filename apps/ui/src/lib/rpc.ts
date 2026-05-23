// JSON-RPC client. Picks Tauri invoke when available, falls back to HTTP /rpc.

export interface RpcRequest {
  jsonrpc: '2.0';
  id: number;
  method: string;
  params?: unknown;
}

export interface RpcResponse<T = unknown> {
  jsonrpc: '2.0';
  id: number | null;
  result?: T;
  error?: { code: number; message: string };
}

export interface RpcClient {
  call<T = unknown>(method: string, params?: unknown): Promise<T>;
}

class HttpRpcClient implements RpcClient {
  private nextId = 1;
  constructor(private url = '/rpc') {}
  async call<T>(method: string, params: unknown = null): Promise<T> {
    const req: RpcRequest = { jsonrpc: '2.0', id: this.nextId++, method, params };
    const resp = await fetch(this.url, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(req),
    });
    const json: RpcResponse<T> = await resp.json();
    if (json.error) throw new Error(`${json.error.code}: ${json.error.message}`);
    return json.result as T;
  }
}

interface TauriGlobal {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
}

class TauriRpcClient implements RpcClient {
  private nextId = 1;
  constructor(private tauri: TauriGlobal) {}
  async call<T>(method: string, params: unknown = null): Promise<T> {
    const req: RpcRequest = { jsonrpc: '2.0', id: this.nextId++, method, params };
    const json: RpcResponse<T> = await this.tauri.invoke('rpc', { frame: req });
    if (json.error) throw new Error(`${json.error.code}: ${json.error.message}`);
    return json.result as T;
  }
}

export function selectClient(): RpcClient {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const w = window as any;
  const candidates: Array<undefined | ((cmd: string, args?: Record<string, unknown>) => Promise<unknown>)> = [
    w.__TAURI__?.core?.invoke?.bind(w.__TAURI__.core),
    w.__TAURI__?.invoke?.bind(w.__TAURI__),
    w.__TAURI_INTERNALS__?.invoke?.bind(w.__TAURI_INTERNALS__),
  ];
  const invoke = candidates.find((fn) => typeof fn === 'function');
  if (invoke) {
    return new TauriRpcClient({ invoke: invoke as TauriGlobal['invoke'] });
  }
  return new HttpRpcClient();
}

export function b64encode(bytes: Uint8Array): string {
  let bin = '';
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin);
}

export function b64decode(s: string): Uint8Array {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

export function uuidv4(): string {
  if (crypto.randomUUID) return crypto.randomUUID();
  const r = crypto.getRandomValues(new Uint8Array(16));
  r[6] = ((r[6] ?? 0) & 0x0f) | 0x40;
  r[8] = ((r[8] ?? 0) & 0x3f) | 0x80;
  const h = Array.from(r, (b) => b.toString(16).padStart(2, '0')).join('');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

/** Direct Tauri `invoke` for non-JSON-RPC commands (e.g. `check_update`).
 * Returns `null` when running in a browser/dev fallback without Tauri. */
export function tauriInvoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> | null {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const w = window as any;
  const fn =
    w.__TAURI__?.core?.invoke?.bind(w.__TAURI__.core) ??
    w.__TAURI__?.invoke?.bind(w.__TAURI__) ??
    w.__TAURI_INTERNALS__?.invoke?.bind(w.__TAURI_INTERNALS__);
  if (typeof fn !== 'function') return null;
  return fn(cmd, args ?? {}) as Promise<T>;
}
