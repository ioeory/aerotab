<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import Sidebar from './components/Sidebar.svelte';
  import TabBar from './components/TabBar.svelte';
  import PaneGrid from './components/PaneGrid.svelte';
  import ProfileModal from './components/ProfileModal.svelte';
  import SerialModal from './components/SerialModal.svelte';
  import SftpBrowser from './components/SftpBrowser.svelte';
  import SettingsLayout from './components/settings/SettingsLayout.svelte';
  import CommandPalette, { type Action } from './components/CommandPalette.svelte';
  import ProfileSelector, { type PickerItem } from './components/ProfileSelector.svelte';
  import { selectClient } from './lib/rpc';
  import { tabs, type SplitSide } from './lib/tabs.svelte';
  import { applyTheme, BUILTIN_THEMES } from './lib/theme';
  import { applyCustomCss, applyLigatures } from './lib/customCss';
  import { applyWindowSettings, getWindowSettings } from './lib/windowSettings';
  import { settingsCoord } from './lib/settingsStore.svelte';
  import type { SessionMeta, SshProfileSpec, StoredProfile } from './lib/types';
  import { hotkeys } from './lib/hotkeys';
  import { FolderOpen, PanelLeftClose, PanelLeftOpen, PanelRightOpen, X } from '@lucide/svelte';

  const rpc = selectClient();
  const buildId = '0.1.16-ui-20260523';
  let status = $state('idle');
  let coreVersion = $state<string | null>(null);

  let profileModal: { open: (existing?: StoredProfile) => void } | null = $state(null);
  let serialModal: { open: () => Promise<void> } | null = $state(null);
  let sidebar: { refresh: () => Promise<void> } | null = $state(null);
  let settingsOpen = $state(false);
  let settingsRev = $state(0);
  let paletteOpen = $state(false);
  let pickerOpen = $state(false);
  let savedProfiles = $state<StoredProfile[]>([]);
  let sidebarVisible = $state(true);

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
  let sftpDocks = $state<Record<string, SftpDockTarget>>({});
  let sftpDockCollapsed = $state<Record<string, boolean>>({});
  let sftpWindows = $state<SftpWindow[]>([]);
  let sftpWindowSeq = 0;
  const activeSftpKey = $derived(tabs.activeId ?? GLOBAL_SFTP_KEY);
  const currentSftpDock = $derived(sftpDocks[activeSftpKey] ?? null);
  const currentSftpCollapsed = $derived(sftpDockCollapsed[activeSftpKey] ?? false);

  // ── M9 — session restore ────────────────────────────────────────────────
  // A `Restorable` describes how to re-open a session after a restart. We
  // record one per opened session and persist the list under `openTabs`.
  type Restorable =
    | { kind: 'local' }
    | { kind: 'shell'; command: string; args: string[]; label: string }
    | { kind: 'ssh-profile'; id: string }
    | { kind: 'ssh'; title: string; profile: Record<string, unknown> };
  const restoreMap = new Map<string, Restorable>();
  let restoreReady = false; // suppress persistence until first load completes

  function recordRestore(sessionId: string, r: Restorable) {
    restoreMap.set(sessionId, r);
    persistOpenTabs();
  }

  function persistOpenTabs() {
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

  async function replayRestorable(r: Restorable) {
    try {
      if (r.kind === 'local') {
        const meta = await rpc.call<{ id: string; kind: string; title: string }>(
          'session.openLocal', {},
        );
        tabs.add({ id: meta.id, kind: meta.kind, title: meta.title });
        restoreMap.set(meta.id, { kind: 'local' });
      } else if (r.kind === 'shell') {
        const meta = await rpc.call<{ id: string; kind: string; title: string }>(
          'session.openLocal',
          { title: r.label, shell: r.command, shell_args: r.args },
        );
        tabs.add({ id: meta.id, kind: meta.kind, title: meta.title, shellCommand: r.command, shellArgs: r.args });
        restoreMap.set(meta.id, r);
      } else if (r.kind === 'ssh-profile') {
        const meta = await rpc.call<{ id: string; kind: string; title: string }>(
          'session.openSshProfile', { profile_id: r.id },
        );
        tabs.add({ id: meta.id, kind: meta.kind, title: meta.title, profileId: r.id });
        restoreMap.set(meta.id, r);
      } else if (r.kind === 'ssh') {
        const meta = await rpc.call<{ id: string; kind: string; title: string }>(
          'session.openSsh', { title: r.title, profile: r.profile },
        );
        tabs.add({ id: meta.id, kind: meta.kind, title: meta.title, sshProfile: r.profile as unknown as SshProfileSpec });
        restoreMap.set(meta.id, r);
      }
    } catch (e) {
      console.warn('restore', r, e);
    }
  }


  onMount(async () => {
    try {
      const v = await rpc.call<{ version: string }>('core.version');
      coreVersion = v.version;
      status = `connected · core ${v.version}`;
    } catch (e) {
      status = 'core unreachable';
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
        applyWindowSettings(value);
      }
    } catch { /* not configured yet */ }
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
    if (next) tabs.activate(next.id);
  }

  function cyclePane(delta: number) {
    const tab = tabs.tabs.find((t) => t.id === tabs.activeId);
    if (!tab) return;
    const i = tab.panes.findIndex((p) => p.id === tab.activePaneId);
    const next = tab.panes[(i + delta + tab.panes.length) % tab.panes.length];
    if (next) tabs.focusPane(tab.id, next.id);
  }

  function focusPaneDirection(direction: 'left' | 'right' | 'up' | 'down') {
    const tab = tabs.tabs.find((t) => t.id === tabs.activeId);
    if (!tab) return;
    tabs.focusDirectional(tab.id, direction);
  }

  async function closeActivePane() {
    const tab = tabs.tabs.find((t) => t.id === tabs.activeId);
    if (!tab) return;
    const sid = tab.activePaneId;
    const r = tabs.removePane(tab.id, sid);
    if (!r) return;
    try { await rpc.call('session.close', { id: sid }); } catch (e) { console.warn(e); }
  }

  function toggleActivePaneMaximize() {
    const tab = tabs.tabs.find((t) => t.id === tabs.activeId);
    if (!tab) return;
    tabs.toggleMaximize(tab.id, tab.activePaneId);
  }

  function openSftpDock(target: SftpDockTarget, tabId = tabs.activeId ?? GLOBAL_SFTP_KEY) {
    sftpDocks = { ...sftpDocks, [tabId]: target };
    sftpDockCollapsed = { ...sftpDockCollapsed, [tabId]: false };
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

  function closeSftpDock(tabId = activeSftpKey) {
    const { [tabId]: _dropDock, ...restDocks } = sftpDocks;
    const { [tabId]: _dropCollapsed, ...restCollapsed } = sftpDockCollapsed;
    sftpDocks = restDocks;
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

  async function connectProfile(p: StoredProfile) {
    try {
      const meta = await rpc.call<{ id: string; kind: string; title: string }>(
        'session.openSshProfile', { profile_id: p.id },
      );
      tabs.add({ id: meta.id, kind: meta.kind, title: meta.title, profileId: p.id, sshProfile: p.ssh });
      recordRestore(meta.id, { kind: 'ssh-profile', id: p.id });
    } catch (e) {
      onError(`connect: ${(e as Error).message}`);
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
      const sshProfile: SshProfileSpec = {
        host: item.entry.host,
        port: item.entry.port,
        user: item.entry.user ?? 'root',
        auth: item.entry.identity_file
          ? { PublicKey: { key_path: item.entry.identity_file } }
          : 'Agent',
        jump_via: [],
      };
      try {
        const meta = await rpc.call<{ id: string; kind: string; title: string }>(
          'session.openSsh',
          { title: item.entry.alias, profile: sshProfile },
        );
        tabs.add({ id: meta.id, kind: meta.kind, title: meta.title, sshProfile });
        recordRestore(meta.id, { kind: 'ssh', title: item.entry.alias, profile: sshProfile as unknown as Record<string, unknown> });
      } catch (e) { onError(`ssh-config: ${(e as Error).message}`); }
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
      } catch (e) { onError(`quick-connect: ${(e as Error).message}`); }
    }
  }


  function buildActions(): Action[] {
    const acts: Action[] = [
      { id: 'new-tab', title: 'New local tab', shortcut: 'Ctrl+Shift+T', run: () => openLocal() },
      { id: 'split-right', title: 'Split right', shortcut: 'Ctrl+Shift+D', run: () => splitActive('row') },
      { id: 'split-left', title: 'Split left', shortcut: 'Ctrl+Shift+A', run: () => splitActive('row', 'before') },
      { id: 'split-down', title: 'Split down', shortcut: 'Ctrl+Shift+E', run: () => splitActive('col') },
      { id: 'split-up', title: 'Split up', shortcut: 'Ctrl+Shift+W', run: () => splitActive('col', 'before') },
      { id: 'maximize-pane', title: 'Maximize / restore pane', shortcut: 'Alt+Z', run: () => toggleActivePaneMaximize() },
      { id: 'close-pane', title: 'Close current pane', shortcut: 'Ctrl+W', run: () => closeActivePane() },
      { id: 'next-tab', title: 'Next tab', shortcut: 'Ctrl+Tab', run: () => cycleTab(1) },
      { id: 'prev-tab', title: 'Previous tab', shortcut: 'Ctrl+Shift+Tab', run: () => cycleTab(-1) },
      { id: 'focus-left', title: 'Focus pane left', shortcut: 'Alt+←', run: () => focusPaneDirection('left') },
      { id: 'focus-right', title: 'Focus pane right', shortcut: 'Alt+→', run: () => focusPaneDirection('right') },
      { id: 'focus-up', title: 'Focus pane up', shortcut: 'Alt+↑', run: () => focusPaneDirection('up') },
      { id: 'focus-down', title: 'Focus pane down', shortcut: 'Alt+↓', run: () => focusPaneDirection('down') },
      { id: 'next-pane', title: 'Next pane', shortcut: 'Alt+]', run: () => cyclePane(1) },
      { id: 'prev-pane', title: 'Previous pane', shortcut: 'Alt+[', run: () => cyclePane(-1) },
      { id: 'settings', title: 'Open settings', shortcut: 'Ctrl+,', run: () => (settingsOpen = true) },
      { id: 'toggle-sidebar', title: sidebarVisible ? 'Hide sidebar' : 'Show sidebar', shortcut: 'Ctrl+Alt+S', run: () => { void setSidebarVisible(!sidebarVisible); } },
      { id: 'new-profile', title: 'New SSH profile…', run: () => profileModal?.open() },
      { id: 'new-serial', title: 'New serial connection…', run: () => serialModal?.open() },
    ];
    for (const p of savedProfiles) {
      acts.push({
        id: `connect-${p.id}`,
        title: `Connect: ${p.name}`,
        subtitle: p.kind === 'ssh' ? `ssh ${p.ssh.user ?? ''}@${p.ssh.host}` : p.kind,
        run: () => connectProfile(p),
      });
      acts.push({
        id: `sftp-${p.id}`,
        title: `SFTP browser: ${p.name}`,
        run: () => openSftpDock({ name: p.name, ssh: p.ssh }),
      });
    }
    acts.push({
      id: 'open-sftp',
      title: 'Open SFTP for current SSH pane',
      shortcut: 'Ctrl+Alt+F',
      run: () => { void openSftpForActivePane(); },
    });
    acts.push({
      id: 'open-sftp-window',
      title: 'Open SFTP window for current SSH pane',
      run: () => { void openSftpWindowForActivePane(); },
    });
    acts.push({
      id: 'toggle-sftp-dock',
      title: currentSftpCollapsed ? 'Expand SFTP dock' : 'Collapse SFTP dock',
      shortcut: 'Ctrl+Alt+E',
      run: () => toggleCurrentSftpDock(),
    });
    return acts;
  }

  let kbdHandler: ((e: KeyboardEvent) => void) | null = null;
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
    hotkeys.registerHandler('focus-left',  () => focusPaneDirection('left'));
    hotkeys.registerHandler('focus-right', () => focusPaneDirection('right'));
    hotkeys.registerHandler('focus-up',    () => focusPaneDirection('up'));
    hotkeys.registerHandler('focus-down',  () => focusPaneDirection('down'));
    hotkeys.registerHandler('next-pane',   () => cyclePane(1));
    hotkeys.registerHandler('prev-pane',   () => cyclePane(-1));
    hotkeys.registerHandler('palette',     () => { paletteOpen = true; });
    hotkeys.registerHandler('settings',    () => { settingsOpen = true; });
    hotkeys.registerHandler('toggle-sidebar', () => { void setSidebarVisible(!sidebarVisible); });
    hotkeys.registerHandler('search',      () => {
      document.dispatchEvent(new CustomEvent('tabby:search'));
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

    kbdHandler = (e: KeyboardEvent) => { hotkeys.dispatch(e); };
    window.addEventListener('keydown', kbdHandler);
    document.addEventListener('tabby:settings-changed', syncSidebarFromWindowSettings);
  });
  onDestroy(() => {
    if (kbdHandler) window.removeEventListener('keydown', kbdHandler);
    document.removeEventListener('tabby:settings-changed', syncSidebarFromWindowSettings);
  });

  async function refreshProfileList() {
    try { savedProfiles = await rpc.call<StoredProfile[]>('profile.list'); } catch { savedProfiles = []; }
  }
  // Keep palette profile list fresh whenever it opens.
  $effect(() => {
    if (paletteOpen) void refreshProfileList();
  });

  // M9 — re-persist the open-tabs list whenever it changes (add / close /
  // reorder). Reads `tabs.revision` and `activeId` so Svelte tracks both.
  $effect(() => {
    void tabs.revision;
    void tabs.activeId;
    persistOpenTabs();
  });

  // Live-preview bridge: any section that calls `settingsCoord.bumpRev()`
  // (e.g. ColorScheme, Window) needs every open TerminalPane to re-read its
  // settings. PaneGrid passes `settingsRev` down to TerminalPane, which has
  // an effect reacting to it.
  $effect(() => {
    settingsRev = settingsCoord.rev;
  });
</script>

<div class="h-full w-full grid {sidebarVisible ? 'grid-cols-[auto_1fr]' : 'grid-cols-[1fr]'} grid-rows-1 overflow-hidden">
  {#if sidebarVisible}
    <Sidebar
      {rpc}
      bind:this={sidebar}
      openProfileModal={(p) => profileModal?.open(p)}
      openSerialModal={() => serialModal?.open()}
      openSftp={(p) => openSftpDock({ name: p.name, ssh: p.ssh })}
      openSettings={() => (settingsOpen = true)}
      {onError}
    />
  {/if}

  <main class="flex flex-col min-w-0 bg-[var(--color-panel)]">
    <TabBar
      {rpc}
      onAddTab={() => (pickerOpen = true)}
      onSplit={(direction) => { void splitActive(direction); }}
      onOpenSftp={() => { void openSftpForActivePane(); }}
    />

    <div class="flex-1 min-h-0 bg-[var(--color-bg)] border-t border-[var(--color-border-soft)] flex">
      <div class="relative flex-1 min-w-0 min-h-0">
        {#each tabs.tabs as tab (tab.id)}
          <div class="absolute inset-0" hidden={tabs.activeId !== tab.id}>
            <PaneGrid {rpc} {tab} settingsRev={settingsRev} />
          </div>
        {/each}
        {#if tabs.tabs.length === 0}
          <div class="absolute inset-0 grid place-items-center text-[var(--color-fg-muted)] text-[12.5px]">
            <div class="text-center">
              <div class="text-[var(--color-accent)] text-[20px] font-bold mb-2">›_</div>
              <div>Welcome to Tabby v2</div>
              <div class="opacity-70 mt-1">Open a session from the sidebar to begin.</div>
            </div>
          </div>
        {/if}
      </div>
      {#if currentSftpDock}
        {#if currentSftpCollapsed}
          <div class="w-9 shrink-0 border-l border-[var(--color-border-soft)] bg-[var(--color-panel)] flex flex-col items-center py-2 gap-2">
            <button
              type="button"
              class="p-1.5 rounded text-[var(--color-accent)] hover:bg-[var(--color-panel-2)]"
              title="Expand SFTP dock"
              aria-label="Expand SFTP dock"
              onclick={() => setCurrentSftpCollapsed(false)}
            >
              <PanelRightOpen size={15} />
            </button>
            <FolderOpen size={14} class="text-[var(--color-fg-muted)]" />
            <button
              type="button"
              class="mt-auto p-1 rounded text-[var(--color-fg-muted)] hover:text-[var(--color-danger)] hover:bg-[var(--color-panel-2)]"
              title="Close SFTP dock"
              aria-label="Close SFTP dock"
              onclick={() => closeSftpDock()}
            >
              <X size={13} />
            </button>
          </div>
        {:else}
          <div class="shrink-0 min-w-[320px] max-w-[520px] h-full border-l border-[var(--color-border-soft)]" style="width: clamp(320px, 38vw, 460px);">
            {#key `${activeSftpKey}:${currentSftpDock.name}:${currentSftpDock.ssh.host}:${currentSftpDock.ssh.port}`}
              <SftpBrowser
                {rpc}
                source={currentSftpDock}
                mode="dock"
                onClose={() => closeSftpDock()}
                onCollapse={() => setCurrentSftpCollapsed(true)}
                onPopOut={(sudo) => openSftpWindow({ ...currentSftpDock, sudo })}
                {onError}
              />
            {/key}
          </div>
        {/if}
      {/if}
    </div>

    <footer class="px-3 py-1 border-t border-[var(--color-border-soft)] flex items-center gap-3
                   text-[11px] text-[var(--color-fg-muted)]">
      <button type="button"
              class="p-0.5 rounded hover:text-[var(--color-fg)] hover:bg-[var(--color-panel-2)]"
              title={sidebarVisible ? 'Hide sidebar' : 'Show sidebar'}
              aria-label={sidebarVisible ? 'Hide sidebar' : 'Show sidebar'}
              onclick={() => { void setSidebarVisible(!sidebarVisible); }}>
        {#if sidebarVisible}<PanelLeftClose size={13} />{:else}<PanelLeftOpen size={13} />{/if}
      </button>
      <span>{status}</span>
      <span class="ml-auto">{tabs.tabs.length} session{tabs.tabs.length === 1 ? '' : 's'}</span>
      {#if coreVersion}<span>v{coreVersion}</span>{/if}
      <span>{buildId}</span>
    </footer>
  </main>
</div>

<ProfileModal {rpc} bind:this={profileModal} onSaved={() => sidebar?.refresh()} {onError} />
<SerialModal {rpc} bind:this={serialModal} {onError} />
{#if settingsOpen}
  <SettingsLayout
    {rpc}
    onClose={() => (settingsOpen = false)}
    onSettingsChanged={() => (settingsRev += 1)}
    {onError}
  />
{/if}
{#if paletteOpen}
  <CommandPalette actions={buildActions()} onClose={() => (paletteOpen = false)} />
{/if}
{#if pickerOpen}
  <ProfileSelector {rpc} onClose={() => (pickerOpen = false)} onOpen={onPickerItem} />
{/if}
{#each sftpWindows as win (win.id)}
  <SftpBrowser
    {rpc}
    source={win.target}
    mode="modal"
    onClose={() => closeSftpWindow(win.id)}
    {onError}
  />
{/each}
