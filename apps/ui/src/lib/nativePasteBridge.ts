// @ts-ignore -- explicit extension supports Node strip-types regression tests.
import { tracePaste } from './nativePasteTrace.ts';
/** Targeted main-WebView event. Clipboard contents must never be logged. */
export const NATIVE_PASTE_EVENT = 'aerotab:native-paste-request';
/** HWND-owned Ctrl+=/- and Ctrl+wheel font zoom while native-surface is focused. */
export const NATIVE_FONT_DELTA_EVENT = 'aerotab:native-font-delta';
export interface PasteRequest { sessionId: string; generation: string; requestId: number; text: string }
export interface FontDeltaRequest { sessionId: string; generation: string; delta: number }
export function createPasteRequestGate() {
  let disposed = false;
  let last = 0;
  let pending = false;
  return {
    accept(request: PasteRequest, context: { sessionId: string; generation: string | null; eligible: boolean }) {
      tracePaste('request', 'received', request?.requestId);
      if (disposed || !request || request.sessionId !== context.sessionId
        || !context.generation || request.generation !== context.generation
        || !Number.isSafeInteger(request.requestId) || request.requestId <= last || typeof request.text !== 'string') {
        tracePaste('request', disposed ? 'disposed' : !request || request.sessionId !== context.sessionId ? 'session'
          : !context.generation || request.generation !== context.generation ? 'generation'
          : request.requestId <= last ? 'duplicate' : 'request', request?.requestId);
        return null;
      }
      last = request.requestId; // Rejected hidden/pending events cannot be replayed later.
      if (pending || !context.eligible) { tracePaste('request', pending ? 'pending' : 'ineligible', request.requestId); return null; }
      tracePaste('request', 'accepted', request.requestId);
      pending = true;
      const generation = request.generation;
      return {
        // Occlusion/focus changes during confirmation are normal. Replacement is not.
        valid: (current: string | null) => {
          tracePaste('token', disposed ? 'disposed' : current !== generation ? 'generation' : 'accepted', request.requestId);
          return !disposed && current === generation;
        },
        finish: () => { tracePaste('finish', 'completed', request.requestId); pending = false; },
      };
    },
    dispose() { disposed = true; },
  };
}
export async function preparePasteText(raw: string, policy: {
  trim: boolean; flatten: boolean; warn: boolean;
  confirm: (count: number) => Promise<boolean>;
}, valid?: () => Promise<boolean>): Promise<string | null> {
  // Native clipboard CR/CRLF must be visible to the existing multiline policy.
  let text = valid ? raw.replace(/\r\n|\r/g, '\n') : raw;
  if (policy.trim) text = text.replace(/[ \t]+$/gm, '');
  if (policy.flatten) text = text.replace(/\r?\n/g, ' ');
  if (policy.warn && text.includes('\n') && text.length > 200) {
    const approved = await policy.confirm(text.length);
    if (valid) tracePaste('confirm', approved ? 'approved' : 'denied');
    if (!approved) return null;
  }
  if (valid) {
    if (!(await valid())) { tracePaste('policy', 'invalid'); return null; }
    tracePaste('policy', 'accepted');
    return sanitizeNativePaste(text);
  }
  return text;
}

/** Applied AFTER existing trim/flatten/confirmation; never hide LF from that policy. */
export function sanitizeNativePaste(text: string): string {
  return text.replace(/[\x00\x1b\x80-\x9f]/g, '').replace(/\r\n|\n/g, '\r');
}
/** Same Tauri 2 event wire API used elsewhere in the application, scoped to main. */
export function listenNativePaste(handler: (request: PasteRequest) => void): Promise<() => void> {
  return listenNativeEvent(NATIVE_PASTE_EVENT, handler);
}
export async function listenNativeEvent<T>(eventName: string, handler: (request: T) => void): Promise<() => void> {
  const host = window as unknown as {
    __TAURI__?: { event?: { listen: (event: string, callback: (event: { payload: T }) => void, options: unknown) => Promise<() => void> } };
    __TAURI_INTERNALS__?: {
      transformCallback: (callback: (event: { payload: T }) => void) => number;
      unregisterCallback: (id: number) => void;
      invoke: (command: string, args: Record<string, unknown>) => Promise<unknown>;
    };
  };
  const target = { kind: 'WebviewWindow', label: 'main' };
  const callback = (event: { payload: T }) => handler(event.payload);
  if (host.__TAURI__?.event?.listen) return host.__TAURI__.event.listen(eventName, callback, { target });
  const internal = host.__TAURI_INTERNALS__;
  if (!internal) throw new Error('Native clipboard event bridge unavailable');
  const id = internal.transformCallback(callback);
  try {
    const eventId = await internal.invoke('plugin:event|listen', { event: eventName, target, handler: id });
    return () => {
      internal.unregisterCallback(id);
      void internal.invoke('plugin:event|unlisten', { event: eventName, eventId }).catch(() => {});
    };
  } catch (error) { internal.unregisterCallback(id); throw error; }
}
