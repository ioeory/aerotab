/**
 * Live terminal frontend status for the app footer.
 * TerminalPane reports the renderer that actually mounted for each session.
 */

export type TerminalFrontendKind = 'native' | 'xterm-canvas' | 'xterm-webgl' | 'xterm-dom' | 'pending';

class TerminalUiStore {
  /** sessionId → actual mounted frontend */
  bySession = $state<Record<string, TerminalFrontendKind>>({});
  /** Bumped on every report/clear so footer `$derived` always invalidates. */
  revision = $state(0);

  report(sessionId: string, kind: TerminalFrontendKind) {
    if (this.bySession[sessionId] === kind) return;
    this.bySession = { ...this.bySession, [sessionId]: kind };
    this.revision += 1;
  }

  clear(sessionId: string) {
    if (!(sessionId in this.bySession)) return;
    const next = { ...this.bySession };
    delete next[sessionId];
    this.bySession = next;
    this.revision += 1;
  }
}

export const terminalUi = new TerminalUiStore();
