// Preserve the exact reviewed snapshot while its ID is still pending.
export function selectApproval<T extends { id: string }>(current: T | null, pending: T[]): T | null {
  if (current && pending.some(plan => plan.id === current.id)) return current;
  return pending[0] ?? null;
}
