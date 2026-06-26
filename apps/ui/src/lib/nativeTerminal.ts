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

// ---------------------------------------------------------------------------
// Native engine controller (vt100 + PTY, cell-based rendering)
// ---------------------------------------------------------------------------

export interface EngineCellInfo {
  c: string;
  fg: string;
  bg: string;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  cursor: boolean;
}

export interface EngineLineCells {
  row: number;
  cells: EngineCellInfo[];
}

export interface EngineScreenFrame {
  cols: number;
  rows: number;
  cursorX: number;
  cursorY: number;
  lines: EngineLineCells[];
}

export interface EngineResult {
  engineId: string;
}

export class NativeEngineController {
  private rpc: RpcClient;
  private engineId: string | null = null;
  private canvas: HTMLCanvasElement | null = null;
  private ctx: CanvasRenderingContext2D | null = null;
  private pollHandle: number | null = null;
  private charWidth = 0;
  private charHeight = 0;
  private cols = 0;
  private rows = 0;
  private resizeObserver: ResizeObserver | null = null;
  frameCount = 0;

  constructor(rpc: RpcClient) {
    this.rpc = rpc;
  }

  async start(canvas: HTMLCanvasElement, cols = 80, rows = 24) {
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d')!;
    this.cols = cols;
    this.rows = rows;

    const result = await this.rpc.call<EngineResult>('nativeEngine.create', {
      cols,
      rows,
    });
    this.engineId = result.engineId;
    this.setupCanvas();
    this.startPolling();

    this.resizeObserver = new ResizeObserver(() => {
      if (!this.canvas) return;
      const newCols = Math.floor(this.canvas.clientWidth / (this.charWidth || 9));
      const newRows = Math.floor(this.canvas.clientHeight / (this.charHeight || 18));
      if (newCols !== this.cols || newRows !== this.rows) {
        this.cols = newCols;
        this.rows = newRows;
        this.setupCanvas();
        void this.rpc.call('nativeEngine.resize', {
          engineId: this.engineId,
          cols: newCols,
          rows: newRows,
        }).catch(() => {});
      }
    });
    this.resizeObserver.observe(canvas);
  }

  private setupCanvas() {
    if (!this.canvas || !this.ctx) return;
    const dpr = window.devicePixelRatio || 1;
    const cssWidth = this.canvas.clientWidth;
    const cssHeight = this.canvas.clientHeight;
    this.canvas.width = cssWidth * dpr;
    this.canvas.height = cssHeight * dpr;
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.charWidth = cssWidth / this.cols;
    this.charHeight = cssHeight / this.rows;
    this.ctx.font = `${Math.floor(this.charHeight * 0.85)}px monospace`;
    this.ctx.textBaseline = 'top';
  }

  private drawFrame(frame: EngineScreenFrame) {
    if (!this.ctx || !this.canvas) return;
    const ctx = this.ctx;
    const cssWidth = this.canvas.clientWidth;
    const cssHeight = this.canvas.clientHeight;

    // Background
    ctx.fillStyle = '#0b0d12';
    ctx.fillRect(0, 0, cssWidth, cssHeight);

    const cw = cssWidth / this.cols;
    const ch = cssHeight / this.rows;
    ctx.font = `${Math.floor(ch * 0.85)}px monospace`;
    ctx.textBaseline = 'top';

    for (const line of frame.lines) {
      for (let ci = 0; ci < line.cells.length; ci++) {
        const cell = line.cells[ci];
        if (!cell) continue;
        const x = ci * cw;
        const y = line.row * ch;

        // Background
        if (cell.bg !== '#000000') {
          ctx.fillStyle = cell.bg;
          ctx.fillRect(x, y, cw, ch);
        }

        // Text
        ctx.fillStyle = cell.fg;
        if (cell.c !== ' ') {
          ctx.save();
          if (cell.bold) ctx.font = `bold ${Math.floor(ch * 0.85)}px monospace`;
          if (cell.italic) ctx.font = `italic ${Math.floor(ch * 0.85)}px monospace`;
          if (cell.bold && cell.italic) ctx.font = `bold italic ${Math.floor(ch * 0.85)}px monospace`;
          ctx.fillText(cell.c, x, y + (ch - ch * 0.85) / 2);
          ctx.restore();
        }

        // Underline
        if (cell.underline) {
          ctx.strokeStyle = cell.fg;
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(x, y + ch - 2);
          ctx.lineTo(x + cw, y + ch - 2);
          ctx.stroke();
        }
      }
    }

    // Cursor
    if (frame.cursorY < this.rows && frame.cursorX < this.cols) {
      const cx = frame.cursorX * cw;
      const cy = frame.cursorY * ch;
      ctx.strokeStyle = '#ffffff';
      ctx.lineWidth = 1.5;
      ctx.strokeRect(cx + 0.5, cy + 0.5, cw - 1, ch - 1);
    }

    this.frameCount++;
  }

  private startPolling() {
    let lastCols = this.cols;
    let lastRows = this.rows;

    const poll = async () => {
      if (!this.engineId) return;
      try {
        const frame = await this.rpc.call<EngineScreenFrame>(
          'nativeEngine.snapshot',
          { engineId: this.engineId },
        );
        if (frame.cols !== lastCols || frame.rows !== lastRows) {
          lastCols = frame.cols;
          lastRows = frame.rows;
          this.cols = frame.cols;
          this.rows = frame.rows;
          this.setupCanvas();
        }
        this.drawFrame(frame);
      } catch {
        // engine may have exited
      }
      this.pollHandle = requestAnimationFrame(poll);
    };
    this.pollHandle = requestAnimationFrame(poll);
  }

  send(data: string) {
    if (!this.engineId || !this.rpc) return;
    void this.rpc.call('nativeEngine.write', {
      engineId: this.engineId,
      data: btoa(data),
    });
  }

  async close() {
    if (this.pollHandle != null) {
      cancelAnimationFrame(this.pollHandle);
      this.pollHandle = null;
    }
    if (this.resizeObserver) {
      this.resizeObserver.disconnect();
      this.resizeObserver = null;
    }
    if (this.engineId) {
      try {
        await this.rpc.call('nativeEngine.close', { engineId: this.engineId });
      } catch { /* ignore */ }
      this.engineId = null;
    }
  }
}
