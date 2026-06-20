import type { RpcClient } from './rpc';
import type { SshAuth, SshProfileSpec, StoredProfile } from './types';

/** Find a saved SSH profile matching a bastion hop (host + port + user). */
export function findProfileForJumpHop(
  host: string,
  port: number,
  user: string,
  profiles: StoredProfile[],
): (StoredProfile & { kind: 'ssh' }) | undefined {
  const hit = profiles.find(
    (p) => p.kind === 'ssh'
      && p.ssh.host === host
      && p.ssh.port === port
      && p.ssh.user === user,
  );
  return hit?.kind === 'ssh' ? hit : undefined;
}

/** Format jump hops for the profile editor; prefer `@ProfileName` when a saved profile matches. */
export function formatJumpLinesForEdit(jumps: SshProfileSpec[], profiles: StoredProfile[]): string {
  return jumps
    .map((j) => {
      const hit = findProfileForJumpHop(j.host, j.port, j.user, profiles);
      if (hit) return jumpLineForProfile(hit);
      return `${j.user}@${j.host}${j.port === 22 ? '' : ':' + j.port}`;
    })
    .join('\n');
}

/** Parse one bastion line: `user@host[:port]` or `@profile-id` / `@Profile Name`. */
export function parseJumpLine(
  line: string,
  auth: SshAuth,
  profiles: StoredProfile[],
): SshProfileSpec {
  const trimmed = line.trim();
  if (trimmed.startsWith('@')) {
    const ref = trimmed.slice(1).trim();
    const byId = profiles.find((p) => p.id === ref);
    const byName = profiles.find((p) => p.name === ref);
    const hit = byId ?? byName;
    if (!hit || hit.kind !== 'ssh') throw new Error(`jump profile "${ref}" not found`);
    return {
      host: hit.ssh.host,
      port: hit.ssh.port,
      user: hit.ssh.user,
      auth: hit.ssh.auth,
      jump_via: [],
    };
  }
  const at = trimmed.indexOf('@');
  if (at < 0) throw new Error(`jump host "${trimmed}" missing user@`);
  const u = trimmed.slice(0, at);
  const rest = trimmed.slice(at + 1);
  const colon = rest.lastIndexOf(':');
  const h = colon >= 0 ? rest.slice(0, colon) : rest;
  const p = colon >= 0 ? Number(rest.slice(colon + 1)) || 22 : 22;
  const matched = findProfileForJumpHop(h, p, u, profiles);
  if (matched) {
    return {
      host: matched.ssh.host,
      port: matched.ssh.port,
      user: matched.ssh.user,
      auth: matched.ssh.auth,
      jump_via: [],
    };
  }
  return { host: h, port: p, user: u, auth, jump_via: [] };
}

export function parseJumpLines(
  text: string,
  auth: SshAuth,
  profiles: StoredProfile[],
): SshProfileSpec[] {
  return text
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0)
    .map((line) => parseJumpLine(line, auth, profiles));
}

export function splitJumpLines(text: string): string[] {
  return text
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
}

export function joinJumpLines(lines: string[]): string {
  return lines.join('\n');
}

/** Whether a saved profile is already referenced in the jump chain. */
export function isProfileInJumpChain(profile: StoredProfile, chain: string[]): boolean {
  if (profile.kind !== 'ssh') return false;
  const byName = jumpLineForProfile(profile).toLowerCase();
  const byId = `@${profile.id}`.toLowerCase();
  return chain.some((line) => {
    const lower = line.toLowerCase();
    return lower === byName || lower === byId;
  });
}

/** Display title for one jump chain line in the editor. */
export function jumpLineTitle(line: string, profiles: StoredProfile[]): string {
  const trimmed = line.trim();
  if (!trimmed.startsWith('@')) return trimmed;
  const ref = trimmed.slice(1).trim();
  const hit = profiles.find((p) => p.id === ref || p.name === ref);
  if (hit && hit.kind === 'ssh') return hit.name;
  return ref;
}

/** Endpoint subtitle for one jump chain line. */
export function jumpLineSubtitle(line: string, profiles: StoredProfile[]): string {
  const trimmed = line.trim();
  if (trimmed.startsWith('@')) {
    const ref = trimmed.slice(1).trim();
    const hit = profiles.find((p) => p.id === ref || p.name === ref);
    if (hit && hit.kind === 'ssh') {
      const portSuffix = hit.ssh.port === 22 ? '' : `:${hit.ssh.port}`;
      return `${hit.ssh.user}@${hit.ssh.host}${portSuffix}`;
    }
    return trimmed;
  }
  return trimmed;
}

/** Match profile against picker search (name, group, host, port, user, tags). */
export function profileMatchesJumpSearch(profile: StoredProfile, query: string): boolean {
  if (profile.kind !== 'ssh') return false;
  const needle = query.trim().toLowerCase();
  if (!needle) return true;
  const hay = [
    profile.name,
    profile.group ?? '',
    profile.ssh.host,
    String(profile.ssh.port),
    profile.ssh.user,
    ...(profile.tags ?? []),
  ].join(' ').toLowerCase();
  return hay.includes(needle);
}

/** Format a saved SSH profile as a ProxyJump line (`@Name`). */
export function jumpLineForProfile(profile: StoredProfile): string {
  return `@${profile.name}`;
}

function hopMatches(a: SshProfileSpec, b: SshProfileSpec): boolean {
  return a.host === b.host && a.port === b.port && a.user === b.user;
}

/** One-time SSH spec: connect to `target` via `jump` as the first hop. */
export function profileSpecViaJump(
  target: StoredProfile & { kind: 'ssh' },
  jump: StoredProfile & { kind: 'ssh' },
  profiles: StoredProfile[],
): SshProfileSpec {
  const hop = parseJumpLine(jumpLineForProfile(jump), target.ssh.auth, profiles);
  const existing = target.ssh.jump_via ?? [];
  if (
    hopMatches(hop, target.ssh)
    || existing.some((h) => hopMatches(h, hop))
  ) {
    return { ...target.ssh };
  }
  return { ...target.ssh, jump_via: [hop, ...existing] };
}

/** SSH profiles that can be selected as a one-time jump host for `target`. */
export function jumpHostCandidates(
  target: StoredProfile,
  profiles: StoredProfile[],
): Array<StoredProfile & { kind: 'ssh' }> {
  if (target.kind !== 'ssh') return [];
  return profiles.filter(
    (p): p is StoredProfile & { kind: 'ssh' } =>
      p.kind === 'ssh' && p.id !== target.id,
  );
}

export function reorderJumpLines(lines: string[], from: number, to: number): string[] {
  if (from === to || from < 0 || to < 0 || from >= lines.length || to >= lines.length) return lines;
  const next = lines.slice();
  const [item] = next.splice(from, 1);
  if (!item) return lines;
  next.splice(to, 0, item);
  return next;
}

export function formatManualJumpLine(user: string, host: string, port: number): string {
  const u = user.trim();
  const h = host.trim();
  const p = Number(port) || 22;
  if (!u || !h) throw new Error('jump host requires user and host');
  return `${u}@${h}${p === 22 ? '' : `:${p}`}`;
}

/** Resolve profiles in the given id order (skips unknown ids). */
export function profilesInSelectionOrder(
  profiles: StoredProfile[],
  orderedIds: string[],
): StoredProfile[] {
  const byId = new Map(profiles.map((p) => [p.id, p]));
  return orderedIds
    .map((id) => byId.get(id))
    .filter((p): p is StoredProfile => !!p && p.kind === 'ssh');
}

/** Merge profile jump lines into existing textarea content (deduped). */
export function appendJumpProfileLines(existingText: string, profiles: StoredProfile[]): string {
  const lines = existingText
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
  const seen = new Set(lines.map((l) => l.toLowerCase()));
  for (const p of profiles) {
    if (p.kind !== 'ssh') continue;
    const line = jumpLineForProfile(p);
    const key = line.toLowerCase();
    if (seen.has(key)) continue;
    lines.push(line);
    seen.add(key);
  }
  return lines.join('\n');
}

export async function loadProfilesForJumps(rpc: RpcClient): Promise<StoredProfile[]> {
  try {
    const list = await rpc.call<StoredProfile[]>('profile.list');
    return list.filter((p) => p.kind === 'ssh');
  } catch {
    return [];
  }
}
