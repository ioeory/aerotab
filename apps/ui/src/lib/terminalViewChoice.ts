/** Session-scoped views of ONE Rust core, never persisted to global settings. */
export type SharedCoreView = 'native' | 'native-surface';
export type TerminalViewChoices = Record<string, SharedCoreView>;
export async function chooseTerminalView(
  choices: TerminalViewChoices,
  sessionId: string,
  actual: string,
  capabilities: () => Promise<{ available: boolean; geometryUnit?: string; reason?: string }>,
): Promise<TerminalViewChoices> {
  if (actual !== 'native' && actual !== 'native-surface') throw new Error('legacy_renderer: use terminal settings for legacy xterm');
  if (actual === 'native') {
    const caps = await capabilities();
    if (!caps.available || caps.geometryUnit !== 'css-px') {
      throw new Error(`native_surface_unavailable: ${caps.reason ?? 'unsupported host capability/geometry'}`);
    }
  }
  return { ...choices, [sessionId]: actual === 'native' ? 'native-surface' : 'native' };
}
