/** Keyboard/focus contract shared by workspace menus, without global terminal focus. */
export function workspaceMenu(node: HTMLElement, close: () => void) {
  const previous = node.ownerDocument.activeElement as HTMLElement | null;
  node.classList.add('workspace-menu');
  const items = () => Array.from(node.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'))
    .filter((item) => !item.disabled && !item.hidden);
  for (const item of items()) item.setAttribute('role', 'menuitem');
  items()[0]?.focus({ preventScroll: true });

  function onKeydown(event: KeyboardEvent) {
    event.stopPropagation();
    if (event.key === 'Escape' || event.key === 'Tab') {
      if (event.key === 'Escape') event.preventDefault();
      close();
      return;
    }
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const enabled = items();
    if (!enabled.length) return;
    const current = enabled.indexOf(node.ownerDocument.activeElement as HTMLButtonElement);
    const index = event.key === 'Home' ? 0
      : event.key === 'End' ? enabled.length - 1
      : (current + (event.key === 'ArrowDown' ? 1 : -1) + enabled.length) % enabled.length;
    enabled[index]?.focus({ preventScroll: true });
  }
  node.addEventListener('keydown', onKeydown);
  return {
    destroy() {
      node.removeEventListener('keydown', onKeydown);
      const active = node.ownerDocument.activeElement;
      // An action/dialog may already own focus: never steal it back.
      if (active === node || node.contains(active)) previous?.focus({ preventScroll: true });
    },
  };
}
