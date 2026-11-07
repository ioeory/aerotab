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

import { SettingsSaveRegistry, type SaveFn } from './settingsSaveRegistry';
export type { SaveFn } from './settingsSaveRegistry';

class SettingsCoordinator {
  /** True when any registered section reports unsaved changes. */
  dirty = $state(false);
  /** Notification counter — increment to force live-preview consumers to
   * re-read settings (e.g. TerminalPane font/theme). */
  rev = $state(0);

  saving = $state(false);
  failedSection = $state<string | null>(null);
  #savers = new SettingsSaveRegistry();
  #generation = 0;
  #dirtySections = new Set<string>();
  #sectionGenerations = new Map<string, number>();

  registerSaver(id: string, fn: SaveFn): void {
    this.#savers.register(id, fn);
  }

  unregisterSaver(id: string): void {
    this.#savers.unregister(id);
  }

  markDirty(id = '*'): void {
    this.#generation += 1;
    this.#dirtySections.add(id);
    this.#sectionGenerations.set(id, this.#generation);
    this.dirty = true;
  }

  markClean(id?: string): void {
    if (id) this.#dirtySections.delete(id);
    else this.#dirtySections.clear();
    this.dirty = this.#dirtySections.size > 0;
  }

  bumpRev(): void {
    this.rev += 1;
    // Belt-and-braces: also fire a DOM event so consumers that can't depend
    // on cross-module $state tracking (e.g. components mounted outside this
    // module's reactive graph) still see the change.
    if (typeof document !== 'undefined') {
      document.dispatchEvent(new CustomEvent('aerotab:settings-changed', { detail: this.rev }));
    }
  }

  async saveSection(id: string): Promise<void> {
    await this.#save(new Set([id]), id);
  }

  async saveAll(): Promise<void> {
    await this.#save(this.#dirtySections.has('*') ? undefined : new Set(this.#dirtySections));
  }

  async #save(only?: ReadonlySet<string>, sectionId?: string): Promise<void> {
    if (this.saving) return;
    this.saving = true;
    this.failedSection = null;
    const generation = this.#generation;
    const sectionGeneration = sectionId ? this.#sectionGenerations.get(sectionId) : undefined;
    try {
      await this.#savers.save(only);
      if (sectionId) {
        // Unrelated sections may still be edited while this section saves.
        // Only a newer edit in this section prevents clearing its own draft.
        if (sectionGeneration === this.#sectionGenerations.get(sectionId)) this.markClean(sectionId);
      } else if (generation === this.#generation) this.markClean();
      this.bumpRev();
    } catch (error) {
      this.failedSection = (error as { sectionId?: string }).sectionId ?? null;
      throw error;
    } finally {
      this.saving = false;
    }
  }
}

export const settingsCoord = new SettingsCoordinator();
