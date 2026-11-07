import type { HostStats, StoredProfile } from './types';
export interface CpuCore { name: string; percent: number }
export interface NetworkInterface { name: string; rx_bytes: number; tx_bytes: number }
export interface DiskIo { name: string; read_bytes: number; write_bytes: number }
export interface MountUsage { device: string; mount_point: string; total_kb: number; used_kb: number; percent: number }
export interface MonitoringHostStats extends HostStats {
  os?: string | null;
  load5?: number | null;
  load15?: number | null;
  swap_total_kb?: number | null;
  swap_used_kb?: number | null;
  cpu_cores?: CpuCore[] | null;
  network_interfaces?: NetworkInterface[] | null;
  disk_io?: DiskIo[] | null;
  mounts?: MountUsage[] | null;
}
export interface NetworkRate extends NetworkInterface { rx_bytes_per_second: number | null; tx_bytes_per_second: number | null }
export interface DiskRate extends DiskIo { read_bytes_per_second: number | null; write_bytes_per_second: number | null }
export interface MonitoringSample { at: number; stats: MonitoringHostStats; probeMs: number; network_rates: NetworkRate[]; disk_rates: DiskRate[] }
export interface MonitoringSnapshot { history: MonitoringSample[]; latest?: MonitoringSample; error?: string; sampling: boolean; failures: number }
export interface MonitoringPanel { id: string; name: string; profileIds: string[]; intervalMs: number; rangeMinutes: number; view: 'list' | 'chart'; sort: 'name' | 'cpu' | 'memory'; search: string }
export type MonitoringProfile = Extract<StoredProfile, { kind: 'ssh' }>;
