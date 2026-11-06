/**
 * Live terminal frontend status for the app footer.
 * TerminalPane reports the renderer that actually mounted for each session.
 */

import { traceView } from './viewCausalTrace';

export type TerminalFrontendKind = 'native-surface' | 'native' | 'xterm-canvas' | 'xterm-webgl' | 'xterm-dom' | 'pending' | 'unavailable';

class TerminalUiStore {
  /** sessionId → actual mounted frontend */
  bySession = $state<Record<string, TerminalFrontendKind>>({});
  /** Bumped on every report/clear so footer `$derived` always invalidates. */
  revision = $state(0);

  private nextRequest = 0;
  private requests = new Map<string, number>();

  beginRequest(sessionId: string): number {
    const epoch = ++this.nextRequest;
    this.requests.set(sessionId, epoch);
    this.publish(sessionId, 'pending');
    return epoch;
  }

  requestEpoch(sessionId: string): number | undefined { return this.requests.get(sessionId); }

  finishRequest(sessionId: string, epoch: number | undefined, kind: TerminalFrontendKind): boolean {
    if (epoch === undefined || this.requests.get(sessionId) !== epoch) return false;
    this.requests.delete(sessionId);
    this.publish(sessionId, kind);
    return true;
  }

  report(sessionId: string, kind: TerminalFrontendKind) {
    if (this.requests.has(sessionId)) return; // only the matching transition may publish a terminal outcome
    this.publish(sessionId, kind);
  }

  private publish(sessionId: string, kind: TerminalFrontendKind) {
    traceView({ stage: 'store.report', status: kind });
    if (this.bySession[sessionId] === kind) return;
    this.bySession = { ...this.bySession, [sessionId]: kind };
    this.revision += 1;
  }

  clear(sessionId: string) {
    this.requests.delete(sessionId);
    if (!(sessionId in this.bySession)) return;
    const next = { ...this.bySession };
    delete next[sessionId];
    this.bySession = next;
    this.revision += 1;
  }
}

export const terminalUi = new TerminalUiStore();
