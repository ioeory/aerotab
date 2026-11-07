import type { RpcClient } from './rpc';
import type { LocalUploadTransferRequest, RemoteCrossTransferRequest } from './sftpTransferTypes';
import { getFileTransferService } from './fileTransferService';
export type TransferEnqueueHandler = { enqueueRemote(request: RemoteCrossTransferRequest): void; enqueueLocal(request: LocalUploadTransferRequest): void };
let activeRpc: RpcClient | undefined;
/** Compatibility shim: tab IDs no longer own execution or hold pending jobs. */
export const transferTabBridge = {
  configure(rpc: RpcClient) { activeRpc = rpc; },
  register(_tabId: string, _handler: TransferEnqueueHandler) {},
  unregister(_tabId: string) {},
  enqueueRemote(_tabId: string, r: RemoteCrossTransferRequest) {
    if (!activeRpc) throw new Error('File transfer service is not initialized');
    return getFileTransferService(activeRpc).queue.enqueue({ name: r.name, source: { kind: 'remote', sessionId: r.sourceSessionId, profileId: r.sourceProfileId, path: r.sourcePath, label: r.sourceLabel }, destination: { kind: 'remote', sessionId: r.destSessionId, profileId: r.destProfileId, path: r.destPath, label: r.destLabel }, entryKind: r.sourceKind, size: r.sourceSize });
  },
  enqueueLocal(_tabId: string, r: LocalUploadTransferRequest) {
    if (!activeRpc) throw new Error('File transfer service is not initialized');
    return getFileTransferService(activeRpc).queue.enqueue({ name: r.name, source: { kind: 'local', path: r.sourcePath }, destination: { kind: 'remote', sessionId: r.destSessionId, profileId: r.destProfileId, path: r.destPath, label: r.destLabel }, entryKind: r.sourceKind, size: r.sourceSize });
  },
};
