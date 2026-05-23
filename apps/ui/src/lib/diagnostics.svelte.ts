import { b64encode, tauriInvoke, type RpcClient } from './rpc';

export type DiagnosticCategory = 'connection' | 'sync' | 'transfer' | 'profile' | 'workspace' | 'settings' | 'update' | 'app';
export type DiagnosticLevel = 'info' | 'warn' | 'error';

export interface DiagnosticEvent {
  id: string;
  ts: string;
  level: DiagnosticLevel;
  category: DiagnosticCategory;
  source: string;
  message: string;
}

interface DiagnosticPack {
  schemaVersion: 1;
  exportedAt: string;
  app: {
    buildId: string;
    coreVersion: string | null;
    userAgent: string;
    platform: string;
    language: string;
  };
  summary: Record<DiagnosticCategory, number>;
  events: DiagnosticEvent[];
}

const STORAGE_KEY = 'tabby.diagnostics.events.v1';
const MAX_EVENTS = 250;

function safeRandomId(): string {
  if (crypto.randomUUID) return crypto.randomUUID();
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function sanitizeMessage(value: unknown): string {
  const text = value instanceof Error ? value.message : String(value ?? '');
  return text
    .replace(/(password|passphrase|secret|token|private[_ -]?key)\s*[:=]\s*[^\s,;]+/gi, '$1=[redacted]')
    .replace(/-----BEGIN [^-]+PRIVATE KEY-----[\s\S]*?-----END [^-]+PRIVATE KEY-----/g, '[redacted-private-key]')
    .replace(/(Authorization:\s*)(Bearer|Basic)\s+[^\s]+/gi, '$1[redacted]')
    .slice(0, 700);
}

function loadInitial(): DiagnosticEvent[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    const parsed = raw ? JSON.parse(raw) : [];
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((event): event is DiagnosticEvent => (
      event && typeof event === 'object'
      && typeof event.id === 'string'
      && typeof event.ts === 'string'
      && typeof event.message === 'string'
    )).slice(-MAX_EVENTS);
  } catch {
    return [];
  }
}

function persist(events: DiagnosticEvent[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(events.slice(-MAX_EVENTS)));
  } catch {
    // Best effort only. Diagnostics must never break the primary workflow.
  }
}

class DiagnosticsStore {
  events = $state<DiagnosticEvent[]>(loadInitial());

  record(category: DiagnosticCategory, source: string, message: unknown, level: DiagnosticLevel = 'error') {
    const next = [
      ...this.events,
      {
        id: safeRandomId(),
        ts: new Date().toISOString(),
        level,
        category,
        source,
        message: sanitizeMessage(message),
      },
    ].slice(-MAX_EVENTS);
    this.events = next;
    persist(next);
  }

  clear() {
    this.events = [];
    persist([]);
  }

  summary() {
    const out: Record<DiagnosticCategory, number> = {
      connection: 0,
      sync: 0,
      transfer: 0,
      profile: 0,
      workspace: 0,
      settings: 0,
      update: 0,
      app: 0,
    };
    for (const event of this.events) out[event.category] += 1;
    return out;
  }

  pack(buildId: string, coreVersion: string | null): DiagnosticPack {
    return {
      schemaVersion: 1,
      exportedAt: new Date().toISOString(),
      app: {
        buildId,
        coreVersion,
        userAgent: navigator.userAgent,
        platform: navigator.platform,
        language: navigator.language,
      },
      summary: this.summary(),
      events: this.events,
    };
  }
}

export const diagnostics = new DiagnosticsStore();

export function categoryForRpcMethod(method: string): DiagnosticCategory {
  if (method.startsWith('sync.')) return 'sync';
  if (method.startsWith('sftp.')) return 'transfer';
  if (method.startsWith('profile.')) return 'profile';
  if (method.startsWith('settings.') || method.startsWith('secret.') || method.startsWith('vault.')) return 'settings';
  if (method.startsWith('session.openSsh') || method.startsWith('ssh.')) return 'connection';
  return 'app';
}

export function categoryForStatus(message: string): DiagnosticCategory {
  const text = message.toLowerCase();
  if (text.includes('sync')) return 'sync';
  if (text.includes('sftp') || text.includes('transfer') || text.includes('upload') || text.includes('download')) return 'transfer';
  if (text.includes('profile') || text.includes('connect') || text.includes('ssh')) return 'connection';
  if (text.includes('workspace')) return 'workspace';
  if (text.includes('setting') || text.includes('vault') || text.includes('secret')) return 'settings';
  if (text.includes('update')) return 'update';
  return 'app';
}

export function instrumentRpcClient(client: RpcClient): RpcClient {
  return {
    async call<T = unknown>(method: string, params?: unknown): Promise<T> {
      try {
        return await client.call<T>(method, params);
      } catch (error) {
        diagnostics.record(categoryForRpcMethod(method), method, error, 'error');
        throw error;
      }
    },
  };
}

export async function exportDiagnosticPack(buildId: string, coreVersion: string | null): Promise<'saved' | 'downloaded' | 'cancelled'> {
  const pack = JSON.stringify(diagnostics.pack(buildId, coreVersion), null, 2);
  const filename = `tabby-diagnostics-${new Date().toISOString().replace(/[:.]/g, '-')}.json`;
  const pick = tauriInvoke<string | null>('pick_save_file', { defaultName: filename });
  if (pick) {
    const path = await pick;
    if (!path) return 'cancelled';
    const write = tauriInvoke<void>('local_write_chunk', {
      path,
      offset: 0,
      data: b64encode(new TextEncoder().encode(pack)),
      create: true,
    });
    if (!write) throw new Error('desktop file writer is not available');
    await write;
    return 'saved';
  }

  const blob = new Blob([pack], { type: 'application/json' });
  const href = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = href;
  link.download = filename;
  document.body.appendChild(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(href);
  return 'downloaded';
}
