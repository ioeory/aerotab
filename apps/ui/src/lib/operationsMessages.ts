const en = {
  title:'Operations', incident:'Incident', inspection:'Host inspection', recordTitle:'Incident title', conclusion:'Conclusion', evidence:'Evidence', evidenceText:'Evidence excerpt', source:'Source', addEvidence:'Add evidence excerpt', addA:'Add A to incident', addB:'Add B to incident', selectEvidence:'Include this excerpt when saving', evidenceLimit:'The incident already has 12 excerpts. Remove one before adding another.',
  runbook:'Manual runbook', step:'Step', expected:'Expected result', actual:'Actual result', done:'Verified', addStep:'Add step', remove:'Remove', template:'Runbook template', apply:'Add template steps', limit:'Not enough space (maximum 20 steps). Remove steps before applying this template.',
  save:'Review local save', saveConfirm:'Save to this device', cancel:'Cancel', restore:'Restore saved record', restoreConfirm:'Replace the current record with the saved copy?', clear:'Delete saved copy', clearConfirm:'Delete the local copy? Your current edits will remain open.',
  saveHelp:'Incidents stay on this device. Title, conclusion and steps are saved after confirmation. Evidence starts unchecked for new excerpts. Storage is not encrypted or synced; review for secrets before saving.', saved:'Saved locally.', restored:'Saved record restored. No sessions were opened.', deleted:'Saved copy deleted.', saveError:'Could not access local storage.', added:'Added to the current incident.',
  inspectHelp:'Lists SSH panes in this workspace and existing monitoring samples. Refresh checks the local session registry only; it does not connect or run probes.', refresh:'Refresh session status', refreshError:'Could not check the session registry.', noHosts:'No SSH panes in this workspace.', live:'Session registered', absent:'Session no longer registered', unknown:'Not checked', connecting:'Connecting', failed:'Connection failed', noSample:'No cached monitoring sample', cached:'Cached sample', sampleError:'Latest monitoring attempt failed', cpu:'CPU', memory:'Memory', disk:'Disk', addHost:'Add to incident', focusPane:'Show pane',
  sshFailure:'SSH failure', portInUse:'Port in use', diskFull:'Disk full',
  sshStep:'Verify endpoint, network reachability, and SSH authentication method.', portStep:'Verify which process is listening and whether its port matches the service configuration.', diskStep:'Verify filesystem usage and identify which directory is growing.',
};
const zh: Record<keyof typeof en, string> = {
  title:'运维工作台', incident:'故障记录', inspection:'主机巡检', recordTitle:'故障标题', conclusion:'结论', evidence:'证据', evidenceText:'证据摘录', source:'来源', addEvidence:'添加证据摘录', addA:'将 A 加入故障记录', addB:'将 B 加入故障记录', selectEvidence:'保存时包含这条摘录', evidenceLimit:'故障记录已有 12 条摘录，移除一条后再添加。',
  runbook:'人工排障手册', step:'步骤', expected:'预期结果', actual:'实际结果', done:'已验证', addStep:'添加步骤', remove:'移除', template:'排障模板', apply:'添加模板步骤', limit:'最多 20 个步骤，请先移除部分步骤再应用模板。',
  save:'预览本地保存', saveConfirm:'保存到本机', cancel:'取消', restore:'恢复已保存的记录', restoreConfirm:'用已保存的副本替换当前记录？', clear:'删除本地副本', clearConfirm:'删除本地副本？当前编辑内容仍会保留。',
  saveHelp:'事件仅保存在本机。确认后保存标题、结论和步骤；新增证据默认不勾选。记录不加密、不同步，保存前请检查敏感信息。', saved:'已保存到本机。', restored:'已恢复本地记录，未打开任何会话。', deleted:'已删除本地副本。', saveError:'无法访问本机存储。', added:'已加入当前故障记录。',
  inspectHelp:'列出当前工作区中的 SSH 窗格及已有监控样本。刷新只查询本机会话列表，不建立连接或运行探针。', refresh:'刷新会话状态', refreshError:'无法查询会话列表。', noHosts:'此工作区没有 SSH 窗格。', live:'会话在册', absent:'会话已不在列表', unknown:'未检查', connecting:'连接中', failed:'连接失败', noSample:'暂无缓存监控样本', cached:'缓存样本', sampleError:'最近一次监控采样失败', cpu:'CPU', memory:'内存', disk:'磁盘', addHost:'加入故障记录', focusPane:'显示窗格',
  sshFailure:'SSH 连接失败', portInUse:'端口占用', diskFull:'磁盘空间已满',
  sshStep:'核对目标地址、网络可达性和 SSH 认证方式。', portStep:'核对监听进程及其端口是否与服务配置一致。', diskStep:'核对文件系统用量，定位正在增长的目录。',
};
const projectEn = {
  sessionOverview:'SSH session overview', exportMarkdown:'Save Markdown file', exported:'Markdown file saved.', exportFailed:'Could not save the Markdown file.',
  project:'Project', environment:'Environment', library:'Incident library', search:'Search incidents', archived:'Show archived', newEvent:'New incident', untitled:'Untitled incident', archive:'Archive saved incident', reopen:'Reopen incident', timeline:'Timeline', discard:'Discard current unsaved edits?', proceed:'Continue',
  created:'Created', updated:'Updated', reopened:'Reopened', archivedEvent:'Archived', imported:'Imported from local draft',
  attachScene:'Capture active SSH layout', sceneAttached:'SSH layout attached', removeScene:'Remove layout', reopenScene:'Reopen SSH panes', sceneHelp:'Only saved SSH profile references are included. Reopening makes new connections.', noScene:'No saved SSH profiles in the active tab.', missingProfiles:'Missing SSH profile IDs', sceneFailed:'Some SSH panes could not be opened.', newFromHost:'Start incident from this pane',
  stepKind:'Step type', check:'Check', evidenceStep:'Collect evidence', confirmation:'Manual confirmation', skipReason:'Skip reason (optional)', handoff:'Preview Markdown handoff', copyHandoff:'Copy Markdown', copiedHandoff:'Copied.', copyFailed:'Could not copy. Select and copy the Markdown manually.', evidenceTime:'Captured', review:'Build save preview', close:'Close', includeHandoff:'Include in handoff', selectEvidence:'Include this excerpt when saving',
};
const projectZh: Record<keyof typeof projectEn,string> = {
  sessionOverview:'SSH 会话概览', exportMarkdown:'保存 Markdown 文件', exported:'Markdown 文件已保存。', exportFailed:'无法保存 Markdown 文件。',
  project:'项目', environment:'环境', library:'事件库', search:'搜索事件', archived:'查看已归档', newEvent:'新建事件', untitled:'未命名事件', archive:'归档已保存的事件', reopen:'重新打开事件', timeline:'时间线', discard:'放弃当前未保存的编辑？', proceed:'继续',
  created:'创建', updated:'更新', reopened:'重新打开', archivedEvent:'归档', imported:'从本机草稿迁移',
  attachScene:'记录当前 SSH 窗格布局', sceneAttached:'已关联 SSH 布局', removeScene:'移除布局', reopenScene:'重开 SSH 窗格', sceneHelp:'仅关联已保存的 SSH 配置。重开会建立新连接。', noScene:'当前标签没有可记录的已保存 SSH 配置。', missingProfiles:'已失效的 SSH 配置 ID', sceneFailed:'部分 SSH 窗格无法打开。', newFromHost:'从此窗格新建事件',
  stepKind:'步骤类型', check:'检查', evidenceStep:'收集证据', confirmation:'人工确认', skipReason:'跳过原因（可选）', handoff:'预览 Markdown 交接', copyHandoff:'复制 Markdown', copiedHandoff:'已复制。', copyFailed:'无法复制，请选中 Markdown 手动复制。', evidenceTime:'采集时间', review:'生成保存预览', close:'关闭', includeHandoff:'纳入交接', selectEvidence:'保存时包含这条摘录',
};
export const operationsMessages = { en: { ...en, ...projectEn }, 'zh-CN': { ...zh, ...projectZh } };
export type OperationsMessage = keyof typeof operationsMessages.en;
