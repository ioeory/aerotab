export const TROUBLESHOOT_TEMPLATE_KEY = 'aerotab.aiTroubleshoot.templates.v1';
export const troubleshootBuiltinIds = ['ssh-failure', 'port-in-use', 'disk-full'] as const;
export interface TroubleshootTemplate {
  id: string;
  name: string; // Stored title; keep the v1 field name for existing templates.
  description?: string;
  steps?: string;
  prompt: string;
}
export const templateNameLimit = 100;
export const templateDescriptionLimit = 500;
export const templateStepsLimit = 2_000;
export const templatePromptLimit = 2_000;
const templateCountLimit = 50;

function validated(entries: unknown): TroubleshootTemplate[] | null {
  if (!Array.isArray(entries) || entries.length > templateCountLimit) return null;
  const ids = new Set<string>(troubleshootBuiltinIds);
  const output: TroubleshootTemplate[] = [];
  for (const entry of entries) {
    if (!entry || typeof entry !== 'object') return null;
    const { id, name, description, steps, prompt } = entry;
    if (typeof id !== 'string' || !/^[a-zA-Z0-9-]{1,80}$/.test(id) || ids.has(id)
      || typeof name !== 'string' || !name.trim() || name.length > templateNameLimit
      || (description !== undefined && (typeof description !== 'string' || description.length > templateDescriptionLimit))
      || (steps !== undefined && (typeof steps !== 'string' || steps.length > templateStepsLimit))
      || typeof prompt !== 'string' || !prompt.trim() || prompt.length > templatePromptLimit) return null;
    ids.add(id);
    // Persist only the explicit template fields, never live troubleshooting context.
    const template: TroubleshootTemplate = { id, name: name.trim(), prompt: prompt.trim() };
    if (description?.trim()) template.description = description.trim();
    if (steps?.trim()) template.steps = steps.trim();
    output.push(template);
  }
  return output;
}

export function loadTroubleshootTemplates(storage: Pick<Storage, 'getItem'>): TroubleshootTemplate[] {
  try {
    const value = JSON.parse(storage.getItem(TROUBLESHOOT_TEMPLATE_KEY) ?? 'null');
    return value?.version === 1 ? validated(value.templates) ?? [] : [];
  } catch { return []; }
}

export function saveTroubleshootTemplates(storage: Pick<Storage, 'getItem' | 'setItem'>, entries: TroubleshootTemplate[]): boolean {
  const templates = validated(entries);
  if (!templates) return false;
  try {
    const raw = storage.getItem(TROUBLESHOOT_TEMPLATE_KEY);
    if (raw !== null && JSON.parse(raw)?.version !== 1) return false;
    storage.setItem(TROUBLESHOOT_TEMPLATE_KEY, JSON.stringify({ version: 1, templates }));
    return true;
  } catch { return false; }
}
