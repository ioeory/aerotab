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
