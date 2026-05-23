// Tabby v2 frontend entry. Mounts the Svelte app and applies global styles.

import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';

const target = document.getElementById('root');
if (!target) throw new Error('#root not found');

mount(App, { target });
