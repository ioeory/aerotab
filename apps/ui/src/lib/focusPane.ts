/** Ask the active (or given) terminal pane to focus its xterm instance. */
export function dispatchFocusPane(sessionId?: string) {
  document.dispatchEvent(
    new CustomEvent('tabby:focus-pane', { detail: { sessionId } }),
  );
}

/** Refit xterm after layout changes (tab switch, maximize, etc.). */
export function dispatchFitPane(sessionId?: string) {
  document.dispatchEvent(
    new CustomEvent('tabby:fit-pane', { detail: { sessionId } }),
  );
}
