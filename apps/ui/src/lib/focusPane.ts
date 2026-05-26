/** Ask the active (or given) terminal pane to focus its xterm instance. */
import { shouldFocusTerminal } from './modalFocus';
import { scheduleFitAllPanes, scheduleTerminalFit } from './terminalFit';

export function dispatchFocusPane(sessionId?: string) {
  if (!shouldFocusTerminal()) return;
  document.dispatchEvent(
    new CustomEvent('aerotab:focus-pane', { detail: { sessionId } }),
  );
}

/** Refit xterm after layout changes (tab switch, maximize, etc.). */
export function dispatchFitPane(sessionId?: string) {
  scheduleTerminalFit(() => {
    document.dispatchEvent(
      new CustomEvent('aerotab:fit-pane', { detail: { sessionId } }),
    );
  });
}

/** Refit every pane in a tab after maximize/restore (hidden panes need delayed passes). */
export function dispatchFitAllPanes(sessionIds: string[]) {
  scheduleFitAllPanes(
    (sessionId) => {
      document.dispatchEvent(
        new CustomEvent('aerotab:fit-pane', { detail: { sessionId } }),
      );
    },
    sessionIds,
  );
}
