// Run: npm run dev -- --port 5185, then open /tests/workbench-harness.html.
// Read window.workbenchHarness: {done, failed, checks, events} for automation.
// Uses real Workbench, NoteEditor/NotesStore and SftpTransferQueue with mock data.
// This does NOT test NativeTerminalView/Tauri compositor visibility or live SFTP IO.
import { mount } from 'svelte';
import Harness from './WorkbenchHarness.svelte';
import '../src/app.css';
mount(Harness, {target:document.getElementById('app')!});
