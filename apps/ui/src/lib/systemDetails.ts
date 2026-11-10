import type { MountUsage, NetworkRate } from './monitoringTypes';

/** Overview is a named subset, not host bandwidth utilization. Raw interfaces remain visible. */
export function networkSummary(rows: NetworkRate[]): { rx: number | null; tx: number | null } {
  const names = new Set(rows.map(row => row.name));
  const selected = rows.filter(row => row.name !== 'lo' && row.name !== 'lo0' && !/^(veth|docker|br-|virbr|ovs-system)/.test(row.name) && !(row.name.endsWith('-ovs') && names.has(row.name.slice(0, -4))));
  const sum = (key: 'rx_bytes_per_second' | 'tx_bytes_per_second'): number | null => selected.length && selected.every(row => row[key] != null && Number.isFinite(row[key]) && row[key]! >= 0) ? selected.reduce((total, row) => total + row[key]!, 0) : null;
  return { rx: sum('rx_bytes_per_second'), tx: sum('tx_bytes_per_second') };
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
