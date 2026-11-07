import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
const source = readFileSync(new URL('../../../../src-tauri/src/ssh/stats.rs', import.meta.url), 'utf8');
const command = /const HOST_STATS_COMMAND: &str = r#"([\s\S]*?)"#;/.exec(source)?.[1];
test('actual Linux probe emits OS, per-core CPU, network, disk IO, mounts, swap and load without interactive terminal', { skip: process.platform !== 'linux' }, () => {
  assert.ok(command);
  const output = execFileSync('/bin/sh', ['-c', command], { encoding: 'utf8', timeout: 10000, maxBuffer: 2 * 1024 * 1024 });
  for (const key of ['os', 'load1', 'load5', 'load15', 'swap_total_kb', 'swap_used_kb', 'cpu_percent', 'cpu_core', 'network', 'disk_io', 'mount']) assert.match(output, new RegExp(`^${key}=.+$`, 'm'), key);
  const cpu = output.split('\n').filter(row => row.startsWith('cpu_core='));
  for (const row of cpu) { const value = Number(row.split('|')[1]); assert.ok(Number.isFinite(value) && value >= 0 && value <= 100); }
  const network = output.split('\n').find(row => row.startsWith('network=lo|'));
  assert.ok(network); assert.match(network, /^network=lo\|\d+\|\d+$/);
});
