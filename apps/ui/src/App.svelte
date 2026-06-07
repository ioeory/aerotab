<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import Sidebar from './components/Sidebar.svelte';
  import TabBar from './components/TabBar.svelte';
  import PaneGrid from './components/PaneGrid.svelte';
  import AppConfirmDialog from './components/AppConfirmDialog.svelte';
  import ProfileModal from './components/ProfileModal.svelte';
  import { appConfirm, appPrompt } from './lib/confirm.svelte';
  import VaultUnlockModal from './components/VaultUnlockModal.svelte';
  import SerialModal from './components/SerialModal.svelte';
  import SftpBrowser from './components/SftpBrowser.svelte';
  import FileTransferWindow from './components/FileTransferWindow.svelte';
  import BatchCommandModal from './components/BatchCommandModal.svelte';
  import SettingsLayout from './components/settings/SettingsLayout.svelte';
  import CommandPalette, { type Action } from './components/CommandPalette.svelte';
  import ProfileSelector, { type PickerItem } from './components/ProfileSelector.svelte';
  import { selectClient, uuidv4 } from './lib/rpc';
  import { tabs, type PaneNode, type SplitDir, type SplitSide, type Tab } from './lib/tabs.svelte';
  import { applyStoredSettingsToUi } from './lib/applyStoredSettings';
  import { applyTheme, BUILTIN_THEMES } from './lib/theme';
  import { applyCustomCss, applyLigatures } from './lib/customCss';
  import { applyWindowSettings, getWindowSettings } from './lib/windowSettings';
  import { settingsCoord } from './lib/settingsStore.svelte';
  import { i18n } from './lib/i18n.svelte';
  import { categoryForStatus, diagnostics, exportDiagnosticPack, instrumentRpcClient } from './lib/diagnostics.svelte';
  import type { HostStats, SessionMeta, SshProfileSpec, StoredProfile } from './lib/types';
  import { hotkeys, shouldDeferToTextInput } from './lib/hotkeys';
  import { dispatchFitAllPanes, dispatchFocusPane } from './lib/focusPane';
  import { isModalOverlayActive } from './lib/modalFocus';
  import { installPaneDragGlobalHandlers, subscribePanePointerDrop } from './lib/paneDrag';
  import { b64encode } from './lib/rpc';
  import { withRpcTimeout } from './lib/rpcTimeout';
  import { closeSessionsInBackground } from './lib/sessionClose';
  import { notifySessionsClosing } from './lib/sessionLifecycle';
  import { queueTabClose, queueTabsClose, type TabCloseSink } from './lib/tabCloseQueue';
  import { broadcastTargetIds } from './lib/broadcast';
  import {
    bootstrapSyncEngine,
    ensureSyncEngineConfigured,
    loadPersistedSyncSettings,
    selectedSyncGroups,
  } from './lib/syncConfig';
  import { bootstrapVault } from './lib/vaultBootstrap';
  import { sshProfileFromSshConfig, type SshConfigEntry } from './lib/sshConfigJump';
  import { PROFILES_CHANGED } from './lib/profileEvents';
  import { profileEndpointLabel } from './lib/profileMeta';
  import { startHorizontalPanelResize } from './lib/panelResize';
  import { FolderOpen, PanelLeftClose, PanelLeftOpen, PanelRightOpen, RefreshCw, X } from '@lucide/svelte';
  import logoUrl from './assets/logo.png';

  const rpc = instrumentRpcClient(selectClient());
  const buildId = '0.2.10-ui-20260526';
  type SettingsSectionId =
    | 'application'
    | 'appearance'
    | 'profiles'
    | 'terminal'
    | 'ai'
    | 'colorscheme'
    | 'configsync'
    | 'hotkeys'
    | 'plugins'
    | 'shell'
    | 'ssh'
    | 'vault'
    | 'window'
    | 'configfile';
  let status = $state(i18n.t('app.status.idle'));
  let coreVersion = $state<string | null>(null);
  let hostStats = $state<HostStats | null>(null);
  let hostStatsStatus = $state<'idle' | 'loading' | 'ok' | 'unavailable'>('idle');
  let hostStatsEnabled = $state(true);
  let hostStatsIntervalSec = $state(30);
  let hostStatsPollHandle: number | null = null;
  let hostStatsSeq = 0;
  let hostStatsUpdatedAt = $state<number | null>(null);

  let profileModal: {
    open: (
      existing?: StoredProfile,
      options?: import('./components/ProfileModal.svelte').ProfileModalOpenOptions,
    ) => void;
  } | null = $state(null);
  let vaultUnlockModal: { open: () => Promise<void> } | null = $state(null);
  let serialModal: { open: () => Promise<void> } | null = $state(null);
  let sidebar: { refresh: () => Promise<void> } | null = $state(null);
  let settingsOpen = $state(false);
  let settingsInitialSection = $state<SettingsSectionId>('appearance');
  let settingsRev = $state(0);
  let paletteOpen = $state(false);
  let batchCommandOpen = $state(false);
  let pickerOpen = $state(false);
  let savedProfiles = $state<StoredProfile[]>([]);
  let sessionWorkspaces = $state<SessionWorkspace[]>([]);
  let workspaceOpening = $state(false);
  const WORKSPACE_SESSION_TIMEOUT_MS = 20_000;
  let sidebarVisible = $state(true);
  const SIDEBAR_WIDTH_MIN = 180;
  const SIDEBAR_WIDTH_MAX = 480;
  const SIDEBAR_WIDTH_DEFAULT = 240;
  let sidebarWidthPx = $state(SIDEBAR_WIDTH_DEFAULT);
  const SFTP_DOCK_WIDTH_MIN = 280;
  const SFTP_DOCK_WIDTH_MAX = 720;
  let sftpDockWidthPx = $state(400);

  interface SftpDockTarget {
    name: string;
    ssh: SshProfileSpec;
    sudo?: boolean;
  }
  interface SftpWindow {
    id: string;
    target: SftpDockTarget;
  }
  const GLOBAL_SFTP_KEY = '__global__';
  /** Tab (or global) id → dock visible. Target follows the tab's active SSH pane. */
  let sftpDockOpen = $state<Record<string, boolean>>({});
  /** Fallback target when dock is open but the active pane is not SSH (e.g. sidebar / global). */
  let sftpDockPinned = $state<Record<string, SftpDockTarget>>({});
  let sftpDockCollapsed = $state<Record<string, boolean>>({});
  let sftpWindows = $state<SftpWindow[]>([]);
  let sftpWindowSeq = 0;
  type WorkspaceView = 'terminal' | 'transfer';
  let workspaceView = $state<WorkspaceView>('terminal');
  let transferWorkspaceMounted = $state(false);
  let transferInitialTarget = $state<SftpDockTarget | null>(null);
  let transferInitialProfileId = $state<string | null>(null);
  let paneSftpTarget = $state<SftpDockTarget | null>(null);
  let paneSftpTargetSeq = 0;
  /** Per-tab broadcast mode: one keystroke → all SSH panes in the tab. */
  let broadcastByTab = $state<Record<string, boolean>>({});

  const activeSftpKey = $derived(tabs.activeId ?? GLOBAL_SFTP_KEY);
  const activeTab = $derived(tabs.tabs.find((t) => t.id === tabs.activeId));
  const activePane = $derived(activeTab ? tabs.activePane(activeTab) : undefined);
  const broadcastOn = $derived(!!(tabs.activeId && broadcastByTab[tabs.activeId]));
  const broadcastTargets = $derived(broadcastTargetIds(activeTab));

  $effect(() => {
    void tabs.revision;
    void activeTab?.activePaneId;
    const tabId = tabs.activeId;
    if (!tabId || !sftpDockOpen[tabId]) {
      paneSftpTarget = null;
      return;
    }
    const pane = activeTab ? tabs.activePane(activeTab) : undefined;
    if (!pane?.profileId || pane.sshProfile) {
      paneSftpTarget = null;
      return;
    }
    const seq = ++paneSftpTargetSeq;
    void rpc.call<StoredProfile>('profile.get', { id: pane.profileId })
      .then((profile) => {
        if (seq !== paneSftpTargetSeq) return;
        if (profile.kind !== 'ssh') {
          paneSftpTarget = null;
          return;
        }
        paneSftpTarget = { name: profile.name, ssh: profile.ssh };
      })
      .catch(() => {
        if (seq === paneSftpTargetSeq) paneSftpTarget = null;
      });
  });

  const currentSftpDock = $derived.by((): SftpDockTarget | null => {
    const key = activeSftpKey;
    if (!sftpDockOpen[key]) return null;
    if (activePane?.sshProfile) {
      return { name: activePane.title || 'SSH session', ssh: activePane.sshProfile };
    }
    if (activePane?.profileId) return paneSftpTarget;
    return sftpDockPinned[key] ?? null;
  });
  const currentSftpCollapsed = $derived(sftpDockCollapsed[activeSftpKey] ?? false);
  const sftpDockSessionId = $derived(
    activePane?.kind === 'ssh' || activePane?.sshProfile ? activePane?.id ?? null : null,
  );

  // ── M9 — session restore ────────────────────────────────────────────────
  // A `Restorable` describes how to re-open a session after a restart. We
  // record one per opened session and persist the list under `openTabs`.
  type Restorable =
    | { kind: 'local' }
    | { kind: 'shell'; command: string; args: string[]; label: string }
    | { kind: 'ssh-profile'; id: string }
    | { kind: 'ssh'; title: string; profile: Record<string, unknown> };
  interface OpenedRestorable {
    session: SessionMeta;
    restore: Restorable;
  }
  type WorkspaceNode =
    | { type: 'leaf'; paneIndex: number }
    | { type: 'split'; direction: SplitDir; ratios: number[]; children: WorkspaceNode[] };
  interface WorkspaceTab {
    title: string;
    customTitle?: string | null;
    layout: WorkspaceNode;
    activePaneIndex: number;
    maximizedPaneIndex?: number | null;
    panes: Restorable[];
    /** `true` = dock open (target follows active SSH pane); legacy snapshots may store a full target. */
    sftpDock?: boolean | SftpDockTarget | null;
    sftpDockCollapsed?: boolean;
  }
  interface SessionWorkspace {
    id: string;
    name: string;
    createdAt: number;
    updatedAt: number;
    tabs: WorkspaceTab[];
  }
  const restoreMap = new Map<string, Restorable>();
  let restoreReady = false; // suppress persistence until first load completes

  let persistOpenTabsTimer: ReturnType<typeof setTimeout> | null = null;

  function recordRestore(sessionId: string, r: Restorable) {
    restoreMap.set(sessionId, r);
    schedulePersistOpenTabs();
  }

  function persistOpenTabsNow() {
    if (!restoreReady) return;
    const out = tabs.tabs
      .map((t) => {
        const head = t.panes[0];
        if (!head) return null;
        const r = restoreMap.get(head.id);
        if (!r) return null;
        return { title: t.title, restore: r };
      })
      .filter(Boolean);
    rpc.call('settings.set', { key: 'openTabs', value: out }).catch(() => { /* ignore */ });
  }

  function schedulePersistOpenTabs() {
    if (!restoreReady) return;
    if (persistOpenTabsTimer) clearTimeout(persistOpenTabsTimer);
    persistOpenTabsTimer = setTimeout(() => {
      persistOpenTabsTimer = null;
      persistOpenTabsNow();
    }, 250);
  }

  function restorableLabel(r: Restorable): string {
    if (r.kind === 'ssh-profile') return `profile:${r.id}`;
    if (r.kind === 'ssh') return r.title;
    if (r.kind === 'shell') return r.label;
    return r.kind;
  }

  async function openRestorableSession(r: Restorable): Promise<OpenedRestorable> {
    if (r.kind === 'local') {
      const meta = await rpc.call<{ id: string; kind: string; title: string }>(
        'session.openLocal', {},
      );
      return { session: { id: meta.id, kind: meta.kind, title: meta.title }, restore: { kind: 'local' } };
    }
    if (r.kind === 'shell') {
      const meta = await rpc.call<{ id: string; kind: string; title: string }>(
        'session.openLocal',
        { title: r.label, shell: r.command, shell_args: r.args },
      );
      return {
        session: { id: meta.id, kind: meta.kind, title: meta.title, shellCommand: r.command, shellArgs: r.args },
        restore: r,
      };
    }
    if (r.kind === 'ssh-profile') {
      const meta = await rpc.call<{ id: string; kind: string; title: string }>(
        'session.openSshProfile', { profile_id: r.id },
      );
      return { session: { id: meta.id, kind: meta.kind, title: meta.title, profileId: r.id }, restore: r };
    }
    const meta = await rpc.call<{ id: string; kind: string; title: string }>(
      'session.openSsh', { title: r.title, profile: r.profile },
    );
    return {
      session: { id: meta.id, kind: meta.kind, title: meta.title, sshProfile: r.profile as unknown as SshProfileSpec },
      restore: r,
    };
  }

  async function openRestorableSessionTimed(r: Restorable): Promise<OpenedRestorable> {
    return withRpcTimeout(
      openRestorableSession(r),
      WORKSPACE_SESSION_TIMEOUT_MS,
      restorableLabel(r),
    );
  }

  async function replayRestorable(r: Restorable) {
    try {
      const opened = await openRestorableSession(r);
      tabs.add(opened.session);
      restoreMap.set(opened.session.id, opened.restore);
    } catch (e) {
      console.warn('restore', r, e);
    }
  }

  function cloneJson<T>(value: T): T {
    return JSON.parse(JSON.stringify(value)) as T;
  }

  function normalizeRatios(ratios: number[], count: number): number[] {
    const values = ratios.slice(0, count).map((value) => (Number.isFinite(value) && value > 0 ? value : 1));
    while (values.length < count) values.push(1);
    const total = values.reduce((sum, value) => sum + value, 0) || count || 1;
    return values.map((value) => value / total);
  }

  function snapshotWorkspaceNode(node: PaneNode, paneIndex: Map<string, number>): WorkspaceNode | null {
    if (node.type === 'leaf') {
      const index = paneIndex.get(node.pane.id);
      return typeof index === 'number' ? { type: 'leaf', paneIndex: index } : null;
    }
    const children: WorkspaceNode[] = [];
    const ratios: number[] = [];
    for (let index = 0; index < node.children.length; index++) {
      const child = node.children[index];
      if (!child) continue;
      const snap = snapshotWorkspaceNode(child, paneIndex);
      if (!snap) continue;
      children.push(snap);
      ratios.push(node.ratios[index] ?? 1);
    }
    if (children.length === 0) return null;
    if (children.length === 1) return children[0] ?? null;
    return { type: 'split', direction: node.direction, ratios: normalizeRatios(ratios, children.length), children };
  }

  function instantiateWorkspaceNode(node: WorkspaceNode, opened: Array<OpenedRestorable | null>): PaneNode | null {
    if (node.type === 'leaf') {
      const item = opened[node.paneIndex];
      return item ? { type: 'leaf', id: item.session.id, pane: item.session } : null;
    }
    const children = node.children
      .map((child) => instantiateWorkspaceNode(child, opened))
      .filter((child): child is PaneNode => !!child);
    if (children.length === 0) return null;
    if (children.length === 1) return children[0] ?? null;
    return {
      type: 'split',
      id: uuidv4(),
      direction: node.direction,
      children,
      ratios: normalizeRatios(node.ratios, children.length),
    };
  }

  function normalizeSessionWorkspaces(value: unknown): SessionWorkspace[] {
    if (!Array.isArray(value)) return [];
    return value.filter((item): item is SessionWorkspace => {
      if (!item || typeof item !== 'object') return false;
      const row = item as Record<string, unknown>;
      return typeof row.id === 'string'
        && typeof row.name === 'string'
        && Array.isArray(row.tabs);
    });
  }

  async function loadSessionWorkspaces() {
    try {
      const result = await rpc.call<{ value: unknown }>('settings.get', { key: 'sessionWorkspaces' });
      sessionWorkspaces = normalizeSessionWorkspaces(result.value);
    } catch {
      sessionWorkspaces = [];
    }
  }

  async function saveSessionWorkspaces(next: SessionWorkspace[]) {
    sessionWorkspaces = next;
    await rpc.call('settings.set', { key: 'sessionWorkspaces', value: next });
  }

  function snapshotCurrentWorkspace(name: string): SessionWorkspace | null {
    const workspaceTabs: WorkspaceTab[] = [];
    for (const tab of tabs.tabs) {
      const paneIndex = new Map<string, number>();
      const panes: Restorable[] = [];
      for (const pane of tab.panes) {
        const restore = restoreMap.get(pane.id);
        if (!restore) continue;
        paneIndex.set(pane.id, panes.length);
        panes.push(cloneJson(restore));
      }
      const layout = snapshotWorkspaceNode(tab.layout, paneIndex);
      if (!layout || panes.length === 0) continue;
      const activePaneIndex = paneIndex.get(tab.activePaneId) ?? 0;
      const maximizedPaneIndex = tab.maximizedPaneId ? paneIndex.get(tab.maximizedPaneId) ?? null : null;
      workspaceTabs.push({
        title: tabs.displayTitle(tab),
        customTitle: tab.customTitle ?? null,
        layout,
        activePaneIndex,
        maximizedPaneIndex,
        panes,
        sftpDock: sftpDockOpen[tab.id] ? true : null,
        sftpDockCollapsed: !!sftpDockCollapsed[tab.id],
      });
    }
    if (workspaceTabs.length === 0) return null;
    const now = Date.now();
    return { id: uuidv4(), name, createdAt: now, updatedAt: now, tabs: workspaceTabs };
  }

  async function saveCurrentSessionWorkspace() {
    if (tabs.tabs.length === 0) {
      onError(i18n.t('workspace.noOpenTabs'));
      return;
    }
    const fallbackName = i18n.t('workspace.defaultName', { count: sessionWorkspaces.length + 1 });
    const name = (await appPrompt(i18n.t('workspace.namePrompt'), { defaultValue: fallbackName }))?.trim();
    if (!name) return;
    const snapshot = snapshotCurrentWorkspace(name);
    if (!snapshot) {
      onError(i18n.t('workspace.emptyNoRestorable'));
      return;
    }
    await saveSessionWorkspaces([snapshot, ...sessionWorkspaces.filter((item) => item.name !== name)]);
    status = i18n.t('workspace.saved', { name });
  }

  async function openSessionWorkspace(workspace: SessionWorkspace) {
    if (workspaceOpening) return;
    workspaceOpening = true;
    status = i18n.t('workspace.opening', { name: workspace.name });
    let openedTabs = 0;
    const nextSftpOpen = { ...sftpDockOpen };
    const nextSftpPinned = { ...sftpDockPinned };
    const nextSftpCollapsed = { ...sftpDockCollapsed };
    try {
      for (const tab of workspace.tabs) {
        const opened = await Promise.all(
          tab.panes.map(async (restore) => {
            try {
              return await openRestorableSessionTimed(restore);
            } catch (e) {
              onError(`workspace ${restorableLabel(restore)}: ${(e as Error).message}`);
              return null;
            }
          }),
        );
        const layout = instantiateWorkspaceNode(tab.layout, opened);
        if (!layout) continue;
        const active = opened[tab.activePaneIndex]?.session.id;
        const maximized = typeof tab.maximizedPaneIndex === 'number'
          ? opened[tab.maximizedPaneIndex]?.session.id ?? null
          : null;
        const created = tabs.addLayout(tab.title, layout, active, maximized);
        if (tab.customTitle?.trim()) {
          tabs.setCustomTitle(created.id, tab.customTitle);
        } else if (tab.title?.trim() && tab.title.trim() !== tabs.autoTitle(created)) {
          tabs.setCustomTitle(created.id, tab.title);
        }
        for (const item of opened) {
          if (item) restoreMap.set(item.session.id, item.restore);
        }
        if (tab.sftpDock) {
          nextSftpOpen[created.id] = true;
          if (typeof tab.sftpDock === 'object' && tab.sftpDock) {
            nextSftpPinned[created.id] = cloneJson(tab.sftpDock);
          }
          nextSftpCollapsed[created.id] = !!tab.sftpDockCollapsed;
        }
        openedTabs += 1;
        await tick();
        await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      }
      if (openedTabs > 0) {
        sftpDockOpen = nextSftpOpen;
        sftpDockPinned = nextSftpPinned;
        sftpDockCollapsed = nextSftpCollapsed;
        schedulePersistOpenTabs();
        status = i18n.t('workspace.opened', { name: workspace.name });
      } else {
        onError(i18n.t('workspace.openFailed', { name: workspace.name }));
      }
    } finally {
      workspaceOpening = false;
    }
  }

  async function deleteSessionWorkspace(workspace: SessionWorkspace) {
    if (!(await appConfirm(i18n.t('workspace.deleteConfirm', { name: workspace.name }), { danger: true, confirmLabel: i18n.t('common.delete') }))) return;
    await saveSessionWorkspaces(sessionWorkspaces.filter((item) => item.id !== workspace.id));
    status = i18n.t('workspace.deleted', { name: workspace.name });
  }

  async function exportDiagnosticsFromPalette() {
    try {
      const result = await exportDiagnosticPack(buildId, coreVersion);
      if (result !== 'cancelled') status = i18n.t('application.diagnostics.exported');
    } catch (e) {
      onError(`diagnostics: ${(e as Error).message}`);
    }
  }

  async function showSyncStatusFromPalette() {
    try {
      const s = await rpc.call<{
        configured: boolean;
        kind: string | null;
        lastSyncMs: number | null;
        autoIntervalMs: number | null;
      }>('sync.status', {});
      if (!s.configured) {
        status = i18n.t('sync.notConfigured');
        return;
      }
      status = i18n.t('sync.statusLine', {
        backend: s.kind ?? '?',
        last: s.lastSyncMs ? new Date(s.lastSyncMs).toLocaleString() : i18n.t('sync.never'),
      });
    } catch (e) {
      onError(`sync status: ${(e as Error).message}`);
    }
  }

  /** Reload profiles, theme, hotkeys, etc. after sync or settings save. */
  async function refreshAppFromSettingsStore() {
    settingsRev += 1;
    sidebar?.refresh();
    await applyStoredSettingsToUi(rpc);
  }

  async function syncNowFromPalette() {
    try {
      await ensureSyncEngineConfigured(rpc);
      const settings = await loadPersistedSyncSettings(rpc);
      const groups = selectedSyncGroups(settings);
      if (groups.length === 0) {
        status = i18n.t('sync.noGroups');
        return;
      }
      status = i18n.t('sync.syncing');
      const stats = await rpc.call<Record<string, unknown>>('sync.now', { groups });
      await refreshAppFromSettingsStore();
      status = i18n.t('sync.complete', { count: Object.keys(stats).filter((k) => k !== '_bridge').length });
    } catch (e) {
      onError(`sync now: ${(e as Error).message}`);
    }
  }


  onMount(async () => {
    await i18n.load(rpc);
    await loadSftpDockWidth();
    await loadSessionWorkspaces();
    status = i18n.t('app.status.idle');
    try {
      const v = await rpc.call<{ version: string }>('core.version');
      coreVersion = v.version;
      status = i18n.t('app.status.connectedCore', { version: v.version });
    } catch (e) {
      status = i18n.t('app.status.coreUnreachable');
      console.error(e);
    }
    // Pull the persisted theme (if any) and apply before first paint of panes.
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'theme' });
      const name = typeof r.value === 'string' ? r.value : 'tokyo-night';
      const theme = BUILTIN_THEMES.find((t) => t.name === name) ?? BUILTIN_THEMES[0];
      if (theme) applyTheme(theme);
    } catch {
      /* settings store may not be available — keep CSS defaults. */
    }
    // Apply user custom CSS + ligature preference.
    try {
      const a = await rpc.call<{ value: unknown }>('settings.get', { key: 'appearance' });
      if (a.value && typeof a.value === 'object') {
        const v = a.value as Record<string, unknown>;
        if (typeof v.customCss === 'string') applyCustomCss(v.customCss);
        if (typeof v.ligatures === 'boolean') applyLigatures(v.ligatures);
      }
    } catch {
      /* ignore — appearance not configured yet. */
    }
    // Apply persisted window-chrome settings (opacity / spaciness / tabs
    // location / frame style) so the UI matches the saved config on launch.
    try {
      const w = await rpc.call<{ value: unknown }>('settings.get', { key: 'window' });
      if (w.value && typeof w.value === 'object') {
        const value = w.value as Record<string, unknown>;
        if (typeof value.sidebarVisible === 'boolean') sidebarVisible = value.sidebarVisible;
        if (typeof value.sidebarWidthPx === 'number') {
          sidebarWidthPx = Math.max(
            SIDEBAR_WIDTH_MIN,
            Math.min(SIDEBAR_WIDTH_MAX, value.sidebarWidthPx),
          );
        }
        applyWindowSettings(value);
      }
    } catch { /* not configured yet */ }
    await loadHostStatsSettings();
    void (async () => {
      try {
        const boot = await bootstrapSyncEngine(rpc);
        if (boot === 'no_keyring_secret' || boot === 'no_git_https_secret') {
          console.warn('sync bootstrap: credentials missing — open Config sync to re-key');
        }
      } catch (e) {
        console.warn('sync bootstrap:', e);
      }
    })();
    try {
      const vaultBoot = await bootstrapVault(rpc);
      if (vaultBoot === 'needs_password') {
        await vaultUnlockModal?.open();
      }
    } catch (e) {
      console.warn('vault bootstrap:', e);
    }
    // Startup behaviour: restore previously-open tabs (M9) and/or auto-open
    // a fresh local terminal. Both default to ON when not configured so a
    // fresh install gives the user a working terminal on launch and remembers
    // sessions across restarts. Set the flags to `false` explicitly to opt
    // out from the Terminal settings page.
    try {
      const t = await rpc.call<{ value: unknown }>('settings.get', { key: 'terminal' });
      const v = (t.value ?? {}) as Record<string, unknown>;
      let restored = 0;
      const wantRestore = v.restoreTabs !== false;
      const wantAutoOpen = v.autoOpenTerminal !== false;
      if (wantRestore) {
        try {
          const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'openTabs' });
          const list = Array.isArray(r.value) ? (r.value as Array<{ restore: Restorable }>) : [];
          for (const e of list) {
            if (!e || !e.restore) continue;
            await replayRestorable(e.restore);
            restored++;
          }
        } catch { /* nothing saved yet */ }
      }
      if (wantAutoOpen && tabs.tabs.length === 0 && restored === 0) {
        void openLocal();
      }
    } catch {
      /* ignore */
    }
    restoreReady = true;
  });

  function onError(msg: string) {
    status = msg;
    diagnostics.record(categoryForStatus(msg), 'status', msg, 'error');
  }

  function focusActivePane() {
    const tab = tabs.tabs.find((candidate) => candidate.id === tabs.activeId);
    const pane = tab ? tabs.activePane(tab) : undefined;
    if (pane) dispatchFocusPane(pane.id);
  }

  function openSettings(section: SettingsSectionId = 'appearance') {
    settingsInitialSection = section;
    settingsOpen = true;
  }

  function closeSettings() {
    settingsOpen = false;
    requestAnimationFrame(() => focusActivePane());
  }

  function toggleBroadcast() {
    const tabId = tabs.activeId;
    if (!tabId) return;
    broadcastByTab = { ...broadcastByTab, [tabId]: !broadcastByTab[tabId] };
  }

  function profileCommandSubtitle(p: StoredProfile): string {
    if (p.kind === 'rdp') return `${p.rdp.host}:${p.rdp.port} RDP`;
    if (p.kind === 'vnc') return `${p.vnc.host}:${p.vnc.port} VNC`;
    const pieces = [p.ssh.user ? `${p.ssh.user}@${p.ssh.host}` : p.ssh.host];
    if (p.group) pieces.push(`group:${p.group}`);
    if (p.tags?.length) pieces.push(p.tags.map((tag) => `tag:${tag}`).join(' '));
    if (p.favorite) pieces.push('favorite');
    return pieces.join(' · ');
  }

  function profileCommandKeywords(p: StoredProfile): string[] {
    const host =
      p.kind === 'ssh' ? p.ssh.host : p.kind === 'rdp' ? p.rdp.host : p.vnc.host;
    const user = p.kind === 'ssh' ? p.ssh.user ?? '' : '';
    return [
      p.name,
      p.group ?? '',
      ...(p.tags ?? []),
      host,
      user,
      p.kind,
      p.favorite ? 'favorite starred pinned' : '',
    ].filter(Boolean);
  }

  async function loadHostStatsSettings() {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'ssh' });
      if (r.value && typeof r.value === 'object') {
        const v = r.value as Record<string, unknown>;
        hostStatsEnabled = typeof v.hostStatsEnabled === 'boolean' ? v.hostStatsEnabled : true;
        if (typeof v.hostStatsIntervalSec === 'number') {
          hostStatsIntervalSec = Math.max(10, Math.min(3600, v.hostStatsIntervalSec));
        }
      }
    } catch {
      hostStatsEnabled = true;
      hostStatsIntervalSec = 30;
    }
  }

  function currentActivePane(): SessionMeta | undefined {
    const tab = tabs.tabs.find((t) => t.id === tabs.activeId);
    return tab ? tabs.activePane(tab) : undefined;
  }

  function activeHostStatsKey(): string | null {
    const pane = currentActivePane();
    if (!pane) return null;
    if (pane.sshProfile) return `ssh:${pane.sshProfile.user}@${pane.sshProfile.host}:${pane.sshProfile.port}`;
    if (pane.profileId) return `profile:${pane.profileId}`;
    return null;
  }

  async function resolveHostStatsTarget(): Promise<SshProfileSpec | null> {
    const pane = currentActivePane();
    if (!pane) return null;
    if (pane.sshProfile) return pane.sshProfile;
    if (pane.profileId) {
      const profile = await rpc.call<StoredProfile>('profile.get', { id: pane.profileId });
      return profile.kind === 'ssh' ? profile.ssh : null;
    }
    return null;
  }

  function clearHostStatsPoll() {
    if (hostStatsPollHandle != null) {
      window.clearInterval(hostStatsPollHandle);
      hostStatsPollHandle = null;
    }
  }

  async function refreshHostStats() {
    const seq = ++hostStatsSeq;
    if (!hostStatsEnabled) return;
    try {
      const profile = await resolveHostStatsTarget();
      if (seq !== hostStatsSeq) return;
      if (!profile) {
        hostStats = null;
        hostStatsStatus = 'idle';
        return;
      }
      if (!hostStats) hostStatsStatus = 'loading';
      const stats = await rpc.call<HostStats>('ssh.hostStats', { profile });
      if (seq !== hostStatsSeq) return;
      hostStats = stats;
      hostStatsStatus = 'ok';
      hostStatsUpdatedAt = Date.now();
    } catch {
      if (seq !== hostStatsSeq) return;
      hostStats = null;
      hostStatsStatus = 'unavailable';
    }
  }

  function formatHostStatsUpdated(ts: number | null): string {
    if (!ts) return '';
    return new Date(ts).toLocaleTimeString();
  }

  function formatPercent(value: number | null | undefined): string {
    return typeof value === 'number' && Number.isFinite(value) ? `${value.toFixed(0)}%` : '—';
  }

  function formatUptime(seconds: number | null | undefined): string {
    if (typeof seconds !== 'number' || !Number.isFinite(seconds) || seconds < 0) return '—';
    const days = Math.floor(seconds / 86400);
    const hours = Math.floor((seconds % 86400) / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    if (days > 0) return `${days}d ${hours}h`;
    if (hours > 0) return `${hours}h ${minutes}m`;
    return `${minutes}m`;
  }

  function formatHostStats(stats: HostStats): string {
    const parts: string[] = [];
    if (typeof stats.cpu_percent === 'number') parts.push(`CPU ${formatPercent(stats.cpu_percent)}`);
    if (typeof stats.mem_percent === 'number') parts.push(`Mem ${formatPercent(stats.mem_percent)}`);
    if (typeof stats.disk_percent === 'number') parts.push(`Disk ${formatPercent(stats.disk_percent)}`);
    if (typeof stats.uptime_seconds === 'number') parts.push(`Up ${formatUptime(stats.uptime_seconds)}`);
    return parts.join(' · ') || stats.hostname || i18n.t('app.footer.statsLoading');
  }

  function hostStatsTitle(stats: HostStats): string {
    const bits = [stats.hostname, stats.kernel, typeof stats.load1 === 'number' ? `load ${stats.load1.toFixed(2)}` : null]
      .filter(Boolean);
    return bits.join(' · ') || i18n.t('app.footer.hostStatsUnavailable');
  }

  async function openLocal(): Promise<string | null> {
    try {
      // Honour the "default shell" chosen on the Shell settings page if set.
      let extra: Record<string, unknown> = {};
      try {
        const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'defaultShell' });
        if (typeof r.value === 'string' && r.value) {
          const d = await rpc.call<{ shells: Array<{ id: string; command: string; args: string[]; label: string }> }>(
            'profile.discover',
          );
          const hit = d.shells.find((s) => s.id === r.value);
          if (hit) extra = { title: hit.label, shell: hit.command, shell_args: hit.args };
        }
      } catch { /* fall back to backend default */ }
      const meta = await rpc.call<{ id: string; kind: string; title: string }>(
        'session.openLocal', extra,
      );
      const session: SessionMeta = { id: meta.id, kind: meta.kind, title: meta.title };
      if (typeof extra.shell === 'string') {
        session.shellCommand = extra.shell;
        session.shellArgs = Array.isArray(extra.shell_args) ? extra.shell_args as string[] : [];
      }
      tabs.add(session);
      recordRestore(meta.id, { kind: 'local' });
      return meta.id;
    } catch (e) {
      onError(`open local: ${(e as Error).message}`);
      return null;
    }
  }

  async function splitActive(direction: 'row' | 'col', side: SplitSide = 'after') {
    const tab = tabs.tabs.find((t) => t.id === tabs.activeId);
    if (!tab) return openLocal();
    const activePane = tabs.activePane(tab);
    try {
      let session: SessionMeta;
      let restore: Restorable;
      if (activePane?.profileId) {
        const meta = await rpc.call<{ id: string; kind: string; title: string }>(
          'session.openSshProfile', { profile_id: activePane.profileId },
        );
        session = { id: meta.id, kind: meta.kind, title: meta.title, profileId: activePane.profileId, sshProfile: activePane.sshProfile };
        restore = { kind: 'ssh-profile', id: activePane.profileId };
      } else if (activePane?.sshProfile) {
        const meta = await rpc.call<{ id: string; kind: string; title: string }>(
          'session.openSsh', { title: activePane.title, profile: activePane.sshProfile },
        );
        session = { id: meta.id, kind: meta.kind, title: meta.title, sshProfile: activePane.sshProfile };
        restore = { kind: 'ssh', title: activePane.title, profile: activePane.sshProfile as unknown as Record<string, unknown> };
      } else if (activePane?.shellCommand) {
        const meta = await rpc.call<{ id: string; kind: string; title: string }>(
          'session.openLocal', { title: activePane.title, shell: activePane.shellCommand, shell_args: activePane.shellArgs ?? [] },
        );
        session = { id: meta.id, kind: meta.kind, title: meta.title, shellCommand: activePane.shellCommand, shellArgs: activePane.shellArgs ?? [] };
        restore = { kind: 'shell', command: activePane.shellCommand, args: activePane.shellArgs ?? [], label: activePane.title };
      } else {
        const meta = await rpc.call<{ id: string; kind: string; title: string }>(
          'session.openLocal', {},
        );
        session = { id: meta.id, kind: meta.kind, title: meta.title };
        restore = { kind: 'local' };
      }
      tabs.addPane(tab.id, session, direction, side);
      recordRestore(session.id, restore);
    } catch (e) { onError(`split: ${(e as Error).message}`); }
  }

  function cycleTab(delta: number) {
    if (tabs.tabs.length === 0) return;
    const i = tabs.tabs.findIndex((t) => t.id === tabs.activeId);
    const next = tabs.tabs[(i + delta + tabs.tabs.length) % tabs.tabs.length];
    if (next) {
      tabs.activate(next.id);
      requestAnimationFrame(() => focusActivePane());
    }
  }

  function cyclePane(delta: number) {
    const tab = tabs.tabs.find((t) => t.id === tabs.activeId);
    if (!tab) return;
    const i = tab.panes.findIndex((p) => p.id === tab.activePaneId);
    const next = tab.panes[(i + delta + tab.panes.length) % tab.panes.length];
    if (next) {
      tabs.focusPane(tab.id, next.id);
      requestAnimationFrame(() => dispatchFocusPane(next.id));
    }
  }

  function focusPaneDirection(direction: 'left' | 'right' | 'up' | 'down') {
    const tab = tabs.tabs.find((t) => t.id === tabs.activeId);
    if (!tab) return;
    tabs.focusDirectional(tab.id, direction);
    requestAnimationFrame(() => focusActivePane());
  }

  function closeActivePane() {
    const tab = tabs.tabs.find((t) => t.id === tabs.activeId);
    if (!tab) return;
    const sid = tab.activePaneId;
    const r = tabs.removePane(tab.id, sid);
    if (!r) return;
    notifySessionsClosing([sid]);
    restoreMap.delete(sid);
    closeSessionsInBackground(rpc, [sid]);
  }

  function toggleActivePaneMaximize() {
    const tab = tabs.tabs.find((t) => t.id === tabs.activeId);
    if (!tab) return;
    tabs.toggleMaximize(tab.id, tab.activePaneId);
    dispatchFitAllPanes(tab.panes.map((p) => p.id));
  }

  function clearSftpDockForTabIds(tabIds: Iterable<string>) {
    const drop = new Set(tabIds);
    if (drop.size === 0) return;
    sftpDockOpen = Object.fromEntries(Object.entries(sftpDockOpen).filter(([k]) => !drop.has(k)));
    sftpDockPinned = Object.fromEntries(Object.entries(sftpDockPinned).filter(([k]) => !drop.has(k)));
    sftpDockCollapsed = Object.fromEntries(Object.entries(sftpDockCollapsed).filter(([k]) => !drop.has(k)));
  }

  function clearRestoreForSessionIds(sessionIds: Iterable<string>) {
    for (const id of sessionIds) restoreMap.delete(id);
  }

  const tabCloseSink: TabCloseSink = {
    removeTabIds: (tabIds) => {
      tabs.removeMany(tabIds);
    },
    clearSftpTabIds: clearSftpDockForTabIds,
    clearRestoreSessionIds: clearRestoreForSessionIds,
    closeSessions: (sessionIds) => {
      closeSessionsInBackground(rpc, sessionIds);
    },
  };

  function closeTabSessions(tab: Tab) {
    if (!tabs.tabs.some((t) => t.id === tab.id)) return;
    queueTabClose(tab, tabCloseSink);
  }

  function closeTabsBatch(tabList: Tab[]) {
    queueTabsClose(tabList, tabCloseSink);
  }

  function closeOtherTabs(keepId: string) {
    closeTabsBatch(tabs.tabs.filter((t) => t.id !== keepId));
  }

  function closeTabsToRight(fromIndex: number) {
    closeTabsBatch(tabs.tabs.slice(fromIndex + 1));
  }

  function closeAllTabs() {
    closeTabsBatch([...tabs.tabs]);
  }

  async function duplicateTab(source: Tab) {
    const paneIndex = new Map<string, number>();
    const panes: Restorable[] = [];
    for (const pane of source.panes) {
      const restore = restoreMap.get(pane.id);
      if (!restore) {
        onError(i18n.t('tabbar.duplicateFailed'));
        return;
      }
      paneIndex.set(pane.id, panes.length);
      panes.push(cloneJson(restore));
    }
    const layoutSnap = snapshotWorkspaceNode(source.layout, paneIndex);
    if (!layoutSnap) {
      onError(i18n.t('tabbar.duplicateFailed'));
      return;
    }
    const opened: Array<OpenedRestorable | null> = [];
    const dupOpened = await Promise.all(
      panes.map(async (restore) => {
        try {
          return await openRestorableSessionTimed(restore);
        } catch (e) {
          onError(`duplicate ${restorableLabel(restore)}: ${(e as Error).message}`);
          return null;
        }
      }),
    );
    opened.push(...dupOpened);
    const layout = instantiateWorkspaceNode(layoutSnap, opened);
    if (!layout) return;
    const activeIdx = paneIndex.get(source.activePaneId) ?? 0;
    const active = opened[activeIdx]?.session.id;
    const maxIdx = source.maximizedPaneId ? paneIndex.get(source.maximizedPaneId) : undefined;
    const maximized = typeof maxIdx === 'number' ? opened[maxIdx]?.session.id ?? null : null;
    const baseName = tabs.displayTitle(source);
    const created = tabs.addLayout(`${baseName} (copy)`, layout, active, maximized);
    if (source.customTitle?.trim()) {
      tabs.setCustomTitle(created.id, `${source.customTitle} (copy)`);
    }
    for (const item of opened) {
      if (item) restoreMap.set(item.session.id, item.restore);
    }
    if (sftpDockOpen[source.id]) {
      sftpDockOpen = { ...sftpDockOpen, [created.id]: true };
      if (sftpDockCollapsed[source.id]) {
        sftpDockCollapsed = { ...sftpDockCollapsed, [created.id]: true };
      }
    }
    tabs.activate(created.id);
    requestAnimationFrame(() => focusActivePane());
  }

  async function loadSftpDockWidth() {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'sftp' });
      if (r.value && typeof r.value === 'object') {
        const v = r.value as Record<string, unknown>;
        if (typeof v.dockWidthPx === 'number') {
          sftpDockWidthPx = Math.max(SFTP_DOCK_WIDTH_MIN, Math.min(SFTP_DOCK_WIDTH_MAX, v.dockWidthPx));
        }
      }
    } catch { /* optional */ }
  }

  async function persistSidebarWidth() {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'window' });
      const current = r.value && typeof r.value === 'object' ? (r.value as Record<string, unknown>) : {};
      await rpc.call('settings.set', {
        key: 'window',
        value: { ...current, sidebarWidthPx: sidebarWidthPx },
      });
    } catch (e) {
      onError(`window settings: ${(e as Error).message}`);
    }
  }

  function onSidebarResizePointerDown(ev: PointerEvent) {
    startHorizontalPanelResize(ev, {
      startWidthPx: sidebarWidthPx,
      minPx: SIDEBAR_WIDTH_MIN,
      maxPx: SIDEBAR_WIDTH_MAX,
      growWhenDraggingRight: true,
      onWidth: (w) => { sidebarWidthPx = w; },
      onEnd: () => { void persistSidebarWidth(); },
    });
  }

  async function persistSftpDockWidth() {
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'sftp' });
      const current = r.value && typeof r.value === 'object' ? r.value as Record<string, unknown> : {};
      await rpc.call('settings.set', {
        key: 'sftp',
        value: { ...current, dockWidthPx: sftpDockWidthPx },
      });
    } catch (e) {
      onError(`sftp settings: ${(e as Error).message}`);
    }
  }

  function onSftpDockResizePointerDown(ev: PointerEvent) {
    startHorizontalPanelResize(ev, {
      startWidthPx: sftpDockWidthPx,
      minPx: SFTP_DOCK_WIDTH_MIN,
      maxPx: SFTP_DOCK_WIDTH_MAX,
      growWhenDraggingRight: false,
      onWidth: (w) => { sftpDockWidthPx = w; },
      onEnd: () => { void persistSftpDockWidth(); },
    });
  }

  function openSftpDock(target: SftpDockTarget, tabId = tabs.activeId ?? GLOBAL_SFTP_KEY) {
    sftpDockOpen = { ...sftpDockOpen, [tabId]: true };
    sftpDockCollapsed = { ...sftpDockCollapsed, [tabId]: false };
    const tab = tabs.tabs.find((t) => t.id === tabId);
    const pane = tab ? tabs.activePane(tab) : undefined;
    const sshPane = pane && (pane.sshProfile || pane.profileId);
    if (!sshPane) {
      sftpDockPinned = { ...sftpDockPinned, [tabId]: target };
    } else {
      const { [tabId]: _drop, ...rest } = sftpDockPinned;
      sftpDockPinned = rest;
    }
  }

  function openSftpWindow(target: SftpDockTarget) {
    sftpWindowSeq += 1;
    sftpWindows = [
      ...sftpWindows,
      { id: `sftp-window-${Date.now()}-${sftpWindowSeq}`, target },
    ];
  }

  function closeSftpWindow(id: string) {
    sftpWindows = sftpWindows.filter((window) => window.id !== id);
  }

  function showTerminalWorkspace() {
    workspaceView = 'terminal';
    requestAnimationFrame(() => focusActivePane());
  }

  function openFileTransferWindow(initialTarget: SftpDockTarget | null = null, initialProfileId: string | null = null) {
    transferInitialTarget = initialTarget;
    transferInitialProfileId = initialProfileId;
    transferWorkspaceMounted = true;
    workspaceView = 'transfer';
  }

  function closeSftpDock(tabId = activeSftpKey) {
    const { [tabId]: _open, ...restOpen } = sftpDockOpen;
    const { [tabId]: _pin, ...restPinned } = sftpDockPinned;
    const { [tabId]: _dropCollapsed, ...restCollapsed } = sftpDockCollapsed;
    sftpDockOpen = restOpen;
    sftpDockPinned = restPinned;
    sftpDockCollapsed = restCollapsed;
  }

  function setCurrentSftpCollapsed(collapsed: boolean) {
    if (!currentSftpDock) return;
    sftpDockCollapsed = { ...sftpDockCollapsed, [activeSftpKey]: collapsed };
  }

  function toggleCurrentSftpDock() {
    if (!currentSftpDock) {
      void openSftpForActivePane();
      return;
    }
    setCurrentSftpCollapsed(!currentSftpCollapsed);
  }

  async function activeSftpTarget(): Promise<{ target: SftpDockTarget; tabId: string } | null> {
    const tab = tabs.tabs.find((t) => t.id === tabs.activeId);
    const pane = tab ? tabs.activePane(tab) : undefined;
    if (!tab || !pane) {
      onError('sftp: no active SSH pane');
      return null;
    }
    if (pane.sshProfile) {
      return { target: { name: pane.title || 'SSH session', ssh: pane.sshProfile }, tabId: tab.id };
    }
    if (pane.profileId) {
      try {
        const profile = await rpc.call<StoredProfile>('profile.get', { id: pane.profileId });
        if (profile.kind !== 'ssh') {
          onError('sftp: profile is not SSH');
          return null;
        }
        return { target: { name: profile.name, ssh: profile.ssh }, tabId: tab.id };
      } catch (e) {
        onError(`sftp profile: ${(e as Error).message}`);
      }
      return null;
    }
    onError('sftp: active pane is not an SSH session');
    return null;
  }

  async function openSftpForActivePane() {
    const resolved = await activeSftpTarget();
    if (resolved) openSftpDock(resolved.target, resolved.tabId);
  }

  async function openSftpWindowForActivePane() {
    const resolved = await activeSftpTarget();
    if (resolved) openSftpWindow(resolved.target);
  }

  async function setSidebarVisible(visible: boolean, persist = true) {
    sidebarVisible = visible;
    applyWindowSettings({ sidebarVisible: visible });
    if (!persist) return;
    try {
      const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'window' });
      const current = r.value && typeof r.value === 'object' ? r.value as Record<string, unknown> : {};
      await rpc.call('settings.set', { key: 'window', value: { ...current, sidebarVisible: visible } });
    } catch (e) {
      onError(`sidebar: ${(e as Error).message}`);
    }
  }

  function syncSidebarFromWindowSettings() {
    const next = getWindowSettings().sidebarVisible;
    if (typeof next === 'boolean') sidebarVisible = next;
  }

  async function onAppSettingsChanged() {
    syncSidebarFromWindowSettings();
    await i18n.load(rpc);
    status = coreVersion
      ? i18n.t('app.status.connectedCore', { version: coreVersion })
      : i18n.t('app.status.idle');
    void loadHostStatsSettings();
  }

  function sshSpecEndpointLabel(ssh: SshProfileSpec): string {
    const port = ssh.port === 22 ? '' : `:${ssh.port}`;
    return `${ssh.user}@${ssh.host}${port}`;
  }

  async function connectProfile(p: StoredProfile) {
    try {
      if (p.kind === 'rdp' || p.kind === 'vnc') {
        await rpc.call('remote.openProfile', { profile_id: p.id });
        return;
      }
      const meta = await rpc.call<{ id: string; kind: string; title: string }>(
        'session.openSshProfile', { profile_id: p.id },
      );
      tabs.add({ id: meta.id, kind: meta.kind, title: meta.title, profileId: p.id, sshProfile: p.ssh });
      recordRestore(meta.id, { kind: 'ssh-profile', id: p.id });
    } catch (e) {
      onError(`connect ${p.name} (${profileEndpointLabel(p)}): ${(e as Error).message}`);
    }
  }

  // M2 — open a chosen profile-picker item (user profile, built-in shell,
  // ~/.ssh/config entry, or quick-connect address).
  async function onPickerItem(item: PickerItem) {
    pickerOpen = false;
    if (item.kind === 'profile') {
      await connectProfile(item.profile);
      return;
    }
    if (item.kind === 'shell') {
      try {
        const meta = await rpc.call<{ id: string; kind: string; title: string }>(
          'session.openLocal',
          { title: item.shell.label, shell: item.shell.command, shell_args: item.shell.args },
        );
        tabs.add({ id: meta.id, kind: meta.kind, title: meta.title, shellCommand: item.shell.command, shellArgs: item.shell.args });
        recordRestore(meta.id, {
          kind: 'shell',
          command: item.shell.command,
          args: item.shell.args,
          label: item.shell.label,
        });
      } catch (e) { onError(`shell: ${(e as Error).message}`); }
      return;
    }
    if (item.kind === 'ssh-config') {
      let catalog: SshConfigEntry[] = [];
      try {
        const d = await rpc.call<{ sshConfig: SshConfigEntry[] }>('profile.discover');
        catalog = d.sshConfig ?? [];
      } catch { catalog = []; }
      const entry: SshConfigEntry = {
        ...item.entry,
        proxy_jump: item.entry.proxy_jump ?? [],
      };
      const sshProfile = sshProfileFromSshConfig(entry, catalog);
      try {
        const meta = await rpc.call<{ id: string; kind: string; title: string }>(
          'session.openSsh',
          { title: item.entry.alias, profile: sshProfile },
        );
        tabs.add({ id: meta.id, kind: meta.kind, title: meta.title, sshProfile });
        recordRestore(meta.id, { kind: 'ssh', title: item.entry.alias, profile: sshProfile as unknown as Record<string, unknown> });
      } catch (e) { onError(`ssh-config ${item.entry.alias} (${sshSpecEndpointLabel(sshProfile)}): ${(e as Error).message}`); }
      return;
    }
    if (item.kind === 'address') {
      // Quick-connect: parse `[user@]host[:port]`.
      const m = /^(?:([^@\s]+)@)?([^:\s]+)(?::(\d+))?$/.exec(item.address.trim());
      if (!m) { onError(`bad address: ${item.address}`); return; }
      const user = m[1];
      const host = m[2] ?? '';
      const port = m[3] ? Number(m[3]) : 22;
      if (!host) { onError(`bad address: ${item.address}`); return; }
      const sshProfile: SshProfileSpec = { host, port, user: user ?? 'root', auth: 'Agent', jump_via: [] };
      try {
        const meta = await rpc.call<{ id: string; kind: string; title: string }>(
          'session.openSsh',
          { title: host, profile: sshProfile },
        );
        tabs.add({ id: meta.id, kind: meta.kind, title: meta.title, sshProfile });
        recordRestore(meta.id, { kind: 'ssh', title: host, profile: sshProfile as unknown as Record<string, unknown> });
      } catch (e) { onError(`quick-connect ${sshSpecEndpointLabel(sshProfile)}: ${(e as Error).message}`); }
    }
  }


  function buildActions(): Action[] {
    const acts: Action[] = [
      { id: 'new-tab', title: i18n.t('action.newLocalTab'), shortcut: 'Ctrl+Shift+T', run: () => openLocal() },
      { id: 'split-right', title: i18n.t('action.splitRight'), shortcut: 'Ctrl+Shift+D', run: () => splitActive('row') },
      { id: 'split-left', title: i18n.t('action.splitLeft'), shortcut: 'Ctrl+Shift+A', run: () => splitActive('row', 'before') },
      { id: 'split-down', title: i18n.t('action.splitDown'), shortcut: 'Ctrl+Shift+E', run: () => splitActive('col') },
      { id: 'split-up', title: i18n.t('action.splitUp'), shortcut: 'Ctrl+Shift+W', run: () => splitActive('col', 'before') },
      { id: 'maximize-pane', title: i18n.t('action.maximizePane'), shortcut: 'Alt+Z', run: () => toggleActivePaneMaximize() },
      { id: 'close-pane', title: i18n.t('action.closePane'), shortcut: 'Ctrl+W', run: () => closeActivePane() },
      { id: 'next-tab', title: i18n.t('action.nextTab'), shortcut: 'Ctrl+Tab', run: () => cycleTab(1) },
      { id: 'prev-tab', title: i18n.t('action.previousTab'), shortcut: 'Ctrl+Shift+Tab', run: () => cycleTab(-1) },
      { id: 'focus-left', title: i18n.t('action.focusPaneLeft'), shortcut: 'Alt+←', run: () => focusPaneDirection('left') },
      { id: 'focus-right', title: i18n.t('action.focusPaneRight'), shortcut: 'Alt+→', run: () => focusPaneDirection('right') },
      { id: 'focus-up', title: i18n.t('action.focusPaneUp'), shortcut: 'Alt+↑', run: () => focusPaneDirection('up') },
      { id: 'focus-down', title: i18n.t('action.focusPaneDown'), shortcut: 'Alt+↓', run: () => focusPaneDirection('down') },
      { id: 'next-pane', title: i18n.t('action.nextPane'), shortcut: 'Alt+]', run: () => cyclePane(1) },
      { id: 'prev-pane', title: i18n.t('action.previousPane'), shortcut: 'Alt+[', run: () => cyclePane(-1) },
      { id: 'settings', title: i18n.t('action.openSettings'), shortcut: 'Ctrl+,', run: () => openSettings() },
      { id: 'profile-health', title: i18n.t('action.profileHealthCheck'), subtitle: i18n.t('settings.nav.profiles'), run: () => openSettings('profiles') },
      { id: 'sync-status', title: i18n.t('action.syncStatus'), subtitle: i18n.t('settings.nav.configSync'), run: () => showSyncStatusFromPalette() },
      { id: 'sync-now', title: i18n.t('action.syncNow'), subtitle: i18n.t('settings.nav.configSync'), run: () => syncNowFromPalette() },
      { id: 'sync-settings', title: i18n.t('action.openSyncSettings'), subtitle: i18n.t('settings.nav.configSync'), run: () => openSettings('configsync') },
      { id: 'workspace-save', title: i18n.t('action.saveSessionWorkspace'), run: () => saveCurrentSessionWorkspace() },
      { id: 'diagnostics-export', title: i18n.t('application.exportDiagnostics'), subtitle: i18n.t('application.diagnostics'), run: () => exportDiagnosticsFromPalette() },
      { id: 'toggle-sidebar', title: sidebarVisible ? i18n.t('action.hideSidebar') : i18n.t('action.showSidebar'), shortcut: 'Ctrl+Alt+S', run: () => { void setSidebarVisible(!sidebarVisible); } },
      { id: 'new-profile', title: i18n.t('action.newSshProfile'), run: () => profileModal?.open() },
      { id: 'new-serial', title: i18n.t('action.newSerialConnection'), run: () => serialModal?.open() },
    ];
    for (const workspace of sessionWorkspaces) {
      acts.push({
        id: `workspace-open-${workspace.id}`,
        title: i18n.t('action.openSessionWorkspace', { name: workspace.name }),
        subtitle: i18n.t('workspace.tabsSummary', { count: workspace.tabs.length, suffix: workspace.tabs.length === 1 ? '' : 's' }),
        run: () => openSessionWorkspace(workspace),
      });
      acts.push({
        id: `workspace-delete-${workspace.id}`,
        title: i18n.t('action.deleteSessionWorkspace', { name: workspace.name }),
        subtitle: i18n.t('workspace.tabsSummary', { count: workspace.tabs.length, suffix: workspace.tabs.length === 1 ? '' : 's' }),
        run: () => deleteSessionWorkspace(workspace),
      });
    }
    acts.push({
      id: 'batch-command',
      title: i18n.t('action.batchCommand'),
      subtitle: i18n.t('batchCommand.scopeActiveTab'),
      keywords: ['batch', 'command', 'writeMany', 'multiplex'],
      shortcut: 'Ctrl+Shift+Enter',
      run: () => { batchCommandOpen = true; },
    });
    acts.push({
      id: 'toggle-broadcast',
      title: broadcastOn ? i18n.t('action.broadcastOff') : i18n.t('action.broadcastOn'),
      subtitle: i18n.t('action.broadcastHint'),
      keywords: ['broadcast', 'multiplex', 'fanout'],
      shortcut: 'Ctrl+Shift+B',
      run: () => toggleBroadcast(),
    });
    for (const p of savedProfiles) {
      acts.push({
        id: `connect-${p.id}`,
        title: i18n.t('action.connectProfile', { name: p.name }),
        subtitle: profileCommandSubtitle(p),
        keywords: profileCommandKeywords(p),
        run: () => connectProfile(p),
      });
      if (p.kind === 'ssh') {
        acts.push({
          id: `sftp-${p.id}`,
          title: i18n.t('action.sftpBrowserProfile', { name: p.name }),
          subtitle: profileCommandSubtitle(p),
          keywords: ['sftp', 'transfer', ...profileCommandKeywords(p)],
          run: () => openSftpDock({ name: p.name, ssh: p.ssh }),
        });
        acts.push({
          id: `file-transfer-${p.id}`,
          title: i18n.t('action.fileTransferProfile', { name: p.name }),
          subtitle: profileCommandSubtitle(p),
          keywords: ['file', 'transfer', 'sftp', ...profileCommandKeywords(p)],
          run: () => { void openFileTransferWindow({ name: p.name, ssh: p.ssh }, p.id); },
        });
      }
    }
    acts.push({
      id: 'open-sftp',
      title: i18n.t('action.openSftpCurrent'),
      shortcut: 'Ctrl+Alt+F',
      run: () => { void openSftpForActivePane(); },
    });
    acts.push({
      id: 'open-sftp-window',
      title: i18n.t('action.openSftpWindowCurrent'),
      run: () => { void openSftpWindowForActivePane(); },
    });
    acts.push({
      id: 'open-file-transfer-window',
      title: i18n.t('action.openFileTransferWindow'),
      keywords: ['file', 'transfer', 'sftp', 'remote'],
      run: () => { void openFileTransferWindow(); },
    });
    acts.push({
      id: 'toggle-sftp-dock',
      title: currentSftpCollapsed ? i18n.t('action.expandSftpDock') : i18n.t('action.collapseSftpDock'),
      shortcut: 'Ctrl+Alt+E',
      run: () => toggleCurrentSftpDock(),
    });
    return acts;
  }

  let kbdHandler: ((e: KeyboardEvent) => void) | null = null;

  const onProfilesChanged = () => {
    void refreshProfileList();
    void sidebar?.refresh();
  };

  async function refreshProfileList() {
    try { savedProfiles = await rpc.call<StoredProfile[]>('profile.list'); } catch { savedProfiles = []; }
  }

  onMount(() => {
    // Wire action handlers (M5). Bindings are owned by HotkeyManager and
    // loaded from settings asynchronously below.
    hotkeys.registerHandler('new-tab',     () => { void openLocal(); });
    hotkeys.registerHandler('open-profile', () => { pickerOpen = true; });
    hotkeys.registerHandler('close-pane',  () => { void closeActivePane(); });
    hotkeys.registerHandler('next-tab',    () => cycleTab(1));
    hotkeys.registerHandler('prev-tab',    () => cycleTab(-1));
    hotkeys.registerHandler('split-right', () => { void splitActive('row'); });
    hotkeys.registerHandler('split-left',  () => { void splitActive('row', 'before'); });
    hotkeys.registerHandler('split-down',  () => { void splitActive('col'); });
    hotkeys.registerHandler('split-up',    () => { void splitActive('col', 'before'); });
    hotkeys.registerHandler('maximize-pane', () => toggleActivePaneMaximize());
    hotkeys.registerHandler('open-sftp', () => { void openSftpForActivePane(); });
    hotkeys.registerHandler('toggle-sftp-dock', () => toggleCurrentSftpDock());
    hotkeys.registerHandler('toggle-broadcast', () => toggleBroadcast());
    hotkeys.registerHandler('batch-command', () => { batchCommandOpen = true; });
    hotkeys.registerHandler('focus-left',  () => focusPaneDirection('left'));
    hotkeys.registerHandler('focus-right', () => focusPaneDirection('right'));
    hotkeys.registerHandler('focus-up',    () => focusPaneDirection('up'));
    hotkeys.registerHandler('focus-down',  () => focusPaneDirection('down'));
    hotkeys.registerHandler('next-pane',   () => cyclePane(1));
    hotkeys.registerHandler('prev-pane',   () => cyclePane(-1));
    hotkeys.registerHandler('palette',     () => { paletteOpen = true; });
    hotkeys.registerHandler('settings',    () => openSettings());
    hotkeys.registerHandler('toggle-sidebar', () => { void setSidebarVisible(!sidebarVisible); });
    hotkeys.registerHandler('search',      () => {
      document.dispatchEvent(new CustomEvent('aerotab:search'));
    });
    hotkeys.registerHandler('terminal-copy', () => {
      document.dispatchEvent(new CustomEvent('aerotab:terminal-copy'));
    });
    hotkeys.registerHandler('terminal-font-increase', () => {
      document.dispatchEvent(new CustomEvent('aerotab:terminal-font-delta', { detail: 1 }));
    });
    hotkeys.registerHandler('terminal-font-decrease', () => {
      document.dispatchEvent(new CustomEvent('aerotab:terminal-font-delta', { detail: -1 }));
    });
    hotkeys.registerHandler('session-ended-close', () => {
      document.dispatchEvent(new CustomEvent('aerotab:session-ended-action', { detail: 'close' }));
    });
    hotkeys.registerHandler('session-ended-reconnect', () => {
      document.dispatchEvent(new CustomEvent('aerotab:session-ended-action', { detail: 'reconnect' }));
    });

    // Load user-overridden bindings from settings.
    void (async () => {
      try {
        const r = await rpc.call<{ value: unknown }>('settings.get', { key: 'hotkeys' });
        if (r.value && typeof r.value === 'object') {
          hotkeys.loadFromMap(r.value as Record<string, string[]>);
        }
      } catch {
        // ignore — defaults remain in effect
      }
    })();

    kbdHandler = (e: KeyboardEvent) => {
      if (isModalOverlayActive()) return;
      if (e.key === 'F5' || ((e.ctrlKey || e.metaKey) && !e.altKey && (e.key === 'r' || e.key === 'R'))) {
        e.preventDefault();
        return;
      }
      if (shouldDeferToTextInput(e)) return;
      hotkeys.dispatch(e);
    };
    window.addEventListener('keydown', kbdHandler, true);
    const onSessionReplaced = (ev: Event) => {
      const detail = (ev as CustomEvent<{ oldId: string; session: SessionMeta }>).detail;
      if (!detail?.oldId || !detail.session) return;
      const restore = restoreMap.get(detail.oldId);
      if (restore) {
        restoreMap.delete(detail.oldId);
        restoreMap.set(detail.session.id, restore);
        schedulePersistOpenTabs();
      }
    };
    document.addEventListener('aerotab:settings-changed', onAppSettingsChanged);
    document.addEventListener(PROFILES_CHANGED, onProfilesChanged);
    document.addEventListener('aerotab:session-replaced', onSessionReplaced);
    installPaneDragGlobalHandlers();
    let winResizeTimer: ReturnType<typeof setTimeout> | null = null;
    const onWindowResize = () => {
      if (winResizeTimer) clearTimeout(winResizeTimer);
      winResizeTimer = setTimeout(() => {
        winResizeTimer = null;
        const ids = tabs.tabs.flatMap((t) => t.panes.map((p) => p.id));
        if (ids.length) dispatchFitAllPanes(ids);
      }, 100);
    };
    window.addEventListener('resize', onWindowResize);
    const unsubPaneDrop = subscribePanePointerDrop(({ source, hit }) => {
      if (hit.kind === 'pane') {
        tabs.movePaneBetweenTabs(source.tabId, source.paneId, hit.tabId, hit.paneId, hit.side);
      } else {
        tabs.mergePaneIntoTab(source.tabId, source.paneId, hit.tabId);
        tabs.activate(hit.tabId);
      }
      requestAnimationFrame(() => dispatchFocusPane(source.paneId));
    });
    return () => {
      window.removeEventListener('resize', onWindowResize);
      if (winResizeTimer) clearTimeout(winResizeTimer);
      document.removeEventListener(PROFILES_CHANGED, onProfilesChanged);
      document.removeEventListener('aerotab:session-replaced', onSessionReplaced);
      unsubPaneDrop();
    };
  });
  onDestroy(() => {
    if (kbdHandler) window.removeEventListener('keydown', kbdHandler, true);
    document.removeEventListener('aerotab:settings-changed', onAppSettingsChanged);
    document.removeEventListener(PROFILES_CHANGED, onProfilesChanged);
    clearHostStatsPoll();
  });
  // Keep palette profile list fresh whenever it opens.
  $effect(() => {
    if (paletteOpen) {
      void refreshProfileList();
      void loadSessionWorkspaces();
    }
  });

  // M9 — re-persist the open-tabs list whenever it changes (add / close /
  // reorder). Reads `tabs.revision` and `activeId` so Svelte tracks both.
  $effect(() => {
    void tabs.revision;
    void tabs.activeId;
    schedulePersistOpenTabs();
  });

  // Live-preview bridge: any section that calls `settingsCoord.bumpRev()`
  // (e.g. ColorScheme, Window) needs every open TerminalPane to re-read its
  // settings. PaneGrid passes `settingsRev` down to TerminalPane, which has
  // an effect reacting to it.
  $effect(() => {
    settingsRev = settingsCoord.rev;
  });

  // Block wheel/pointer from reaching xterm while a full-screen modal is open.
  $effect(() => {
    const overlayOpen =
      settingsOpen
      || paletteOpen
      || pickerOpen
      || batchCommandOpen
      || sftpWindows.length > 0;
    if (overlayOpen) {
      document.body.dataset.modalOverlay = 'true';
      const active = document.activeElement;
      if (active instanceof HTMLElement && active.closest('.xterm, .terminal-surface')) {
        active.blur();
      }
    } else {
      delete document.body.dataset.modalOverlay;
    }
  });

  $effect(() => {
    void tabs.revision;
    void tabs.activeId;
    void hostStatsEnabled;
    void hostStatsIntervalSec;
    const key = activeHostStatsKey();
    hostStatsSeq += 1;
    clearHostStatsPoll();
    hostStats = null;
    if (!hostStatsEnabled || !key) {
      hostStatsStatus = 'idle';
      return;
    }
    hostStatsStatus = 'loading';
    void refreshHostStats();
    hostStatsPollHandle = window.setInterval(() => {
      void refreshHostStats();
    }, Math.max(10, hostStatsIntervalSec) * 1000);
    return () => clearHostStatsPoll();
  });
</script>

<div class="h-full w-full flex flex-row overflow-hidden">
  {#if sidebarVisible}
    <div
      class="shrink-0 flex flex-col h-full min-w-0 border-r border-[var(--color-border-soft)]"
      style="width: {sidebarWidthPx}px; min-width: {SIDEBAR_WIDTH_MIN}px; max-width: min({SIDEBAR_WIDTH_MAX}px, 40vw);"
    >
      <Sidebar
        {rpc}
        bind:this={sidebar}
        openProfileModal={(p, opts) => profileModal?.open(p, opts)}
        openSerialModal={() => serialModal?.open()}
        openSftp={(p) => { if (p.kind === 'ssh') openSftpDock({ name: p.name, ssh: p.ssh }); }}
        openSettings={() => openSettings()}
        workspaceView={workspaceView}
        onShowTerminal={showTerminalWorkspace}
        onShowTransfer={() => openFileTransferWindow()}
        {onError}
      />
    </div>
    <button
      type="button"
      aria-label={i18n.t('sidebar.resizeSidebar')}
      class="shrink-0 w-[3px] cursor-col-resize bg-[var(--color-border-soft)] hover:bg-[var(--color-accent)] border-0 p-0"
      onpointerdown={onSidebarResizePointerDown}
    ></button>
  {/if}

  <main class="flex flex-col flex-1 min-w-0 bg-[var(--color-panel)]">
    <div class="{workspaceView === 'terminal' ? 'flex' : 'hidden'} flex-col flex-1 min-h-0 min-w-0">
      <TabBar
        {rpc}
        onAddTab={() => (pickerOpen = true)}
        onSplit={(direction) => { void splitActive(direction); }}
        onOpenSftp={() => { void openSftpForActivePane(); }}
        onDuplicateTab={(tab) => { void duplicateTab(tab); }}
        onCloseTab={(tab) => { closeTabSessions(tab); }}
        onCloseOthers={(id) => { closeOtherTabs(id); }}
        onCloseToRight={(idx) => { closeTabsToRight(idx); }}
        onCloseAll={() => { closeAllTabs(); }}
      />

      <div class="flex-1 min-h-0 bg-[var(--color-bg)] border-t border-[var(--color-border-soft)] flex">
        <div class="relative flex-1 min-w-0 min-h-0">
          {#each tabs.tabs as tab (tab.id)}
            <div class="absolute inset-0" hidden={tabs.activeId !== tab.id}>
              <PaneGrid
                {rpc}
                {tab}
                settingsRev={settingsRev}
                tabVisible={workspaceView === 'terminal' && tabs.activeId === tab.id}
                broadcastEnabled={broadcastOn}
                broadcastTargetIds={broadcastTargets}
                onOpenSftp={() => { void openSftpForActivePane(); }}
                onSplitRight={() => { void splitActive('row'); }}
                onSplitDown={() => { void splitActive('col'); }}
                {onError}
              />
            </div>
          {/each}
          {#if tabs.tabs.length === 0}
            <div class="absolute inset-0 grid place-items-center text-[var(--color-fg-muted)] text-[12.5px]">
              <div class="text-center max-w-[280px]">
                <img src={logoUrl} alt="" class="aerotab-logo mx-auto mb-3 w-12 h-12 rounded-lg opacity-90" width="48" height="48" />
                <div class="text-[var(--color-fg)] font-medium">{i18n.t('app.empty.title')}</div>
                <div class="opacity-70 mt-1">{i18n.t('app.empty.subtitle')}</div>
              </div>
            </div>
          {/if}
        </div>
        {#if currentSftpDock}
          {#if currentSftpCollapsed}
            <div class="w-9 shrink-0 border-l border-[var(--color-border-soft)] bg-[var(--color-panel)] flex flex-col items-center py-2 gap-2 shadow-[inset_1px_0_0_var(--color-border-soft)]">
              <button
                type="button"
                class="btn-ghost p-1.5 text-[var(--color-accent)]"
                title={i18n.t('sftp.expandDock')}
                aria-label={i18n.t('sftp.expandDock')}
                onclick={() => setCurrentSftpCollapsed(false)}
              >
                <PanelRightOpen size={15} />
              </button>
              <FolderOpen size={14} class="text-[var(--color-fg-muted)]" />
              <button
                type="button"
                class="btn-ghost mt-auto p-1 hover:!text-[var(--color-danger)]"
                title={i18n.t('sftp.closeDock')}
                aria-label={i18n.t('sftp.closeDock')}
                onclick={() => closeSftpDock()}
              >
                <X size={13} />
              </button>
            </div>
          {:else}
            <button
              type="button"
              aria-label={i18n.t('sftp.resizeDock')}
              class="shrink-0 w-[3px] cursor-col-resize bg-[var(--color-border-soft)] hover:bg-[var(--color-accent)] border-0 p-0"
              onpointerdown={onSftpDockResizePointerDown}
            ></button>
            <div
              class="shrink-0 h-full border-l border-[var(--color-border-soft)] min-w-0"
              style="width: {sftpDockWidthPx}px; max-width: min({SFTP_DOCK_WIDTH_MAX}px, 55vw);"
            >
              {#if currentSftpDock}
                <SftpBrowser
                  {rpc}
                  registryId={`dock-${activeSftpKey}`}
                  terminalSessionId={sftpDockSessionId}
                  source={currentSftpDock}
                  mode="dock"
                  onClose={() => closeSftpDock()}
                  onCollapse={() => setCurrentSftpCollapsed(true)}
                  onPopOut={(sudo) => openSftpWindow({ ...currentSftpDock, sudo })}
                  {onError}
                />
              {/if}
            </div>
          {/if}
        {/if}
      </div>
    </div>

    {#if transferWorkspaceMounted}
      <div class="{workspaceView === 'transfer' ? 'block' : 'hidden'} flex-1 min-h-0 bg-[var(--color-bg)] border-t border-[var(--color-border-soft)]">
        <FileTransferWindow
          {rpc}
          embedded={true}
          initialTarget={transferInitialTarget}
          initialProfileId={transferInitialProfileId}
          onClose={showTerminalWorkspace}
          {onError}
        />
      </div>
    {/if}

    <footer class="px-3 py-1.5 border-t border-[var(--color-border-soft)] bg-[var(--color-panel)] flex items-center gap-3
                   text-[11px] text-[var(--color-fg-muted)] font-mono">
      <button type="button"
              class="btn-ghost p-0.5"
              title={sidebarVisible ? i18n.t('app.footer.hideSidebar') : i18n.t('app.footer.showSidebar')}
              aria-label={sidebarVisible ? i18n.t('app.footer.hideSidebar') : i18n.t('app.footer.showSidebar')}
              onclick={() => { void setSidebarVisible(!sidebarVisible); }}>
        {#if sidebarVisible}<PanelLeftClose size={13} />{:else}<PanelLeftOpen size={13} />{/if}
      </button>
      <span>{status}</span>
      {#if hostStatsEnabled && hostStatsStatus === 'ok' && hostStats}
        <span class="hidden lg:inline-flex items-center gap-2 truncate max-w-[520px]" title={hostStatsTitle(hostStats)}>
          <span class="truncate">{formatHostStats(hostStats)}</span>
          {#if hostStatsUpdatedAt}
            <span class="shrink-0 opacity-70">{i18n.t('app.footer.statsUpdated', { time: formatHostStatsUpdated(hostStatsUpdatedAt) })}</span>
          {/if}
        </span>
        <button
          type="button"
          class="btn-ghost hidden lg:inline p-0.5"
          title={i18n.t('app.footer.refreshStats')}
          aria-label={i18n.t('app.footer.refreshStats')}
          onclick={() => { void refreshHostStats(); }}
        >
          <RefreshCw size={12} />
        </button>
      {:else if hostStatsEnabled && hostStatsStatus === 'loading'}
        <span class="hidden lg:inline text-[var(--color-fg-muted)]">{i18n.t('app.footer.statsLoading')}</span>
      {:else if hostStatsEnabled && hostStatsStatus === 'unavailable'}
        <span class="hidden lg:inline text-[var(--color-fg-muted)]" title={i18n.t('app.footer.hostStatsUnavailable')}>{i18n.t('app.footer.statsUnavailable')}</span>
      {/if}
      <span class="ml-auto">{i18n.t('app.footer.sessions', { count: tabs.tabs.length, suffix: tabs.tabs.length === 1 ? '' : 's' })}</span>
      {#if coreVersion}<span>v{coreVersion}</span>{/if}
      <span>{buildId}</span>
    </footer>
  </main>
</div>

<ProfileModal
  {rpc}
  bind:this={profileModal}
  onSaved={() => { void refreshProfileList(); void sidebar?.refresh(); }}
  onClosed={() => focusActivePane()}
  {onError}
/>
<VaultUnlockModal
  {rpc}
  bind:this={vaultUnlockModal}
  {onError}
  onUnlocked={() => { status = i18n.t('sync.vaultUnlocked'); }}
/>
<SerialModal {rpc} bind:this={serialModal} {onError} />
<AppConfirmDialog />
{#if settingsOpen}
  <SettingsLayout
    {rpc}
    {buildId}
    initialSection={settingsInitialSection}
    onClose={closeSettings}
    onSettingsChanged={() => { void refreshAppFromSettingsStore(); }}
    {onError}
  />
{/if}
{#if paletteOpen}
  <CommandPalette
    actions={buildActions()}
    onClose={() => {
      paletteOpen = false;
      requestAnimationFrame(() => focusActivePane());
    }}
  />
{/if}
{#if batchCommandOpen}
  <BatchCommandModal
    tabs={tabs.tabs}
    activeTabId={tabs.activeId}
    onSend={async (sessionIds, command) => {
      const data = b64encode(new TextEncoder().encode(command));
      await rpc.call('session.writeMany', { ids: sessionIds, data });
    }}
    onClose={() => {
      batchCommandOpen = false;
      requestAnimationFrame(() => focusActivePane());
    }}
  />
{/if}
{#if pickerOpen}
  <ProfileSelector
    {rpc}
    onClose={() => {
      pickerOpen = false;
      requestAnimationFrame(() => focusActivePane());
    }}
    onOpen={onPickerItem}
  />
{/if}
{#each sftpWindows as win (win.id)}
  <SftpBrowser
    {rpc}
    registryId={`win-${win.id}`}
    source={win.target}
    mode="modal"
    onClose={() => closeSftpWindow(win.id)}
    {onError}
  />
{/each}
