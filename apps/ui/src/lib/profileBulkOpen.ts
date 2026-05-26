import type { RpcClient } from './rpc';
import { tabs, type SplitDir } from './tabs.svelte';
import type { SessionMeta, StoredProfile } from './types';

export const BULK_OPEN_CONFIRM_THRESHOLD = 4;

export interface ProfileBulkOpenDeps {
  rpc: RpcClient;
  onError: (msg: string) => void;
  confirmMany?: (count: number) => Promise<boolean>;
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
      const meta = await rpc.call<SessionMeta>('session.openSsh', {
        title: p.name,
        rows: 24,
        cols: 80,
        profile: p.ssh,
      });
      tabs.add({ ...meta, profileId: p.id, sshProfile: p.ssh });
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
  }

  for (const p of remoteList) {
    try {
      await rpc.call('remote.openProfile', { profile_id: p.id });
    } catch (e) {
      onError(`remote: ${(e as Error).message}`);
    }
  }
  if (sshList.length === 0) return;

  let tabId = tabs.activeId ?? undefined;
  let tab = tabId ? tabs.tabs.find((t) => t.id === tabId) : undefined;

  for (let i = 0; i < sshList.length; i++) {
    const p = sshList[i]!;
    const meta = await rpc.call<SessionMeta>('session.openSsh', {
      title: p.name,
      rows: 24,
      cols: 80,
      profile: p.ssh,
    });
    const pane = { ...meta, profileId: p.id, sshProfile: p.ssh };
    if (i === 0 && !tab) {
      tabs.add(pane);
      tabId = tabs.activeId ?? undefined;
      tab = tabId ? tabs.tabs.find((t) => t.id === tabId) : undefined;
    } else if (tabId) {
      const direction: SplitDir = i % 2 === 0 ? 'row' : 'col';
      tabs.addPane(tabId, pane, direction);
    }
  }
  if (tabId) tabs.activate(tabId);
}
