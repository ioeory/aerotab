// @ts-ignore Explicit extension supports Node's strip-types test runner and Vite.
import { applyTheme, BUILTIN_THEMES, normalizeAccent, themeMode } from './theme.ts';
import type { Theme } from './theme';

export interface AppearancePreferences {
  mode: 'system' | 'dark' | 'light';
  darkPalette: string;
  lightPalette: string;
  /** null restores the selected palette's accent. Never affects status colours. */
  accent: string | null;
}

export const DEFAULT_APPEARANCE_PREFERENCES: Readonly<AppearancePreferences> = Object.freeze({
  mode: 'dark', darkPalette: 'termora-dark', lightPalette: 'termora-light', accent: null,
});

/** Persist as `uiAppearance`; leave legacy `theme` and terminal fields untouched.
 * A missing value migrates the legacy `theme` string without changing its ID. */
export function normalizeAppearancePreferences(value?: unknown, legacyTheme?: unknown, _palettes?: readonly { name: string; bg: string }[]): AppearancePreferences {
  const p = value && typeof value === 'object' ? value as Record<string, unknown> : {};
  const legacy = BUILTIN_THEMES.find(t => t.name === legacyTheme);
  const valid = (name: unknown, mode: 'light' | 'dark', fallback: string) =>
    BUILTIN_THEMES.find(t => t.name === name && themeMode(t) === mode)?.name ?? fallback;
  return {
    mode: p.mode === 'system' || p.mode === 'light' || p.mode === 'dark' ? p.mode : legacy ? themeMode(legacy) : 'dark',
    darkPalette: valid(p.darkPalette, 'dark', legacy && themeMode(legacy) === 'dark' ? legacy.name : 'termora-dark'),
    lightPalette: valid(p.lightPalette, 'light', legacy && themeMode(legacy) === 'light' ? legacy.name : 'termora-light'),
    accent: normalizeAccent(p.accent),
  };
}

export function resolveAppearanceTheme(preferences: AppearancePreferences, systemDark = false): Theme {
  const dark = preferences.mode === 'dark' || (preferences.mode === 'system' && systemDark);
  return BUILTIN_THEMES.find(t => t.name === (dark ? preferences.darkPalette : preferences.lightPalette)) ?? BUILTIN_THEMES[0]!;
}

export function isLightPalette(palette: { name: string; bg: string }): boolean {
  const theme = BUILTIN_THEMES.find(t => t.name === palette.name);
  return theme ? themeMode(theme) === 'light' : false;
}

export function resolveAppearancePalette(preferences: AppearancePreferences, systemDark = false): string {
  return resolveAppearanceTheme(preferences, systemDark).name;
}

/** Small section-local adapter for settings coordinators that already own
 * system-theme listeners. Apply failures repaint the last persisted baseline. */
export function createAppearanceDraft(initial: AppearancePreferences, effects: {
  render: (preferences: AppearancePreferences) => void;
  persist: (preferences: AppearancePreferences) => Promise<unknown>;
}) {
  let applied = normalizeAppearancePreferences(initial);
  let draft = { ...applied };
  let pending: Promise<AppearancePreferences> | null = null;
  return {
    preview(value: AppearancePreferences) {
      draft = normalizeAppearancePreferences(value);
      effects.render({ ...draft });
      return { ...draft };
    },
    cancel() {
      draft = { ...applied };
      effects.render({ ...draft });
      return { ...draft };
    },
    apply(value: AppearancePreferences): Promise<AppearancePreferences> {
      if (pending) return pending;
      const saved = normalizeAppearancePreferences(value);
      pending = Promise.resolve().then(() => effects.persist({ ...saved })).then(() => {
        applied = saved;
        draft = { ...saved };
        effects.render({ ...draft });
        return { ...applied };
      }, error => {
        draft = { ...applied };
        effects.render({ ...applied });
        throw error;
      }).finally(() => { pending = null; });
      return pending;
    },
  };
}

export function applyAppearancePreferences(preferences: AppearancePreferences, systemDark = false): void {
  applyTheme(resolveAppearanceTheme(preferences, systemDark), preferences.accent);
}

export interface AppearanceSnapshot {
  applied: AppearancePreferences;
  draft: AppearancePreferences;
  previewing: boolean;
  saving: boolean;
}

type MediaSource = Pick<MediaQueryList, 'matches' | 'addEventListener' | 'removeEventListener'>;
interface CoordinatorOptions {
  initial?: unknown;
  legacyTheme?: unknown;
  apply?: (preferences: AppearancePreferences, systemDark: boolean) => void;
  media?: MediaSource;
}

/** Framework-independent, window-local preview transaction. The caller owns RPC
 * persistence: await commit(p => saveAppearanceAndLegacyTheme(p)). A rejected
 * save keeps the old applied baseline. cancel() handles Escape/close/discard.
 * Subscribe once at the application level and dispose on window teardown. */
export function createAppearanceCoordinator(options: CoordinatorOptions = {}) {
  let applied = normalizeAppearancePreferences(options.initial, options.legacyTheme);
  let draft = { ...applied };
  let pending: Promise<void> | null = null;
  let disposed = false;
  const listeners = new Set<(snapshot: AppearanceSnapshot) => void>();
  const media = options.media ?? (typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia('(prefers-color-scheme: dark)') : undefined);
  let systemDark = media?.matches ?? false;
  const paint = options.apply ?? applyAppearancePreferences;
  const same = (a: AppearancePreferences, b: AppearancePreferences) =>
    a.mode === b.mode && a.darkPalette === b.darkPalette && a.lightPalette === b.lightPalette && a.accent === b.accent;
  const getSnapshot = (): AppearanceSnapshot => ({ applied: { ...applied }, draft: { ...draft }, previewing: !same(applied, draft), saving: pending !== null });
  const emit = () => { if (!disposed) for (const listener of listeners) listener(getSnapshot()); };
  const render = () => { if (!disposed) { paint({ ...draft }, systemDark); emit(); } };
  const onSystem = (event: MediaQueryListEvent) => {
    systemDark = event.matches;
    if (draft.mode === 'system') render();
  };
  media?.addEventListener('change', onSystem);
  render();

  return {
    getSnapshot,
    subscribe(listener: (snapshot: AppearanceSnapshot) => void) {
      listeners.add(listener);
      listener(getSnapshot());
      return () => { listeners.delete(listener); };
    },
    preview(patch: Partial<AppearancePreferences>) {
      draft = normalizeAppearancePreferences({ ...draft, ...patch });
      render();
    },
    reset() { draft = { ...DEFAULT_APPEARANCE_PREFERENCES }; render(); },
    cancel() { draft = { ...applied }; render(); },
    /** Use after startup/config sync; external persisted state becomes baseline. */
    replace(value: unknown, legacyTheme?: unknown) {
      applied = normalizeAppearancePreferences(value, legacyTheme);
      draft = { ...applied };
      render();
    },
    commit(persist: (preferences: AppearancePreferences) => Promise<unknown>): Promise<void> {
      if (pending) return pending;
      const saved = { ...draft };
      let operation: Promise<unknown>;
      try { operation = Promise.resolve(persist({ ...saved })); }
      catch (error) { operation = Promise.reject(error); }
      pending = operation.then(() => { applied = saved; }).finally(() => { pending = null; render(); });
      emit();
      return pending;
    },
    dispose() {
      if (disposed) return;
      // Do not leave an uncommitted preview on screen after owner teardown.
      draft = { ...applied };
      paint({ ...draft }, systemDark);
      disposed = true;
      media?.removeEventListener('change', onSystem);
      listeners.clear();
    },
  };
}
