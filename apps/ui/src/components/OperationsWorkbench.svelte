<script lang="ts">
  import { onMount } from 'svelte';
  import HostInspection from './HostInspection.svelte';
  import { profileChoice } from './HostChoiceList.svelte';
  import { getProfileCatalog } from '../lib/profileCatalog.svelte';
  import { archiveEvent, handoffMarkdown, instantiateRunbook, migrateLegacy, saveEvent, searchEvents, type EventScene, type OpsEvent } from '../lib/opsProject';
  import { listOpsEvents, openOpsProject, writeOpsEvent } from '../lib/opsProjectStorage';
  import { uuidv4, type RpcClient } from '../lib/rpc';
  import { exportMarkdownFile } from '../lib/notesFiles';
  import { i18n } from '../lib/i18n.svelte';
  import { getHostMonitoring } from '../lib/hostMonitoring.svelte';
  import { loadTroubleshootTemplates, type TroubleshootTemplate } from '../lib/aiTroubleshootTemplates';
  import { groupInspection, incidentEvidenceLimit, incidentHasContent, incidentStepLimit, loadIncident, prepareIncident, type IncidentEvidence, type IncidentRecord, type InspectionPane } from '../lib/operationsWorkbench';
  import { operationsMessages, type OperationsMessage } from '../lib/operationsMessages';
  import type { SessionMeta } from '../lib/types';

  let { rpc, visible, record = $bindable(), panes, onFocusPane, onAddEvidence, captureScene, reopenScene, profileIds = [] }: {
    rpc: RpcClient; visible: boolean; record: IncidentRecord; panes: readonly InspectionPane[];
    onFocusPane: (id: string) => void; onAddEvidence: (entry: IncidentEvidence) => boolean;
    captureScene?: (paneId?: string) => EventScene | null;
    reopenScene?: (scene: EventScene) => Promise<boolean>;
    profileIds?: readonly string[];
  } = $props();
  const t = (key: OperationsMessage) => operationsMessages[i18n.resolvedLocale][key];
  const monitoring = $derived(getHostMonitoring(rpc));
  const catalog = $derived(getProfileCatalog(rpc));
  const inspectionTargets = $derived([
    ...panes.filter(pane => !pane.pending && (!live || !checked.has(pane.id) || live.has(pane.id))).map(pane => ({
      id: `session:${pane.id}`, sessionId: pane.id, profileId: pane.profileId, label: pane.label,
      detail: pane.hostLabel, group: i18n.t('hostPicker.openSessions'), section: 'session',
      profile: catalog.profiles.find(profile => profile.id === pane.profileId),
    })),
    ...catalog.profiles.filter(profile => profile.kind === 'ssh').map(profile => ({
      ...profileChoice(profile), id: `profile:${profile.id}`, profileId: profile.id,
    })),
  ]);
  let page = $state<'incident' | 'inspection'>('incident');
  let storage: Storage | undefined;
  let db: IDBDatabase | undefined;
  let ready = $state(false);
  let busy = $state(false);
  let events = $state<OpsEvent[]>([]);
  let eventId = $state<string | null>(null);
  let project = $state('');
  let environment = $state('');
  let scene = $state<EventScene | null>(null);
  let baseline = $state('');
  let search = $state('');
  let archived = $state(false);
  let pending = $state<(() => void) | null>(null);
  let preview = $state<OpsEvent | null>(null);
  let action = $state<'save' | 'handoff' | 'scene' | null>(null);
  const selected = $derived(events.find(event => event.id === eventId));
  const visibleEvents = $derived(searchEvents(events, search, archived));
  const signature = () => JSON.stringify([record, project, environment, scene]);
  const dirty = $derived(baseline ? baseline !== signature() : incidentHasContent(record) || !!project || !!environment || !!scene);
  const draft = () => saveEvent(selected ?? null, eventId ?? uuidv4(), { project, environment }, record, included, Date.now(), scene);
  let markdown = $state('');
  function references(node: EventScene['layout']): string[] {
    return 'profileId' in node ? [node.profileId] : node.children.flatMap(references);
  }
  const missing = $derived(scene ? [...new Set(references(scene.layout))].filter(id => !profileIds.includes(id)) : []);
  let included = $state<number[]>([]);
  let notice = $state<OperationsMessage | null>(null);
  let templates = $state<TroubleshootTemplate[]>([]);
  let templateId = $state('');
  let live = $state<Set<string> | null>(null);
  let refreshing = $state(false);
  let checkedAt = $state<number | null>(null);
  let checked = $state<Set<string>>(new Set());
  const groups = $derived(groupInspection(panes, live, checked));
  const snapshots = $derived.by(() => {
    if (!visible || page !== 'inspection') return undefined;
    void monitoring.revision;
    return new Map(groups.map(group => [group.key, group.profileId ? monitoring.service.snapshot(group.profileId) : undefined]));
  });
  const builtins = [
    { id:'ssh-failure', name:'sshFailure', step:'sshStep' },
    { id:'port-in-use', name:'portInUse', step:'portStep' },
    { id:'disk-full', name:'diskFull', step:'diskStep' },
  ] as const;

  onMount(() => {
    let active = true;
    void (async () => {
      try {
        try { storage = window.localStorage; } catch { /* IndexedDB can still be available. */ }
        const opened = await openOpsProject();
        if (!active) { opened.close(); return; }
        db = opened;
        try { await migrateLegacy(loadIncident(storage), event => writeOpsEvent(opened, event, true)); }
        catch { if (active) notice = 'saveError'; }
        const loaded = await listOpsEvents(opened);
        if (active) { events = loaded; ready = true; }
      } catch { if (active) notice = 'saveError'; }
    })();
    return () => { active = false; db?.close(); };
  });
  $effect(() => { if (!visible) { action = null; preview = null; pending = null; } });
  function editTemplateList() { if (storage) templates = loadTroubleshootTemplates(storage); }
  function startSave() {
    const savedEvidence = new Set(selected?.record.evidence.map(item => JSON.stringify(item)) ?? []);
    included = record.evidence.flatMap((item, index) => savedEvidence.has(JSON.stringify(item)) ? [index] : []);
    preview = null; notice = null; action = 'save';
  }
  async function persist(next: OpsEvent) {
    if (!db || busy) return false;
    busy = true;
    try {
      // Rebuild the plain record and layout before crossing the IndexedDB boundary.
      const value = JSON.parse(JSON.stringify(next)) as OpsEvent;
      await writeOpsEvent(db, value);
      events = [...events.filter(event => event.id !== value.id), value];
      return true;
    } catch { notice = 'saveError'; return false; }
    finally { busy = false; }
  }
  async function confirmSave() {
    const next = preview;
    if (!next || !await persist(next)) return;
    eventId = next.id; baseline = JSON.stringify([next.record, next.project, next.environment, next.scene]);
    notice = 'saved'; action = null; preview = null;
  }
  function switchEvent(next: OpsEvent | null, seed?: IncidentEvidence, attach?: EventScene | null) {
    if (busy) return;
    const apply = () => {
      eventId = next?.id ?? null; project = next?.project ?? ''; environment = next?.environment ?? '';
      scene = next?.scene ?? null;
      record = next ? prepareIncident(next.record, next.record.evidence.map((_, index) => index)) : { title:'', conclusion:'', evidence:[], steps:[] };
      baseline = signature();
      if (seed) { record.title = seed.label.slice(0, 160); record.evidence.push(seed); scene = attach ?? null; }
      page = 'incident'; action = null; preview = null; pending = null; notice = next ? 'restored' : null;
    };
    if (dirty) pending = apply; else apply();
  }
  async function toggleArchive() {
    if (!selected || dirty) return;
    const next = archiveEvent(selected, !selected.archived);
    if (await persist(next)) archived = next.archived;
  }
  async function connectScene() {
    if (!scene || !reopenScene || missing.length || busy) return;
    busy = true; action = null;
    try { if (!await reopenScene(scene)) notice = 'sceneFailed'; }
    catch { notice = 'sceneFailed'; }
    finally { busy = false; }
  }
  function buildHandoff() {
    const snapshot = draft();
    snapshot.timeline = selected?.timeline ?? [];
    // draft() has already filtered the evidence; all remaining entries are intentional.
    return handoffMarkdown(snapshot, snapshot.record.evidence.map((_, index) => index), i18n.resolvedLocale);
  }
  function startHandoff() { included = []; markdown = buildHandoff(); action = 'handoff'; }
  async function copyHandoff() {
    try { await navigator.clipboard.writeText(markdown); notice = 'copiedHandoff'; }
    catch { notice = 'copyFailed'; }
  }
  async function exportHandoff() {
    if (busy || !markdown.trim()) return;
    busy = true;
    try { if (await exportMarkdownFile(record.title || 'incident', markdown)) notice = 'exported'; }
    catch { notice = 'exportFailed'; }
    finally { busy = false; }
  }
  function toggleEvidence(index: number, checked: boolean) {
    included = checked ? [...included, index] : included.filter(value => value !== index);
  }
  function removeEvidence(index: number) {
    record.evidence.splice(index, 1);
    action = null; preview = null;
  }
  function applyTemplate() {
    const builtin = builtins.find(item => item.id === templateId);
    const labels = builtin ? [t(builtin.step)] : templates.find(item => item.id === templateId)?.steps?.split(/\r?\n/).map(line => line.trim()).filter(Boolean) ?? [];
    if (labels.length + record.steps.length > incidentStepLimit) { notice = 'limit'; return; }
    const name = builtin ? t(builtin.name) : templates.find(item => item.id === templateId)?.name ?? templateId;
    record.steps.push(...instantiateRunbook(templateId, name, labels));
    notice = null;
  }
  async function refresh() {
    if (refreshing) return;
    refreshing = true; notice = null;
    const queried = new Set(panes.filter(pane => !pane.pending).map(pane => pane.id));
    try {
      const sessions = await rpc.call<SessionMeta[]>('session.list');
      if (!Array.isArray(sessions)) throw new Error('invalid session list');
      live = new Set(sessions.filter(session => session.kind === 'Ssh').map(session => session.id));
      checked = queried;
      checkedAt = Date.now();
    } catch { live = null; checkedAt = null; notice = 'refreshError'; }
    finally { refreshing = false; }
  }
  function percent(value?: number | null) { return typeof value === 'number' && Number.isFinite(value) ? `${Math.round(value)}%` : '—'; }
  function metrics(stats: { cpu_percent?: number | null; mem_percent?: number | null; disk_percent?: number | null }) {
    return `${t('cpu')} ${percent(stats.cpu_percent)} · ${t('memory')} ${percent(stats.mem_percent)} · ${t('disk')} ${percent(stats.disk_percent)}`;
  }
  function hostEvidence(label: string, status: OperationsMessage, detail?: string): IncidentEvidence {
    return { label, text: [t(status), detail].filter(Boolean).join('\n'), source: 'inspection', at: Date.now() };
  }
  function addHost(label: string, status: OperationsMessage, detail?: string) {
    const added = onAddEvidence(hostEvidence(label, status, detail));
    notice = added ? 'added' : 'evidenceLimit';
    if (added) page = 'incident';
  }
</script>

<section class="operations" aria-label={t('title')} tabindex="-1">
  <div class="actions">
    <button type="button" aria-pressed={page === 'incident'} onclick={() => { page = 'incident'; notice = null; }}>{t('incident')}</button>
    <button type="button" aria-pressed={page === 'inspection'} onclick={() => { page = 'inspection'; notice = null; action = null; preview = null; }}>{t('inspection')}</button>
  </div>
  {#if notice}<p role="status">{t(notice)}</p>{/if}
  {#if pending}
    <section class="box"><p>{t('discard')}</p><div class="actions">
      <button type="button" disabled={busy} onclick={() => pending?.()}>{t('proceed')}</button>
      <button type="button" onclick={() => { pending = null; }}>{t('cancel')}</button>
    </div></section>
  {/if}
  {#if page === 'incident'}
    <section class="box" aria-label={t('library')}>
      <div class="actions"><h4>{t('library')}</h4><button type="button" disabled={busy} onclick={() => switchEvent(null)}>{t('newEvent')}</button></div>
      <label>{t('search')}<input bind:value={search} maxlength="160" /></label>
      <label class="inline"><input type="checkbox" bind:checked={archived} />{t('archived')}</label>
      <div class="event-list">
        {#each visibleEvents as event (event.id)}
          <button type="button" aria-pressed={event.id === eventId} disabled={busy} onclick={() => switchEvent(event)}>{event.record.title || t('untitled')}{event.project || event.environment ? ` · ${[event.project, event.environment].filter(Boolean).join(' / ')}` : ''}</button>
        {/each}
      </div>
    </section>
    <label>{t('project')}<input maxlength="100" bind:value={project} /></label>
    <label>{t('environment')}<input maxlength="100" bind:value={environment} /></label>
    {#if captureScene}
      <section class="box">
        <p>{t('sceneHelp')}</p>
        <div class="actions"><button type="button" disabled={busy} onclick={() => { const captured = captureScene?.(); if (captured) { scene = captured; notice = 'sceneAttached'; } else notice = 'noScene'; }}>{t('attachScene')}</button></div>
        {#if scene}
          <p>{scene.title}</p>
          {#if missing.length}<p>{t('missingProfiles')}: {missing.join(', ')}</p>{/if}
          <div class="actions">
            <button type="button" disabled={busy || !!missing.length || !reopenScene} onclick={() => { action = 'scene'; }}>{t('reopenScene')}</button>
            <button type="button" disabled={busy} onclick={() => { scene = null; action = null; }}>{t('removeScene')}</button>
          </div>
        {/if}
      </section>
    {/if}
    <label>{t('recordTitle')}<input maxlength="160" bind:value={record.title} /></label>
    <label>{t('conclusion')}<textarea rows="3" maxlength="4000" bind:value={record.conclusion}></textarea></label>
    <section class="box" aria-label={t('evidence')}>
      <h4>{t('evidence')} ({record.evidence.length})</h4>
      {#each record.evidence as entry, index}
        <div class="item">
          <label>{t('source')}<input maxlength="240" bind:value={entry.label} /></label>
          {#if entry.at !== undefined}<p>{t('evidenceTime')}: {new Date(entry.at).toLocaleString()}</p>{/if}
          <label>{t('evidenceText')}<textarea rows="3" maxlength="4000" bind:value={entry.text}></textarea></label>
          <button type="button" onclick={() => removeEvidence(index)}>{t('remove')}</button>
        </div>
      {/each}
      <button type="button" disabled={record.evidence.length >= incidentEvidenceLimit} onclick={() => record.evidence.push({ label:'', text:'', source:'manual', at:Date.now() })}>{t('addEvidence')}</button>
    </section>
    <section class="box" aria-label={t('runbook')}>
      <h4>{t('runbook')} ({record.steps.filter(step => step.done).length}/{record.steps.length})</h4>
      <label>{t('template')}
        <select bind:value={templateId} onfocus={editTemplateList}>
          <option value="">—</option>
          {#each builtins as item (item.id)}<option value={item.id}>{t(item.name)}</option>{/each}
          {#each templates.filter(item => item.steps) as item (item.id)}<option value={item.id}>{item.name}</option>{/each}
        </select>
      </label>
      <div class="actions">
        <button type="button" disabled={!templateId} onclick={applyTemplate}>{t('apply')}</button>
        <button type="button" disabled={record.steps.length >= incidentStepLimit} onclick={() => record.steps.push({ label:'', expected:'', actual:'', done:false })}>{t('addStep')}</button>
      </div>
      {#each record.steps as step, index}
        <div class="item">
          <label>{t('step')} {index + 1}<input maxlength="240" bind:value={step.label} /></label>
          {#if step.template}<p>{step.template.name} · {step.template.version}</p>{/if}
          <label>{t('stepKind')}<select value={step.kind ?? 'check'} onchange={e => { step.kind = e.currentTarget.value as typeof step.kind; }}>
            <option value="check">{t('check')}</option><option value="evidence">{t('evidenceStep')}</option><option value="confirmation">{t('confirmation')}</option>
          </select></label>
          <label>{t('expected')}<textarea rows="2" maxlength="1000" bind:value={step.expected}></textarea></label>
          <label>{t('actual')}<textarea rows="2" maxlength="2000" bind:value={step.actual}></textarea></label>
          <label>{t('skipReason')}<input maxlength="1000" value={step.skipReason ?? ''} oninput={e => { step.skipReason = e.currentTarget.value || undefined; if (step.skipReason?.trim()) step.done = false; }} /></label>
          <div class="actions">
            <label class="inline"><input type="checkbox" bind:checked={step.done} disabled={!!step.skipReason?.trim()} />{t('done')}</label>
            <button type="button" onclick={() => { record.steps.splice(index, 1); }}>{t('remove')}</button>
          </div>
        </div>
      {/each}
    </section>
    <div class="actions">
      <button type="button" disabled={!ready || busy} onclick={startSave}>{t('save')}</button>
      <button type="button" disabled={busy || !incidentHasContent(record)} onclick={startHandoff}>{t('handoff')}</button>
      {#if selected}<button type="button" disabled={dirty || busy} onclick={toggleArchive}>{t(selected.archived ? 'reopen' : 'archive')}</button>{/if}
    </div>
    {#if action === 'save'}
      <section class="box" aria-label={t('save')}>
        <p>{t('saveHelp')}</p>
        {#if preview}
          <p>{preview.project} / {preview.environment}</p>
          {#if preview.scene}<p>{t('sceneAttached')}: {preview.scene.title}</p>{/if}
          {#if preview.record.title}<p><strong>{t('recordTitle')}:</strong> {preview.record.title}</p>{/if}
          {#if preview.record.conclusion}<p><strong>{t('conclusion')}:</strong> {preview.record.conclusion}</p>{/if}
          {#each preview.record.steps as step}<p><strong>{step.done ? '✓' : '○'} {step.label}</strong><br />{step.expected}<br />{step.actual}{#if step.skipReason}<br />{step.skipReason}{/if}</p>{/each}
          {#each preview.record.evidence as entry}<p><strong>{entry.label}</strong><br />{entry.text}</p>{/each}
          <button type="button" disabled={busy || !incidentHasContent(preview.record)} onclick={confirmSave}>{t('saveConfirm')}</button>
        {:else}
          {#each record.evidence as entry, index}
            <label class="inline"><input type="checkbox" checked={included.includes(index)} onchange={event => toggleEvidence(index, event.currentTarget.checked)} />{t('selectEvidence')} · {entry.label || index + 1}</label>
          {/each}
          <button type="button" onclick={() => { preview = draft(); }}>{t('review')}</button>
        {/if}
        <button type="button" onclick={() => { action = null; preview = null; }}>{t('cancel')}</button>
      </section>
    {:else if action === 'scene'}
      <section class="box"><p>{t('sceneHelp')}</p><div class="actions">
        <button type="button" disabled={busy || !!missing.length} onclick={connectScene}>{t('reopenScene')}</button>
        <button type="button" onclick={() => { action = null; }}>{t('cancel')}</button>
      </div></section>
    {:else if action === 'handoff'}
      <section class="box" aria-label={t('handoff')}>
        {#each record.evidence as entry, index}
          <label class="inline"><input type="checkbox" checked={included.includes(index)} onchange={e => { toggleEvidence(index, e.currentTarget.checked); markdown = buildHandoff(); }} />{t('includeHandoff')} · {entry.label || index + 1}</label>
        {/each}
        <label>{t('handoff')}<textarea rows="10" bind:value={markdown}></textarea></label>
        <div class="actions"><button type="button" onclick={copyHandoff}>{t('copyHandoff')}</button><button type="button" disabled={busy || !markdown.trim()} onclick={exportHandoff}>{t('exportMarkdown')}</button><button type="button" onclick={() => { action = null; }}>{t('close')}</button></div>
      </section>
    {/if}
    {#if selected}
      <section class="box" aria-label={t('timeline')}><h4>{t('timeline')}</h4>
        {#each [...selected.timeline].reverse() as entry}
          <p>{new Date(entry.at).toLocaleString()} · {t(entry.kind === 'archived' ? 'archivedEvent' : entry.kind)}</p>
        {/each}
      </section>
    {/if}
  {/if}
  <div hidden={page !== 'inspection'}>
    <HostInspection {rpc} targets={inspectionTargets} onIncident={(entry, id) => switchEvent(null, entry, id ? { title: entry.label, layout: { profileId: id } } : null)} />
    <details class="box session-overview">
      <summary>{t('sessionOverview')}</summary>
    <p>{t('inspectHelp')}</p>
    <div class="actions">
      <button type="button" disabled={refreshing} onclick={refresh}>{t('refresh')}</button>
      {#if checkedAt}<time datetime={new Date(checkedAt).toISOString()}>{new Date(checkedAt).toLocaleTimeString()}</time>{/if}
    </div>
    {#if !groups.length}<p>{t('noHosts')}</p>{/if}
    {#each groups as group (group.key)}
      {@const snapshot = snapshots?.get(group.key)}
      {@const latest = snapshot?.latest}
      <section class="box" aria-label={group.label}>
        <h4>{group.label}</h4>
        {#if latest}<p>{t('cached')} · {new Date(latest.at).toLocaleTimeString()} · {metrics(latest.stats)}</p>{:else}<p>{t('noSample')}</p>{/if}
        {#if snapshot?.error}<p>{t('sampleError')}</p>{/if}
        {#each group.panes as pane (pane.id)}
          <div class="item">
            <p>{pane.label} · {t(pane.status)}</p>
            <div class="actions">
              <button type="button" onclick={() => onFocusPane(pane.id)}>{t('focusPane')}</button>
              <button type="button" onclick={() => addHost(pane.label, pane.status, latest && `${t('cached')} · ${new Date(latest.at).toLocaleString()} · ${metrics(latest.stats)}`)}>{t('addHost')}</button>
              <button type="button" disabled={busy} onclick={() => switchEvent(null, hostEvidence(pane.label, pane.status, latest && `${t('cached')} · ${new Date(latest.at).toLocaleString()} · ${metrics(latest.stats)}`), captureScene?.(pane.id))}>{t('newFromHost')}</button>
            </div>
          </div>
        {/each}
      </section>
    {/each}
    </details>
  </div>
</section>

<style>
  .operations{display:grid;align-content:start;gap:9px;padding:10px;min-width:0;font-size:12px;color:var(--color-fg)}
  h4,p{margin:0;overflow-wrap:anywhere;white-space:pre-wrap}p,time{color:var(--color-fg-muted)}
  .actions,.inline{display:flex;align-items:center;gap:6px;flex-wrap:wrap}
  .box,.item,label{display:grid;gap:6px;min-width:0}.box{border:1px solid var(--color-border);border-radius:6px;padding:8px}.item{border-top:1px solid var(--color-border);padding-top:6px}
  label.inline{display:flex}textarea{resize:vertical}button,input,select,textarea{min-width:0;max-width:100%;font:inherit;color:inherit;background:var(--color-bg-soft);border:1px solid var(--color-border);border-radius:4px;padding:5px 7px}
  button:disabled{opacity:.55}button[aria-pressed='true']{border-color:var(--color-accent);color:var(--color-accent)}
</style>
