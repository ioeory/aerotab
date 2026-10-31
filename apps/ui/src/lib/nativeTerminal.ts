import type { RpcClient } from './rpc';
import type { SessionMeta, SshProfileSpec } from './types';

export interface NativeProgramInfo {
  id: string;
  path: string;
}

export interface EmbedCapabilities {
  embedSupported: boolean;
  platform: string;
  note: string;
  programs: NativeProgramInfo[];
}

export interface EmbedResult {
  instanceId: string;
  pid: number;
  program: string;
  platform: string;
  message?: string;
}

export interface EmbedRectDip {
  x: number;
  y: number;
  width: number;
  height: number;
  devicePixelRatio: number;
}

export interface NativeDetectResult {
  programs: NativeProgramInfo[];
  embed_supported: boolean;
  embed_note: string;
}

export interface NativeSpawnResult {
  instance_id: string;
  pid: number;
  program: string;
  mode: 'detached' | 'embed';
  message?: string;
}

function formatUserHost(p: SshProfileSpec): string {
  if (p.port !== 22) return `${p.user}@${p.host}:${p.port}`;
  return `${p.user}@${p.host}`;
}

/** OpenSSH argv for spawning `ssh` inside a native terminal emulator. */
export function buildNativeSshArgv(profile: SshProfileSpec): string[] {
  const args: string[] = [];
  for (const hop of profile.jump_via ?? []) {
    args.push('-J', formatUserHost(hop));
  }
  if (profile.port !== 22) {
    args.push('-p', String(profile.port));
  }
  const auth = profile.auth;
  if (auth && typeof auth === 'object') {
    if ('PublicKey' in auth) {
      args.push('-i', auth.PublicKey.key_path);
      args.push('-o', 'PreferredAuthentications=publickey');
    } else if ('Agent' in auth) {
      args.push('-o', 'PreferredAuthentications=publickey');
    }
  }
  args.push('-o', 'BatchMode=yes');
  args.push('-o', 'PasswordAuthentication=no');
  args.push(`${profile.user}@${profile.host}`);
  return args;
}

export function buildNativeArgvForSession(session: SessionMeta): string[] | null {
  if (session.kind === 'Ssh' || session.kind === 'ssh') {
    if (!session.sshProfile) return null;
    return ['ssh', ...buildNativeSshArgv(session.sshProfile)];
  }
  if (session.kind === 'LocalShell' || session.kind === 'local') {
    if (session.shellCommand) {
      return [session.shellCommand, ...(session.shellArgs ?? [])];
    }
    return []; // empty = Rust spawns default shell
  }
  return null;
}

export async function detectNativeTerminals(rpc: RpcClient): Promise<EmbedCapabilities> {
  return rpc.call<EmbedCapabilities>('nativeTerminal.embedCapabilities');
}

export interface NativeSpawnResult {
  instanceId: string;
  pid: number;
  program: string;
  mode: 'detached' | 'embed';
  message?: string;
}

export async function spawnDetachedNativeTerminal(
  rpc: RpcClient,
  session: SessionMeta,
  program?: string,
): Promise<NativeSpawnResult> {
  const argv = buildNativeArgvForSession(session);
  if (argv === null) {
    throw new Error(
      'native terminal: unsupported session or missing SSH profile / shell command',
    );
  }
  const caps = await getEmbedCapabilities(rpc);
  if (!caps.programs.length) {
    throw new Error(
      'No native terminal on PATH. Install alacritty, ghostty, or kitty.',
    );
  }
  const prog = program ?? caps.programs[0]?.id;
  return rpc.call<NativeSpawnResult>('nativeTerminal.spawn', {
    program: prog,
    title: session.title,
    argv,
    mode: 'detached',
  });
}

export async function getEmbedCapabilities(rpc: RpcClient): Promise<EmbedCapabilities> {
  return detectNativeTerminals(rpc);
}

/**
 * Converts a DOM element's bounding rect to dip (device-independent pixel)
 * coordinates suitable for the Rust embed layer, then offsets by the Tauri
 * window position to get physical screen coordinates.
 */
async function screenRectFromElement(
  rpc: RpcClient,
  el: HTMLElement,
): Promise<EmbedRectDip> {
  const dpr = window.devicePixelRatio || 1;
  const rect = el.getBoundingClientRect();
  const dip: EmbedRectDip = {
    x: rect.x,
    y: rect.y,
    width: rect.width,
    height: rect.height,
    devicePixelRatio: dpr,
  };
  return dip;
}

export class NativeEmbedController {
  private rpc: RpcClient;
  private instanceId: string | null = null;
  private resizeObserver: ResizeObserver | null = null;
  private element: HTMLElement | null = null;
  private syncHandle: number | null = null;
  embedResult: EmbedResult | null = null;

  constructor(rpc: RpcClient) {
    this.rpc = rpc;
  }

  /**
   * Launch a native terminal and attach it to the given DOM element's position.
   */
  async embed(
    element: HTMLElement,
    session: SessionMeta,
    program?: string,
  ): Promise<EmbedResult> {
    const argv = buildNativeArgvForSession(session);
    if (argv === null) {
      throw new Error(
        'native embed: unsupported session or missing SSH profile / shell command',
      );
    }
    const caps = await getEmbedCapabilities(this.rpc);
    if (!caps.programs.length) {
      throw new Error(
        'No native terminal on PATH. Install alacritty, ghostty, or kitty.',
      );
    }
    const prog = program ?? caps.programs[0]?.id;
    if (!caps.embedSupported) {
      const detached = await spawnDetachedNativeTerminal(this.rpc, session, prog);
      const result: EmbedResult = {
        instanceId: detached.instanceId,
        pid: detached.pid,
        program: detached.program,
        platform: caps.platform,
        message: detached.message ?? caps.note,
      };
      this.embedResult = result;
      console.log(
        `[native-terminal] detached fallback on ${caps.platform}: ${result.program} pid=${result.pid}`,
      );
      return result;
    }
    const rect = await screenRectFromElement(this.rpc, element);
    const result = await this.rpc.call<EmbedResult>('nativeTerminal.embedStart', {
      program: prog,
      title: session.title,
      argv,
      rect,
    });
    this.instanceId = result.instanceId;
    this.embedResult = result;
    this.element = element;
    this.startGeometrySync(element);
    console.log(
      `[native-embed] ${result.program} (${result.platform}) pid=${result.pid} id=${result.instanceId} — ${result.message}`,
    );
    return result;
  }

  /**
   * Sync the embedded terminal's position to the DOM element on every frame.
   */
  private startGeometrySync(element: HTMLElement) {
    this.stopGeometrySync();
    this.resizeObserver = new ResizeObserver(() => {
      this.scheduleSync(element);
    });
    this.resizeObserver.observe(element);
    this.scheduleSync(element);
  }

  private scheduleSync(element: HTMLElement) {
    if (this.syncHandle != null) return;
    this.syncHandle = requestAnimationFrame(async () => {
      this.syncHandle = null;
      if (!this.instanceId) return;
      try {
        const rect = await screenRectFromElement(this.rpc, element);
        await this.rpc.call('nativeTerminal.embedSyncGeometry', {
          instanceId: this.instanceId,
          rect,
        });
      } catch {
        /* geometry sync failure is non-fatal */
      }
    });
  }

  private stopGeometrySync() {
    if (this.resizeObserver) {
      this.resizeObserver.disconnect();
      this.resizeObserver = null;
    }
    if (this.syncHandle != null) {
      cancelAnimationFrame(this.syncHandle);
      this.syncHandle = null;
    }
  }

  /**
   * Close the embedded terminal.
   */
  async close(): Promise<boolean> {
    this.stopGeometrySync();
    if (!this.instanceId) return false;
    try {
      const res = await this.rpc.call<{ removed: boolean }>(
        'nativeTerminal.embedEnd',
        { instanceId: this.instanceId },
      );
      this.instanceId = null;
      return res.removed;
    } catch {
      return false;
    }
  }

  get active(): boolean {
    return this.instanceId != null;
  }
}
