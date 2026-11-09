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
  return JSON.stringify(Object.keys(raw).filter(k => !ignored.has(k) && !/password|passphrase|secret|token/i.test(k)).sort().map(k => [k, raw[k]]));
}
