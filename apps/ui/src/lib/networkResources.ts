export type NetworkRpc = { call<T>(method: string, params?: unknown): Promise<T> };
export type NetworkProfile = { id: string; name: string; kind: string };
export type ProxyConfig =
  | { kind: 'ssh_jump'; profile_id: string }
  | { kind: 'socks5' | 'http_connect'; host: string; port: number; vault_entry_id: string | null };
export type ProxyResource = { id: string; name: string; config: ProxyConfig };
export type ForwardingRule = { id: string; name: string; profile_id: string; kind: 'local' | 'remote' | 'dynamic'; bind_host: string; bind_port: number; target_host: string; target_port: number };
export type ForwardingInstance = { rule_id: string; tunnel: { id: string; status: string; bind_host: string; bind_port: number; error?: string } };
export type VaultMeta = { id: string; label: string; kind: string; username?: string };
export function newProxyResource(): ProxyResource {
  return { id: crypto.randomUUID(), name: '', config: { kind: 'socks5', host: '127.0.0.1', port: 1080, vault_entry_id: null } };
}
export function newForwardingRule(profileId = ''): ForwardingRule {
  return { id: crypto.randomUUID(), name: '', profile_id: profileId, kind: 'local', bind_host: '127.0.0.1', bind_port: 8080, target_host: '127.0.0.1', target_port: 80 };
}
export function proxyConfig(kind: ProxyConfig['kind'], profileId = ''): ProxyConfig {
  return kind === 'ssh_jump' ? { kind, profile_id: profileId } : { kind, host: '127.0.0.1', port: kind === 'socks5' ? 1080 : 8080, vault_entry_id: null };
}
export function networkError(error: unknown): string { return error instanceof Error ? error.message : String(error); }
