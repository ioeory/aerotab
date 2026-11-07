import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';

const diagnosticSource = await readFile(new URL('./diagnostics.svelte.ts', import.meta.url), 'utf8');
const appSource = await readFile(new URL('../App.svelte', import.meta.url), 'utf8');
function functionSource(source, name) {
  const ast = ts.createSourceFile('test.ts', source, ts.ScriptTarget.Latest, true);
  const declaration = ast.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === name);
  assert.ok(declaration, `missing ${name}`);
  return declaration.getText(ast).replace(/^export\s+/, '');
}
const helpers = ['sanitizeMessage', 'categoryForStatus'].map(name => functionSource(diagnosticSource, name)).join('\n');
function compile(source, bindings = {}) {
  const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  return new Function(...Object.keys(bindings), js)(...Object.values(bindings));
}

test('status classification accepts rejected Error objects and other non-string values', () => {
  const classify = compile(`${helpers}\nreturn categoryForStatus;`);
  assert.equal(classify(new Error('SFTP unavailable')), 'transfer');
  for (const value of [null, undefined, 42, {}, new SyntaxError('Unexpected end of JSON input')]) {
    assert.equal(classify(value), 'app');
  }
});

test('settings close rejection reports a sanitized string without throwing a second error', async () => {
  const script = appSource.slice(appSource.indexOf('>') + 1, appSource.indexOf('</script>'));
  const records = [];
  const statuses = [];
  const onError = compile(`${helpers}\n${functionSource(script, 'onError')}\nreturn onError;`, {
    diagnostics: { record: (...args) => records.push(args) },
    setStatus: (...args) => statuses.push(args),
  });
  await Promise.reject(new Error('settings unavailable token=hidden')).catch(onError);
  assert.deepEqual(records, [['settings', 'status', 'settings unavailable token=[redacted]', 'error']]);
  assert.deepEqual(statuses, [['settings unavailable token=[redacted]', 'error', 6000]]);
});
