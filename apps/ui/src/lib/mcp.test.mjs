import test from 'node:test';
import assert from 'node:assert/strict';
import { defaultMcpConfig, validateMcpConfig, mcpClientConfig, usableMcpUrl } from './mcp.ts';

test('defaults expose read-only terminal/profile access only', () => {
  const config = defaultMcpConfig();
  assert.deepEqual(Object.entries(config.permissions).filter(([, value]) => value).map(([key]) => key), ['listProfiles', 'readTerminal']);
  assert.equal(config.enabled, false);
  assert.equal('token' in config, false);
});
test('remote binds require explicit opt-in and both TLS paths', () => {
  const config = { ...defaultMcpConfig(), bindHost: '0.0.0.0' };
  assert.equal(validateMcpConfig(config), 'remote');
  config.allowRemoteBind = true;
  assert.equal(validateMcpConfig(config), 'tls');
  config.tlsCertPath = '/cert.pem';
  config.tlsKeyPath = '/key.pem';
  assert.equal(validateMcpConfig(config), null);
  assert.equal(validateMcpConfig({ ...config, port: 65536 }), 'port');
});
test('only actual running connectable URLs can be exported', () => {
  assert.equal(usableMcpUrl({ running: false, url: 'http://127.0.0.1:12/mcp' }), null);
  assert.equal(usableMcpUrl({ running: true, url: 'http://0.0.0.0:12/mcp' }), null);
  assert.equal(usableMcpUrl({ running: true, url: 'http://[::]:12/mcp' }), null);
  assert.equal(usableMcpUrl({ running: true, url: 'http://127.0.0.1:0/mcp' }), null);
  assert.equal(usableMcpUrl({ running: true, url: 'https://host.example:123/mcp' }), 'https://host.example:123/mcp');
});
test('HTTP client exports preserve the status URL and explicit bearer token', () => {
  const url = 'https://host.example:123/mcp';
  const cursor = mcpClientConfig('cursor', url, 'secret');
  assert.equal(cursor.mcpServers.aerotab.url, url);
  assert.equal(cursor.mcpServers.aerotab.headers.Authorization, 'Bearer secret');
  assert.equal(mcpClientConfig('claude', url, 'secret').mcpServers.aerotab.type, 'http');
  assert.equal(mcpClientConfig('generic', url, null).headers.Authorization, 'Bearer <AEROTAB_MCP_TOKEN>');
});
test('stdio export does not embed tokens', () => {
  assert.deepEqual(mcpClientConfig('stdio', null, null), { mcpServers: { aerotab: { command: 'aerotab-mcp', args: [] } } });
});
