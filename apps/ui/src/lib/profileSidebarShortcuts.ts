import { ACTIONS, hotkeys } from './hotkeys';
import type { StoredProfile } from './types';

/** Hotkey action ids for sidebar profile row shortcuts (see Settings → Hotkeys → Profiles). */
export const PROFILE_SIDEBAR_ACTION_IDS = {
  edit: 'profile-edit',
  clone: 'profile-clone',
  remove: 'profile-remove',
  openNewTab: 'profile-open-tab',
  splitRight: 'profile-split-right',
  splitDown: 'profile-split-down',
  sftp: 'profile-open-sftp',
} as const;

export type ProfileSidebarActionKey = keyof typeof PROFILE_SIDEBAR_ACTION_IDS;

const ACTION_ORDER: Array<{
  key: ProfileSidebarActionKey;
  when?: (p: StoredProfile) => boolean;
}> = [
  { key: 'edit' },
  { key: 'clone' },
  { key: 'remove' },
  { key: 'openNewTab' },
  { key: 'splitRight' },
  { key: 'splitDown' },
  { key: 'sftp', when: (p) => p.kind === 'ssh' },
];

export interface ProfileSidebarShortcutHandlers {
  onEdit: (p: StoredProfile) => void;
  onClone: (p: StoredProfile) => void;
  onRemove: (p: StoredProfile) => void;
  onOpenNewTab: (p: StoredProfile) => void;
  onSplitRight: (p: StoredProfile) => void;
  onSplitDown: (p: StoredProfile) => void;
  onOpenSftp?: (p: StoredProfile) => void;
}

const HANDLER_BY_KEY: Record<
  ProfileSidebarActionKey,
  (handlers: ProfileSidebarShortcutHandlers, p: StoredProfile) => void
> = {
  edit: (h, p) => h.onEdit(p),
  clone: (h, p) => h.onClone(p),
  remove: (h, p) => h.onRemove(p),
  openNewTab: (h, p) => h.onOpenNewTab(p),
  splitRight: (h, p) => h.onSplitRight(p),
  splitDown: (h, p) => h.onSplitDown(p),
  sftp: (h, p) => h.onOpenSftp?.(p),
};

/** Primary binding label for menus (first binding, or default). */
export function profileSidebarBindingLabel(key: ProfileSidebarActionKey): string {
  const actionId = PROFILE_SIDEBAR_ACTION_IDS[key];
  const custom = hotkeys.getBindings(actionId);
  if (custom[0]) return custom[0];
  return ACTIONS.find((a) => a.id === actionId)?.defaultBindings[0] ?? '';
}

/** Returns true when a profile shortcut was handled (caller should preventDefault). */
export function handleProfileSidebarShortcut(
  p: StoredProfile,
  ev: KeyboardEvent,
  handlers: ProfileSidebarShortcutHandlers,
): boolean {
  if (ev.defaultPrevented) return false;

  for (const { key, when } of ACTION_ORDER) {
    if (when && !when(p)) continue;
    const actionId = PROFILE_SIDEBAR_ACTION_IDS[key];
    if (!hotkeys.matchesAction(ev, actionId)) continue;
    const run = HANDLER_BY_KEY[key];
    if (key === 'sftp' && !handlers.onOpenSftp) continue;
    run(handlers, p);
    return true;
  }
  return false;
}
