// Shared coordinator for the multi-section Settings UI.
//
// Each Section component reads/writes its own sub-tree of settings via
// `settings.get`/`settings.set`. The parent SettingsLayout needs to know
// when any section becomes dirty so the global "unsaved" badge and Save
// button work, and sections must be saved in one click.
//
// Sections register a `save` callback when they mount and unregister on
// destroy. `saveAll()` invokes them sequentially and bumps `rev` so live
// consumers (TerminalPane) re-read settings.

export type SaveFn = () => Promise<void>;

class SettingsCoordinator {
  /** True when any registered section reports unsaved changes. */
  dirty = $state(false);
  /** Notification counter — increment to force live-preview consumers to
   * re-read settings (e.g. TerminalPane font/theme). */
  rev = $state(0);

  #savers = new Map<string, SaveFn>();

  registerSaver(id: string, fn: SaveFn): void {
    this.#savers.set(id, fn);
  }

  unregisterSaver(id: string): void {
    this.#savers.delete(id);
  }

  markDirty(): void {
    this.dirty = true;
  }

  markClean(): void {
    this.dirty = false;
  }

  bumpRev(): void {
    this.rev += 1;
    // Belt-and-braces: also fire a DOM event so consumers that can't depend
    // on cross-module $state tracking (e.g. components mounted outside this
    // module's reactive graph) still see the change.
    if (typeof document !== 'undefined') {
      document.dispatchEvent(new CustomEvent('tabby:settings-changed', { detail: this.rev }));
    }
  }

  async saveAll(): Promise<void> {
    for (const fn of this.#savers.values()) {
      await fn();
    }
    this.dirty = false;
    this.bumpRev();
  }
}

export const settingsCoord = new SettingsCoordinator();
