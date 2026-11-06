// One instance per session, single ordered caller. Only UTF-8 prefix buffering
// (<=3 bytes), never ANSI buffering. Real xterm STRING write callbacks gate ACK.
export class Utf8Bridge {
  constructor(term) {
    this.term = term;
    this.decoder = new TextDecoder('utf-8', { fatal: false, ignoreBOM: true });
    this.prefix = [];
    this.maxPendingBytes = 0;
    this.callbacks = 0;
  }
  get pendingBytes() { return this.prefix.length; }
  async apply(text) {
    // Even empty decode results use a REAL callback; never fabricate a parser ACK.
    await new Promise(resolve => this.term.write(text, () => { this.callbacks++; resolve(); }));
  }
  async write(bytes) {
    let text = '';
    // Native vte 0.15 is NOT WHATWG replacement policy: raw C1 bytes execute,
    // and an incomplete scalar at EOF stays unprinted. Frame at most one scalar
    // and send valid scalars through the persistent TextDecoder. No ANSI buffering.
    for (const byte of bytes) {
      if (this.prefix.length) {
        const lead = this.prefix[0];
        let lo = 0x80, hi = 0xbf;
        if (this.prefix.length === 1) {
          if (lead === 0xe0) lo = 0xa0;
          if (lead === 0xed) hi = 0x9f;
          if (lead === 0xf0) lo = 0x90;
          if (lead === 0xf4) hi = 0x8f;
        }
        if (byte >= lo && byte <= hi) {
          this.prefix.push(byte);
          const size = lead < 0xe0 ? 2 : lead < 0xf0 ? 3 : 4;
          if (this.prefix.length === size) {
            text += this.decoder.decode(Uint8Array.from(this.prefix), { stream: true });
            this.prefix = [];
          }
          continue;
        }
        text += '\ufffd';
        this.prefix = [];
      }
      if (byte < 0x80 || (byte >= 0x80 && byte <= 0x9f)) text += String.fromCharCode(byte);
      else if (byte >= 0xc2 && byte <= 0xf4) this.prefix = [byte];
      else text += '\ufffd';
    }
    this.maxPendingBytes = Math.max(this.maxPendingBytes, this.pendingBytes);
    await this.apply(text);
  }
  async eof() {
    // Native vte has no EOF flush API: pending incomplete UTF8 is not printed.
    // Decoder only received COMPLETE scalars, so its flush cannot invent U+FFFD.
    await this.apply(this.decoder.decode());
    this.prefix = [];
  }
}
