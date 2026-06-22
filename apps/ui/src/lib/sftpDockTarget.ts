import type { RpcClient } from './rpc';
import type { SessionMeta, SshProfileSpec, StoredProfile } from './types';

export interface DockProfileCacheEntry {
  profileId: string;
  target: SftpDockTarget | null;
}

export function isSshPane(pane: SessionMeta | undefined): boolean {
  if (!pane) return false;
  return pane.kind === 'Ssh' || !!pane.sshProfile || !!pane.profileId;
}

/** Active pane, or sole pane when activePaneId is stale after restore/layout. */
export function resolveTabActivePane(
  panes: SessionMeta[],
  activePaneId: string,
): SessionMeta | undefined {
  const active = panes.find((pane) => pane.id === activePaneId);
  if (active) return active;
  if (panes.length === 1) return panes[0];
  return undefined;
}

export interface SftpDockTarget {
  name: string;
  ssh: SshProfileSpec;
  sudo?: boolean;
}

export type DockTargetState =
  | { status: 'ready'; target: SftpDockTarget }
  | { status: 'loading'; fallback: SftpDockTarget | null }
  | { status: 'empty' };

/** Synchronous dock target from pane, per-tab profile cache, and pinned fallback. */
export function resolveDockTargetState(
  dockOpen: boolean,
  pinned: SftpDockTarget | null | undefined,
  pane: SessionMeta | undefined,
  profileCache: DockProfileCacheEntry | undefined,
  profileLoading: boolean,
): DockTargetState {
  if (!dockOpen) return { status: 'empty' };

  if (pane?.sshProfile) {
    return {
      status: 'ready',
      target: pinned ?? { name: pane.title || 'SSH session', ssh: pane.sshProfile },
    };
  }

  if (pane?.profileId) {
    const entry = profileCache?.profileId === pane.profileId ? profileCache : undefined;
    if (entry?.target) return { status: 'ready', target: entry.target };
    if (entry && !entry.target && !profileLoading) {
      if (pinned) return { status: 'ready', target: pinned };
      return { status: 'empty' };
    }
    if (profileLoading) return { status: 'loading', fallback: pinned ?? null };
    if (pinned) return { status: 'ready', target: pinned };
    return { status: 'empty' };
  }

  if (pinned) return { status: 'ready', target: pinned };
  return { status: 'empty' };
}

export function dockTargetFromState(state: DockTargetState): SftpDockTarget | null {
  if (state.status === 'ready') return state.target;
  if (state.status === 'loading') return state.fallback;
  return null;
}

export function isDockTargetLoading(state: DockTargetState): boolean {
  return state.status === 'loading';
}

export async function fetchProfileDockTarget(
  rpc: RpcClient,
  profileId: string,
): Promise<SftpDockTarget | null> {
  const profile = await rpc.call<StoredProfile>('profile.get', { id: profileId });
  if (profile.kind !== 'ssh') return null;
  return { name: profile.name, ssh: profile.ssh };
}
