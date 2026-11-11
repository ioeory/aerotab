import type { RpcClient } from './rpc';
import type { tabs as tabStore, SplitDir } from './tabs.svelte';
import type { SessionMeta, StoredProfile } from './types';

export interface ProfileSplitTarget { tabId: string; paneId: string; direction: SplitDir }
export interface ProfileConnectionDeps {
  rpc: RpcClient;
  tabs: typeof tabStore;
  uuid: () => string;
  onStart: () => void;
  onError: (message: string) => void;
  onConnected: (session: SessionMeta) => void;
}

async function completeProfileConnection(pane: SessionMeta, deps: ProfileConnectionDeps): Promise<boolean> {
  const { tabs, rpc } = deps;
  try {
    // Resolve saved inheritance and Vault credentials in the backend, never from a UI snapshot.
    const meta = await rpc.call<SessionMeta>('session.openSshProfile', { profile_id: pane.profileId });
    const owner = tabs.tabOf(pane.id);
    if (!owner) {
      await rpc.call('session.close', { id: meta.id });
      return false;
    }
    const session: SessionMeta = { id: meta.id, kind: meta.kind, title: meta.title, profileId: pane.profileId };
    tabs.replacePaneSession(owner.id, pane.id, session);
    deps.onConnected(session);
    return true;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const owner = tabs.tabOf(pane.id);
    if (owner) tabs.replacePaneSession(owner.id, pane.id, { ...pane, connection: { status: 'failed', error: message } });
    deps.onError(message);
    return false;
  }
}

export async function openProfileConnection(profile: StoredProfile, target: ProfileSplitTarget | undefined, deps: ProfileConnectionDeps): Promise<boolean> {
  if (profile.kind === 'rdp' || profile.kind === 'vnc') {
    if (target) return false;
    try { await deps.rpc.call('remote.openProfile', { profile_id: profile.id }); return true; }
    catch (error) { deps.onError(error instanceof Error ? error.message : String(error)); return false; }
  }
  const pane: SessionMeta = { id: `pending-profile:${deps.uuid()}`, kind: 'PendingSsh', title: profile.name, profileId: profile.id, connection: { status: 'connecting' } };
  if (target) {
    if (!deps.tabs.addPane(target.tabId, pane, target.direction, 'after', target.paneId)) return false;
  } else deps.tabs.add(pane);
  deps.onStart();
  return completeProfileConnection(pane, deps);
}

export async function retryProfileConnection(pane: SessionMeta, deps: ProfileConnectionDeps): Promise<boolean> {
  const owner = deps.tabs.tabOf(pane.id);
  const current = owner?.panes.find(candidate => candidate.id === pane.id);
  if (!owner || !current?.profileId || current.connection?.status !== 'failed') return false;
  const connecting: SessionMeta = { ...current, connection: { status: 'connecting' } };
  deps.tabs.replacePaneSession(owner.id, pane.id, connecting);
  deps.onStart();
  return completeProfileConnection(connecting, deps);
}
