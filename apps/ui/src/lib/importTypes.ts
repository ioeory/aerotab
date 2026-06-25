import type { StoredProfile } from './types';

export type ImportSourceKind = 'windterm' | 'termius' | 'ssh-config' | 'csv' | 'putty' | 'mobaxterm' | 'xshell' | 'securecrt' | 'tabby';

export type ImportCandidateStatus = 'ready' | 'duplicate' | 'error';

export interface ImportDetectPath {
  path: string;
  label: string;
}

export interface ImportDetectResult {
  paths: ImportDetectPath[];
}

export interface ImportCandidate {
  sourceId: string;
  source: string;
  name: string;
  group?: string | null;
  tags: string[];
  note?: string | null;
  kind: string;
  status: ImportCandidateStatus;
  warnings: string[];
  errorMessage?: string | null;
  duplicateOf?: string | null;
  profile?: StoredProfile | null;
}

export interface ImportPreviewStats {
  total: number;
  ready: number;
  duplicate: number;
  error: number;
}

export interface ImportPreviewResult {
  source: string;
  path?: string | null;
  candidates: ImportCandidate[];
  stats: ImportPreviewStats;
}

export interface ImportApplyResult {
  created: number;
  skipped: number;
  updated: number;
  errors: string[];
}

export interface ImportSourceCard {
  id: ImportSourceKind;
  titleKey: string;
  descKey: string;
  enabled: boolean;
}

export const IMPORT_SOURCE_CARDS: ImportSourceCard[] = [
  { id: 'windterm', titleKey: 'import.source.windterm', descKey: 'import.source.windtermDesc', enabled: true },
  { id: 'termius', titleKey: 'import.source.termius', descKey: 'import.source.termiusDesc', enabled: true },
  { id: 'ssh-config', titleKey: 'import.source.sshConfig', descKey: 'import.source.sshConfigDesc', enabled: true },
  { id: 'csv', titleKey: 'import.source.csv', descKey: 'import.source.csvDesc', enabled: true },
  { id: 'putty', titleKey: 'import.source.putty', descKey: 'import.source.puttyDesc', enabled: true },
  { id: 'mobaxterm', titleKey: 'import.source.mobaxterm', descKey: 'import.source.mobaxtermDesc', enabled: true },
  { id: 'xshell', titleKey: 'import.source.xshell', descKey: 'import.source.xshellDesc', enabled: true },
  { id: 'securecrt', titleKey: 'import.source.securecrt', descKey: 'import.source.securecrtDesc', enabled: true },
  { id: 'tabby', titleKey: 'import.source.tabby', descKey: 'import.source.tabbyDesc', enabled: true },
];
