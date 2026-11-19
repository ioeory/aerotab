import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const source = name => readFileSync(new URL(name, import.meta.url), 'utf8');

test('terminal explicitly stages AI input from the originating pane, for either renderer', () => {
  const pane = source('../components/TerminalPane.svelte');
  const app = source('../App.svelte');
  assert.match(pane, /async function explainSelection\(\)[\s\S]*?selectionText\(\)[\s\S]*?getSelection\(\)[\s\S]*?onAiTroubleshoot\?\.\(id, text\)/);
  assert.match(pane, /async function referenceLastCommand\(\)[\s\S]*?terminal\.lastCommand\.get[\s\S]*?onAiTroubleshoot\?\.\(id,/);
  assert.match(app, /function stageAiTroubleshoot\(id: string, text: string\)[\s\S]*?\.find\(entry => entry\.id === id\)[\s\S]*?troubleshootSeed = /);
});
