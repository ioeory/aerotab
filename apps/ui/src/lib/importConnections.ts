/** Open the global Import connections wizard (Settings or command palette). */
export const IMPORT_CONNECTIONS_OPEN = 'aerotab:import-connections-open';

export function requestImportConnections(): void {
  if (typeof document === 'undefined') return;
  document.dispatchEvent(new CustomEvent(IMPORT_CONNECTIONS_OPEN));
}
