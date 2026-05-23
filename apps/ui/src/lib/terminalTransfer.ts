export type TerminalTransferProtocol = 'trzsz' | 'zmodem';
export type TerminalTransferDirection = 'upload' | 'download' | 'unknown';

export interface TerminalTransferDetection {
  protocol: TerminalTransferProtocol;
  direction: TerminalTransferDirection;
  marker: string;
}

const MAX_BUFFER = 8192;

function trimBuffer(value: string): string {
  return value.length > MAX_BUFFER ? value.slice(value.length - MAX_BUFFER) : value;
}

function findTrzsz(buffer: string): TerminalTransferDetection | null {
  const match = buffer.match(/::TRZSZ:TRANSFER(?::[A-Z0-9_+./=-]+){0,8}/i);
  if (!match) return null;
  return { protocol: 'trzsz', direction: 'unknown', marker: match[0].slice(0, 80) };
}

function findZmodem(buffer: string): TerminalTransferDetection | null {
  if (/rz\s+waiting\s+to\s+receive/i.test(buffer) || /\*\*\x18B0100[0-9A-Fa-f]{8,}/.test(buffer)) {
    return { protocol: 'zmodem', direction: 'upload', marker: 'rz' };
  }
  if (/sz\s+.*(?:sending|file)/i.test(buffer) || /\*\*\x18B0000[0-9A-Fa-f]{8,}/.test(buffer)) {
    return { protocol: 'zmodem', direction: 'download', marker: 'sz' };
  }
  if (/\*\*\x18B0[0-9A-Fa-f]{12,}/.test(buffer)) {
    return { protocol: 'zmodem', direction: 'unknown', marker: 'zmodem' };
  }
  return null;
}

export class TerminalTransferDetector {
  #buffer = '';
  #lastKey = '';
  #lastAt = 0;

  reset(): void {
    this.#buffer = '';
    this.#lastKey = '';
    this.#lastAt = 0;
  }

  push(text: string, now = Date.now()): TerminalTransferDetection | null {
    if (!text) return null;
    this.#buffer = trimBuffer(this.#buffer + text);
    const detection = findTrzsz(this.#buffer) ?? findZmodem(this.#buffer);
    if (!detection) return null;

    const key = `${detection.protocol}:${detection.direction}:${detection.marker}`;
    if (key === this.#lastKey && now - this.#lastAt < 5000) return null;
    this.#lastKey = key;
    this.#lastAt = now;
    return detection;
  }
}