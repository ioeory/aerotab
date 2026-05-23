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

export async function loadProfilesForJumps(rpc: RpcClient): Promise<StoredProfile[]> {
  try {
    const list = await rpc.call<StoredProfile[]>('profile.list');
    return list.filter((p) => p.kind === 'ssh');
  } catch {
    return [];
  }
}
