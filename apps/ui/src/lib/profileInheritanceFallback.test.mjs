import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('../components/hosts/ProfileEditorForm.svelte', import.meta.url), 'utf8').match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
const ast = ts.createSourceFile('editor.ts', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
function extract(name) {
  const node = ast.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === name);
  assert.ok(node);
  return ts.transpileModule(node.getText(ast), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
}
test('inheriting without a group source leaves each fallback editable', () => {
  const draft = { inherit: ['user', 'port', 'auth', 'proxy_id', 'jump'] };
  const preview = { sources: {} };
  const inherited = new Function('draft', 'inheritedPreview', `${extract('inherited')}; return inherited;`)(draft, preview);
  for (const field of draft.inherit) assert.equal(inherited(field), false);
  preview.sources.auth = 'prod';
  assert.equal(inherited('auth'), true);
  draft.inherit = [];
  assert.equal(inherited('auth'), false);
});
test('forced fields disable local controls even when a legacy host opted out', () => {
  const inherited = new Function('draft', 'inheritedPreview', `${extract('inherited')}; return inherited;`)({ inherit: [] }, { sources: { user: 'prod' }, forced: { user: 'prod' } });
  assert.equal(inherited('user'), true);
  assert.equal(inherited('port'), false);
});
test('submit never resolves a hidden jump draft while the group provides a route', async () => {
  const submit = new Function('inherited', 'joinJumpLines', 'draft', 'saving', 'onError', 'profileFromEditorDraft', 'getProfileCatalog', 'rpc', 'editingId', 'uuidv4', 'onSaved', 'iconKind', 'initialIconKind', 'iconValue', 'initialIconValue', 'jumps', 'originalLines', `${extract('submit')}; return submit;`)(
    field => field === 'jump', () => { throw Error('hidden jump draft must not be read'); },
    { kind: 'ssh', auth: { kind: 'agent' }, icon: null }, false, message => { throw Error(message); },
    () => ({ kind: 'ssh' }), () => ({ upsert: async () => {} }), {}, 'id', () => 'id', () => {},
    'builtin', 'builtin', 'server', 'server', ['@missing'], ''
  );
  await submit({ preventDefault() {} });
});
