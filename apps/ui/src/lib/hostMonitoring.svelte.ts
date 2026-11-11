import { untrack } from 'svelte';
import type { RpcClient } from './rpc';
import type { StoredProfile } from './types';
import type { MonitoringPanel } from './monitoringTypes';
import { MonitoringService, normalizePanels } from './monitoring';
import { PROFILES_CHANGED } from './profileEvents';
import { diagnostics } from './diagnostics.svelte';
import { networkDiagnostic } from './systemDetails';

const services = new WeakMap<RpcClient, HostMonitoring>();
const STORAGE_KEY = 'aerotab.monitoring.panels.v1';
export class HostMonitoring {
  readonly service: MonitoringService;
  panels = $state<MonitoringPanel[]>([]);
  selectedPanelId = $state('');
  running = $state<string[]>([]);
  revision = $state(0);
  persistenceError = $state(false);
  private releases = new Map<string, (() => void)[]>();
  constructor(rpc: RpcClient) {
    this.service = new MonitoringService(rpc, { onDiagnostic: (snapshot, stream, interruptions) => {
      diagnostics.record('connection', 'monitoring.network', JSON.stringify({ stream, ...networkDiagnostic(snapshot), interruptions }), snapshot.error ? 'warn' : 'info');
    } });
    this.service.watch(() => untrack(() => { this.revision++; }));
    if (typeof document !== 'undefined') document.addEventListener(PROFILES_CHANGED, () => this.service.invalidateProfiles());
    try {
      const saved = JSON.parse(globalThis.localStorage?.getItem(STORAGE_KEY) ?? 'null');
      this.panels = normalizePanels(saved?.panels);
      this.selectedPanelId = this.panels.some(p => p.id === saved?.selectedPanelId) ? saved.selectedPanelId : this.panels[0]?.id ?? '';
    } catch { this.persistenceError = true; }
  }
  setProfiles(profiles: StoredProfile[]): void { this.service.setProfiles(profiles); }
  private persist(): void {
    try { localStorage.setItem(STORAGE_KEY, JSON.stringify({ panels: normalizePanels(this.panels), selectedPanelId: this.selectedPanelId })); this.persistenceError = false; }
    catch { this.persistenceError = true; }
  }
  select(id: string): void { this.selectedPanelId = id; this.persist(); }
  create(name: string, profileIds: string[]): string {
    const id = crypto.randomUUID();
    this.panels = [...this.panels, ...normalizePanels([{ id, name, profileIds }])];
    this.select(id); return id;
  }
  update(id: string, patch: Partial<Omit<MonitoringPanel, 'id'>>): void {
    const wasRunning = this.running.includes(id);
    this.panels = normalizePanels(this.panels.map(panel => panel.id === id ? { ...panel, ...patch, id } : panel));
    if (wasRunning && (patch.profileIds || patch.intervalMs)) { this.stop(id); this.start(id); }
    this.persist();
  }
  remove(id: string): void {
    this.stop(id); this.panels = this.panels.filter(p => p.id !== id);
    if (this.selectedPanelId === id) this.selectedPanelId = this.panels[0]?.id ?? '';
    this.persist();
  }
  start(id: string): void {
    if (this.releases.has(id)) return;
    const panel = this.panels.find(p => p.id === id); if (!panel) return;
    this.releases.set(id, panel.profileIds.map(profileId => this.service.subscribe(profileId, panel.intervalMs)));
    this.running = [...this.running, id];
  }
  stop(id: string): void {
    for (const release of this.releases.get(id) ?? []) release();
    this.releases.delete(id); this.running = this.running.filter(item => item !== id);
  }
  references(profileId: string): { id: string; name: string }[] {
    return this.panels.filter(p => p.profileIds.includes(profileId)).map(({ id, name }) => ({ id, name }));
  }
}
/** Retained across workspace component unmounts; only panel.stop releases a running dashboard. */
export function getHostMonitoring(rpc: RpcClient): HostMonitoring {
  let store = services.get(rpc);
  if (!store) { store = new HostMonitoring(rpc); services.set(rpc, store); }
  return store;
}
export function monitoringProfileReferences(rpc: RpcClient, profileId: string): { id: string; name: string }[] {
  return getHostMonitoring(rpc).references(profileId);
}
export function setMonitoringWindowPaused(rpc: RpcClient, paused: boolean): void { getHostMonitoring(rpc).service.setPaused(paused); }
