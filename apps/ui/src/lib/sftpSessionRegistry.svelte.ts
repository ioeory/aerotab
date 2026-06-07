import type { SshProfileSpec } from './types';

/** Tracks open SFTP browser sessions for cross-session file transfer. */

export interface RegisteredSftpSession {
  registryId: string;
  label: string;
  sessionId: string;
  cwd: string;
  profile?: SshProfileSpec;
}

class SftpSessionRegistry {
  sessions = $state<RegisteredSftpSession[]>([]);

  register(entry: RegisteredSftpSession): void {
    this.sessions = [...this.sessions.filter((s) => s.registryId !== entry.registryId), entry];
  }

  unregister(registryId: string): void {
    this.sessions = this.sessions.filter((s) => s.registryId !== registryId);
  }

  updateCwd(registryId: string, cwd: string): void {
    this.sessions = this.sessions.map((s) =>
      s.registryId === registryId ? { ...s, cwd } : s,
    );
  }

  others(registryId: string): RegisteredSftpSession[] {
    return this.sessions.filter((s) => s.registryId !== registryId && s.sessionId);
  }
}

export const sftpSessionRegistry = new SftpSessionRegistry();
