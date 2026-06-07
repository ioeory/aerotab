import type { SshProfileSpec } from './types';

export interface RemoteCrossTransferRequest {
  sourceSessionId: string;
  sourceLabel?: string;
  sourceProfile?: SshProfileSpec;
  sourcePath: string;
  sourceKind: 'File' | 'Dir';
  sourceSize: number;
  destSessionId: string;
  destLabel: string;
  destProfile?: SshProfileSpec;
  destDir: string;
  destPath: string;
  name: string;
}


export interface LocalUploadTransferRequest {
  sourcePath: string;
  sourceKind: 'File' | 'Dir';
  sourceSize: number;
  destSessionId: string;
  destLabel: string;
  destProfile?: SshProfileSpec;
  destDir: string;
  destPath: string;
  name: string;
}
