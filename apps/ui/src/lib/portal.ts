/** Move a DOM node to `document.body` so `position: fixed` uses the viewport. */
export function portal(node: HTMLElement, target: HTMLElement = document.body) {
  target.appendChild(node);
  return {
    destroy() {
      if (node.parentNode === target) target.removeChild(node);
    },
  };
}
