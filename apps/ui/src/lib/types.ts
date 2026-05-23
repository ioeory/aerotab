// Shared data types mirroring the Rust IPC surface.

export interface SessionMeta {
  id: string;
  kind: 'LocalShell' | 'Ssh' | 'Serial' | string;
  title: string;
  /** Frontend-only source profile id for reopening/splitting SSH panes. */
  profileId?: string;
  /** Frontend-only inline SSH profile for quick-connect / ssh_config panes. */
  sshProfile?: SshProfileSpec;
  /** Frontend-only shell command metadata for duplicate splits. */
  shellCommand?: string;
  shellArgs?: string[];
}

export interface SettingEntry {
  key: string;
  value: unknown;
}

export interface KnownHostEntry {
  host: string;
  key_type: string;
  key_b64: string;
}

export interface PluginRow {
  name: string;
  path: string;
}

export type SshAuth =
  | { Password: { secret: string } }
  | { PublicKey: { key_path: string; passphrase?: string } }
  | 'Agent';

export type ProfileIconKind = 'builtin' | 'emoji' | 'file' | 'data' | string;

export interface ProfileIcon {
  kind: ProfileIconKind;
  value: string;
}

export interface SshProfileSpec {
  host: string;
  port: number;
  user: string;
  auth: SshAuth;
  /** Multi-hop bastion chain. Dialed left-to-right; final hop reaches the
   * target described by this profile. Empty = direct dial. */
  jump_via: SshProfileSpec[];
}

export type TunnelKind = 'local' | 'remote' | 'dynamic';

export interface TunnelMeta {
  id: string;
  kind: TunnelKind;
  bind_host: string;
  bind_port: number;
  target_host: string;
  target_port: number;
  ssh_host: string;
  ssh_user: string;
  status: string;
  error?: string;
}

export interface HostStats {
  hostname?: string | null;
  kernel?: string | null;
  uptime_seconds?: number | null;
  load1?: number | null;
  cpu_percent?: number | null;
  mem_total_kb?: number | null;
  mem_used_kb?: number | null;
  mem_percent?: number | null;
  disk_total_kb?: number | null;
  disk_used_kb?: number | null;
  disk_percent?: number | null;
}

export type SftpKind = 'File' | 'Dir' | 'Symlink' | 'Other';

export interface SftpEntry {
  name: string;
  kind: SftpKind;
  size: number;
  mode: number;
  mtime: number | null;
}

export interface StoredProfile {
  schemaVersion?: number;
  id: string;
  name: string;
  group?: string | null;
  tags?: string[];
  icon?: ProfileIcon | null;
  favorite?: boolean;
  kind: 'ssh';
  ssh: SshProfileSpec;
}

export type ProfileHealthStatus = 'ok' | 'warning' | 'error';

export interface ProfileHealthCheck {
  name: string;
  status: ProfileHealthStatus;
  message: string;
}

export interface ProfileHealthResult {
  id: string;
  name: string;
  status: ProfileHealthStatus;
  checks: ProfileHealthCheck[];
}

export type SerialParity = 'None' | 'Even' | 'Odd';
export type SerialStopBits = 'One' | 'Two';
export type SerialFlow = 'None' | 'Software' | 'Hardware';

export interface SerialProfileSpec {
  port: string;
  baud: number;
  data_bits: number;
  parity: SerialParity;
  stop_bits: SerialStopBits;
  flow_control: SerialFlow;
}
