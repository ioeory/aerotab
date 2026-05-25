<script lang="ts">
  // Application: updater controls + core version display.

  import { onDestroy, onMount } from 'svelte';
  import { Download, FileDown, RefreshCw, Trash2 } from '@lucide/svelte';
  import { tauriInvoke, type RpcClient } from '../../../lib/rpc';
  import { diagnostics, exportDiagnosticPack } from '../../../lib/diagnostics.svelte';
  import { i18n, type LocaleSetting } from '../../../lib/i18n.svelte';
  import { settingsCoord } from '../../../lib/settingsStore.svelte';
  import { appConfirm } from '../../../lib/confirm.svelte';
  import type { SessionMeta } from '../../../lib/types';

  interface Props {
    rpc: RpcClient;
    buildId: string;
    onError: (msg: string) => void;
  }
  let { rpc, buildId, onError }: Props = $props();

  let coreVersion = $state<string | null>(null);
  let protocolVersion = $state<number | null>(null);
  let sessions = $state<SessionMeta[]>([]);
  let sessionsBusy = $state(false);
  let sessionsStatus = $state('');

  let updateStatus = $state('');
  let updateAvailable = $state<{ version: string; current: string; notes?: string } | null>(null);
  let updateBusy = $state(false);
  let locale = $state<LocaleSetting>('system');
  let diagnosticsBusy = $state(false);
  let diagnosticsStatus = $state('');

  const localeOptions: LocaleSetting[] = ['system', 'en', 'zh-CN'];

  async function loadVersion() {
    try {
      const v = await rpc.call<{ version: string }>('core.version');
      coreVersion = v.version;
      protocolVersion = await rpc.call<number>('core.protocolVersion', {});
    } catch {
      coreVersion = null;
      protocolVersion = null;
    }
  }

  async function refreshSessions() {
    sessionsBusy = true;
    sessionsStatus = '';
    try {
      sessions = await rpc.call<SessionMeta[]>('session.list', {});
      sessionsStatus = i18n.t('application.sessionCount', {
        count: sessions.length,
        suffix: sessions.length === 1 ? '' : 's',
      });
    } catch (e) {
      onError(`session list: ${(e as Error).message}`);
    } finally {
      sessionsBusy = false;
    }
  }

  async function closeSession(id: string) {
    if (!(await appConfirm(i18n.t('application.closeBackendSessionConfirm')))) return;
    sessionsBusy = true;
    try {
      await rpc.call('session.close', { id });
      await refreshSessions();
    } catch (e) {
      onError(`session close: ${(e as Error).message}`);
    } finally {
      sessionsBusy = false;
    }
  }

  async function checkForUpdates() {
    updateStatus = i18n.t('application.update.checking');
    updateAvailable = null;
    updateBusy = true;
    try {
      const p = tauriInvoke<{ available: boolean; version?: string; current?: string; notes?: string }>(
        'check_update',
      );
      if (!p) { updateStatus = i18n.t('application.update.unavailableDev'); return; }
      const r = await p;
      if (r.available && r.version && r.current) {
        updateAvailable = { version: r.version, current: r.current, notes: r.notes };
        updateStatus = i18n.t('application.update.available', { version: r.version, current: r.current });
      } else {
        updateStatus = i18n.t('application.update.upToDate');
      }
    } catch (e) {
      updateStatus = i18n.t('application.update.error', { message: (e as Error).message ?? String(e) });
    } finally {
      updateBusy = false;
    }
  }

  async function installUpdate() {
    if (!updateAvailable) return;
    updateStatus = i18n.t('application.update.downloading');
    updateBusy = true;
    try {
      const p = tauriInvoke<void>('install_update');
      if (!p) { updateStatus = i18n.t('application.update.unavailableDev'); return; }
      await p;
      updateStatus = i18n.t('application.update.installed');
    } catch (e) {
      updateStatus = i18n.t('application.update.error', { message: (e as Error).message ?? String(e) });
    } finally {
      updateBusy = false;
    }
  }

  async function exportDiagnostics() {
    diagnosticsBusy = true;
    diagnosticsStatus = '';
    try {
      const result = await exportDiagnosticPack(buildId, coreVersion);
      diagnosticsStatus = result === 'cancelled'
        ? i18n.t('application.diagnostics.cancelled')
        : i18n.t('application.diagnostics.exported');
    } catch (e) {
      const message = (e as Error).message ?? String(e);
      diagnostics.record('app', 'diagnostics.export', message, 'error');
      onError(`diagnostics: ${message}`);
    } finally {
      diagnosticsBusy = false;
    }
  }

  async function clearDiagnostics() {
    if (!(await appConfirm(i18n.t('application.diagnostics.clearConfirm'), { danger: true, confirmLabel: i18n.t('common.delete') }))) return;
    diagnostics.clear();
    diagnosticsStatus = i18n.t('application.diagnostics.cleared');
  }

  async function loadApplicationSettings() {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'application' });
      if (r.value && typeof r.value === 'object') {
        const configured = (r.value as Record<string, unknown>).locale;
        if (configured === 'system' || configured === 'en' || configured === 'zh-CN') {
          locale = configured;
        }
      }
      i18n.setLocale(locale);
    } catch {
      locale = 'system';
      i18n.setLocale(locale);
    }
  }

  async function saveApplicationSettings() {
    await rpc.call('settings.set', { key: 'application', value: { locale } });
    i18n.setLocale(locale);
  }

  async function changeLocale(next: LocaleSetting) {
    locale = next;
    i18n.setLocale(locale);
    try {
      await saveApplicationSettings();
      settingsCoord.bumpRev();
    } catch (e) {
      onError(`locale: ${(e as Error).message}`);
    }
  }

  onMount(() => {
    void loadVersion();
    void loadApplicationSettings();
    settingsCoord.registerSaver('application', saveApplicationSettings);
  });
  onDestroy(() => settingsCoord.unregisterSaver('application'));
</script>

<div class="settings-section">
  <h2>{i18n.t('application.title')}</h2>

  <div>
    <div class="section-h">{i18n.t('application.version')}</div>
    <div class="text-[var(--color-fg)]">
      AeroTab — core {coreVersion ?? '…'}
    </div>
    <div class="help">{i18n.t('application.protocolVersion', { version: protocolVersion ?? '…' })}</div>
  </div>

  <div>
    <div class="section-h">{i18n.t('application.language')}</div>
    <label for="app-locale" class="lbl">{i18n.t('application.language')}</label>
    <select
      id="app-locale"
      bind:value={locale}
      onchange={(event) => void changeLocale((event.currentTarget as HTMLSelectElement).value as LocaleSetting)}
      class="select"
    >
      {#each localeOptions as option (option)}
        <option value={option}>{i18n.t(`application.locale.${option}`)}</option>
      {/each}
    </select>
    <div class="help">{i18n.t('application.languageHelp')}</div>
  </div>

  <div>
    <div class="section-h">{i18n.t('application.runtimeSessions')}</div>
    <div class="row">
      <button type="button" class="btn-secondary flex items-center gap-1.5"
              onclick={refreshSessions} disabled={sessionsBusy}>
        <RefreshCw size={12} /> {i18n.t('application.refreshSessions')}
      </button>
      {#if sessionsStatus}<span class="help">{sessionsStatus}</span>{/if}
    </div>
    {#if sessions.length > 0}
      <div class="border border-[var(--color-border)] rounded divide-y divide-[var(--color-border-soft)] mt-2">
        {#each sessions as s (s.id)}
          <div class="flex items-center gap-2 px-2 py-1.5 text-[12px]">
            <div class="min-w-0 flex-1">
              <div class="truncate text-[var(--color-fg)]">{s.title}</div>
              <div class="truncate text-[10.5px] text-[var(--color-fg-muted)] font-mono">
                {s.kind} · {s.id}
              </div>
            </div>
            <button type="button" class="btn-secondary !px-2 !py-1"
                    onclick={() => closeSession(s.id)} disabled={sessionsBusy}
                  title={i18n.t('application.closeBackendSession')}
                  aria-label={i18n.t('application.closeBackendSession')}>
              <Trash2 size={12} />
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <div>
    <div class="section-h">{i18n.t('application.updates')}</div>
    <div class="row">
      <button type="button" class="btn-secondary" onclick={checkForUpdates} disabled={updateBusy}>
        {i18n.t('application.checkForUpdates')}
      </button>
      {#if updateAvailable}
        <button
          type="button"
          class="btn-primary flex items-center gap-1.5"
          onclick={installUpdate}
          disabled={updateBusy}
        >
          <Download size={12} /> {i18n.t('application.installVersion', { version: updateAvailable.version })}
        </button>
      {/if}
    </div>
    {#if updateStatus}
      <div class="help">{updateStatus}</div>
    {/if}
    {#if updateAvailable?.notes}
      <pre class="mt-2 text-[11px] whitespace-pre-wrap bg-[var(--color-bg)]
                  border border-[var(--color-border)] rounded p-2 max-h-32 overflow-auto">{updateAvailable.notes}</pre>
    {/if}
  </div>

  <div>
    <div class="section-h">{i18n.t('application.diagnostics')}</div>
    <div class="help">{i18n.t('application.diagnosticsHelp')}</div>
    <div class="row">
      <button type="button" class="btn-secondary flex items-center gap-1.5"
              onclick={exportDiagnostics} disabled={diagnosticsBusy}>
        <FileDown size={12} /> {i18n.t('application.exportDiagnostics')}
      </button>
      <button type="button" class="btn-secondary" onclick={clearDiagnostics} disabled={diagnosticsBusy || diagnostics.events.length === 0}>
        {i18n.t('application.clearDiagnostics')}
      </button>
      <span class="help">{i18n.t('application.diagnosticsCount', { count: diagnostics.events.length })}</span>
    </div>
    {#if diagnostics.events.length > 0}
      <div class="border border-[var(--color-border)] rounded divide-y divide-[var(--color-border-soft)] mt-2 max-h-40 overflow-auto">
        {#each diagnostics.events.slice(-8).reverse() as event (event.id)}
          <div class="px-2 py-1.5 text-[11px]">
            <div class="flex items-center gap-2">
              <span class="uppercase tracking-[0.08em] text-[var(--color-fg-muted)]">{event.category}</span>
              <span class="text-[var(--color-fg-muted)]">{new Date(event.ts).toLocaleString()}</span>
            </div>
            <div class="truncate text-[var(--color-fg)]">{event.source}: {event.message}</div>
          </div>
        {/each}
      </div>
    {/if}
    {#if diagnosticsStatus}
      <div class="help">{diagnosticsStatus}</div>
    {/if}
  </div>
</div>
