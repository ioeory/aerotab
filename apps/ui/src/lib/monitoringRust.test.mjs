import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
test('actual Rust HostStats parser tests pass independently of unrelated workspace compilation', () => {
  const source = readFileSync(new URL('../../../../src-tauri/src/ssh/stats.rs', import.meta.url), 'utf8');
  const structs = source.slice(source.indexOf('#[derive(Debug, Clone, Serialize'), source.indexOf('pub async fn probe_host_stats'));
  const parserAndTests = source.slice(source.indexOf('fn parse_host_stats('));
  const isolated = `${structs}\n${parserAndTests}`.replaceAll(', Serialize', '');
  const dir = mkdtempSync(join(tmpdir(), 'aerotab-stats-test-'));
  try {
    const file = join(dir, 'stats.rs'); const binary = join(dir, 'stats-tests');
    writeFileSync(file, isolated);
    execFileSync('rustc', ['--edition=2021', '--test', file, '-o', binary], { encoding: 'utf8', timeout: 30000 });
    const output = execFileSync(binary, [], { encoding: 'utf8', timeout: 30000 });
    assert.match(output, /4 passed; 0 failed/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
