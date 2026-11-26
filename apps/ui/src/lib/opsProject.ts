import { incidentEvidenceLimit, prepareIncident, type IncidentRecord, type IncidentStep } from './operationsWorkbench';

export interface EventScope { project: string; environment: string }
export type SceneNode = { profileId: string } | { direction: 'row' | 'col'; ratios: number[]; children: SceneNode[] };
export interface EventScene { title: string; layout: SceneNode }
export type EventAction = 'created' | 'updated' | 'archived' | 'reopened' | 'imported';
export interface OpsEvent extends EventScope {
  id: string; archived: boolean; updatedAt: number; record: IncidentRecord;
  timeline: { kind: EventAction; at: number }[];
  scene: EventScene | null;
}
const clean = (value: string) => value.trim().slice(0, 100);
const allEvidence = Array.from({ length: incidentEvidenceLimit }, (_, index) => index);

export function saveEvent(previous: OpsEvent | null, id: string, scope: EventScope, record: unknown, included: readonly number[], now = Date.now(), scene = previous?.scene ?? null): OpsEvent {
  return { id, project: clean(scope.project), environment: clean(scope.environment), archived: previous?.archived ?? false,
    record: prepareIncident(record, included), updatedAt: now, scene: scene ? copyScene(scene) : null,
    timeline: [...(previous?.timeline ?? []), { kind: previous ? 'updated' as const : 'created' as const, at: now }].slice(-200) };
}
function copyScene(scene: EventScene): EventScene {
  const copy = (node: SceneNode): SceneNode => 'profileId' in node ? { profileId: node.profileId }
    : { direction: node.direction, ratios: [...node.ratios], children: node.children.map(copy) };
  return { title: clean(scene.title), layout: copy(scene.layout) };
}
export function archiveEvent(event: OpsEvent, archived: boolean, at = Date.now()): OpsEvent {
  if (event.archived === archived) return event;
  return { ...event, archived, updatedAt: at, timeline: [...event.timeline, { kind: archived ? 'archived' as const : 'reopened' as const, at }].slice(-200) };
}
export function searchEvents(events: readonly OpsEvent[], query: string, archived: boolean): OpsEvent[] {
  const terms = query.trim().toLowerCase().split(/\s+/);
  return events.filter(event => event.archived === archived && terms.every(term =>
    `${event.project} ${event.environment} ${event.record.title} ${event.record.conclusion}`.toLowerCase().includes(term)))
    .sort((a, b) => b.updatedAt - a.updatedAt);
}
export async function migrateLegacy(legacy: IncidentRecord | null, insert: (event: OpsEvent) => Promise<void>, at = Date.now()) {
  if (!legacy) return;
  const event = saveEvent(null, 'legacy-v1', { project: '', environment: '' }, legacy, allEvidence, at);
  event.timeline = [{ kind: 'imported', at }];
  await insert(event);
}
export function instantiateRunbook(id: string, name: string, labels: readonly string[]): IncidentStep[] {
  const steps = labels.map(label => label.trim().slice(0, 240)).filter(Boolean);
  let hash = 2166136261;
  for (const character of JSON.stringify([name, steps])) hash = Math.imul(hash ^ (character.codePointAt(0) ?? 0), 16777619);
  const version = (hash >>> 0).toString(16).padStart(8, '0');
  return steps.map(label => ({ label, expected: '', actual: '', done: false, kind: 'check', template: { id, name, version } }));
}
export function handoffMarkdown(event: OpsEvent, includeEvidence: readonly number[], locale: 'en' | 'zh-CN'): string {
  const cn = locale === 'zh-CN';
  const text = (value: string) => value.replace(/[\r\n]+/g, ' ').replace(/[\\`*_{}\[\]()#+.!<>|]/g, '\\$&');
  const block = (value: string) => value.replace(/\r\n?/g, '\n').split('\n').map(line => `    ${line}`).join('\n');
  const chosen = prepareIncident(event.record, includeEvidence);
  const action = { created: cn ? '创建' : 'Created', updated: cn ? '更新' : 'Updated', archived: cn ? '归档' : 'Archived', reopened: cn ? '重新打开' : 'Reopened', imported: cn ? '迁移' : 'Imported' };
  const time = (at: number) => new Date(at).toISOString();
  return [
    `# ${text(chosen.title || (cn ? '未命名事件' : 'Untitled incident'))}`,
    `${cn ? '项目 / 环境' : 'Project / Environment'}: ${text(event.project)} / ${text(event.environment)}`,
    ...(chosen.conclusion ? [`## ${cn ? '结论' : 'Conclusion'}`, block(chosen.conclusion)] : []),
    `## ${cn ? '时间线' : 'Timeline'}`, event.timeline.map(item => `- ${time(item.at)} · ${action[item.kind]}`).join('\n'),
    `## ${cn ? '手册执行记录' : 'Runbook execution'}`, ...chosen.steps.flatMap((step, index) => [
      `${index + 1}. ${step.skipReason ? (cn ? '已跳过' : 'Skipped') : step.done ? (cn ? '已验证' : 'Verified') : (cn ? '未验证' : 'Pending')} · ${text(step.label)}${step.template ? ` (${text(step.template.name)} @ ${text(step.template.version)})` : ''}`,
      ...[step.expected && `${cn ? '预期' : 'Expected'}: ${step.expected}`, step.actual && `${cn ? '实际' : 'Actual'}: ${step.actual}`, step.skipReason && `${cn ? '原因' : 'Reason'}: ${step.skipReason}`].filter((value): value is string => !!value).map(block),
    ]),
    ...(chosen.evidence.length ? [`## ${cn ? '证据' : 'Evidence'}`, ...chosen.evidence.flatMap(entry => [`### ${text(entry.label || (cn ? '摘录' : 'Excerpt'))}${entry.at === undefined ? '' : ` · ${time(entry.at)}`}`, block(entry.text)])] : []),
  ].join('\n\n') + '\n';
}
