import type { ProfileSidebarShortcutHandlers } from './profileSidebarShortcuts';

let handlers: ProfileSidebarShortcutHandlers | null = null;

export function registerSidebarProfileHandlers(next: ProfileSidebarShortcutHandlers) {
  handlers = next;
}

export function getSidebarProfileHandlers(): ProfileSidebarShortcutHandlers | null {
  return handlers;
}

export function clearSidebarProfileHandlers() {
  handlers = null;
}
