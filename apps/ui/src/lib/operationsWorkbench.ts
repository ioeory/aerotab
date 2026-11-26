import type { SessionMeta } from './types';

export interface IncidentEvidence {
  label: string; text: string; source?: 'selection' | 'lastCommand' | 'inspection' | 'manual'; at?: number;
}
export interface IncidentStep {
  label: string; expected: string; actual: string; done: boolean; kind?: 'check' | 'evidence' | 'confirmation'; skipReason?: string;
  template?: { id: string; name: string; version: string };
}
export interface IncidentRecord { title: string; conclusion: string; evidence: IncidentEvidence[]; steps: IncidentStep[] }
export const incidentEvidenceLimit = 12;
export const incidentStepLimit = 20;
const key = 'aerotab.operations.incident.v1';
const text = (value: unknown, max: number) => typeof value === 'string' ? Array.from(value.trim()).slice(0, max).join('') : '';
const fields = (value: unknown): Record<string, unknown> => value && typeof value === 'object' ? value as Record<string, unknown> : {};

/** Create an independent, allowlisted snapshot. Evidence is opt-in per item. */
export function prepareIncident(value: unknown, includeEvidence: readonly number[] = []): IncidentRecord {
  const raw = fields(value);
  const selected = new Set(includeEvidence);
  const evidence = Array.isArray(raw.evidence) ? raw.evidence.slice(0, incidentEvidenceLimit).flatMap((item, index) => {
    if (!selected.has(index)) return [];
    const entry = fields(item);
    const excerpt = text(entry.text, 4000);
    if (!excerpt) return [];
    const result: IncidentEvidence = { label: text(entry.label, 240), text: excerpt };
    const source = entry.source;
    if (source === 'selection' || source === 'lastCommand' || source === 'inspection' || source === 'manual') result.source = source;
    if (typeof entry.at === 'number' && Number.isFinite(entry.at) && entry.at >= 0 && entry.at <= 8.64e15) result.at = entry.at;
    return [result];
  }) : [];
  const steps = Array.isArray(raw.steps) ? raw.steps.slice(0, incidentStepLimit).flatMap(item => {
    const entry = fields(item);
    const label = text(entry.label, 240);
    const expected = text(entry.expected, 1000);
    const actual = text(entry.actual, 2000);
    const skipReason = text(entry.skipReason, 1000);
    if (!(label || expected || actual || skipReason)) return [];
    const result: IncidentStep = { label, expected, actual, done: entry.done === true };
    if (entry.kind === 'check' || entry.kind === 'evidence' || entry.kind === 'confirmation') result.kind = entry.kind;
    if (skipReason) { result.skipReason = skipReason; result.done = false; }
    const template = fields(entry.template);
    const id = text(template.id, 80), name = text(template.name, 160), version = text(template.version, 64);
    if (id && name && version) result.template = { id, name, version };
    return [result];
  }) : [];
  return { title: text(raw.title, 160), conclusion: text(raw.conclusion, 4000), evidence, steps };
}

export function incidentHasContent(record: IncidentRecord): boolean {
  return !!(record.title || record.conclusion || record.evidence.length || record.steps.length);
}

const allEvidence = Array.from({ length: incidentEvidenceLimit }, (_, index) => index);
export function loadIncident(storage?: Pick<Storage, 'getItem'>): IncidentRecord | null {
  try {
    const raw = storage?.getItem(key);
    if (!raw || raw.length > 700_000) return null;
    const value = JSON.parse(raw);
    if (value?.version !== 1) return null;
    const record = prepareIncident(value.record, allEvidence);
    return incidentHasContent(record) ? record : null;
  } catch { return null; }
}

export function saveIncident(storage: Pick<Storage, 'setItem'> | undefined, preview: IncidentRecord): boolean {
  const record = prepareIncident(preview, allEvidence);
  if (!storage || !incidentHasContent(record)) return false;
  try { storage.setItem(key, JSON.stringify({ version: 1, record })); return true; }
  catch { return false; }
}

export function clearIncident(storage?: Pick<Storage, 'removeItem'>): boolean {
  if (!storage) return false;
  try { storage.removeItem(key); return true; }
  catch { return false; }
}

export interface InspectionPane { id: string; hostKey: string; hostLabel: string; label: string; kind: SessionMeta['kind']; pending?: 'connecting' | 'failed'; profileId?: string }
export function groupInspection(panes: readonly InspectionPane[], live: ReadonlySet<string> | null, checked?: ReadonlySet<string>) {
  const groups = new Map<string, { key: string; label: string; profileId?: string; panes: (InspectionPane & { status: 'connecting' | 'failed' | 'live' | 'absent' | 'unknown' })[] }>();
  for (const pane of panes) {
    if (pane.kind !== 'Ssh') continue;
    const group = groups.get(pane.hostKey) ?? { key: pane.hostKey, label: pane.hostLabel, profileId: pane.profileId, panes: [] };
    group.panes.push({ ...pane, status: pane.pending ?? (!live || (checked && !checked.has(pane.id)) ? 'unknown' : live.has(pane.id) ? 'live' : 'absent') });
    groups.set(pane.hostKey, group);
  }
  return [...groups.values()];
}
