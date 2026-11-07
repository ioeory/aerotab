import { i18n } from './i18n.svelte';
export const en = {
  title: 'Host monitoring', newPanel: 'New panel', addHosts: 'Add hosts', rename: 'Rename', delete: 'Delete panel',
  deleteConfirm: 'Delete this panel? Profiles will not be deleted.', confirm: 'Confirm', cancel: 'Cancel', panelName: 'Panel name',
  start: 'Start monitoring', stop: 'Stop monitoring', interval: 'Sampling interval', range: 'Time range', sort: 'Sort',
  name: 'Name', cpu: 'CPU', memory: 'Memory', disk: 'Disk', network: 'Network', list: 'List', chart: 'Charts', search: 'Search hosts',
  notStarted: 'Not started', waiting: 'Waiting for first sample', sampling: 'Sampling', stopped: 'Stopped · last sample retained',
  empty: 'Create a panel and select SSH hosts to start monitoring.', noHosts: 'Add SSH hosts to this panel.', noResults: 'No matching hosts',
  historyHint: 'History is collected only while this application is running (up to 1 hour / 3,600 samples per host). No pre-existing or permanent history.',
  selected: 'selected', hosts: 'Hosts', ungrouped: 'Ungrouped', unsupported: 'Only SSH profiles support monitoring',
  picker: 'Select monitored hosts', selectGroup: 'Select group', clearGroup: 'Clear group', removeHost: 'Remove from panel',
  unavailable: 'Not supported / no data', missingProfile: 'Profile unavailable', probeFailed: 'Probe failed; retrying with backoff. Check connectivity and SSH authentication.',
  load: 'Load (1 / 5 / 15 min)', uptime: 'Uptime', os: 'Operating system', kernel: 'Kernel', swap: 'Swap',
  latency: 'SSH probe duration (not ICMP)', cores: 'CPU cores', mounts: 'Mount points', interfaces: 'Network interfaces',
  rx: 'Receive', tx: 'Transmit', total: 'Total', used: 'Used', diskIo: 'Disk I/O', read: 'Read', write: 'Write',
  system: 'System', sshOnly: 'System details require an active SSH pane.', lastSample: 'Last sampled',
  persistenceError: 'Panel preferences could not be saved on this device. Retry after checking local storage.', retry: 'Retry',
  minutes: 'min', seconds: 's', paused: 'Paused by window policy', actualRange: 'Actual sampled interval',
  cpuHistory: 'CPU history', memoryHistory: 'Memory history', diskHistory: 'Disk capacity history', networkHistory: 'Network throughput history',
  editHosts: 'Choose hosts', keepRunning: 'Running panels continue sampling when you leave this page.',
} as const;
export const zh: Record<keyof typeof en, string> = {
  title: '主机监控', newPanel: '新建面板', addHosts: '添加主机', rename: '重命名', delete: '删除面板',
  deleteConfirm: '删除此面板？不会删除主机配置。', confirm: '确认', cancel: '取消', panelName: '面板名称',
  start: '开始监控', stop: '停止监控', interval: '采样间隔', range: '时间范围', sort: '排序', name: '名称', cpu: 'CPU', memory: '内存', disk: '磁盘', network: '网络',
  list: '列表', chart: '图表', search: '搜索主机', notStarted: '未开始', waiting: '等待首次采样', sampling: '正在采样', stopped: '已停止 · 保留最后样本',
  empty: '新建监控面板并选择 SSH 主机后开始监控。', noHosts: '向此面板添加 SSH 主机。', noResults: '没有匹配的主机',
  historyHint: '历史仅保留本次应用运行期间的实际采样（每台主机最多 1 小时 / 3,600 条）。无预存历史，不永久保存。',
  selected: '已选', hosts: '主机', ungrouped: '未分组', unsupported: '仅 SSH 连接支持监控', picker: '选择监控主机',
  selectGroup: '全选分组', clearGroup: '清空分组', removeHost: '从面板移除', unavailable: '不支持 / 暂无数据', missingProfile: '主机配置不存在',
  probeFailed: '探测失败，正在退避重试。请检查网络和 SSH 认证。', load: '负载（1 / 5 / 15 分钟）', uptime: '运行时间', os: '操作系统', kernel: '内核',
  swap: 'Swap', latency: 'SSH 探测耗时（非 ICMP）', cores: 'CPU 逐核心', mounts: '挂载点', interfaces: '网络接口', rx: '接收', tx: '发送',
  total: '总量', used: '已用', diskIo: '磁盘 I/O', read: '读取', write: '写入', system: 'System 系统详情', sshOnly: 'System 详情需要活动 SSH 窗格。',
  lastSample: '最后采样', persistenceError: '面板偏好无法保存到此设备，请检查本地存储后重试。', retry: '重试', minutes: '分钟', seconds: '秒',
  paused: '已按窗口策略暂停', actualRange: '实际采样区间', cpuHistory: 'CPU 历史', memoryHistory: '内存历史', diskHistory: '磁盘容量历史', networkHistory: '网络速率历史',
  editHosts: '选择主机', keepRunning: '离开此页面后，已启动的面板仍会继续采样。',
};
export function monitoringMessage(key: keyof typeof en): string { return i18n.resolvedLocale === 'zh-CN' ? zh[key] : en[key]; }
export function monitoringNumber(value: number | null | undefined, suffix = ''): string {
  return value != null && Number.isFinite(value) ? `${new Intl.NumberFormat(i18n.resolvedLocale, { maximumFractionDigits: 1 }).format(value)}${suffix}` : '—';
}
export function monitoringBytes(value: number | null | undefined, rate = false): string {
  if (value == null || !Number.isFinite(value)) return '—';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB']; let i = 0;
  while (Math.abs(value) >= 1024 && i < units.length - 1) { value /= 1024; i++; }
  return `${monitoringNumber(value)} ${units[i]}${rate ? '/s' : ''}`;
}
