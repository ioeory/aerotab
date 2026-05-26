import type { ProfileHealthResult } from './types';

export function summarizeHealthResults(results: ProfileHealthResult[]): {
  ok: number;
  warning: number;
  error: number;
} {
  return {
    ok: results.filter((r) => r.status === 'ok').length,
    warning: results.filter((r) => r.status === 'warning').length,
    error: results.filter((r) => r.status === 'error').length,
  };
}

/** One line per profile that is not fully healthy (for confirm dialog body). */
export function healthIssueDetailText(results: ProfileHealthResult[]): string {
  return results
    .filter((r) => r.status !== 'ok')
    .map((r) => {
      const issues = r.checks
        .filter((c) => c.status !== 'ok')
        .map((c) => `${c.name}: ${c.message}`)
        .join('; ');
      return `${r.name}: ${issues || r.status}`;
    })
    .join('\n');
}
