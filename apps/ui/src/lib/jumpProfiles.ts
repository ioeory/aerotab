import type { RpcClient } from './rpc';
import type { SshAuth, SshProfileSpec, StoredProfile } from './types';

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

/** Format a saved SSH profile as a ProxyJump line (`@Name`). */
export function jumpLineForProfile(profile: StoredProfile): string {
  return `@${profile.name}`;
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
