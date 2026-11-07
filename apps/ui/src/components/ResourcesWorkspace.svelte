<script lang="ts">
  import { onMount } from 'svelte';
  import type { RpcClient } from '../lib/rpc';
  import type { KnownHostEntry, StoredProfile } from '../lib/types';
  import VaultSection from './settings/sections/VaultSection.svelte';
  import { diagnostics, exportDiagnosticPack, type DiagnosticCategory, type DiagnosticLevel } from '../lib/diagnostics.svelte';
  import { hostKeyFingerprint, notifyResource, subscribeResource } from '../lib/resourceEvents';
  import { resourceText as t } from '../lib/resourceMessages';
  import { appConfirm } from '../lib/confirm.svelte';
  interface Props { rpc: RpcClient; profiles: StoredProfile[]; onError: (message: string) => void; section?: 'vault' | 'known-hosts' | 'logs'; buildId?: string; coreVersion?: string | null }
  let { rpc, profiles: _profiles, onError, section = 'vault', buildId = 'resources', coreVersion = null }: Props = $props();
  let hosts = $state<(KnownHostEntry & { fingerprint: string })[]>([]);
  let busy = $state(false), loaded = $state(false), loadError = $state('');
  let editor = $state(false), host = $state(''), publicKey = $state(''), confirmed = $state(false);
  let expected = $state<KnownHostEntry | null>(null), proposedFingerprint = $state('');
  let category = $state<DiagnosticCategory | ''>(''), level = $state<DiagnosticLevel | ''>('');
  const categories: DiagnosticCategory[] = ['connection', 'sync', 'transfer', 'profile', 'workspace', 'settings', 'update', 'app'];
  const levels: DiagnosticLevel[] = ['info', 'warn', 'error'];
  const events = $derived(diagnostics.events.filter(event => (!category || event.category === category) && (!level || event.level === level)).slice().reverse());
  let generation = 0;
  async function refresh() {
    const request = ++generation; busy = true; loadError = '';
    try {
      const list = await rpc.call<KnownHostEntry[]>('ssh.knownHosts.list', {});
      const rows = await Promise.all(list.map(async entry => ({ ...entry, fingerprint: await hostKeyFingerprint(entry.key_b64).catch(() => '—') })));
      if (request === generation) { hosts = rows; loaded = true; }
    } catch (error) { if (request === generation) { loaded = false; loadError = String(error); onError(loadError); } }
    finally { if (request === generation) busy = false; }
  }
  function begin(entry?: KnownHostEntry) {
    ++fingerprintGeneration;
    expected = entry ? { ...entry } : null; host = entry?.host ?? ''; publicKey = ''; confirmed = false; proposedFingerprint = ''; editor = true;
  }
  let fingerprintGeneration = 0;
  async function preview(value: string) {
    publicKey = value; confirmed = false; proposedFingerprint = ''; const request = ++fingerprintGeneration;
    const parts = value.trim().split(/\s+/);
    const fingerprint = await hostKeyFingerprint(parts[1] ?? '').catch(() => '');
    if (request === fingerprintGeneration) proposedFingerprint = fingerprint;
  }
  async function save() {
    if (!confirmed || busy || !loaded) return;
    const parts = publicKey.trim().split(/\s+/);
    if (!host.trim() || parts.length < 2 || !proposedFingerprint) { onError(t('invalidKey')); return; }
    busy = true;
    try {
      await rpc.call('ssh.knownHosts.put', { host: host.trim(), key_type: parts[0], key_b64: parts[1], expected: expected ? { key_type: expected.key_type, key_b64: expected.key_b64 } : null });
      editor = false; publicKey = ''; notifyResource('knownHosts');
    } catch (error) { onError(String(error)); } finally { busy = false; }
  }
  async function remove(entry: KnownHostEntry) {
    if (!await appConfirm(`${entry.host}\n${t('confirmDelete')}`, { danger: true, confirmLabel: t('remove') })) return;
    busy = true;
    try { await rpc.call('ssh.knownHosts.remove', { host: entry.host }); notifyResource('knownHosts'); }
    catch (error) { onError(String(error)); } finally { busy = false; }
  }
  async function clear() {
    if (await appConfirm(t('confirmClear'), { danger: true, confirmLabel: t('clear') })) diagnostics.clear();
  }
  async function exportLogs() { try { await exportDiagnosticPack(buildId, coreVersion); } catch (error) { onError(String(error)); } }
  onMount(() => { void refresh(); const stop = subscribeResource('knownHosts', () => { void refresh(); }); return () => { stop(); ++generation; ++fingerprintGeneration; }; });
</script>

<section class="resource-workspace" aria-label={section === 'vault' ? t('vault') : section === 'logs' ? t('logs') : t('knownHosts')}>
  {#if section === 'vault'}
    <VaultSection {rpc} {onError} />
  {:else if section === 'known-hosts'}
    <h2>{t('knownHosts')}</h2><p class="hint">{t('trustHint')}</p>
    <div class="toolbar"><button onclick={refresh} disabled={busy}>{t('refresh')}</button><button onclick={() => begin()} disabled={busy || !loaded}>{t('add')}</button></div>
    {#if loadError}<p role="alert">{loadError}</p>{/if}
    {#if editor}
      <form onsubmit={event => { event.preventDefault(); void save(); }}>
        <label>{t('host')}<input bind:value={host} oninput={() => { confirmed = false; }} disabled={!!expected || busy} required placeholder="example.com:22" /></label>
        <label>{t('key')}<textarea bind:value={publicKey} oninput={event => void preview(event.currentTarget.value)} rows="3" disabled={busy} required></textarea></label>
        <p class="fingerprint">{t('fingerprint')}: {proposedFingerprint || '—'}</p>
        <label class="confirm"><input type="checkbox" bind:checked={confirmed} disabled={!proposedFingerprint || busy} />{t('confirmTrust')}</label>
        <div class="toolbar"><button type="submit" disabled={!confirmed || !proposedFingerprint || busy}>{expected ? t('replace') : t('save')}</button><button type="button" onclick={() => { editor = false; publicKey = ''; }} disabled={busy}>{t('cancel')}</button></div>
      </form>
    {/if}
    {#if hosts.length === 0 && loaded}<p class="hint">{t('empty')}</p>{/if}
    <ul>{#each hosts as entry (entry.host)}<li><div class="host"><strong>{entry.host}</strong><code>{entry.key_type}</code><code class="fingerprint">{entry.fingerprint}</code></div><button onclick={() => begin(entry)} disabled={busy}>{t('replace')}</button><button onclick={() => remove(entry)} disabled={busy}>{t('remove')}</button></li>{/each}</ul>
  {:else}
    <h2>{t('logs')}</h2><p class="hint">{t('logsHint')}</p>
    <div class="toolbar"><label>{t('category')}<select bind:value={category}><option value="">{t('all')}</option>{#each categories as value}<option value={value}>{value}</option>{/each}</select></label><label>{t('level')}<select bind:value={level}><option value="">{t('all')}</option>{#each levels as value}<option value={value}>{value}</option>{/each}</select></label><button onclick={exportLogs}>{t('export')}</button><button onclick={clear} disabled={!diagnostics.events.length}>{t('clear')}</button></div>
    {#if !events.length}<p class="hint">{t('empty')}</p>{/if}
    <div class="logs">{#each events as event (event.id)}<details><summary><time datetime={event.ts}>{new Date(event.ts).toLocaleString()}</time><span>{event.category}</span><span class:failure={event.level === 'error'}>{event.level}</span><span class="message">{event.message}</span></summary><dl><dt>{t('details')}</dt><dd><code>{event.source}</code><pre>{event.message}</pre></dd></dl></details>{/each}</div>
  {/if}
</section>

<style>
  .resource-workspace { height: 100%; min-width: 0; overflow: auto; padding: 24px; color: var(--color-fg); }
  h2 { margin: 0 0 12px; font-size: 18px; } .hint { color: var(--color-fg-muted); font-size: 12px; max-width: 780px; }
  .toolbar { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; margin: 14px 0; }
  button, input, select, textarea { font: inherit; color: var(--color-fg); background: var(--color-bg-soft); border: 1px solid var(--color-border); border-radius: 5px; padding: 6px 9px; }
  button { cursor: pointer; } button:disabled { opacity: .45; cursor: not-allowed; } button:focus-visible { outline: 2px solid var(--color-accent); }
  form { border: 1px solid var(--color-border); padding: 16px; border-radius: 8px; max-width: 820px; }
  label { display: flex; gap: 8px; align-items: center; } form label:not(.confirm) { display: grid; gap: 6px; margin: 10px 0; } .confirm { font-size: 12px; }
  ul { list-style: none; margin: 0; padding: 0; } li { display: flex; align-items: center; gap: 8px; border-bottom: 1px solid var(--color-border); padding: 12px 0; }
  .host { flex: 1; display: grid; gap: 5px; min-width: 0; } .fingerprint { overflow-wrap: anywhere; } code { font-size: 12px; }
  details { border-bottom: 1px solid var(--color-border); padding: 10px 0; } summary { cursor: pointer; display: flex; gap: 14px; font-size: 12px; } .message { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; } .failure { color: var(--color-danger); } dd { margin: 8px 0; } pre { white-space: pre-wrap; overflow-wrap: anywhere; font-size: 12px; }
</style>
