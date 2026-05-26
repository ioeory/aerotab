/**
 * Schedule xterm FitAddon runs after layout settles (maximize/restore, display:none,
 * tab switch, window resize). A single rAF is not enough when DOM size was 0.
 */

const FIT_DELAY_MS = [0, 50, 120] as const;

/** Run `run` after paint and again at short delays so cols/rows match the container. */
export function scheduleTerminalFit(run: () => void): void {
  if (typeof window === 'undefined') {
    run();
    return;
  }
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      run();
      for (const ms of FIT_DELAY_MS) {
        window.setTimeout(run, ms);
      }
    });
  });
}

/** Refit all listed session panes (maximize/restore, split drag end, window resize). */
export function scheduleFitAllPanes(fitPane: (sessionId: string) => void, sessionIds: string[]): void {
  scheduleTerminalFit(() => {
    for (const id of sessionIds) fitPane(id);
  });
}
