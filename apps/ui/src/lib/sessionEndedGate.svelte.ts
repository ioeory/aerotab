/** Tracks whether the active terminal pane is in the session-ended state.
 *  Used so global Enter/R hotkeys do not steal keys from the sidebar. */
class SessionEndedGate {
  activeExited = $state(false);

  setActiveExited(exited: boolean) {
    this.activeExited = exited;
  }
}

export const sessionEndedGate = new SessionEndedGate();
