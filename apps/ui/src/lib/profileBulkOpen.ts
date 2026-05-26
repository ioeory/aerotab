import { tick } from 'svelte';
import type { RpcClient } from './rpc';
import { withRpcTimeout } from './rpcTimeout';
import { tabs } from './tabs.svelte';
import type { SessionMeta, StoredProfile } from './types';

export const BULK_OPEN_CONFIRM_THRESHOLD = 4;
/** Per-profile SSH connect timeout during bulk open (unreachable hosts must not block the rest). */
const BULK_SSH_CONNECT_TIMEOUT_MS = 20_000;

export interface ProfileBulkOpenDeps {
  rpc: RpcClient;
  onError: (msg: string) => void;
  confirmMany?: (count: number) => Promise<boolean>;
}

async function openSshSession(rpc: RpcClient, p: StoredProfile & { kind: 'ssh' }): Promise<SessionMeta> {
  const meta = await rpc.call<SessionMeta>('session.openSshProfile', { profile_id: p.id });
  return { ...meta, profileId: p.id, sshProfile: p.ssh };
}

async function tryOpenSshSession(
  rpc: RpcClient,
  p: StoredProfile & { kind: 'ssh' },
): Promise<SessionMeta> {
  return withRpcTimeout(
    openSshSession(rpc, p),
    BULK_SSH_CONNECT_TIMEOUT_MS,
    p.name,
  );
}

type SshOpenOutcome =
  | { ok: true; meta: SessionMeta }
  | { ok: false; name: string; error: string };

async function openSshProfilesParallel(
  rpc: RpcClient,
  sshList: StoredProfile[],
): Promise<SshOpenOutcome[]> {
  return Promise.all(
    sshList.map((p) => {
      const ssh = p as StoredProfile & { kind: 'ssh' };
      return tryOpenSshSession(rpc, ssh)
        .then((meta): SshOpenOutcome => ({ ok: true, meta }))
        .catch((e): SshOpenOutcome => ({
          ok: false,
          name: p.name,
          error: (e as Error).message,
        }));
    }),
  );
}

/** Open each profile in its own tab (SSH pane or system remote viewer). */
export async function openProfilesEachInNewTab(
  list: StoredProfile[],
  deps: ProfileBulkOpenDeps,
): Promise<void> {
  const { rpc, onError } = deps;
  for (const p of list) {
    try {
      if (p.kind === 'rdp' || p.kind === 'vnc') {
        await rpc.call('remote.openProfile', { profile_id: p.id });
        continue;
      }
      tabs.add(await tryOpenSshSession(rpc, p as StoredProfile & { kind: 'ssh' }));
    } catch (e) {
      onError(`${p.name}: ${(e as Error).message}`);
    }
  }
}

/** Open SSH profiles in the active tab (split panes); RDP/VNC via system viewer. */
export async function openProfilesInSameTab(
  list: StoredProfile[],
  deps: ProfileBulkOpenDeps,
): Promise<void> {
  const { rpc, onError, confirmMany } = deps;
  const sshList = list.filter((p) => p.kind === 'ssh');
  const remoteList = list.filter((p) => p.kind === 'rdp' || p.kind === 'vnc');
  if (sshList.length === 0 && remoteList.length === 0) return;

  if (sshList.length > BULK_OPEN_CONFIRM_THRESHOLD && confirmMany) {
    const ok = await confirmMany(sshList.length);
    if (!ok) return;
    await tick();
  }

  for (const p of remoteList) {
    try {
      await rpc.call('remote.openProfile', { profile_id: p.id });
    } catch (e) {
      onError(`remote: ${(e as Error).message}`);
    }
  }
  if (sshList.length === 0) return;

  const outcomes = await openSshProfilesParallel(rpc, sshList);
  const opened = outcomes.filter((o): o is Extract<SshOpenOutcome, { ok: true }> => o.ok).map((o) => o.meta);
  const failures = outcomes.filter((o): o is Extract<SshOpenOutcome, { ok: false }> => !o.ok);

  if (opened.length === 0) {
    const detail = failures.map((f) => `${f.name}: ${f.error}`).slice(0, 5).join('; ');
    onError(detail || 'SSH open failed');
    return;
  }

  const tabTitle = opened.length === 1 ? opened[0]!.title : `SSH ×${opened.length}`;
  const tab = tabs.addSessionsInTab(opened, tabTitle, tabs.activeId);
  if (!tab) {
    onError('Failed to add sessions to tab layout');
    return;
  }

  if (failures.length > 0) {
    const detail = failures.map((f) => `${f.name}: ${f.error}`).slice(0, 5).join('; ');
    onError(`Opened ${opened.length}/${sshList.length}. ${detail}`);
  }
}
