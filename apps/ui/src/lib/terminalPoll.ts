/** Terminal output poll intervals (ms). 0 = stopped. */

export const POLL_ACTIVE_MS = 33;
export const POLL_SPLIT_MS = 50;
export const POLL_HIDDEN_TAB_MS = 1000;

export function terminalPollIntervalMs(opts: {
  active: boolean;
  tabVisible: boolean;
  documentHidden: boolean;
}): number {
  if (opts.documentHidden) return 0;
  if (!opts.tabVisible) return POLL_HIDDEN_TAB_MS;
  if (opts.active) return POLL_ACTIVE_MS;
  return POLL_SPLIT_MS;
}
