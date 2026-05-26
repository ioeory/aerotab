import { tick } from 'svelte';
import { i18n } from './i18n.svelte';
import type { RpcClient } from './rpc';
import { withRpcTimeout } from './rpcTimeout';
import { profileEndpointLabel } from './profileMeta';
import { tabs } from './tabs.svelte';
import type { SessionMeta, StoredProfile } from './types';

export const BULK_OPEN_CONFIRM_THRESHOLD = 4;
/** Per-profile SSH connect timeout during bulk open (unreachable hosts must not block the rest). */
const BULK_SSH_CONNECT_TIMEOUT_MS = 20_000;

export type BulkOpenTabTarget = 'active' | 'new';

export interface ProfileBulkOpenDeps {
  rpc: RpcClient;
  onError: (msg: string) => void;
  /** Shown when some profiles fail (multi-line: profile, host, error). */
  onSummary?: (message: string) => void | Promise<void>;
  confirmMany?: (count: number) => Promise<boolean>;
}

/** `name (user@host)` for bulk-open status and dialogs. */
export function profileBulkLabel(profile: StoredProfile): string {
  return `${profile.name} (${profileEndpointLabel(profile)})`;
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
    profileBulkLabel(p),
  );
}

type SshOpenOutcome =
  | { ok: true; meta: SessionMeta }
  | { ok: false; profile: StoredProfile; error: string };

function formatFailureLine(f: Extract<SshOpenOutcome, { ok: false }>): string {
  return `${profileBulkLabel(f.profile)}: ${f.error}`;
}

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
          profile: p,
          error: (e as Error).message,
        }));
    }),
  );
}

function reportBulkOpenResult(
  openedCount: number,
  total: number,
  failures: Extract<SshOpenOutcome, { ok: false }>[],
  deps: ProfileBulkOpenDeps,
): void {
  if (failures.length === 0) return;
  const lines = failures.map(formatFailureLine);
  const header = i18n.t('profiles.bulkOpenPartialTitle', { opened: openedCount, total });
  const body = lines.join('\n');
  const message = `${header}\n\n${body}`;
  if (deps.onSummary) {
    void Promise.resolve(deps.onSummary(message));
  } else {
    deps.onError(`${header} ${lines.slice(0, 3).join('; ')}`);
  }
}

async function maybeConfirmMany(
  sshCount: number,
  deps: ProfileBulkOpenDeps,
): Promise<boolean> {
  if (sshCount <= BULK_OPEN_CONFIRM_THRESHOLD || !deps.confirmMany) return true;
  const ok = await deps.confirmMany(sshCount);
  if (!ok) return false;
  await tick();
  return true;
}

/** Open each profile in its own tab (SSH pane or system remote viewer). */
export async function openProfilesEachInNewTab(
  list: StoredProfile[],
  deps: ProfileBulkOpenDeps,
): Promise<void> {
  const { rpc, onError } = deps;
  const sshList = list.filter((p) => p.kind === 'ssh');
  const remoteList = list.filter((p) => p.kind === 'rdp' || p.kind === 'vnc');

  if (!(await maybeConfirmMany(sshList.length, deps))) return;

  const remoteFailures: Extract<SshOpenOutcome, { ok: false }>[] = [];
  for (const p of remoteList) {
    try {
      await rpc.call('remote.openProfile', { profile_id: p.id });
    } catch (e) {
      remoteFailures.push({ ok: false, profile: p, error: (e as Error).message });
    }
  }

  const outcomes = sshList.length > 0 ? await openSshProfilesParallel(rpc, sshList) : [];
  const failures = [
    ...remoteFailures,
    ...outcomes.filter((o): o is Extract<SshOpenOutcome, { ok: false }> => !o.ok),
  ];
  let openedCount = remoteList.length - remoteFailures.length;

  for (const o of outcomes) {
    if (!o.ok) continue;
    tabs.add(o.meta);
    openedCount += 1;
  }

  if (openedCount === 0) {
    const detail = failures.map(formatFailureLine).slice(0, 5).join('\n');
    onError(detail || 'Open failed');
    return;
  }

  reportBulkOpenResult(openedCount, list.length, failures, deps);
}

/** Open SSH profiles in one tab (split panes); RDP/VNC via system viewer. */
export async function openProfilesInSameTab(
  list: StoredProfile[],
  deps: ProfileBulkOpenDeps,
  options?: { tabTarget?: BulkOpenTabTarget },
): Promise<void> {
  const { rpc, onError } = deps;
  const tabTarget = options?.tabTarget ?? 'active';
  const sshList = list.filter((p) => p.kind === 'ssh');
  const remoteList = list.filter((p) => p.kind === 'rdp' || p.kind === 'vnc');
  if (sshList.length === 0 && remoteList.length === 0) return;

  if (!(await maybeConfirmMany(sshList.length, deps))) return;

  const remoteFailures: Extract<SshOpenOutcome, { ok: false }>[] = [];
  for (const p of remoteList) {
    try {
      await rpc.call('remote.openProfile', { profile_id: p.id });
    } catch (e) {
      remoteFailures.push({ ok: false, profile: p, error: (e as Error).message });
    }
  }
  if (sshList.length === 0) {
    reportBulkOpenResult(
      remoteList.length - remoteFailures.length,
      list.length,
      remoteFailures,
      deps,
    );
    return;
  }

  const outcomes = await openSshProfilesParallel(rpc, sshList);
  const opened = outcomes.filter((o): o is Extract<SshOpenOutcome, { ok: true }> => o.ok).map((o) => o.meta);
  const failures = [
    ...remoteFailures,
    ...outcomes.filter((o): o is Extract<SshOpenOutcome, { ok: false }> => !o.ok),
  ];

  if (opened.length === 0) {
    const detail = failures.map(formatFailureLine).slice(0, 8).join('\n');
    onError(detail || 'SSH open failed');
    return;
  }

  const tabTitle = opened.length === 1 ? opened[0]!.title : `SSH ×${opened.length}`;
  const preferTabId = tabTarget === 'new' ? undefined : tabs.activeId;
  const tab = tabs.addSessionsInTab(opened, tabTitle, preferTabId);
  if (!tab) {
    onError('Failed to add sessions to tab layout');
    return;
  }

  reportBulkOpenResult(opened.length + (remoteList.length - remoteFailures.length), list.length, failures, deps);
}
