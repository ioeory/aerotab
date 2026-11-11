import type { RpcClient } from './rpc';
import type { StoredProfile } from './types';
import type { MonitoringHostStats, MonitoringSample, MonitoringSnapshot, NetworkRate, DiskRate, MonitoringPanel, ContinuityReason } from './monitoringTypes';

export function networkRates(stats: MonitoringHostStats, previous: MonitoringSample | undefined, at: number): NetworkRate[] {
  const seconds = previous ? (at - (previous.monotonicAt ?? previous.at)) / 1000 : 0;
  const valid = !!previous && seconds > 0 && (stats.uptime_seconds == null || previous.stats.uptime_seconds == null || stats.uptime_seconds >= previous.stats.uptime_seconds);
  return (stats.network_interfaces ?? []).map(row => {
    const old = valid ? previous?.stats.network_interfaces?.find(item => item.name === row.name) : undefined;
    const invalid = [row.rx_bytes, row.tx_bytes, ...(old ? [old.rx_bytes, old.tx_bytes] : [])].some(value => !Number.isFinite(value) || value < 0);
    const unavailableReason: NetworkRate['unavailableReason'] = invalid ? 'invalidCounter' : !valid ? 'baselineUnavailable' : !old ? 'newInterface' : row.rx_bytes < old.rx_bytes || row.tx_bytes < old.tx_bytes ? 'counterReset' : undefined;
    return { ...row, unavailableReason, rx_bytes_per_second: counterRate(row.rx_bytes, old?.rx_bytes, seconds), tx_bytes_per_second: counterRate(row.tx_bytes, old?.tx_bytes, seconds) };
  });
}
function counterRate(value: number, old: number | undefined, seconds: number): number | null {
  return old != null && Number.isFinite(value) && Number.isFinite(old) && value >= old && old >= 0 && seconds > 0 ? (value - old) / seconds : null;
}
function diskRates(stats: MonitoringHostStats, previous: MonitoringSample | undefined, at: number): DiskRate[] {
  const seconds = previous ? (at - (previous.monotonicAt ?? previous.at)) / 1000 : 0;
  const valid = !!previous && (stats.uptime_seconds == null || previous.stats.uptime_seconds == null || stats.uptime_seconds >= previous.stats.uptime_seconds);
  return (stats.disk_io ?? []).map(row => {
    const old = valid ? previous?.stats.disk_io?.find(item => item.name === row.name) : undefined;
    return { ...row, read_bytes_per_second: counterRate(row.read_bytes, old?.read_bytes, seconds), write_bytes_per_second: counterRate(row.write_bytes, old?.write_bytes, seconds) };
  });
}
export function normalizePanels(value: unknown): MonitoringPanel[] {
  if (!Array.isArray(value)) return [];
  const seen = new Set<string>();
  return value.filter(p => p && typeof p.id === 'string' && typeof p.name === 'string' && !seen.has(p.id) && seen.add(p.id)).slice(0, 100).map(p => ({
    id: p.id, name: p.name.trim().slice(0, 120) || p.id,
    profileIds: [...new Set<string>((Array.isArray(p.profileIds) ? p.profileIds : []).filter((id: unknown): id is string => typeof id === 'string'))].slice(0, 500),
    intervalMs: [1000, 2000, 5000, 10000, 30000, 60000].includes(p.intervalMs) ? p.intervalMs : 5000,
    rangeMinutes: [5, 15, 30, 60].includes(p.rangeMinutes) ? p.rangeMinutes : 15,
    view: p.view === 'chart' ? 'chart' : 'list', sort: ['cpu', 'memory'].includes(p.sort) ? p.sort : 'name', search: typeof p.search === 'string' ? p.search.slice(0, 200) : '',
  }));
}
function endpoint(ssh: Extract<StoredProfile, { kind: 'ssh' }>['ssh']): unknown {
  return [ssh.host, ssh.port, ssh.user, ssh.proxy_id, ssh.jump_profile_ids, ssh.jump_via?.map(endpoint)];
}
interface Entry {
  leases: Map<symbol, number>; snapshot: MonitoringSnapshot; generation: number; inFlight: boolean;
  timer?: ReturnType<typeof setTimeout>; due: boolean; continuous: boolean;
  continuityReason?: ContinuityReason; diagnosticAt?: number; stream: number; interruptions: Set<ContinuityReason>;
}
interface Options {
  now?: () => number; monotonicNow?: () => number; setTimer?: typeof setTimeout; clearTimer?: typeof clearTimeout;
  maxConcurrent?: number; maxSamples?: number; maxAgeMs?: number;
  onDiagnostic?: (snapshot: MonitoringSnapshot, stream: number, interruptions: ContinuityReason[]) => void;
}
/** A single queue per RPC client; independent UI leases never stop each other. */
export class MonitoringService {
  private profiles = new Map<string, StoredProfile>();
  private entries = new Map<string, Entry>();
  private listeners = new Set<() => void>();
  private active = 0;
  private now: () => number;
  private monotonicNow: () => number;
  private timer: typeof setTimeout;
  private clear: typeof clearTimeout;
  private maxConcurrent: number;
  private maxSamples: number;
  private maxAgeMs: number;
  private paused = false;
  private nextStream = 0;
  private onDiagnostic?: Options['onDiagnostic'];
  constructor(private rpc: RpcClient, options: Options = {}) {
    this.onDiagnostic = options.onDiagnostic;
    this.monotonicNow = options.monotonicNow ?? (options.now ? options.now : () => performance.now());
    this.now = options.now ?? Date.now; this.timer = options.setTimer ?? setTimeout; this.clear = options.clearTimer ?? clearTimeout;
    this.maxConcurrent = Math.max(1, options.maxConcurrent ?? 4); this.maxSamples = Math.max(1, options.maxSamples ?? 3600); this.maxAgeMs = options.maxAgeMs ?? 3600000;
  }
  setProfiles(profiles: StoredProfile[]): void {
    const identity = (p: StoredProfile | undefined) => p?.kind === 'ssh' ? JSON.stringify(endpoint(p.ssh)) : null;
    for (const [id, entry] of this.entries) {
      if (this.profiles.has(id) && identity(this.profiles.get(id)) !== identity(profiles.find(p => p.id === id))) this.resetEntry(entry);
    }
    this.profiles = new Map(profiles.map(p => [p.id, p])); this.emit(); this.drain();
  }
  invalidateProfiles(): void {
    this.profiles.clear();
    for (const entry of this.entries.values()) this.resetEntry(entry);
    this.emit(); this.drain();
  }
  private resetEntry(entry: Entry): void {
    if (entry.timer) this.clear(entry.timer); entry.timer = undefined;
    entry.continuityReason = 'profileChanged'; entry.interruptions.add('profileChanged');
    entry.generation++; entry.continuous = false; entry.due = entry.leases.size > 0;
    entry.snapshot = { history: [], sampling: false, failures: 0 };
  }
  watch(listener: () => void): () => void { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; }
  private emit(): void { for (const listener of this.listeners) listener(); }
  snapshot(id: string): MonitoringSnapshot {
    return this.entries.get(id)?.snapshot ?? { history: [], sampling: false, failures: 0 };
  }
  activeProfileIds(): string[] { return [...this.entries].filter(([, entry]) => entry.leases.size > 0).map(([id]) => id); }
  /** Call from window-minimize policy, not page visibility: running dashboards survive navigation. */
  setPaused(paused: boolean): void {
    if (this.paused === paused) return;
    this.paused = paused;
    for (const entry of this.entries.values()) {
      if (entry.timer) this.clear(entry.timer); entry.timer = undefined;
      entry.continuityReason = 'windowPause'; entry.interruptions.add('windowPause');
      entry.continuous = false; entry.generation++; entry.due = !paused && entry.leases.size > 0;
      entry.snapshot = { ...entry.snapshot, sampling: false };
    }
    this.emit(); this.drain();
  }
  isPaused(): boolean { return this.paused; }
  subscribe(id: string, intervalMs = 5000): () => void {
    let entry = this.entries.get(id);
    if (!entry) { entry = { leases: new Map(), snapshot: { history: [], sampling: false, failures: 0 }, generation: 0, inFlight: false, due: false, continuous: false, stream: ++this.nextStream, interruptions: new Set() }; this.entries.set(id, entry); }
    const lease = Symbol(id); const first = entry.leases.size === 0;
    entry.leases.set(lease, Math.max(1000, intervalMs));
    if (first) { entry.continuityReason ??= entry.snapshot.latest ? 'subscriptionRestart' : 'firstSample'; entry.continuous = false; entry.generation++; entry.due = true; entry.snapshot = { ...entry.snapshot, error: undefined, failures: 0 }; }
    else if (entry.timer) { this.clear(entry.timer); entry.timer = undefined; this.schedule(entry); }
    this.drain();
    let released = false;
    return () => {
      if (released) return; released = true; entry!.leases.delete(lease);
      if (!entry!.leases.size) {
        entry!.continuityReason = 'subscriptionRestart'; entry!.interruptions.add('subscriptionRestart');
        entry!.generation++; entry!.due = false;
        if (entry!.timer) this.clear(entry!.timer); entry!.timer = undefined;
        entry!.snapshot = { ...entry!.snapshot, sampling: false };
      }
      this.emit();
    };
  }
  private schedule(entry: Entry): void {
    if (!entry.leases.size || this.paused) return;
    const interval = Math.min(...entry.leases.values());
    const delay = Math.min(120000, interval * 2 ** Math.min(entry.snapshot.failures, 5));
    entry.timer = this.timer(() => { entry.timer = undefined; entry.due = true; this.drain(); }, delay);
  }
  private drain(): void {
    if (this.paused) return;
    for (const [id, entry] of this.entries) {
      if (this.active >= this.maxConcurrent) break;
      if (!entry.due || entry.inFlight || !entry.leases.size) continue;
      entry.due = false; entry.inFlight = true; this.active++;
      void this.probe(id, entry);
    }
  }
  private async probe(id: string, entry: Entry): Promise<void> {
    const generation = entry.generation; const start = this.monotonicNow();
    const accept = () => generation === entry.generation && entry.leases.size > 0 && !this.paused;
    entry.snapshot = { ...entry.snapshot, sampling: true }; this.emit();
    try {
      // Resolve on every sample; never persist a credential-bearing profile snapshot.
      let profile = this.profiles.get(id);
      if (!profile) profile = await this.rpc.call<StoredProfile>('profile.get', { id });
      if (!profile || profile.kind !== 'ssh') throw new Error('Monitoring requires an SSH profile');
      // profile_id accompanies the legacy profile payload for a seamless backend migration.
      const stats = await this.rpc.call<MonitoringHostStats>('ssh.hostStats', { profile_id: id, profile: profile.ssh });
      if (!accept()) return;
      const at = this.now(), monotonicAt = this.monotonicNow();
      // A failed probe breaks counter continuity; reboot is checked by rate helpers.
      const latest = entry.snapshot.latest;
      const changedHost = !!latest?.stats.hostname && !!stats.hostname && latest.stats.hostname !== stats.hostname;
      const previous = changedHost || entry.snapshot.failures || !entry.continuous ? undefined : latest;
      entry.continuous = true;
      const reboot = previous?.stats.uptime_seconds != null && stats.uptime_seconds != null && stats.uptime_seconds < previous.stats.uptime_seconds;
      const continuityReason: ContinuityReason | undefined = changedHost ? 'hostChanged' : entry.snapshot.failures ? 'probeFailed' : reboot ? 'reboot' : !previous ? entry.continuityReason ?? 'firstSample' : monotonicAt <= (previous.monotonicAt ?? previous.at) ? 'invalidInterval' : undefined;
      if (continuityReason) entry.interruptions.add(continuityReason);
      entry.continuityReason = undefined;
      const sample: MonitoringSample = { continuityReason, at, monotonicAt, breakBefore: !previous || reboot, stats, probeMs: Math.max(0, monotonicAt - start), network_rates: networkRates(stats, previous, monotonicAt), disk_rates: diskRates(stats, previous, monotonicAt) };
      const history = [...(changedHost ? [] : entry.snapshot.history), sample].filter(s => monotonicAt - (s.monotonicAt ?? s.at) <= this.maxAgeMs).slice(-this.maxSamples);
      entry.snapshot = { history, latest: sample, sampling: false, failures: 0 };
    } catch {
      // Never expose server errors: authentication failures can contain sensitive payloads.
      if (accept()) { entry.interruptions.add('probeFailed'); entry.snapshot = { ...entry.snapshot, error: 'probeFailed', failures: entry.snapshot.failures + 1, sampling: false }; }
    } finally {
      this.active--; entry.inFlight = false;
      if (accept()) {
        this.schedule(entry);
        const now = this.monotonicNow();
        // At most one event per minute per anonymous stream; retain intervening breaks.
        if (this.onDiagnostic && (entry.diagnosticAt == null || now - entry.diagnosticAt >= 60000)) {
          entry.diagnosticAt = now;
          try { this.onDiagnostic(entry.snapshot, entry.stream, [...entry.interruptions]); } catch { /* diagnostics must not interrupt sampling */ }
          entry.interruptions.clear();
        }
      }
      this.emit(); this.drain();
    }
  }
  dispose(): void {
    for (const entry of this.entries.values()) { entry.leases.clear(); entry.generation++; entry.due = false; if (entry.timer) this.clear(entry.timer); }
    this.listeners.clear();
  }
}
