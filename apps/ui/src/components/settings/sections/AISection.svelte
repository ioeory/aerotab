<script lang="ts">
  // AI assistant — M10. Pure-UI settings form; persisted under settings
  // key `ai`. No backend IPC needed at this milestone — the actual chat
  // surface will read these values directly from the settings store when
  // it issues HTTP requests to the chosen provider.
  import { onMount, onDestroy } from 'svelte';
  import { Sparkles } from '@lucide/svelte';
  import type { RpcClient } from '../../../lib/rpc';
  import { settingsCoord } from '../../../lib/settingsStore.svelte';

  interface Props { rpc: RpcClient; onError: (msg: string) => void }
  let { rpc, onError }: Props = $props();

  type Provider = 'openai' | 'anthropic' | 'ollama' | 'custom';

  let enabled = $state(false);
  let provider = $state<Provider>('openai');
  let apiKey = $state('');
  let apiBase = $state('');
  let model = $state('gpt-4o-mini');
  let temperature = $state(0.7);
  let maxTokens = $state(1024);
  let systemPrompt = $state(
    'You are a helpful terminal assistant. Help the user diagnose shell errors and suggest commands.'
  );
  let contextSource = $state(true);
  let contextLines = $state(200);

  function markDirty() { settingsCoord.markDirty(); }

  function defaultBaseFor(p: Provider): string {
    if (p === 'openai') return 'https://api.openai.com/v1';
    if (p === 'anthropic') return 'https://api.anthropic.com/v1';
    if (p === 'ollama') return 'http://localhost:11434/v1';
    return '';
  }

  function defaultModelFor(p: Provider): string {
    if (p === 'openai') return 'gpt-4o-mini';
    if (p === 'anthropic') return 'claude-3-5-haiku-latest';
    if (p === 'ollama') return 'llama3.1';
    return '';
  }

  function onProviderChange() {
    if (!apiBase) apiBase = defaultBaseFor(provider);
    if (!model) model = defaultModelFor(provider);
    markDirty();
  }

  async function load() {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'ai' });
      if (r.value && typeof r.value === 'object') {
        const v = r.value as Record<string, unknown>;
        if (typeof v.enabled === 'boolean') enabled = v.enabled;
        if (v.provider === 'openai' || v.provider === 'anthropic' || v.provider === 'ollama' || v.provider === 'custom') provider = v.provider;
        if (typeof v.apiKey === 'string') apiKey = v.apiKey;
        if (typeof v.apiBase === 'string') apiBase = v.apiBase;
        if (typeof v.model === 'string') model = v.model;
        if (typeof v.temperature === 'number') temperature = v.temperature;
        if (typeof v.maxTokens === 'number') maxTokens = v.maxTokens;
        if (typeof v.systemPrompt === 'string') systemPrompt = v.systemPrompt;
        if (typeof v.contextSource === 'boolean') contextSource = v.contextSource;
        if (typeof v.contextLines === 'number') contextLines = v.contextLines;
      }
    } catch (e) { onError(`ai load: ${(e as Error).message}`); }
  }

  async function save() {
    await rpc.call('settings.set', {
      key: 'ai',
      value: {
        enabled, provider, apiKey, apiBase, model,
        temperature, maxTokens, systemPrompt,
        contextSource, contextLines,
      },
    });
  }

  onMount(() => { settingsCoord.registerSaver('ai', save); void load(); });
  onDestroy(() => settingsCoord.unregisterSaver('ai'));
</script>

<div class="settings-section">
  <h2 class="flex items-center gap-2"><Sparkles size={16} /> AI 助手</h2>

  <div class="section-h">General</div>
  <label class="row">
    <span class="row-label">Enable AI assistant</span>
    <input type="checkbox" bind:checked={enabled} onchange={markDirty} />
  </label>

  <div class="section-h">Provider</div>
  <label class="row">
    <span class="row-label">Provider</span>
    <select bind:value={provider} onchange={onProviderChange} disabled={!enabled}>
      <option value="openai">OpenAI</option>
      <option value="anthropic">Anthropic (Claude)</option>
      <option value="ollama">Ollama (local)</option>
      <option value="custom">Custom (OpenAI-compatible)</option>
    </select>
  </label>
  <label class="row">
    <span class="row-label">API base URL</span>
    <input type="text" bind:value={apiBase} oninput={markDirty} disabled={!enabled}
           placeholder={defaultBaseFor(provider)} />
  </label>
  <label class="row">
    <span class="row-label">API key</span>
    <input type="password" bind:value={apiKey} oninput={markDirty} disabled={!enabled}
           placeholder={provider === 'ollama' ? '(not required)' : 'sk-...'} />
  </label>
  <label class="row">
    <span class="row-label">Model</span>
    <input type="text" bind:value={model} oninput={markDirty} disabled={!enabled} />
  </label>

  <div class="section-h">Generation</div>
  <label class="row">
    <span class="row-label">Temperature (0–2)</span>
    <input type="number" min="0" max="2" step="0.1"
           bind:value={temperature} oninput={markDirty} disabled={!enabled} />
  </label>
  <label class="row">
    <span class="row-label">Max tokens</span>
    <input type="number" min="64" max="32000" step="64"
           bind:value={maxTokens} oninput={markDirty} disabled={!enabled} />
  </label>
  <label class="row align-top">
    <span class="row-label">System prompt</span>
    <textarea rows="4" bind:value={systemPrompt} oninput={markDirty} disabled={!enabled}></textarea>
  </label>

  <div class="section-h">Context</div>
  <label class="row">
    <span class="row-label">Include recent terminal output</span>
    <input type="checkbox" bind:checked={contextSource} onchange={markDirty} disabled={!enabled} />
  </label>
  <label class="row">
    <span class="row-label">Context lines</span>
    <input type="number" min="10" max="2000" step="10"
           bind:value={contextLines} oninput={markDirty} disabled={!enabled || !contextSource} />
  </label>

  <p class="hint">
    The API key is stored in plaintext inside the settings store. For high-value
    credentials, use the Vault section instead and reference them from your
    integration scripts.
  </p>
</div>

<style>
  .section-h {
    margin-top: 16px;
    margin-bottom: 6px;
    font-size: 11.5px;
    text-transform: uppercase;
    color: var(--color-fg-muted);
    letter-spacing: 0.04em;
  }
  .hint { color: var(--color-fg-muted); font-size: 12px; margin: 12px 0 0; max-width: 600px; }
  .row {
    display: grid;
    grid-template-columns: 220px 1fr;
    align-items: center;
    gap: 10px;
    padding: 4px 0;
  }
  .row.align-top { align-items: flex-start; }
  .row-label { font-size: 12.5px; }
  .row input[type='text'],
  .row input[type='password'],
  .row input[type='number'],
  .row select,
  .row textarea {
    padding: 4px 8px;
    background: var(--color-bg-soft);
    color: var(--color-fg);
    border: 1px solid var(--color-border);
    border-radius: 4px;
    font-size: 12.5px;
    width: 100%;
    max-width: 380px;
    font-family: inherit;
  }
  .row textarea { resize: vertical; }
  .row input:focus, .row select:focus, .row textarea:focus {
    outline: none; border-color: var(--color-accent);
  }
  .row input:disabled, .row select:disabled, .row textarea:disabled { opacity: 0.5; }
</style>
