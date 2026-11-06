// Prototype adapter contract, not wired into production. Route DOM/IME/paste and
// native keyboard events HERE, not secondary onData. The fixed native authority
// must encode mode-dependent keys/mouse and own protocol replies. send() is the
// transport/filter/broadcast seam corresponding to existing sendSessionInput.
export class SessionInputRouter {
  constructor({sessionId, secondary, send}) {
    this.sessionId = sessionId;
    this.send = send;
    this.pending = null;
    this.generation = 0;
    secondary.options.disableStdin = true;
  }
  emit(kind, bytes) { return this.send({sessionId:this.sessionId, kind, bytes}); }
  userText(text) {
    if (this.pending !== null) throw new Error('input frozen during switch; retry after commit/cancel');
    return this.emit('user-text', new TextEncoder().encode(text));
  }
  userBinary(binary) {
    if (this.pending !== null) throw new Error('input frozen during switch');
    return this.emit('user-binary', Uint8Array.from(binary, c => c.charCodeAt(0) & 255));
  }
  protocol(source, bytes) {
    if (source !== 'native') throw new Error('fixed native authority only');
    // Protocol replies and PTY output ingestion must continue while user input
    // is frozen, otherwise partial UTF8/query barriers can deadlock.
    return this.emit('protocol', bytes);
  }
  beginSwitch() { this.pending = ++this.generation; return this.pending; }
  validate(token) { if (token !== this.pending) throw new Error('stale switch'); }
  commitSwitch(token, {parsed, presented}) {
    this.validate(token);
    if (!parsed || !presented) throw new Error('not ready');
    this.pending = null;
  }
  cancelSwitch(token) { this.validate(token); this.pending = null; }
}
