import type { Tab } from './tabs.svelte';
import type { SessionMeta } from './types';

export interface AiPaneDirectoryEntry {
  id: string;
  title: string;
  tabTitle: string;
  tabNumber: number;
  paneNumber: number;
  position?: PanePosition;
  active: boolean;
}

type PanePosition = 'left' | 'right' | 'top' | 'bottom';
type DirectoryLayout =
  | { type: 'leaf'; pane: { id: string } }
  | { type: 'split'; direction: 'row' | 'col'; children: readonly DirectoryLayout[] };

type DirectoryTab = Pick<Tab, 'id' | 'kind' | 'title' | 'activePaneId'> & {
  panes: readonly Pick<SessionMeta, 'id' | 'title' | 'connection'>[];
  layout?: DirectoryLayout;
};

function simpleSplitPositions(layout?: DirectoryLayout): ReadonlyMap<string, PanePosition> {
  if (layout?.type !== 'split' || layout.children.length !== 2) return new Map();
  const [first, second] = layout.children;
  if (first?.type !== 'leaf' || second?.type !== 'leaf') return new Map();
  return layout.direction === 'row'
    ? new Map([[first.pane.id, 'left'], [second.pane.id, 'right']])
    : new Map([[first.pane.id, 'top'], [second.pane.id, 'bottom']]);
}

/** A display-only index. `session.list` remains authoritative for AI bindings. */
export function buildAiPaneDirectory(tabs: readonly DirectoryTab[], activeTabId?: string | null): AiPaneDirectoryEntry[] {
  return tabs.flatMap((tab, tabIndex) => {
    if (tab.kind !== 'terminal') return [];
    const positions = simpleSplitPositions(tab.layout);
    return tab.panes.flatMap((pane, paneIndex) => pane.connection ? [] : [{
      id: pane.id,
      title: pane.title,
      tabTitle: tab.title,
      tabNumber: tabIndex + 1,
      paneNumber: paneIndex + 1,
      position: positions.get(pane.id),
      active: tab.id === activeTabId && pane.id === tab.activePaneId,
    }]);
  });
}

const positionLabels = {
  en: { left: 'Left', right: 'Right', top: 'Top', bottom: 'Bottom' },
  'zh-CN': { left: '左', right: '右', top: '上', bottom: '下' },
};

export function aiPaneLabel(
  session: Pick<SessionMeta, 'id' | 'title'>,
  directory: readonly AiPaneDirectoryEntry[],
  locale: keyof typeof positionLabels = 'en',
): string {
  const pane = directory.find(entry => entry.id === session.id);
  const title = [pane?.title, session.title].map(value => value?.trim())
    .find(value => value && value !== session.id) ?? (locale === 'zh-CN' ? '未命名终端' : 'Untitled terminal');
  if (!pane) return title;
  const tab = pane.tabTitle.trim();
  const number = locale === 'zh-CN' ? `窗格 ${pane.paneNumber}` : `#${pane.paneNumber}`;
  const position = pane.position ? ` (${positionLabels[locale][pane.position]})` : '';
  return `${pane.tabNumber}${tab && tab !== session.id ? `. ${tab}` : ''} / ${number}${position} ${title}`;
}
