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
    onAddIncidentEvidence?: (entry: import('../../lib/operationsWorkbench').IncidentEvidence) => boolean;
  }
</script>

<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte';
  import {
    buildTroubleshootReview, createTroubleshootRun, isTroubleshootCommand, sanitizeTroubleshootText,
    buildTroubleshootResume, loadTroubleshootResume, saveTroubleshootResume, removeTroubleshootResume,
    buildTroubleshootComparison, TROUBLESHOOT_EVIDENCE_MAX_BYTES, type TroubleshootEvidence,
    troubleshootShells, type TroubleshootPurpose, type TroubleshootOrigin,
    type TroubleshootResume, type TroubleshootRunView, type TroubleshootShell,
  } from '../../lib/aiTroubleshoot';
  import { aiTroubleshootMessages, type TroubleshootMessageKey } from '../../lib/aiTroubleshootMessages';
  import { i18n } from '../../lib/i18n.svelte';
  import { uuidv4 } from '../../lib/rpc';
  import AiTroubleshootTemplates from './AiTroubleshootTemplates.svelte';

  import { operationsMessages, type OperationsMessage } from '../../lib/operationsMessages';

  let { rpc, providerId, model, endpoint, enabled, seed, active = true, onAddIncidentEvidence }: AiTroubleshootProps = $props();
  const ops = (key: OperationsMessage) => operationsMessages[i18n.resolvedLocale][key];
  let incidentStatus = $state<'added' | 'evidenceLimit' | null>(null);
  function addToIncident(side: 'A' | 'B') {
    const entry = evidence[side];
    const text = sanitizeTroubleshootText(entry.text);
    if (!text || !onAddIncidentEvidence) return;
    incidentStatus = onAddIncidentEvidence({ label: `${side} · ${entry.origin?.label ?? t('manual')}`, text, source: entry.source ?? 'manual', at: Date.now() }) ? 'added' : 'evidenceLimit';
  }
  const t = (key: TroubleshootMessageKey) => aiTroubleshootMessages[i18n.resolvedLocale][key];
  let input = $state('');
  let origin = $state<TroubleshootOrigin>();
  let purpose = $state<TroubleshootPurpose>('terminal_explain');
  let shell = $state<TroubleshootShell | ''>('');
  const evidenceSides = ['A', 'B'] as const;
  let comparing = $state(false);
  let evidence = $state<Record<(typeof evidenceSides)[number], TroubleshootEvidence>>({ A: { text: '' }, B: { text: '' } });
  let view = $state<TroubleshootRunView>({ status: 'idle', text: '' });
  let showResult = $state(false);
  let command = $state('');
  let appliedTemplate = $state('');
  let derivedFromOutput = $state(false);
  let comparisonFollowUp = $state(false);
  let savedDraft = $state<TroubleshootResume | null>(null);
  let draftReview = $state<{ action: 'save' | 'restore'; draft: TroubleshootResume } | null>(null);
  let restoredCommandShell = $state<TroubleshootShell | ''>('');
  let resumeStatus = $state<'resumeSaved' | 'resumeRestored' | 'resumeDeleted' | 'resumeError' | ''>('');
  let storage: Storage | undefined;
  let storageReady = $state(false);
  const commandShell = $derived(restoredCommandShell || (showResult && view.draft?.kind === 'draft' ? view.draft.shell : ''));
  const canSave = $derived(!!(commandShell && command.trim())
    || (purpose === 'command_draft' && !derivedFromOutput && !!(input.trim() || appliedTemplate)));
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
  const review = $derived(comparing
    ? buildTroubleshootComparison(evidence.A, evidence.B, { providerId, model, endpoint }, i18n.resolvedLocale)
    : buildTroubleshootReview(purpose, input, { providerId, model, endpoint }, origin, shell || undefined));

  $effect(() => {
    if (!active || !enabled) run.stop();
  });

  $effect(() => {
    if (!active || !enabled || !seed || (seed.nonce === lastSeedNonce && seed.id === lastSeedId)) return;
    if (seed.evidence && busy) return;
    lastSeedNonce = seed.nonce;
    lastSeedId = seed.id;
    const incoming = seed;
    untrack(() => {
      run.stop();
      if (incoming.evidence) {
        selectComparison();
        resetResult();
        evidence[incoming.evidence.side] = {
          text: sanitizeTroubleshootText(incoming.text, TROUBLESHOOT_EVIDENCE_MAX_BYTES),
          source: incoming.evidence.source,
          origin: { id: incoming.id, label: incoming.label },
        };
      } else {
        resetResult();
        setPrompt('terminal_explain', sanitizeTroubleshootText(incoming.text), { id: incoming.id, label: incoming.label });
      }
    });
  });

  onMount(() => {
    try {
      storage = window.localStorage;
      savedDraft = loadTroubleshootResume(storage);
      storageReady = true;
    } catch { resumeStatus = 'resumeError'; }
  });

  $effect(() => {
    if (!active) draftReview = null;
  });

  onDestroy(() => { copyRevision++; run.dispose(); });

  function reviewSave() {
    if (!active || busy || !storageReady) return;
    const draft = buildTroubleshootResume({
      question: purpose === 'command_draft' && !derivedFromOutput ? input : '',
      template: appliedTemplate, shell, command, commandShell,
    });
    if (draft) { resumeStatus = ''; draftReview = { action: 'save', draft }; }
    else resumeStatus = 'resumeError';
  }

  function reviewRestore() {
    if (!active || busy || !savedDraft) return;
    resumeStatus = '';
    draftReview = { action: 'restore', draft: savedDraft };
  }

  function confirmDraft() {
    if (!active || busy || !draftReview) return;
    const { action, draft } = draftReview;
    if (action === 'save') {
      if (!saveTroubleshootResume(storage, draft)) { resumeStatus = 'resumeError'; return; }
      savedDraft = draft;
      resumeStatus = 'resumeSaved';
    } else {
      resetResult();
      setPrompt('command_draft', draft.question);
      appliedTemplate = draft.template;
      shell = draft.shell;
      command = draft.command;
      restoredCommandShell = draft.commandShell;
      resumeStatus = 'resumeRestored';
    }
    draftReview = null;
  }

  function deleteSavedDraft() {
    if (!active || busy) return;
    if (!removeTroubleshootResume(storage)) { resumeStatus = 'resumeError'; return; }
    savedDraft = null;
    draftReview = null;
    resumeStatus = 'resumeDeleted';
  }

  function resetResult() {
    showResult = false;
    restoredCommandShell = '';
    draftReview = null;
    resumeStatus = '';
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
    comparing = false;
    origin = source;
    derivedFromOutput = false;
    comparisonFollowUp = false;
    appliedTemplate = '';
    shell = '';
  }

  function selectComparison() {
    if (busy || comparing) return;
    resetResult();
    comparing = true;
  }

  function selectMode(mode: TroubleshootPurpose) {
    if (busy) return;
    if (comparing) {
      resetResult();
      comparing = false;
      if (purpose !== mode) setPrompt(mode, '');
      return;
    }
    if (purpose === mode) return;
    resetResult();
    setPrompt(mode, '');
  }

  function applyTemplate(prompt: string, name: string) {
    if (busy) return;
    resetResult();
    setPrompt('command_draft', prompt);
    appliedTemplate = name;
  }

  function send() {
    const confirmed = review;
    if (!confirmed || !active || !enabled || busy) return;
    resetResult();
    command = '';
    showResult = run.start(rpc, confirmed);
  }

  function useSelection() {
    if (!active || !outputField || view.status !== 'done' || view.review?.purpose !== 'terminal_explain') return;
    const text = sanitizeTroubleshootText(outputField.value.slice(outputField.selectionStart, outputField.selectionEnd));
    if (!text) { needSelection = true; return; }
    const fromComparison = comparing;
    resetResult();
    setPrompt('command_draft', text, view.review.origin);
    derivedFromOutput = true;
    comparisonFollowUp = fromComparison;
  }

  async function copy() {
    if (!active || !commandShell || !isTroubleshootCommand(command)) return;
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
    <button type="button" aria-pressed={!comparing && purpose === 'terminal_explain'} disabled={busy}
      onclick={() => selectMode('terminal_explain')}>{t('explain')}</button>
    <button type="button" aria-pressed={!comparing && purpose === 'command_draft'} disabled={busy}
      onclick={() => selectMode('command_draft')}>{t('commandDraft')}</button>
    <button type="button" aria-pressed={comparing} disabled={busy} onclick={selectComparison}>{t('compareTitle')}</button>
  </div>
  {#if comparing}
    <p class="muted">{t('compareHelp')}</p>
    {#if incidentStatus}<p role="status">{ops(incidentStatus)}</p>{/if}
    {#each evidenceSides as side (side)}
      <div class="box" data-evidence-side={side}>
        <label>
          {t('compareEvidence')} {side}
          <textarea rows="5" value={evidence[side].text} disabled={busy} spellcheck="false"
            oninput={event => { evidence[side] = { ...evidence[side], text: event.currentTarget.value }; }}></textarea>
        </label>
        <p class="muted">{evidence[side].origin?.label ?? t('manual')}{evidence[side].source ? ` · ${t(evidence[side].source === 'lastCommand' ? 'compareSourceLast' : 'compareSourceSelection')}` : ''}</p>
        <div class="actions">
          <button type="button" disabled={busy || (!evidence[side].text && !evidence[side].origin)}
            onclick={() => { evidence[side] = { text: '' }; }}>{t('compareRemove')} {side}</button>
          {#if onAddIncidentEvidence}
            <button type="button" disabled={!evidence[side].text.trim() || !active} onclick={() => addToIncident(side)}>{ops(side === 'A' ? 'addA' : 'addB')}</button>
          {/if}
        </div>
      </div>
    {/each}
  {:else}
  <AiTroubleshootTemplates bind:this={templateEditor} {t} disabled={busy || !active} onApply={applyTemplate} />
  <section class="box" aria-label={t('resumeTitle')}>
    <h4>{t('resumeTitle')}</h4>
    <p class="muted">{t('resumeHelp')}</p>
    {#if appliedTemplate}<p>{t('templates')}: {appliedTemplate}</p>{/if}
    <div class="actions">
      <button type="button" disabled={!active || busy || !storageReady || !canSave} onclick={reviewSave}>{t('resumeSave')}</button>
      {#if savedDraft}
        <button type="button" disabled={!active || busy} onclick={reviewRestore}>{t('resumeRestore')}</button>
        <button type="button" disabled={!active || busy} onclick={deleteSavedDraft}>{t('resumeDelete')}</button>
      {/if}
    </div>
    {#if draftReview}
      <div class="box">
        <h4>{t(draftReview.action === 'save' ? 'resumeSavePreview' : 'resumeRestorePreview')}</h4>
        <p class="muted">{t(draftReview.action === 'save' ? 'resumeSaveWarning' : 'resumeRestoreWarning')}</p>
        {#if draftReview.draft.template}<p>{t('templates')}: {draftReview.draft.template}</p>{/if}
        {#if draftReview.draft.shell}<p>{t('shell')}: {draftReview.draft.shell}</p>{/if}
        {#if draftReview.draft.question}
          <p>{t('draftInput')}</p><pre>{draftReview.draft.question}</pre>
        {/if}
        {#if draftReview.draft.command}
          <p>{t('resultCommand')} · {draftReview.draft.commandShell}</p><pre>{draftReview.draft.command}</pre>
        {/if}
        <div class="actions">
          <button type="button" class="primary" disabled={!active || busy} onclick={confirmDraft}>{t(draftReview.action === 'save' ? 'templateSave' : 'resumeConfirmRestore')}</button>
          <button type="button" onclick={() => draftReview = null}>{t('templateCancel')}</button>
        </div>
      </div>
    {/if}
    {#if resumeStatus}<p class="muted" role={resumeStatus === 'resumeError' ? 'alert' : 'status'}>{t(resumeStatus)}</p>{/if}
  </section>

  {#if comparisonFollowUp}<p class="muted" role="status">{t('compareFollowUpHelp')}</p>{/if}
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
          <dt>{t('origin')}</dt><dd>{comparing ? t('compareTitle') : displayed.origin?.label || displayed.origin?.id || t('manual')}</dd>
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

  {#if restoredCommandShell}
    <div class="box">
      <label>
        {t('resultCommand')} · {restoredCommandShell}
        <input type="text" bind:value={command} maxlength="4096" oninput={clearCopy} spellcheck="false" />
      </label>
      <p class="muted">{t('resumeCommandWarning')}</p>
      <div class="actions">
        <button type="button" disabled={!isTroubleshootCommand(command)} onclick={copy}>{t('copy')}</button>
        {#if copyStatus}<span class="muted" role="status">{t(copyStatus)}</span>{/if}
      </div>
    </div>
  {:else if showResult && view.review?.purpose === 'terminal_explain' && view.text}
    <section class="box" aria-label={t(comparing ? 'compareResult' : 'explanation')}>
      <label>
        {t(comparing ? 'compareResult' : 'explanation')}
        <textarea readonly rows="8" bind:this={outputField} value={view.text}></textarea>
      </label>
      {#if view.status === 'done'}
        <div class="actions">
          <button type="button" onclick={useSelection}>{t(comparing ? 'compareFollowUp' : 'summarySelection')}</button>
          {#if !comparing}
            <button type="button" onclick={() => templateEditor?.openBlankEditor()}>{t('templateFromAnalysis')}</button>
          {/if}
        </div>
        {#if needSelection}<p class="muted" role="status">{t('selectedHelp')}</p>{/if}
      {/if}
    </section>
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
