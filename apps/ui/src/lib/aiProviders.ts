export type ProviderProtocol = 'openai' | 'custom' | 'anthropic' | 'ollama';
export interface AiProvider { id: string; name: string; provider: ProviderProtocol; apiBase: string; model: string; enabled: boolean; hasKey: boolean }
export interface AiProvidersResult { providers: AiProvider[]; defaultProviderId: string | null }
export interface ProviderDraft extends Omit<AiProvider, 'hasKey'> { key: string; makeDefault: boolean }
export const providerPresets: { id: string; name: string; provider: ProviderProtocol; apiBase: string; model: string }[] = [
  { id: 'openai', name: 'OpenAI', provider: 'openai', apiBase: 'https://api.openai.com/v1', model: 'gpt-4o-mini' },
  { id: 'anthropic', name: 'Anthropic', provider: 'anthropic', apiBase: 'https://api.anthropic.com/v1', model: 'claude-sonnet-4-5' },
  { id: 'ollama', name: 'Ollama', provider: 'ollama', apiBase: 'http://localhost:11434/v1', model: 'llama3.1' },
  { id: 'deepseek', name: 'DeepSeek', provider: 'custom', apiBase: 'https://api.deepseek.com/v1', model: 'deepseek-chat' },
  { id: 'custom', name: 'Custom', provider: 'custom', apiBase: 'https://api.example.com/v1', model: '' },
];
export function newProviderDraft(presetId = 'openai'): ProviderDraft {
  const preset = providerPresets.find(p => p.id === presetId) ?? providerPresets[0]!;
  return { id: crypto.randomUUID(), name: preset.name, provider: preset.provider, apiBase: preset.apiBase, model: preset.model, enabled: true, key: '', makeDefault: false };
}
export function providerDraft(p: AiProvider): ProviderDraft {
  return { id: p.id, name: p.name, provider: p.provider, apiBase: p.apiBase, model: p.model, enabled: p.enabled, key: '', makeDefault: false };
}
export function providerSaveParams(d: ProviderDraft) {
  return { provider: { id: d.id, name: d.name.trim(), provider: d.provider, apiBase: d.apiBase.trim(), model: d.model.trim(), enabled: d.enabled }, ...(d.key ? { key: d.key } : {}), ...(d.makeDefault ? { makeDefault: true } : {}) };
}
export function providerDraftChanged(d: ProviderDraft, saved: AiProvider | null): boolean {
  return !saved || !!d.key || d.makeDefault || (['id', 'name', 'provider', 'apiBase', 'model', 'enabled'] as const).some(k => d[k] !== saved[k]);
}
export function validateProviderDraft(d: ProviderDraft): 'required' | 'invalidBase' | null {
  if (!d.name.trim() || !d.model.trim()) return 'required';
  try {
    const url = new URL(d.apiBase.trim());
    const local = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
    if (url.username || url.password || url.search || url.hash || (url.protocol !== 'https:' && !(url.protocol === 'http:' && local))) return 'invalidBase';
  } catch { return 'invalidBase'; }
  return null;
}
