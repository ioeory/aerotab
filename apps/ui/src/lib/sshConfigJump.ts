import type { SshAuth, SshProfileSpec } from './types';

export interface SshConfigEntry {
  alias: string;
  host: string;
  port: number;
  user: string | null;
  identity_file: string | null;
  proxy_jump?: string[];
}

function authForEntry(entry: SshConfigEntry): SshAuth {
  if (entry.identity_file) {
    return { PublicKey: { key_path: entry.identity_file } };
  }
  return 'Agent';
}

/** Build jump_via chain from OpenSSH ProxyJump aliases (one hop level deep). */
export function jumpViaFromSshConfig(
  entry: SshConfigEntry,
  catalog: SshConfigEntry[],
): SshProfileSpec[] {
  const hops: SshProfileSpec[] = [];
  for (const token of entry.proxy_jump ?? []) {
    const hop = catalog.find((e) => e.alias === token || e.host === token);
    if (!hop) continue;
    hops.push({
      host: hop.host,
      port: hop.port,
      user: hop.user ?? 'root',
      auth: authForEntry(hop),
      jump_via: [],
    });
  }
  return hops;
}

export function sshProfileFromSshConfig(
  entry: SshConfigEntry,
  catalog: SshConfigEntry[],
): SshProfileSpec {
  return {
    host: entry.host,
    port: entry.port,
    user: entry.user ?? 'root',
    auth: authForEntry(entry),
    jump_via: jumpViaFromSshConfig(entry, catalog),
  };
}
