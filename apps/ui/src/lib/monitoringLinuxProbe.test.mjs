import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import ts from 'typescript';

const load = async file => import(`data:text/javascript;base64,${Buffer.from(ts.transpileModule(readFileSync(new URL(file, import.meta.url), 'utf8'), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText).toString('base64')}`);
const { MonitoringService } = await load('./monitoring.ts');
const { networkSummary, sparklinePaths } = await load('./systemDetails.ts');
const flush = () => new Promise(resolve => setImmediate(resolve));

// Real local Linux /proc data and the actual Rust parser, with no SSH connection,
// credentials, remote host or desktop required. Synthetic timing only.
test('actual Linux probe and Rust parser produce continuous network overview across equivalent profile refreshes', { skip: process.platform !== 'linux' }, async () => {
  const source = readFileSync(new URL('../../../../src-tauri/src/ssh/stats.rs', import.meta.url), 'utf8');
  const command = source.match(/const HOST_STATS_COMMAND: &str = r#"([\s\S]*?)"#;/)?.[1];
  assert.ok(command, 'actual Linux probe command must be found');
  const structs = source.slice(source.indexOf('#[derive(Debug, Clone, Serialize, PartialEq)]'), source.indexOf('pub async fn probe_host_stats')).replaceAll(', Serialize', '');
  const parser = source.slice(source.indexOf('fn parse_host_stats('), source.indexOf('#[cfg(test)]'));
  const main = `fn main() { use std::io::Read; let mut input = String::new(); std::io::stdin().read_to_string(&mut input).unwrap(); let stats = parse_host_stats(&input); for row in stats.network_interfaces.unwrap_or_default() { println!("{}|{}|{}", row.name, row.rx_bytes, row.tx_bytes); } }`;
  const dir = mkdtempSync(join(tmpdir(), 'aerotab-network-probe-'));
  let service;
  try {
    const binary = join(dir, 'parser');
    const compiled = spawnSync('rustc', ['--edition=2021', '-o', binary, '-'], { input: structs + parser + main, encoding: 'utf8', timeout: 30000 });
    assert.equal(compiled.status, 0, compiled.stderr || String(compiled.error));
    let monotonic = 1000;
    const timers = new Map(); let timerId = 0;
    const profile = () => ({ id: 'local-probe', kind: 'ssh', name: 'local test only', ssh: { host: 'unused.invalid', port: 22, user: 'unused', jump_via: [] } });
    service = new MonitoringService({ async call(method) {
      assert.equal(method, 'ssh.hostStats');
      const probe = spawnSync('/bin/sh', ['-c', command], { encoding: 'utf8', timeout: 15000, maxBuffer: 2 * 1024 * 1024 });
      assert.equal(probe.status, 0, probe.stderr || String(probe.error));
      const parsed = spawnSync(binary, [], { input: probe.stdout, encoding: 'utf8', timeout: 10000 });
      assert.equal(parsed.status, 0, parsed.stderr || String(parsed.error));
      const network_interfaces = parsed.stdout.trim().split('\n').filter(Boolean).map(line => {
        const [name, rx, tx] = line.split('|');
        assert.match(rx, /^\d+$/); assert.match(tx, /^\d+$/);
        return { name, rx_bytes: Number(rx), tx_bytes: Number(tx) };
      });
      assert.ok(network_interfaces.length > 0, 'local /proc should yield interfaces through actual Rust parser');
      return { network_interfaces };
    } }, { now: () => 1700000000000 + monotonic, monotonicNow: () => monotonic, setTimer: fn => { timers.set(++timerId, fn); return timerId; }, clearTimer: id => timers.delete(id) });
    service.setProfiles([profile()]);
    service.subscribe('local-probe', 1000);
    await flush();
    assert.equal(service.snapshot('local-probe').failures, 0);
    const first = service.snapshot('local-probe').latest;
    assert.ok(first);
    assert.ok(first.network_rates.every(row => row.rx_bytes_per_second === null && row.tx_bytes_per_second === null));
    for (let round = 0; round < 4; round++) {
      service.setProfiles([{ ...profile(), name: `renamed ${round}` }]);
      monotonic += 1000;
      const pending = [...timers.values()]; timers.clear(); pending.forEach(fn => fn());
      await flush();
      const snapshot = service.snapshot('local-probe');
      assert.equal(snapshot.failures, 0);
      assert.equal(snapshot.history.length, round + 2, 'same endpoint refresh must retain continuity');
      assert.ok(snapshot.latest.network_rates.every(row => row.rx_bytes_per_second != null && row.tx_bytes_per_second != null), 'stable interfaces must have genuine delta rates');
    }
    const history = service.snapshot('local-probe').history;
    const hasOverviewInterface = history.at(-1).network_rates.some(row => networkSummary([row]).rx != null);
    if (hasOverviewInterface) {
      const points = history.map(sample => { const summary = networkSummary(sample.network_rates); return { at: sample.monotonicAt, value: summary.rx == null || summary.tx == null ? null : summary.rx + summary.tx, breakBefore: sample.breakBefore }; });
      assert.notEqual(points.at(-1).value, null, 'overview must receive valid rates beyond first sample');
      assert.ok(sparklinePaths(points, Math.max(1, ...points.map(point => point.value ?? 0))).length > 0, 'actual Linux samples must leave the waiting-for-trend state');
    }
  } finally {
    service?.dispose();
    rmSync(dir, { recursive: true, force: true });
  }
});
