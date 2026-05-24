/** Fired after profiles are created, updated, or deleted. */
export const PROFILES_CHANGED = 'aerotab:profiles-changed';

export function notifyProfilesChanged(): void {
  if (typeof document === 'undefined') return;
  document.dispatchEvent(new CustomEvent(PROFILES_CHANGED));
}
