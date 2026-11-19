<script lang="ts">
  import { onMount, tick } from 'svelte';
  import type { TroubleshootMessageKey } from '../../lib/aiTroubleshootMessages';
  import {
    loadTroubleshootTemplates, saveTroubleshootTemplates, templateNameLimit, templateDescriptionLimit,
    templateStepsLimit, templatePromptLimit, troubleshootBuiltinIds, type TroubleshootTemplate,
  } from '../../lib/aiTroubleshootTemplates';
  import { uuidv4 } from '../../lib/rpc';

  let { t, disabled, onApply }: {
    t: (key: TroubleshootMessageKey) => string;
    disabled: boolean;
    onApply: (prompt: string) => void;
  } = $props();

  let entries = $state<TroubleshootTemplate[]>([]);
  let expanded = $state(false);
  let editing = $state<string | null>(null);
  let titleField = $state<HTMLInputElement>();
  let name = $state('');
  let description = $state('');
  let steps = $state('');
  let prompt = $state('');
  let error = $state(false);
  let storage: Storage | undefined;

  onMount(() => {
    try {
      storage = window.localStorage;
      entries = loadTroubleshootTemplates(storage);
    } catch { storage = undefined; }
  });

  function openEditor(id: string, fields?: Omit<TroubleshootTemplate, 'id'>) {
    expanded = true;
    editing = id;
    name = fields?.name ?? '';
    description = fields?.description ?? '';
    steps = fields?.steps ?? '';
    prompt = fields?.prompt ?? '';
    error = false;
    void tick().then(() => titleField?.focus());
  }

  export function openBlankEditor() {
    openEditor('');
  }

  function edit(entry: TroubleshootTemplate) {
    openEditor(entry.id, entry);
  }

  function duplicate(entry: Omit<TroubleshootTemplate, 'id'>) {
    openEditor('', entry);
  }

  function persist(next: TroubleshootTemplate[]) {
    error = !storage || !saveTroubleshootTemplates(storage, next);
    if (!error && storage) entries = loadTroubleshootTemplates(storage);
    return !error;
  }

  function save() {
    if (!name.trim() || !prompt.trim() || editing === null) return;
    const entry = { id: editing || uuidv4(), name, description, steps, prompt };
    const next = editing ? entries.map(old => old.id === editing ? entry : old) : [...entries, entry];
    if (persist(next)) editing = null;
  }

  function remove(entry: TroubleshootTemplate) {
    if (persist(entries.filter(old => old.id !== entry.id)) && editing === entry.id) editing = null;
  }
</script>

<details bind:open={expanded}>
  <summary>{t('templates')}</summary>
  <p>{t('templateHelp')}</p>
  <h4>{t('templatesBuiltin')}</h4>
  <ul>
    {#each troubleshootBuiltinIds as id (id)}
      {@const entry = { name: t(id), prompt: t(`${id}Prompt`) }}
      <li>
        <span class="template-title">{entry.name}</span>
        <button type="button" {disabled} aria-label={`${t('templateApply')}: ${entry.name}`} onclick={() => onApply(entry.prompt)}>{t('templateApply')}</button>
        <button type="button" aria-label={`${t('templateDuplicate')}: ${entry.name}`} onclick={() => duplicate(entry)}>{t('templateDuplicate')}</button>
      </li>
    {/each}
  </ul>
  <h4>{t('templatesCustom')}</h4>
  {#if entries.length}
    <ul>
      {#each entries as entry (entry.id)}
        <li>
          <span class="template-title">{entry.name}</span>
          <button type="button" {disabled} aria-label={`${t('templateApply')}: ${entry.name}`} onclick={() => onApply(entry.prompt)}>{t('templateApply')}</button>
          <button type="button" aria-label={`${t('templateEdit')}: ${entry.name}`} onclick={() => edit(entry)}>{t('templateEdit')}</button>
          <button type="button" aria-label={`${t('templateDuplicate')}: ${entry.name}`} onclick={() => duplicate(entry)}>{t('templateDuplicate')}</button>
          <button type="button" aria-label={`${t('templateDelete')}: ${entry.name}`} onclick={() => remove(entry)}>{t('templateDelete')}</button>
          {#if entry.description || entry.steps}
            <div class="template-notes">
              {#if entry.description}<p>{entry.description}</p>{/if}
              {#if entry.steps}<p>{entry.steps}</p>{/if}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if editing === null}
    <button type="button" onclick={openBlankEditor}>{t('templateAdd')}</button>
  {:else}
    <form onsubmit={(event) => { event.preventDefault(); save(); }}>
      <label>{t('templateName')}<input bind:this={titleField} bind:value={name} maxlength={templateNameLimit} required /></label>
      <label>{t('templateDescription')}<textarea bind:value={description} maxlength={templateDescriptionLimit} rows="2"></textarea></label>
      <label>{t('templateSteps')}<textarea bind:value={steps} maxlength={templateStepsLimit} rows="3"></textarea></label>
      <label>{t('templatePrompt')}<textarea bind:value={prompt} maxlength={templatePromptLimit} rows="3" required></textarea></label>
      <div class="actions">
        <button type="submit" disabled={!name.trim() || !prompt.trim()}>{t('templateSave')}</button>
        <button type="button" onclick={() => { editing = null; error = false; }}>{t('templateCancel')}</button>
      </div>
    </form>
  {/if}
  {#if error}<p role="alert">{t('templateError')}</p>{/if}
</details>

<style>
  details{border:1px solid var(--color-border);border-radius:6px;padding:9px;min-width:0}
  summary{cursor:pointer;font-weight:600}
  p{color:var(--color-fg-muted);font-size:12px}
  h4,ul{margin:8px 0}
  ul{padding:0;list-style:none;display:grid;gap:5px}
  li,.actions{display:flex;align-items:center;gap:6px;flex-wrap:wrap}
  .template-title{flex:1;overflow-wrap:anywhere;min-width:5ch}
  .template-notes{flex-basis:100%;min-width:0}
  .template-notes p{margin:3px 0;white-space:pre-wrap;overflow-wrap:anywhere}
  form,label{display:grid;gap:4px}
  form{gap:8px}
  input,textarea,button{color:var(--color-fg);background:var(--color-bg-soft);border:1px solid var(--color-border);border-radius:5px;padding:5px 8px;font:inherit}
  textarea{resize:vertical}
  button:not(:disabled){cursor:pointer}
  button:disabled{opacity:.55}
  :is(summary,button,input,textarea):focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}
</style>
