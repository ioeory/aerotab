import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const source = name => readFileSync(new URL(name, import.meta.url), 'utf8');

test('terminal explicitly stages AI input from the originating pane, for either renderer', () => {
  const pane = source('../components/TerminalPane.svelte');
  const app = source('../App.svelte');
  assert.match(pane, /async function explainSelection\(side\?[\s\S]*?selectionText\(\)[\s\S]*?getSelection\(\)[\s\S]*?onAiTroubleshoot\?\.\(id, text, side \? \{ side, source: 'selection' \}/);
  assert.match(pane, /async function referenceLastCommand\(side\?[\s\S]*?if \(captureId !== id \|\| !canTrackLastCommand\) return;[\s\S]*?terminal\.lastCommand\.get[\s\S]*?source: 'lastCommand'/);
  assert.match(app, /function stageAiTroubleshoot\(id: string, text: string, evidence\?[\s\S]*?\.find\(entry => entry\.id === id\)[\s\S]*?troubleshootSeed = [^;]+evidence/);
});
