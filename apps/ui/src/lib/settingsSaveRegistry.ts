export type SaveFn = () => Promise<void>;
export class SettingsSectionSaveError extends Error {
  readonly sectionId: string;
  constructor(sectionId: string, cause: unknown) {
    super(`${sectionId}: ${cause instanceof Error ? cause.message : String(cause)}`, { cause });
    this.sectionId = sectionId;
  }
}

/** Stable insertion order preserves the existing sections' save dependencies. */
export class SettingsSaveRegistry {
  #savers = new Map<string, SaveFn>();
  register(id: string, save: SaveFn) { this.#savers.set(id, save); }
  unregister(id: string) { this.#savers.delete(id); }
  async save(only?: ReadonlySet<string>) {
    for (const [id, save] of [...this.#savers]) {
      if (only && !only.has(id)) continue;
      try { await save(); }
      catch (error) { throw new SettingsSectionSaveError(id, error); }
    }
  }
}
