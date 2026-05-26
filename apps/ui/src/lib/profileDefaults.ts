/** Default ports and helpers when creating or switching connection types. */

export type ProfileKind = 'ssh' | 'rdp' | 'vnc';

export const DEFAULT_PROFILE_PORT: Record<ProfileKind, number> = {
  ssh: 22,
  rdp: 3389,
  vnc: 5900,
};

export function defaultPortForKind(kind: ProfileKind): number {
  return DEFAULT_PROFILE_PORT[kind];
}
