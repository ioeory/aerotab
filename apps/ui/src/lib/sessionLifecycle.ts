export const SESSIONS_CLOSING = 'aerotab:sessions-closing';

export interface SessionsClosingDetail {
  sessionIds: string[];
}

export function notifySessionsClosing(sessionIds: string[]): void {
  if (sessionIds.length === 0) return;
  document.dispatchEvent(
    new CustomEvent<SessionsClosingDetail>(SESSIONS_CLOSING, {
      detail: { sessionIds },
    }),
  );
}
