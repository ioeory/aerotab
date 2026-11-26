<script lang="ts">
  import { i18n } from '../lib/i18n.svelte';
  import type { RpcClient } from '../lib/rpc';
  import type { IncidentEvidence } from '../lib/operationsWorkbench';

  import HostChoiceList, { type HostChoice } from './HostChoiceList.svelte';

  type Target = HostChoice & ({ sessionId: string; profileId?: string } | { sessionId?: never; profileId: string });
  type Status = 'ok' | 'warning' | 'failed' | 'unavailable';
  type Check = { kind: 'resources' | 'inodes' } | { kind: 'service'; unit: string } | { kind: 'tcp_listen'; port: number };
  type Result = { status: Status; detail: string };
  type Row = { target: Target; at: number; checks: Check[]; results: Result[] };
  let { rpc, targets, onIncident }: { rpc: RpcClient; targets: readonly Target[]; onIncident: (evidence: IncidentEvidence, profileId?: string) => void } = $props();
  const cn = $derived(i18n.resolvedLocale === 'zh-CN');
  const m = (zh: string, en: string) => cn ? zh : en;
  let selected = $state<string[]>([]);
  let resources = $state(true), inodes = $state(true);
  let services = $state(''), ports = $state('');
  let running = $state(false), finished = $state(0), total = $state(0);
  let rows = $state<Row[]>([]);
  let issuesOnly = $state(true);
  let error = $state(false);
  const selectedTargets = $derived(targets.filter(target => selected.includes(target.id)));
  const weight: Record<Status, number> = { ok: 0, unavailable: 1, warning: 2, failed: 3 };
  const severity = (row: Row) => Math.max(...row.results.map(result => weight[result.status]));
  const displayed = $derived(rows.filter(row => !issuesOnly || severity(row) > 0).sort((a, b) => severity(b) - severity(a)));
  const statusLabel = (status: Status) => ({ ok:m('正常','OK'), warning:m('警告','Warning'), failed:m('异常','Failed'), unavailable:m('无法检查','Unavailable') })[status];
  function checkLabel(check: Check) {
    switch (check.kind) {
      case 'resources': return m('CPU / 内存 / 磁盘 / 负载','CPU / RAM / disk / load');
      case 'inodes': return m('根分区 inode','Root filesystem inodes');
      case 'service': return check.unit;
      case 'tcp_listen': return `TCP ${check.port}`;
    }
  }
  function describe(detail: string) {
    const labels: Record<string, [string, string]> = {
      active:['服务运行中','Service is active'], inactive:['服务未运行','Service is inactive'], failed:['服务失败','Service failed'],
      'not-found':['找不到该服务','Unit not found'], masked:['服务已屏蔽','Unit is masked'], activating:['服务正在启动','Service is starting'],
      deactivating:['服务正在停止','Service is stopping'], reloading:['服务正在重载','Service is reloading'],
      listening:['主机上存在监听','Listening on this host'], 'not-listening':['主机上没有监听','Not listening on this host'],
      'no-systemd-data':['systemd 不可用或无法读取该服务','systemd unavailable or unit status unreadable'],
      'no-listener-data':['无法读取主机监听列表','Cannot read listener list'], 'no-data':['未返回有效数据','No valid data'],
      'host-key':['SSH 主机密钥验证失败','SSH host key verification failed'], auth:['SSH 认证失败','SSH authentication failed'],
      'profile-unavailable':['SSH 配置或凭据无法读取','SSH profile or credentials unavailable'],
      'session-unavailable':['SSH 会话已结束或不可用','SSH session has ended or is unavailable'],
      'inspection busy':['巡检并发已满，请稍后重试','Inspection is busy; retry later'],
      'ssh-or-probe':['SSH 连接失败、超时或拒绝远端检查','SSH connection or remote check failed or timed out'],
    };
    const label = labels[detail];
    return label ? m(...label) : detail;
  }
  const selectedIds = $derived(selectedTargets.map(target => target.id));
  $effect(() => { if (selectedIds.length !== selected.length) selected = selectedIds; });
  async function start() {
    if (running || !selectedTargets.length || selectedTargets.length > 50) return;
    const parse = (value: string) => [...new Set(value.trim().split(/[\s,，]+/).filter(Boolean))];
    const units = parse(services).map(value => value.endsWith('.service') ? value : `${value}.service`);
    const tcp = parse(ports);
    const checks: Check[] = [...(resources ? [{kind:'resources'} as const] : []), ...(inodes ? [{kind:'inodes'} as const] : []),
      ...[...new Set(units)].map(unit => ({kind:'service' as const, unit})), ...[...new Set(tcp.map(Number))].map(port => ({kind:'tcp_listen' as const, port}))];
    error = !checks.length || checks.length > 16 || units.some(unit => !/^[a-zA-Z0-9][a-zA-Z0-9_.@:-]{0,111}\.service$/.test(unit))
      || tcp.some(value => !/^\d{1,5}$/.test(value) || Number(value) < 1 || Number(value) > 65535);
    if (error) return;
    const queue = [...selectedTargets];
    running = true; rows = []; finished = 0; total = queue.length;
    try {
      await Promise.all(Array.from({ length: Math.min(4, queue.length) }, async () => {
        let target: Target | undefined;
        while ((target = queue.shift())) {
          let results: Result[];
          try {
            const endpoint = target.sessionId ? { session_id: target.sessionId } : { profile_id: target.profileId };
            results = await rpc.call<Result[]>('ssh.inspectHost', { ...endpoint, checks });
            if (!Array.isArray(results) || results.length !== checks.length || results.some(result => !result || !Object.hasOwn(weight, result.status) || typeof result.detail !== 'string')) throw Error('no-data');
          } catch (cause) {
            const message = cause instanceof Error ? cause.message.replace(/^-?\d+:\s*/, '') : '';
            const reason = ['no-data', 'host-key', 'auth', 'profile-unavailable', 'session-unavailable', 'inspection busy'].includes(message) ? message : 'ssh-or-probe';
            results = checks.map(() => ({status:'unavailable',detail:reason}));
          }
          rows.push({ target, at: Date.now(), checks, results }); finished += 1;
        }
      }));
    } finally { running = false; }
  }
  function fileIncident(row: Row) {
    onIncident({ label: row.target.label, source:'inspection', at: row.at,
      text: row.checks.map((check, i) => `${checkLabel(check)}: ${statusLabel(row.results[i]!.status)} · ${describe(row.results[i]!.detail)}`).join('\n') }, row.target.profileId);
  }
</script>

<section aria-label={m('主动巡检','Active inspection')}>
  <p>{m('可选择已保存的 SSH 主机或已打开的 SSH 会话。巡检使用独立通道，不写入终端，也不修改服务。','Choose saved SSH hosts or open SSH sessions. Inspection uses a separate channel and does not write to the terminal or change services.')}</p>
  <details open>
    <summary>{m('选择主机或会话','Choose hosts or sessions')} ({selectedTargets.length}/{targets.length})</summary>
    <HostChoiceList choices={targets} selected={selected} onChange={ids => { selected = ids; }} disabled={running} limit={50} groupSelection />
  </details>
  <fieldset disabled={running}>
    <legend>{m('检查项目','Checks')}</legend>
    <label class="inline"><input type="checkbox" bind:checked={resources} />{m('资源：CPU、内存、磁盘 ≥ 90% 告警，显示 1 分钟负载','Resources: CPU, RAM or disk ≥ 90% warns; includes 1m load')}</label>
    <label class="inline"><input type="checkbox" bind:checked={inodes} />{m('根分区 inode ≥ 90% 告警','Root inodes ≥ 90% warns')}</label>
    <label>{m('预期运行的 systemd 服务（空格或逗号分隔）','Expected systemd services (space or comma separated)')}<input bind:value={services} maxlength="1200" placeholder="nginx, docker.service" /></label>
    <label>{m('预期监听的本机 TCP 端口（不检查应用响应）','Expected local TCP listeners (does not check app responses)')}<input bind:value={ports} maxlength="160" placeholder="80, 443, 5432" /></label>
  </fieldset>
  {#if error}<p role="alert">{m('请选择 1–16 项有效检查，端口须在 1–65535。','Choose 1–16 valid checks. Ports must be between 1 and 65535.')}</p>{/if}
  <button type="button" disabled={running || !selectedTargets.length} onclick={start}>{m('开始巡检','Start inspection')}</button>
  {#if total}
    <p role="status">{m('已完成','Completed')} {finished}/{total} · {m('需关注','Needs attention')} {rows.filter(row => severity(row) > 0).length}</p>
    <label class="inline"><input type="checkbox" bind:checked={issuesOnly} />{m('仅显示需关注','Show only hosts needing attention')}</label>
    {#if !running && !displayed.length}<p>{m('本次未发现需关注项。取消筛选可查看全部结果。','No issues in this run. Clear the filter to see all results.')}</p>{/if}
    {#each displayed as row (row.target.id)}
      <article>
        <strong>{row.target.label}</strong> · <time datetime={new Date(row.at).toISOString()}>{new Date(row.at).toLocaleTimeString()}</time>
        <ul>
          {#each row.results as result, index}
            <li class:ok={result.status === 'ok'}><strong>{statusLabel(result.status)}</strong> · {checkLabel(row.checks[index]!)} — {describe(result.detail)}</li>
          {/each}
        </ul>
        {#if severity(row) > 0}<button type="button" onclick={() => fileIncident(row)}>{m('从巡检结果新建事件','Start incident from result')}</button>{/if}
      </article>
    {/each}
  {/if}
</section>

<style>
  section,fieldset,label,article{display:grid;gap:6px;min-width:0}section{gap:9px}p,ul{margin:0}ul{padding-left:16px}li{overflow-wrap:anywhere}article,fieldset{border:1px solid var(--color-border-soft);border-radius:6px;padding:8px}.inline{display:flex;align-items:center;gap:6px}button,input:not([type=checkbox]){background:var(--color-panel);color:inherit;border:1px solid var(--color-border-soft);border-radius:4px;padding:5px}button{justify-self:start}button:disabled{opacity:.55}li strong{color:var(--color-accent)}li.ok strong{color:var(--color-fg-muted)}
</style>
