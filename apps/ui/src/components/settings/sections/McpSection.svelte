<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { Plug } from '@lucide/svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import { i18n } from '../../../lib/i18n.svelte';
  import { settingsCoord } from '../../../lib/settingsStore.svelte';
  import { defaultMcpConfig, validateMcpConfig, usableMcpUrl, mcpClientConfig, type McpStatus, type McpClient, type McpPermission } from '../../../lib/mcp';

  let { rpc, onError }: { rpc: RpcClient; onError: (message: string) => void } = $props();
  let config = $state(defaultMcpConfig());
  let roots = $state('');
  let status = $state<McpStatus>({ running: false, url: null, error: null, tokenConfigured: false });
  let token = $state<string | null>(null);
  let busy = $state(false);
  let loaded = $state(false);
  let message = $state('');
  const permissions = Object.keys(defaultMcpConfig().permissions) as McpPermission[];
  const clients: McpClient[] = ['cursor', 'claude', 'generic', 'stdio'];
  const url = $derived(usableMcpUrl(status));

  function dirty() { settingsCoord.markDirty('mcp'); message = ''; }
  function report(error: unknown) { onError(`MCP: ${error instanceof Error ? error.message : String(error)}`); }

  async function refresh() { status = await rpc.call<McpStatus>('mcp.status'); }

  async function load() {
    busy = true;
    try {
      const result = await rpc.call<{ value: unknown }>('settings.get', { key: 'mcp' });
      if (result.value && typeof result.value === 'object') {
        // Whitelist fields: token/key material must never enter the settings payload.
        const value = result.value as Record<string, unknown>;
        for (const key of ['enabled', 'allowRemoteBind'] as const) {
          if (typeof value[key] === 'boolean') config[key] = value[key];
        }
        for (const key of ['bindHost', 'additionalContext', 'tlsCertPath', 'tlsKeyPath'] as const) {
          if (typeof value[key] === 'string') config[key] = value[key];
        }
        if (typeof value.port === 'number') config.port = value.port;
        if (Array.isArray(value.localRoots)) roots = value.localRoots.filter((root): root is string => typeof root === 'string').join('\n');
        if (value.permissions && typeof value.permissions === 'object') {
          const saved = value.permissions as Record<string, unknown>;
          for (const key of permissions) if (typeof saved[key] === 'boolean') config.permissions[key] = saved[key];
        }
      }
      await refresh();
      loaded = true;
    } catch (error) { report(error); }
    finally { busy = false; }
  }

  async function save() {
    if (!loaded) return;
    if (busy) throw new Error(i18n.t('mcp.busy'));
    config.localRoots = roots.split(/\r?\n/).map((root) => root.trim()).filter(Boolean);
    const invalid = validateMcpConfig(config);
    if (invalid) throw new Error(i18n.t(`mcp.validation.${invalid}`));
    busy = true;
    try {
      status = await rpc.call<McpStatus>('mcp.configure', { config: JSON.parse(JSON.stringify(config)) });
      if (status.error) throw new Error(status.error);
      message = i18n.t('mcp.applied');
    } finally { busy = false; }
  }

  async function action(run: () => Promise<void>) {
    if (busy) return;
    busy = true;
    message = '';
    try { await run(); } catch (error) { report(error); }
    finally { busy = false; }
  }

  async function reveal() {
    await action(async () => {
      token = (await rpc.call<{ token: string }>('mcp.getToken', {})).token;
      await refresh();
    });
  }

  async function rotate() {
    await action(async () => {
      // Rotation returns a secret, but never reveals it implicitly.
      await rpc.call<{ token: string }>('mcp.rotateToken', {});
      token = null;
      await refresh();
      message = i18n.t('mcp.rotated');
    });
  }

  async function copy(client: McpClient) {
    await action(async () => {
      await navigator.clipboard.writeText(JSON.stringify(mcpClientConfig(client, url, token), null, 2));
      message = i18n.t(token || client === 'stdio' ? 'mcp.copied' : 'mcp.copiedPlaceholder');
    });
  }

  onMount(() => { settingsCoord.registerSaver('mcp', save); void load(); });
  onDestroy(() => { token = null; settingsCoord.unregisterSaver('mcp'); });
</script>

<div class="settings-section">
  <h2 class="flex items-center gap-2"><Plug size={16} />{i18n.t('settings.nav.mcp')}</h2>
  <p class="hint">{i18n.t('mcp.intro')}</p>
  <fieldset disabled={busy || !loaded}>
    <legend class="section-h">{i18n.t('mcp.server')}</legend>
    <label class="row"><span>{i18n.t('mcp.enabled')}</span><input type="checkbox" bind:checked={config.enabled} onchange={dirty} /></label>
    <label class="row"><span>{i18n.t('mcp.bindHost')}</span><input type="text" bind:value={config.bindHost} oninput={dirty} /></label>
    <label class="row"><span>{i18n.t('mcp.port')}</span><input type="number" min="0" max="65535" step="1" bind:value={config.port} oninput={dirty} /></label>
    <p class="hint">{i18n.t('mcp.portHint')}</p>
    <label class="row"><span>{i18n.t('mcp.allowRemote')}</span><input type="checkbox" bind:checked={config.allowRemoteBind} onchange={dirty} /></label>
    <label class="row"><span>{i18n.t('mcp.certPath')}</span><input type="text" bind:value={config.tlsCertPath} oninput={dirty} /></label>
    <label class="row"><span>{i18n.t('mcp.keyPath')}</span><input type="text" bind:value={config.tlsKeyPath} oninput={dirty} /></label>
    <p class="hint">{i18n.t('mcp.tlsHint')}</p>
    <label class="row align-top"><span>{i18n.t('mcp.context')}</span><textarea rows="3" bind:value={config.additionalContext} oninput={dirty}></textarea></label>
    <label class="row align-top"><span>{i18n.t('mcp.localRoots')}</span><textarea rows="3" bind:value={roots} oninput={dirty}></textarea></label>
    <p class="hint">{i18n.t('mcp.rootsHint')}</p>
    <div class="section-h">{i18n.t('mcp.permissions')}</div>
    <p class="hint">{i18n.t('mcp.permissionsHint')}</p>
    {#each permissions as permission (permission)}
      <label class="row"><span>{i18n.t(`mcp.permission.${permission}`)}</span><input type="checkbox" bind:checked={config.permissions[permission]} onchange={dirty} /></label>
    {/each}
  </fieldset>

  <div class="section-h">{i18n.t('mcp.status')}</div>
  <div class="flex items-center gap-2 flex-wrap">
    <span>{i18n.t(status.running ? 'mcp.running' : 'mcp.stopped')}</span>
    <button type="button" class="btn-secondary" disabled={busy} onclick={() => action(refresh)}>{i18n.t('common.refresh')}</button>
  </div>
  {#if url}<p class="hint break-all">{url}</p>{/if}
  {#if status.error}<p role="alert" class="hint text-[var(--color-danger)]">{status.error}</p>{/if}
  <p class="hint">{i18n.t('mcp.saveHint')}</p>

  <div class="section-h">{i18n.t('mcp.token')}</div>
  <p class="hint">{i18n.t(status.tokenConfigured ? 'mcp.tokenConfigured' : 'mcp.tokenMissing')}</p>
  <div class="flex gap-2 flex-wrap mt-2">
    <button type="button" class="btn-secondary" disabled={busy || !status.tokenConfigured} onclick={() => token ? token = null : void reveal()}>{i18n.t(token ? 'mcp.hide' : 'mcp.reveal')}</button>
    <button type="button" class="btn-secondary" disabled={busy} onclick={rotate}>{i18n.t('mcp.rotate')}</button>
  </div>
  {#if token}<label class="row"><span>{i18n.t('mcp.token')}</span><input type="text" readonly value={token} spellcheck="false" autocomplete="off" /></label>{/if}
  <p class="hint">{i18n.t('mcp.tokenHint')}</p>

  <div class="section-h">{i18n.t('mcp.clients')}</div>
  <p class="hint">{i18n.t('mcp.clientsHint')}</p>
  <div class="flex gap-2 flex-wrap mt-2">
    {#each clients as client (client)}
      <button type="button" class="btn-secondary" disabled={busy || (client !== 'stdio' && !url)} onclick={() => copy(client)}>{i18n.t(`mcp.copy.${client}`)}</button>
    {/each}
  </div>
  <p class="hint">{i18n.t('mcp.stdioHint')}</p>
  <p class="hint" role="status" aria-live="polite">{message}</p>
</div>

<style>
  fieldset { border: 0; padding: 0; margin: 0; min-width: 0; }
  .section-h { margin-top: 16px; margin-bottom: 6px; font-size: 11.5px; color: var(--color-fg-muted); letter-spacing: 0.04em; }
  .hint { color: var(--color-fg-muted); font-size: 12px; margin: 8px 0; max-width: 600px; }
  .row { display: grid; grid-template-columns: 220px 1fr; align-items: center; gap: 10px; padding: 4px 0; }
  .row.align-top { align-items: flex-start; }
  .row input[type='text'], .row input[type='number'], .row textarea { padding: 4px 8px; background: var(--color-bg-soft); color: var(--color-fg); border: 1px solid var(--color-border); border-radius: 4px; font-size: 12.5px; width: 100%; min-width: 0; max-width: 380px; font-family: inherit; }
  .row textarea { resize: vertical; }
  .row input:focus-visible, .row textarea:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; }
  fieldset:disabled { opacity: 0.6; }
</style>
