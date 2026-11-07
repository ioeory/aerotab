export const resourcePages = ['monitoring', 'keychain', 'proxies', 'forwarding', 'scripts', 'notes', 'knownHosts', 'logs'] as const;
export type ResourcePage = typeof resourcePages[number];
export type WorkspacePage = 'hosts' | 'sessions' | 'files' | ResourcePage;

export function isResourcePage(value: string): value is ResourcePage {
  return (resourcePages as readonly string[]).includes(value);
}
