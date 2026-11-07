<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import type { RpcClient } from '../lib/rpc';
  import type { StoredProfile } from '../lib/types';
  import { ScriptRunner, scriptPayload, type ScriptInput, type SavedScript, type ScriptTask, type ScriptConfirmation } from '../lib/scripts';
  import { scriptsText as t } from '../lib/scriptsMessages';
  let { rpc, profiles, onError }: { rpc: RpcClient; profiles: StoredProfile[]; onError: (message: string) => void } = $props();
  let scripts = $state<SavedScript[]>([]);
  let draft = $state<ScriptInput | null>(null);
  let version = $state(0);
  let saved = $state('');
  let busy = $state(false);
  let error = $state('');
  let timeout = $state(30);
  let targetId = $state('');
  let confirmation = $state<ScriptConfirmation | null>(null);
  let task = $state<ScriptTask | null>(null);
  let tasks = $state<ScriptTask[]>([]);
  let deletePending = $state(false);
  let alive = true;
  let polling = false;
  let timer: ReturnType<typeof setInterval> | undefined;
  const runner = new ScriptRunner(rpc);
  const dirty = $derived(draft !== null && JSON.stringify(scriptPayload(draft)) !== saved);
  const sshProfiles = $derived(profiles.filter(p => p.kind === 'ssh'));
  function report(cause: unknown) { error = cause instanceof Error ? cause.message : String(cause); onError(error); }
  async function action(work: () => Promise<void>) {
    if (busy) return;
    busy = true; error = '';
    try { await work(); } catch (cause) { report(cause); } finally { busy = false; }
  }
  async function load() {
    scripts = await rpc.call<SavedScript[]>('scripts.list');
    tasks = await rpc.call<ScriptTask[]>('scripts.tasks');
    if (!task && tasks.length) { task = tasks[tasks.length - 1] ?? null; runner.task = task; }
  }
  function dismiss() { runner.dismiss(); confirmation = null; deletePending = false; }
  function choose(script: SavedScript) {
    if (dirty) { report(t('unsaved')); return; }
    dismiss();
    draft = scriptPayload(script); version = script.version; saved = JSON.stringify(scriptPayload(script));
    targetId = script.profile_id ?? '';
  }
  function create() {
    if (dirty) { report(t('unsaved')); return; }
    dismiss();
    draft = { id: crypto.randomUUID(), name: '', command: '', description: '', profile_id: null };
    version = 0; saved = ''; targetId = '';
  }
  async function save() {
    if (!draft) return;
    const result = await rpc.call<SavedScript>('scripts.put', { script: scriptPayload(draft), expectedVersion: version });
    saved = JSON.stringify(scriptPayload(result)); draft = scriptPayload(result); version = result.version;
    dismiss(); await load();
  }
  async function prepare() {
    if (!draft || dirty) { report(t('unsaved')); return; }
    if (!sshProfiles.some(p => p.id === targetId)) { report(t('missing')); return; }
    confirmation = await runner.prepare(draft.id, targetId, timeout);
  }
  async function run() {
    confirmation = null;
    task = await runner.confirm();
    if (task) tasks = [...tasks, task];
  }
  async function refreshTask() {
    if (!task || task.status !== 'running' || polling) return;
    polling = true;
    try { const result = await runner.refresh(); if (alive) task = result; }
    catch (cause) { if (alive) report(cause); }
    finally { polling = false; }
  }
  onMount(() => {
    void action(load);
    timer = setInterval(() => { void refreshTask(); }, 750);
  });
  // Unmount stops observation, not execution. Reopening lists retained tasks.
  onDestroy(() => { alive = false; clearInterval(timer); runner.dismiss(); });
</script>

<section class="scripts-workspace" aria-label={t('title')}>
  <header><h2>{t('title')}</h2><button disabled={busy || dirty} onclick={create}>{t('new')}</button><button disabled={busy} onclick={() => action(load)}>{t('refresh')}</button></header>
  <p class="hint">{t('warning')}</p>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <div class="layout">
    <nav aria-label={t('title')}>
      {#each scripts as script (script.id)}
        <button class:active={draft?.id === script.id} disabled={busy} onclick={() => choose(script)}>{script.name}</button>
      {:else}<p>{t('empty')}</p>{/each}
    </nav>
    <main>
      {#if draft}
        <div class="fields">
          <label>{t('name')}<input maxlength="256" bind:value={draft.name} disabled={busy || !!confirmation} /></label>
          <label>{t('description')}<textarea rows="2" maxlength="16384" bind:value={draft.description} disabled={busy || !!confirmation}></textarea></label>
          <label>{t('command')}<textarea class="command" rows="8" maxlength="65536" bind:value={draft.command} disabled={busy || !!confirmation} spellcheck="false"></textarea></label>
          <label>{t('target')}
            <select bind:value={draft.profile_id} disabled={busy || !!confirmation}>
              <option value={null}>{t('none')}</option>
              {#each sshProfiles as profile (profile.id)}<option value={profile.id}>{profile.name}</option>{/each}
              {#if draft.profile_id && !sshProfiles.some(p => p.id === draft?.profile_id)}<option value={draft.profile_id}>{t('missing')}</option>{/if}
            </select>
          </label>
        </div>
        <div class="actions">
          <button disabled={busy || !dirty || !draft.name.trim() || !draft.command.trim() || !!confirmation} onclick={() => action(save)}>{t('save')}</button>
          {#if dirty}<span role="status">{t('dirty')}</span><button disabled={busy} onclick={() => { draft = null; saved = ''; dismiss(); }}>{t('discard')}</button>{/if}
          <button disabled={busy || version === 0 || !!confirmation} onclick={() => deletePending = true}>{t('remove')}</button>
        </div>
        {#if deletePending}
          <div class="confirmation" role="alert">
            <p>{t('deleteConfirm')}</p>
            <button disabled={busy} onclick={() => action(async () => { if (!draft) return; await rpc.call('scripts.delete', { id: draft.id, expectedVersion: version }); draft = null; saved = ''; dismiss(); await load(); })}>{t('remove')}</button>
            <button disabled={busy} onclick={dismiss}>{t('cancel')}</button>
          </div>
        {/if}
        <div class="run-controls">
          <label>{t('target')}<select bind:value={targetId} disabled={busy || !!confirmation}>
            <option value="">{t('select')}</option>
            {#each sshProfiles as profile (profile.id)}<option value={profile.id}>{profile.name} — {profile.kind === 'ssh' ? `${profile.ssh.user}@${profile.ssh.host}:${profile.ssh.port}` : ''}</option>{/each}
          </select></label>
          <label>{t('timeout')}<input type="number" min="1" max="300" bind:value={timeout} disabled={busy || !!confirmation} /></label>
          <button disabled={busy || dirty || version === 0 || !targetId || !!confirmation || task?.status === 'running'} onclick={() => action(prepare)}>{t('run')}</button>
        </div>
        {#if confirmation}
          <div class="confirmation" role="alert">
            <p>{t('confirmation')}</p><strong>{confirmation.script_name} → {confirmation.target}</strong>
            <p>{t('timeout')}: {confirmation.timeout_seconds}</p><pre>{confirmation.command}</pre>
            <button disabled={busy} onclick={() => action(run)}>{t('confirm')}</button><button disabled={busy} onclick={dismiss}>{t('cancel')}</button>
          </div>
        {/if}
      {:else}<p>{t('select')}</p>{/if}
      <section class="task" aria-label={t('output')}>
        <h3>{t('output')}</h3>
        {#if tasks.length}<select aria-label={t('output')} value={task?.id ?? ''} disabled={busy || polling} onchange={(event) => { task = tasks.find(t => t.id === event.currentTarget.value) ?? null; runner.task = task; }}>
          {#each tasks as entry (entry.id)}<option value={entry.id}>{entry.target} · {entry.id.slice(0, 8)}</option>{/each}
        </select>{/if}
        {#if task}
          <p role="status">{task.target} · {t(task.status)} {task.exit_code !== null ? `· ${t('exit')}: ${task.exit_code}` : ''}</p>
          {#if task.status === 'running'}<button disabled={busy} onclick={() => action(async () => { task = await runner.cancel(); })}>{t('stop')}</button>{/if}
          {#if task.error}<p class="error">{task.error}</p>{/if}
          <h4>{t('stdout')}</h4><pre>{task.stdout}</pre><h4>{t('stderr')}</h4><pre>{task.stderr}</pre>
        {:else}<p>{t('idle')}</p>{/if}
      </section>
    </main>
  </div>
</section>

<style>
  .scripts-workspace { height:100%; overflow:auto; padding:16px; color:var(--text,inherit); }
  header,.actions { display:flex; gap:8px; align-items:center; flex-wrap:wrap; } h2 { margin:0; flex:1; } .hint { opacity:.7; font-size:12px; }
  .layout { display:grid; grid-template-columns:minmax(140px,220px) minmax(0,1fr); gap:16px; } nav { border-right:1px solid var(--border,#4446); padding-right:12px; } nav button { display:block; width:100%; text-align:left; margin-bottom:6px; overflow-wrap:anywhere; } .active { outline:1px solid var(--accent,#589af0); }
  main { min-width:0; } .fields { display:grid; gap:10px; } label { display:flex; flex-direction:column; gap:5px; font-size:13px; } input,textarea,select { color:inherit; background:var(--bg-input,#8881); border:1px solid var(--border,#8885); border-radius:5px; padding:8px; min-width:0; } textarea { resize:vertical; } .command,pre { font-family:var(--font-mono,monospace); } button { border:1px solid var(--border,#8885); border-radius:5px; padding:6px 10px; color:inherit; background:var(--bg-button,#8882); cursor:pointer; } button:disabled { opacity:.45; cursor:default; } .actions { margin:12px 0; } .run-controls { display:flex; flex-wrap:wrap; align-items:end; gap:10px; padding:12px 0; } .confirmation { padding:12px; border:1px solid var(--accent,#589af0); border-radius:6px; margin:12px 0; } .confirmation button { margin-right:8px; } pre { white-space:pre-wrap; overflow-wrap:anywhere; max-height:320px; overflow:auto; padding:10px; background:#8881; } .error { color:var(--danger,#e06c75); } .task { border-top:1px solid var(--border,#8885); margin-top:16px; } h4 { margin-bottom:4px; }
  @media(max-width:720px) { .layout { grid-template-columns:1fr; } nav { border-right:0; border-bottom:1px solid var(--border,#8885); padding-bottom:8px; } }
</style>
