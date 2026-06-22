import type { LocalUploadTransferRequest, RemoteCrossTransferRequest } from './sftpTransferTypes';

export type TransferEnqueueHandler = {
  enqueueRemote: (request: RemoteCrossTransferRequest) => void;
  enqueueLocal: (request: LocalUploadTransferRequest) => void;
};

type Pending = {
  remote: RemoteCrossTransferRequest[];
  local: LocalUploadTransferRequest[];
};

const handlersByTab = new Map<string, TransferEnqueueHandler>();
const pendingByTab = new Map<string, Pending>();

function ensurePending(tabId: string): Pending {
  let pending = pendingByTab.get(tabId);
  if (!pending) {
    pending = { remote: [], local: [] };
    pendingByTab.set(tabId, pending);
  }
  return pending;
}

function flushPending(tabId: string, handler: TransferEnqueueHandler) {
  const pending = pendingByTab.get(tabId);
  if (!pending) return;
  for (const request of pending.remote) handler.enqueueRemote(request);
  for (const request of pending.local) handler.enqueueLocal(request);
  pendingByTab.delete(tabId);
}

export const transferTabBridge = {
  register(tabId: string, handler: TransferEnqueueHandler) {
    handlersByTab.set(tabId, handler);
    flushPending(tabId, handler);
  },
  unregister(tabId: string) {
    handlersByTab.delete(tabId);
  },
  enqueueRemote(tabId: string, request: RemoteCrossTransferRequest) {
    const h = handlersByTab.get(tabId);
    if (h) {
      h.enqueueRemote(request);
      return;
    }
    ensurePending(tabId).remote.push(request);
  },
  enqueueLocal(tabId: string, request: LocalUploadTransferRequest) {
    const h = handlersByTab.get(tabId);
    if (h) {
      h.enqueueLocal(request);
      return;
    }
    ensurePending(tabId).local.push(request);
  },
};
