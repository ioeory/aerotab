import { i18n } from '../../lib/i18n.svelte';

/** Hosts-only copy, ready to move into the central dictionary by its owner. */
const labels = {
  all: ['All hosts', '全部主机'],
  grid: ['Grid view', '网格视图'],
  list: ['List view', '列表视图'],
  tree: ['Tree view', '树形视图'],
  groups: ['Groups', '分组'],
  toggleGroups: ['Show / hide groups', '显示／隐藏分组'],
  resources: ['hosts', '台主机'],
  active: ['Session open', '会话已打开'],
} as const;
export function hostLabel(key: keyof typeof labels): string {
  return i18n.t(`hosts.${key}`);
}
