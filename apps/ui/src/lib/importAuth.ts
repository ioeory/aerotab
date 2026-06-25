import type { ImportCandidate } from './importTypes';
import type { SshAuth, SshProfileSpec, StoredProfile } from './types';

export type ImportBatchAuthMode = 'keep' | 'password' | 'key' | 'agent' | 'vault';

export interface ImportBatchAuthConfig {
  userOverride: string;
  mode: ImportBatchAuthMode;
  password: string;
  keyPath: string;
  keyPassphrase: string;
  vaultEntryId: string;
  vaultPassphraseEntryId: string;
}

export function sshEndpointKey(ssh: SshProfileSpec): string {
  return `ssh:${ssh.user.toLowerCase()}@${ssh.host.toLowerCase()}:${ssh.port}`;
}

export function buildBatchAuth(config: ImportBatchAuthConfig): SshAuth | null {
  switch (config.mode) {
    case 'keep':
      return null;
    case 'password':
      return { Password: { secret: config.password } };
    case 'key':
      return {
        PublicKey: {
          key_path: config.keyPath.trim(),
          passphrase: config.keyPassphrase.trim() || undefined,
        },
      };
    case 'agent':
      return 'Agent';
    case 'vault':
      return {
        VaultRef: {
          entry_id: config.vaultEntryId.trim(),
          passphrase_entry_id: config.vaultPassphraseEntryId.trim() || undefined,
        },
      };
    default:
      return null;
  }
}

function cloneAuth(auth: SshAuth): SshAuth {
  if (auth === 'Agent') return 'Agent';
  return JSON.parse(JSON.stringify(auth)) as SshAuth;
}

export function applyBatchAuthToCandidates(
  candidates: ImportCandidate[],
  selectedIds: Set<string>,
  config: ImportBatchAuthConfig,
): number {
  const auth = buildBatchAuth(config);
  const user = config.userOverride.trim();
  let count = 0;
  for (const c of candidates) {
    if (!selectedIds.has(c.sourceId)) continue;
    if (c.status === 'error' || !c.profile || c.profile.kind !== 'ssh') continue;
    if (user) c.profile.ssh.user = user;
    if (auth) c.profile.ssh.auth = cloneAuth(auth);
    if (user || auth) count += 1;
  }
  return count;
}

export function matchAuthFromExistingProfiles(
  candidates: ImportCandidate[],
  selectedIds: Set<string>,
  existing: StoredProfile[],
): { matched: number; unmatched: number } {
  const byEndpoint = new Map<string, StoredProfile>();
  for (const p of existing) {
    if (p.kind !== 'ssh') continue;
    byEndpoint.set(sshEndpointKey(p.ssh), p);
  }

  let matched = 0;
  let unmatched = 0;
  for (const c of candidates) {
    if (!selectedIds.has(c.sourceId)) continue;
    if (c.status === 'error' || !c.profile || c.profile.kind !== 'ssh') continue;
    const hit = byEndpoint.get(sshEndpointKey(c.profile.ssh));
    if (hit?.kind === 'ssh') {
      c.profile.ssh.user = hit.ssh.user;
      c.profile.ssh.auth = cloneAuth(hit.ssh.auth);
      matched += 1;
    } else {
      unmatched += 1;
    }
  }
  return { matched, unmatched };
}

export interface ImportApplyItemPayload {
  sourceId: string;
  overwrite: boolean;
  user?: string;
  auth?: SshAuth;
}

export function buildImportApplyItems(
  candidates: ImportCandidate[],
  selectedIds: Set<string>,
): ImportApplyItemPayload[] {
  const items: ImportApplyItemPayload[] = [];
  for (const sourceId of selectedIds) {
    const row = candidates.find((c) => c.sourceId === sourceId);
    if (!row) continue;
    const item: ImportApplyItemPayload = {
      sourceId,
      overwrite: row.status === 'duplicate',
    };
    if (row.profile?.kind === 'ssh') {
      item.user = row.profile.ssh.user;
      item.auth = row.profile.ssh.auth;
    }
    items.push(item);
  }
  return items;
}
