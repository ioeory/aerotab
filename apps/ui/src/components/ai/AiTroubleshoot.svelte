<script lang="ts" module>
  import type { RpcClient } from '../../lib/rpc';
  import type { TroubleshootSeed } from '../../lib/aiTroubleshoot';

  export interface AiTroubleshootProps {
    rpc: RpcClient;
    providerId: string;
    model: string;
    endpoint: string;
    enabled: boolean;
    seed?: TroubleshootSeed;
    active?: boolean;
  }
</script>

<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    buildTroubleshootReview, createTroubleshootRun, isTroubleshootCommand, sanitizeTroubleshootText,
    troubleshootShells, type TroubleshootPurpose, type TroubleshootOrigin,
    type TroubleshootRunView, type TroubleshootShell,
  } from '../../lib/aiTroubleshoot';
  import { aiTroubleshootMessages, type TroubleshootMessageKey } from '../../lib/aiTroubleshootMessages';
  import { i18n } from '../../lib/i18n.svelte';
  import { uuidv4 } from '../../lib/rpc';
  import AiTroubleshootTemplates from './AiTroubleshootTemplates.svelte';

  let { rpc, providerId, model, endpoint, enabled, seed, active = true }: AiTroubleshootProps = $props();
  const t = (key: TroubleshootMessageKey) => aiTroubleshootMessages[i18n.resolvedLocale][key];
  let input = $state('');
  let origin = $state<TroubleshootOrigin>();
  let purpose = $state<TroubleshootPurpose>('terminal_explain');
  let shell = $state<TroubleshootShell | ''>('');
  let view = $state<TroubleshootRunView>({ status: 'idle', text: '' });
  let showResult = $state(false);
  let command = $state('');
  let outputField = $state<HTMLTextAreaElement>();
  let templateEditor = $state<ReturnType<typeof AiTroubleshootTemplates>>();
  let needSelection = $state(false);
  let copyStatus = $state<'copied' | 'copyFailed' | ''>('');
  let copyRevision = 0;
  let lastSeedNonce: number | undefined;
  let lastSeedId: string | undefined;

  const run = createTroubleshootRun(next => {
    view = next;
    if (next.draft?.kind === 'draft') command = next.draft.command;
  }, () => uuidv4() as ReturnType<Crypto['randomUUID']>);

  const busy = $derived(view.status === 'sending' || view.status === 'running'
    || view.status === 'stopping' || view.status === 'stopFailed');
  const status = $derived(view.status === 'idle' || view.status === 'done' ? '' : t(view.status));
  const review = $derived(buildTroubleshootReview(purpose, input, { providerId, model, endpoint }, origin, shell || undefined));

  $effect(() => {
    if (!active || !enabled) run.stop();
  });

  $effect(() => {
    if (!active || !seed || (seed.nonce === lastSeedNonce && seed.id === lastSeedId)) return;
    lastSeedNonce = seed.nonce;
    lastSeedId = seed.id;
    run.stop();
    resetResult();
    setPrompt('terminal_explain', sanitizeTroubleshootText(seed.text), { id: seed.id, label: seed.label });
  });

  onDestroy(() => { copyRevision++; run.dispose(); });

  function resetResult() {
    showResult = false;
    clearCopy();
    needSelection = false;
  }

  function clearCopy() {
    copyRevision++;
    copyStatus = '';
  }

  function setPrompt(mode: TroubleshootPurpose, text: string, source?: TroubleshootOrigin) {
    input = text;
    purpose = mode;
    origin = source;
    shell = '';
  }

  function selectMode(mode: TroubleshootPurpose) {
    if (busy || purpose === mode) return;
    resetResult();
    setPrompt(mode, '');
  }

  function applyTemplate(prompt: string) {
    if (busy) return;
    resetResult();
    setPrompt('command_draft', prompt);
  }

  function send() {
    const confirmed = review;
    if (!confirmed || !active || !enabled || busy) return;
    resetResult();
    command = '';
    showResult = run.start(rpc, confirmed);
  }

  function useSelection() {
    if (!outputField || view.status !== 'done' || view.review?.purpose !== 'terminal_explain') return;
    const text = outputField.value.slice(outputField.selectionStart, outputField.selectionEnd).trim();
    if (!text) { needSelection = true; return; }
    resetResult();
    setPrompt('command_draft', sanitizeTroubleshootText(text), view.review.origin);
  }

  async function copy() {
    if (!active || !showResult || !isTroubleshootCommand(command)) return;
    const revision = ++copyRevision;
    try {
      await navigator.clipboard.writeText(command);
      if (revision === copyRevision && active) copyStatus = 'copied';
    } catch {
      if (revision === copyRevision && active) copyStatus = 'copyFailed';
    }
  }
</script>

<section class="troubleshoot" aria-label={t('title')} hidden={!active}>
  <h3>{t('title')}</h3>
  <p class="muted">{t('textOnly')}</p>
  <div class="actions">
    <button type="button" aria-pressed={purpose === 'terminal_explain'} disabled={busy}
      onclick={() => selectMode('terminal_explain')}>{t('explain')}</button>
    <button type="button" aria-pressed={purpose === 'command_draft'} disabled={busy}
      onclick={() => selectMode('command_draft')}>{t('commandDraft')}</button>
  </div>
  <AiTroubleshootTemplates bind:this={templateEditor} {t} disabled={busy || !active} onApply={applyTemplate} />

  <label>
    {t(purpose === 'terminal_explain' ? 'outputInput' : 'draftInput')}
    <textarea rows="5" bind:value={input} disabled={busy} spellcheck="false"></textarea>
  </label>
  {#if purpose === 'command_draft'}
    <label>
      {t('shell')}
      <select bind:value={shell} disabled={busy}>
        <option value="">{t('chooseShell')}</option>
        {#each troubleshootShells as value (value)}<option {value}>{value}</option>{/each}
      </select>
    </label>
  {/if}
  {#if !enabled}
    <p class="muted">{t('disabled')}</p>
  {:else if !model || !endpoint}
    <p class="muted">{t('missingTarget')}</p>
  {/if}
  {#if review || (busy && view.review)}
    {@const displayed = busy ? view.review : review}
    {#if displayed}
      <section class="box" aria-label={t('preflight')}>
        <h4>{t('preflight')}</h4>
        <p class="muted">{t('preflightHelp')}</p>
        <dl>
          <dt>{t('model')}</dt><dd>{displayed.model}</dd>
          <dt>{t('endpoint')}</dt><dd>{displayed.endpoint}</dd>
          <dt>{t('origin')}</dt><dd>{displayed.origin?.label || displayed.origin?.id || t('manual')}</dd>
        </dl>
        <pre>{displayed.message}</pre>
      </section>
    {/if}
  {/if}
  <div class="actions">
    {#if busy}
      <button type="button" disabled={view.status === 'stopping'} onclick={run.stop}>{t('stop')}</button>
    {:else}
      <button type="button" class="primary" disabled={!review || !enabled || !active} onclick={send}>{t('send')}</button>
    {/if}
    {#if status}<span class="muted" role="status">{status}</span>{/if}
  </div>

  {#if showResult && view.review?.purpose === 'terminal_explain' && view.text}
    <div class="box">
      <label>
        {t('explanation')}
        <textarea readonly rows="8" bind:this={outputField} value={view.text}></textarea>
      </label>
      {#if view.status === 'done'}
        <div class="actions">
          <button type="button" onclick={useSelection}>{t('summarySelection')}</button>
          <button type="button" onclick={() => templateEditor?.openBlankEditor()}>{t('templateFromAnalysis')}</button>
        </div>
        {#if needSelection}<p class="muted" role="status">{t('selectedHelp')}</p>{/if}
      {/if}
    </div>
  {:else if showResult && view.draft?.kind === 'clarify'}
    <div class="box">
      <h4>{t('clarify')}</h4>
      <p>{view.draft.question}</p>
    </div>
  {:else if showResult && view.draft?.kind === 'draft'}
    <div class="box">
      <label>
        {t('resultCommand')} · {view.draft.shell}
        <input type="text" bind:value={command} maxlength="4096" oninput={clearCopy} spellcheck="false" />
      </label>
      <dl>
        <dt>{t('expected')}</dt><dd>{view.draft.expected}</dd>
        <dt>{t('sideEffects')}</dt><dd>{view.draft.sideEffects}</dd>
      </dl>
      <p class="muted">{t('reviewCommand')}</p>
      <div class="actions">
        <button type="button" disabled={!isTroubleshootCommand(command)} onclick={copy}>{t('copy')}</button>
        {#if copyStatus}<span class="muted" role="status">{t(copyStatus)}</span>{/if}
      </div>
    </div>
  {/if}
</section>

<style>
  .troubleshoot{display:grid;align-content:start;gap:10px;min-width:0;min-height:0;overflow:auto;padding:12px;color:var(--color-fg);font-size:13px}
  .troubleshoot[hidden]{display:none}
  .actions{display:flex;align-items:center;gap:7px;flex-wrap:wrap}
  .box{border:1px solid var(--color-border);border-radius:6px;padding:9px;min-width:0;display:grid;gap:7px}
  .muted{color:var(--color-fg-muted);font-size:12px}
  label{display:grid;gap:4px;min-width:0}
  input,select,textarea,button{color:var(--color-fg);background:var(--color-bg-soft);border:1px solid var(--color-border);border-radius:5px;padding:6px 9px;font:inherit}
  textarea,input,pre{font:12px/1.5 var(--font-mono,monospace)}
  textarea{resize:vertical}
  button:not(:disabled){cursor:pointer}
  button:disabled{opacity:.55}
  button[aria-pressed='true'],.primary{color:var(--color-accent);border-color:var(--color-accent)}
  :is(button,input,select,textarea):focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}
  pre{margin:0;background:var(--color-bg-soft);padding:8px;max-height:180px;overflow:auto;white-space:pre-wrap;overflow-wrap:anywhere;user-select:text}
  h3,h4,p,dl,dd{margin:0;overflow-wrap:anywhere}
  dl{display:grid;grid-template-columns:auto minmax(0,1fr);gap:4px 8px}
  dt{color:var(--color-fg-muted)}
  dd{white-space:pre-wrap}
</style>
