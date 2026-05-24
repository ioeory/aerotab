// AeroTab frontend entry. Mounts the Svelte app and applies global styles.

import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';

const target = document.getElementById('root');
if (!target) throw new Error('#root not found');

// Block the WebView default context menu (Back / Refresh / Print, etc.).
// Components that show an app menu set `data-aerotab-context-menu` on their root.
document.addEventListener(
  'contextmenu',
  (ev) => {
    const el = ev.target;
    if (el instanceof Element && el.closest('[data-aerotab-context-menu]')) return;
    ev.preventDefault();
  },
  { capture: true },
);

mount(App, { target });
