interface LayoutNode { readonly parentElement: LayoutNode | null }
interface Rect { x: number; y: number; width: number; height: number }
const barriers = new WeakMap<LayoutNode, (source: LayoutNode) => Promise<{ pane: Rect; slot: Rect }>>();
/** Register the actual PaneGrid owner; no global events or per-frame layout work. */
export function registerPaneLayoutBarrier(root: LayoutNode, settle: (source: LayoutNode) => Promise<{ pane: Rect; slot: Rect }>) {
  barriers.set(root, settle);
  return () => { if (barriers.get(root) === settle) barriers.delete(root); };
}
/** CSS/DPI round-trips may drift by a fraction of a pixel; stale panes differ by whole rows. */
const LAYOUT_EPS_PX = 1;
function layoutMatches(pane: Rect, slot: Rect): boolean {
  return (['x', 'y', 'width', 'height'] as const).every(
    (key) => Number.isFinite(pane[key]) && Number.isFinite(slot[key]) && Math.abs(pane[key] - slot[key]) <= LAYOUT_EPS_PX,
  );
}
/** One explicit cache flush + Svelte DOM commit, then slot/pane verification. */
export async function settlePaneLayout(source: LayoutNode | null): Promise<void> {
  if (!source) throw new Error('pane layout source unavailable');
  for (let node: LayoutNode | null = source; node; node = node.parentElement) {
    const settle = barriers.get(node);
    if (!settle) continue;
    const { pane, slot } = await settle(source);
    if (!layoutMatches(pane, slot)) {
      throw new Error('pane layout is not consistent with current slot');
    }
    return;
  }
  throw new Error('pane layout coordinator unavailable');
}
