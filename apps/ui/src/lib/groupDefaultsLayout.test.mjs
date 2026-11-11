import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parse, compile } from 'svelte/compiler';
import { render } from 'svelte/server';
import { existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import ts from 'typescript';

const source = readFileSync(new URL('../components/hosts/GroupDefaultsModal.svelte', import.meta.url), 'utf8');
const ast = parse(source, { modern: true });
const element = (nodes, name) => nodes.find(node => node.type === 'RegularElement' && node.name === name);
const hasClass = (node, name) => node.attributes.some(attr => attr.name === 'class' && attr.value[0]?.data.split(/\s+/).includes(name));
const declarations = selector => {
  const rule = ast.css.children.find(node => node.type === 'Rule' && source.slice(node.prelude.start, node.prelude.end).trim() === selector);
  assert.ok(rule, `scoped CSS rule ${selector} exists`);
  return rule.block.children.filter(node => node.type === 'Declaration');
};
const style = selector => Object.fromEntries(declarations(selector).map(node => [node.property, node.value.trim()]));

// Optional real layout coverage without adding browser dependencies to the app.
// Run with CHROMIUM_BIN=/path/to/chrome when Chromium is not in this cache.
const chromium = process.env.CHROMIUM_BIN ?? `${process.env.HOME}/.cache/ms-playwright/chromium-1148/chrome-linux/chrome`;
test('real Chromium keeps actions visible in short viewports at shell zoom 0.8–1.5', { skip: !existsSync(chromium) }, async () => {
  const script = source.match(/<script[^>]*>([\s\S]*?)<\/script>/)[1];
  const scriptAst = ts.createSourceFile('modal.ts', script, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const withoutImports = scriptAst.statements.filter(node => !ts.isImportDeclaration(node)).map(node => node.getText(scriptAst)).join('\n');
  const instrumented = source.replace(script, `
    const onDestroy = () => {};
    const i18n = { t: key => key };
    ${withoutImports}
  `);
  const compiled = compile(instrumented, { filename: 'GroupDefaultsModal.svelte', generate: 'server' });
  const code = compiled.js.code.replace(/from (['"])(svelte(?:\/[^'"]*)?)\1/g, (_, quote, name) => `from ${JSON.stringify(import.meta.resolve(name))}`);
  const { default: Modal } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
  const markup = render(Modal, { props: { rpc: {} } }).body;
  const mount = scriptAst.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === 'mountOutsideShell');
  assert.ok(mount, 'production mount action exists');
  const action = ts.transpileModule(mount.getText(scriptAst), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const css = compiled.css.code;
  for (const zoom of [0.8, 1, 1.25, 1.5]) {
    const html = `<html><head><style>body { margin:0 } #root { zoom:${zoom} } dialog { border:1px solid; padding:0 } ${css}</style></head><body><main id="root">${markup}</main><script>
      ${action}
      const dialog = document.querySelector('dialog');
      const cleanup = mountOutsideShell(dialog);
      const fieldset = dialog.querySelector('fieldset');
      for (let i = 0; i < 10; i++) fieldset.appendChild(fieldset.querySelector('section').cloneNode(true));
      dialog.showModal();
      const body = dialog.querySelector('.group-defaults-body');
      const footer = dialog.querySelector('footer');
      const rect = node => { const r = node.getBoundingClientRect(); return { top:r.top, bottom:r.bottom, height:r.height, width:r.width }; };
      const before = rect(footer);
      body.scrollTop = body.scrollHeight;
      const result = { viewport:innerHeight, dialog:rect(dialog), footer:rect(footer), before, scrollTop:body.scrollTop, scrollHeight:body.scrollHeight, clientHeight:body.clientHeight, outside:dialog.parentElement === document.body, buttons:[...footer.querySelectorAll('button')].map(rect) };
      cleanup.destroy();
      result.removed = !dialog.isConnected;
      document.body.textContent = JSON.stringify(result);
    </script></body></html>`;
    const run = spawnSync(chromium, ['--headless', '--no-sandbox', '--disable-gpu', '--no-proxy-server', '--dump-dom', '--window-size=800,600', `data:text/html;base64,${Buffer.from(html).toString('base64')}`], { encoding: 'utf8', timeout: 30000, maxBuffer: 2 * 1024 * 1024 });
    assert.equal(run.status, 0, run.stderr);
    const result = JSON.parse(run.stdout.match(/<body>([\s\S]*?)<\/body>/)[1]);
    assert.ok(result.outside && result.removed, `zoom ${zoom}: mount and cleanup`);
    assert.ok(result.dialog.top >= 0 && result.dialog.bottom <= result.viewport, `zoom ${zoom}: dialog fits viewport`);
    assert.ok(result.scrollHeight > result.clientHeight && result.scrollTop > 0, `zoom ${zoom}: body scrolls`);
    assert.equal(result.before.top, result.footer.top, `zoom ${zoom}: footer remains fixed`);
    for (const button of result.buttons) assert.ok(button.top >= 0 && button.bottom <= result.viewport && button.height > 0, `zoom ${zoom}: action remains visible`);
    console.log(`zoom=${zoom} viewport=${result.viewport} dialog=${result.dialog.height} footerBottom=${result.footer.bottom} scroll=${result.scrollTop}`);
  }
});

test('group defaults escapes shell zoom and removes its detached dialog on destroy', () => {
  const dialog = element(ast.fragment.nodes, 'dialog');
  assert.ok(dialog.attributes.some(attr => attr.type === 'UseDirective' && attr.name === 'mountOutsideShell'));
  assert.match(source, /document\.body\.appendChild\(node\)/);
  assert.match(source, /destroy\(\)\s*\{\s*node\.remove\(\)/);
});

test('dialog and form use scoped border-box viewport limits with dynamic-height fallback', () => {
  const dialog = style('dialog.group-defaults-dialog');
  assert.equal(dialog['box-sizing'], 'border-box');
  assert.equal(dialog.padding, '0');
  assert.equal(dialog.overflow, 'hidden');
  assert.deepEqual(declarations('dialog.group-defaults-dialog').filter(node => node.property === 'max-height').map(node => node.value.trim()), ['calc(100vh - 32px)', 'calc(100dvh - 32px)']);
  const form = style('.group-defaults-form');
  assert.equal(form['box-sizing'], 'border-box');
  assert.equal(form.display, 'flex');
  assert.equal(form['flex-direction'], 'column');
  assert.equal(form['min-height'], '0');
  assert.equal(form['max-height'], 'inherit');
  assert.equal(form.overflow, 'hidden');
});

test('all variable content scrolls independently while the action footer cannot shrink', () => {
  const dialog = element(ast.fragment.nodes, 'dialog');
  const form = element(dialog.fragment.nodes, 'form');
  const body = form.fragment.nodes.find(node => node.type === 'RegularElement' && hasClass(node, 'group-defaults-body'));
  assert.ok(body, 'form has a dedicated scroll region');
  assert.ok(element(body.fragment.nodes, 'fieldset'), 'configuration fields are inside the scroll region');
  assert.ok(element(form.fragment.nodes, 'footer'), 'actions are outside the scroll region');
  const scroll = style('.group-defaults-body');
  assert.equal(scroll['min-height'], '0');
  assert.equal(scroll['overflow-y'], 'auto');
  assert.equal(scroll['overscroll-behavior'], 'contain');
  assert.equal(style('.group-defaults-footer')['flex-shrink'], '0');
});
