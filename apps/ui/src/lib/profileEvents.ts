/** Fired after profiles are created, updated, or deleted. */
export const PROFILES_CHANGED = 'aerotab:profiles-changed';

export interface ProfilesChangedDetail {
  profileId?: string;
  /** Profile group path — listeners may expand matching folders. */
  group?: string | null;
}

export function notifyProfilesChanged(detail?: ProfilesChangedDetail): void {
  if (typeof document === 'undefined') return;
  document.dispatchEvent(new CustomEvent(PROFILES_CHANGED, { detail }));
}
