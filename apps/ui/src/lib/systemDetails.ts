import type { MountUsage, NetworkRate, MonitoringSnapshot } from './monitoringTypes';

/** Overview is a named subset, not host bandwidth utilization. Raw interfaces remain visible. */
export function networkSummary(rows: NetworkRate[]): { rx: number | null; tx: number | null } {
  const selected = overviewInterfaces(rows);
  const sum = (key: 'rx_bytes_per_second' | 'tx_bytes_per_second'): number | null => selected.length && selected.every(row => row[key] != null && Number.isFinite(row[key]) && row[key]! >= 0) ? selected.reduce((total, row) => total + row[key]!, 0) : null;
  return { rx: sum('rx_bytes_per_second'), tx: sum('tx_bytes_per_second') };
}
function overviewInterfaces(rows: NetworkRate[]): NetworkRate[] {
  const names = new Set(rows.map(row => row.name));
  return rows.filter(row => row.name !== 'lo' && row.name !== 'lo0' && !/^(veth|docker|br-|virbr|ovs-system)/.test(row.name) && !(row.name.endsWith('-ovs') && names.has(row.name.slice(0, -4))));
}
export function networkState(snapshot: MonitoringSnapshot) {
  const sample = snapshot.latest, rows = sample?.network_rates ?? [], selected = overviewInterfaces(rows);
  const valid = (value: number | null) => value != null && Number.isFinite(value) && value >= 0;
  const readyCount = selected.filter(row => valid(row.rx_bytes_per_second) && valid(row.tx_bytes_per_second)).length;
  const status = snapshot.error ? 'probeFailed' : !sample ? 'waitingFirst' : !rows.length ? 'interfacesUnavailable'
    : !selected.length ? 'noEligibleInterfaces' : readyCount === selected.length ? 'ready' : readyCount ? 'incomplete'
    : sample.continuityReason ? 'waitingSecond' : 'invalidCounters';
  return { ...networkSummary(rows), status, interfaceCount: rows.length, eligibleCount: selected.length, filteredCount: rows.length - selected.length, readyCount } as const;
}
/** Safe local evidence: never copy endpoint, hostname, interface names, counters or errors. */
export function networkDiagnostic(snapshot: MonitoringSnapshot) {
  const state = networkState(snapshot);
  return { status: state.status, interfaceCount: state.interfaceCount, eligibleCount: state.eligibleCount, filteredCount: state.filteredCount, readyCount: state.readyCount,
    sampleCount: snapshot.history.length, failures: snapshot.failures, continuityReason: snapshot.latest?.continuityReason ?? 'continuous',
    rateIssues: Object.fromEntries((['baselineUnavailable', 'newInterface', 'invalidCounter', 'counterReset'] as const).map(reason => [reason, (snapshot.latest?.network_rates ?? []).filter(row => row.unavailableReason === reason).length])) };
}
export function isVirtualMount(mount: Pick<MountUsage, 'device' | 'mount_point'>): boolean {
  return /^(tmpfs|devtmpfs|udev|proc|sysfs|cgroup2?|overlay|shm|devfs|none)$/.test(mount.device) || /^\/(proc|sys|dev)(\/|$)/.test(mount.mount_point);
}
export function capacityTone(value: number | null | undefined): 'normal' | 'warning' | 'danger' {
  return value != null && value >= 90 ? 'danger' : value != null && value >= 80 ? 'warning' : 'normal';
}
export function sparklinePaths(input: { at: number; value: number | null | undefined; breakBefore?: boolean }[], ceiling = 100): string[] {
  const points = input.slice(-60); if (points.length < 2) return [];
  const start = points[0]!.at, duration = points.at(-1)!.at - start;
  if (duration <= 0 || !Number.isFinite(ceiling) || ceiling <= 0) return [];
  const paths: string[] = []; let segment: string[] = [];
  const finish = () => { if (segment.length > 1) paths.push(`M${segment.join(' L')}`); segment = []; };
  for (const point of points) {
    if (point.breakBefore) finish();
    if (point.value == null || !Number.isFinite(point.value)) { finish(); continue; }
    const x = (point.at - start) / duration * 200, y = 30 - Math.max(0, Math.min(1, point.value / ceiling)) * 28;
    segment.push(`${x.toFixed(1)},${y.toFixed(1)}`);
  }
  finish(); return paths;
}
