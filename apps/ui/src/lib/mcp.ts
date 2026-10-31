export interface McpStatus {
  running: boolean;
  url: string | null;
  error: string | null;
  tokenConfigured: boolean;
}

export interface McpConfig {
  enabled: boolean;
  bindHost: string;
  allowRemoteBind: boolean;
  port: number;
  additionalContext: string;
  tlsCertPath: string;
  tlsKeyPath: string;
  localRoots: string[];
  permissions: Record<McpPermission, boolean>;
}

export type McpPermission = 'listProfiles' | 'readTerminal' | 'readFiles' | 'writeFiles'
  | 'runCommand' | 'openSession' | 'manageProfiles' | 'tunnels' | 'remoteDesktop' | 'callRpc';

export function defaultMcpConfig(): McpConfig {
  return {
    enabled: false, bindHost: '127.0.0.1', allowRemoteBind: false, port: 0,
    additionalContext: '', tlsCertPath: '', tlsKeyPath: '', localRoots: [],
    permissions: {
      listProfiles: true, readTerminal: true, readFiles: false, writeFiles: false,
      runCommand: false, openSession: false, manageProfiles: false, tunnels: false,
      remoteDesktop: false, callRpc: false,
    },
  };
}

export function validateMcpConfig(config: McpConfig): 'host' | 'port' | 'remote' | 'tls' | null {
  if (!config.bindHost.trim()) return 'host';
  if (!Number.isInteger(config.port) || config.port < 0 || config.port > 65535) return 'port';
  const host = config.bindHost.trim().toLowerCase();
  const loopback = host === 'localhost' || host === '::1' || host === '[::1]'
    || /^127(?:\.(?:\d{1,3})){3}$/.test(host);
  if (!loopback && !config.allowRemoteBind) return 'remote';
  if (!loopback && (!config.tlsCertPath.trim() || !config.tlsKeyPath.trim())) return 'tls';
  return null;
}

export function usableMcpUrl(status: Pick<McpStatus, 'running' | 'url'>): string | null {
  if (!status.running || !status.url) return null;
  try {
    const url = new URL(status.url);
    if (!['http:', 'https:'].includes(url.protocol) || ['0.0.0.0', '[::]'].includes(url.hostname)
      || url.port === '0' || url.username || url.password) return null;
    return status.url;
  } catch { return null; }
}

export type McpClient = 'cursor' | 'claude' | 'generic' | 'stdio';

export function mcpClientConfig(client: McpClient, url: string | null, token: string | null) {
  if (client === 'stdio') return { mcpServers: { aerotab: { command: 'aerotab-mcp', args: [] } } };
  if (!url) throw new Error('MCP server has no connectable URL');
  const headers = { Authorization: `Bearer ${token ?? '<AEROTAB_MCP_TOKEN>'}` };
  if (client === 'generic') return { url, headers };
  return { mcpServers: { aerotab: { ...(client === 'claude' ? { type: 'http' } : {}), url, headers } } };
}
