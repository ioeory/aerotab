export interface WizardState { storage: boolean; tested: boolean; password: boolean; content: boolean; configured: boolean; preview: boolean }
export function wizardCanEnter(step: number, state: WizardState): boolean {
  return step >= 0 && step <= 4 && [state.storage, state.tested, state.password, state.content].slice(0, step).every(Boolean);
}
export function syncSetupComplete(state: WizardState): boolean {
  return wizardCanEnter(4, state) && state.configured && state.preview;
}
/** Never include credentials. Configuration changes require a new first-merge approval. */
export function syncTargetFingerprint(settings: object): string {
  const raw = settings as Record<string, unknown>;
  const ignored = new Set(['firstSyncApproved', 'approvedTarget', 'autoSyncEnabled', 'autoSyncMinutes', 'protocol', 'setupVersion']);
  return JSON.stringify(Object.keys(raw).filter(k => raw[k] !== undefined && !ignored.has(k) && !/password|passphrase|secret|token/i.test(k)).sort().map(k => [k, canonicalSyncValue(raw[k])]));
}
function canonicalSyncValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalSyncValue);
  if (value && typeof value === 'object') {
    const raw = value as Record<string, unknown>;
    return Object.fromEntries(Object.keys(raw).filter(k => raw[k] !== undefined).sort().map(k => [k, canonicalSyncValue(raw[k])]));
  }
  return value;
}
/** Accept old fingerprints only when their full non-secret configuration still matches. */
export function hasSyncApproval(settings: { firstSyncApproved?: boolean; approvedTarget?: string }): boolean {
  if (!settings.firstSyncApproved || !settings.approvedTarget) return false;
  try {
    const entries: unknown = JSON.parse(settings.approvedTarget);
    if (!Array.isArray(entries) || !entries.every(row => Array.isArray(row) && row.length === 2 && typeof row[0] === 'string')) return false;
    return syncTargetFingerprint(Object.fromEntries(entries)) === syncTargetFingerprint(settings);
  } catch { return false; }
}
