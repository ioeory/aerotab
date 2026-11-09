import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const root = new URL('../../../../', import.meta.url);
const shell = readFileSync(new URL('src-tauri/src/bin/aerotab-app.rs', root), 'utf8');
test('explicit release smoke switch exits into an isolated builder before normal startup', () => {
  const main = shell.slice(shell.indexOf('fn main() {'));
  assert.match(main, /^fn main\(\) \{\s*if std::env::args_os\(\).skip\(1\).any\(\|arg\| arg == "--smoke-test"\) \{\s*run_isolated_smoke\(\);\s*return;/);
});
test('smoke builder has isolated identity and WebView storage with no business plugins or RPC', () => {
  const start = shell.indexOf('fn run_isolated_smoke()');
  assert.ok(start >= 0, 'isolated smoke builder must exist');
  const smoke = shell.slice(start, shell.indexOf('fn main() {', start));
  for (const token of ['std::env::temp_dir()', 'uuid::Uuid::new_v4()', '.identifier =', '.windows.clear()', '.data_directory(', 'smoke.html', '[SMOKE TEST - ISOLATED]']) assert.ok(smoke.includes(token), token);
  for (const forbidden of ['register_all', 'SettingsStore::open', 'migrate_app_data', 'keyring', 'tauri_plugin_updater', 'McpRuntime', 'invoke_handler', 'bootstrapSync']) assert.ok(!smoke.includes(forbidden), forbidden);
});
test('sandbox reports guest stages and detects initialization failure rather than waiting blindly', () => {
  const script = readFileSync(new URL('tools/windows-isolated-smoke.ps1', root), 'utf8');
  for (const token of ['smoke-progress.txt', 'guest-started', 'installer-started', 'installer-finished', 'window-verified', 'Windows Sandbox failed to initialize', 'UIAutomationClient', 'smoke-launcher-error.txt']) assert.ok(script.includes(token), token);
});
test('smoke install uses a separate destination, explicit switch and checks versioned title', () => {
  const script = readFileSync(new URL('tools/install-windows-smoke.sh', root), 'utf8');
  for (const token of ['SMOKE_ISOLATED:-1', '--smoke-test', 'MainWindowTitle', '[SMOKE TEST - ISOLATED]', '/D=']) assert.ok(script.includes(token), token);
});
