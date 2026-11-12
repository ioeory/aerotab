import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const picker = readFileSync(new URL('../components/ProfileSelector.svelte', import.meta.url), 'utf8');
const transpile = s => ts.transpileModule(s, { compilerOptions: { target: 99, module: 99 } }).outputText;

function onMountBody() {
  const text = picker.match(/<script[^>]*>([\s\S]*?)<\/script>/)?.[1] ?? '';
  const ast = ts.createSourceFile('picker.ts', text, 99, true, ts.ScriptKind.TS);
  let found = null;
  const visit = node => {
    if (!found && ts.isCallExpression(node) && node.expression.getText(ast) === 'onMount') {
      found = node.arguments[0].getText(ast);
    }
    ts.forEachChild(node, visit);
  };
  visit(ast);
  assert.ok(found, 'ProfileSelector registers onMount');
  return found;
}

test('picker focuses the search input on open without waiting for profile/discover RPCs', async () => {
  const pending = new Promise(() => {});
  const focused = [];
  const env = {
    rpc: { call: () => pending },
    getProfileCatalog: () => ({ refresh: () => pending, profiles: [] }),
    tick: () => Promise.resolve(),
    scheduleModalFieldFocus: fn => fn(),
    inputEl: { focus: () => focused.push('input') },
    profiles: [], shells: [], sshConfig: [], recentIds: [],
  };
  const mount = new Function(...Object.keys(env), `${transpile(`const mount = ${onMountBody()};`)}\nreturn mount;`)(...Object.values(env));
  mount();
  await new Promise(r => setTimeout(r, 0));
  assert.deepEqual(focused, ['input'], 'search input focused while catalog/discover RPCs are still pending');
});
