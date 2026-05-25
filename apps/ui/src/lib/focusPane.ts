/** Ask the active (or given) terminal pane to focus its xterm instance. */
export function dispatchFocusPane(sessionId?: string) {
  document.dispatchEvent(
    new CustomEvent('aerotab:focus-pane', { detail: { sessionId } }),
  );
}

/** Refit xterm after layout changes (tab switch, maximize, etc.). */
export function dispatchFitPane(sessionId?: string) {
  document.dispatchEvent(
    new CustomEvent('aerotab:fit-pane', { detail: { sessionId } }),
  );
}

/** Refit every pane in a tab after maximize/restore (hidden panes need a second pass). */
export function dispatchFitAllPanes(sessionIds: string[]) {
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      for (const id of sessionIds) dispatchFitPane(id);
    });
  });
}
