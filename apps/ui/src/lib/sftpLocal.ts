import type { LocalEntry } from './types';

export const SFTP_DRAG_LOCAL = 'application/x-aerotab-sftp-local';
export const SFTP_DRAG_REMOTE = 'application/x-aerotab-sftp-remote';

export interface LocalDragPayload {
  path: string;
  name: string;
  kind: LocalEntry['kind'];
  size: number;
}

export interface RemoteDragPayload {
  path: string;
  name: string;
  kind: 'File' | 'Dir';
  size: number;
}

export function joinLocalPath(base: string, name: string): string {
  const sep = base.includes('\\') ? '\\' : '/';
  const trimmed = base.replace(/[/\\]+$/, '');
  if (!trimmed) return name;
  return `${trimmed}${sep}${name}`;
}

export function parentLocalPath(p: string): string {
  const sep = p.includes('\\') ? '\\' : '/';
  const parts = p.split(/[/\\]/).filter(Boolean);
  if (parts.length === 0) return p;
  if (parts.length === 1) {
    const head = parts[0]!;
    return head.includes(':') ? `${head}\\` : sep;
  }
  parts.pop();
  return parts.join(sep);
}

export function localBreadcrumbs(cwd: string): { label: string; path: string }[] {
  const win = cwd.includes('\\') || /^[A-Za-z]:/.test(cwd);
  if (win) {
    const normalized = cwd.replace(/\//g, '\\');
    const parts = normalized.split('\\').filter(Boolean);
    if (parts.length === 0) return [{ label: cwd, path: cwd }];
    const out: { label: string; path: string }[] = [];
    const first = parts[0]!;
    let acc = first.includes(':') ? `${first}\\` : first;
    out.push({ label: first, path: acc });
    for (const part of parts.slice(1)) {
      acc = joinLocalPath(acc, part);
      out.push({ label: part, path: acc });
    }
    return out;
  }
  const parts = cwd.split('/').filter(Boolean);
  const out: { label: string; path: string }[] = [{ label: '/', path: '/' }];
  let acc = '';
  for (const part of parts) {
    acc = acc ? `${acc}/${part}` : `/${part}`;
    out.push({ label: part, path: acc });
  }
  return out.length > 1 ? out : [{ label: '/', path: cwd || '/' }];
}

export function parseLocalDrag(data: string): LocalDragPayload | null {
  try {
    return JSON.parse(data) as LocalDragPayload;
  } catch {
    return null;
  }
}

export function parseRemoteDrag(data: string): RemoteDragPayload | null {
  try {
    return JSON.parse(data) as RemoteDragPayload;
  } catch {
    return null;
  }
}
