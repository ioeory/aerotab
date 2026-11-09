// Local UI fixture: mock providers and terminal only; no external requests or credentials.
import { mount } from 'svelte';
import Harness from './AIHarness.svelte';
import '../src/app.css';
mount(Harness, { target: document.getElementById('app')! });
