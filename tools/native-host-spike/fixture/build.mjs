import { createRequire } from 'node:module';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
const dir = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(path.resolve(dir, '../../../apps/ui/package.json'));
const { compile } = require('svelte/compiler');
const { rolldown: rollup } = await import(require.resolve('rolldown'));
const source = await readFile(path.join(dir, 'Fixture.svelte'), 'utf8');
const compiled = compile(source, { filename: path.join(dir, 'Fixture.svelte'), generate: 'client', css: 'injected' });
const bundle = await rollup({
  input: 'fixture-entry',
  plugins: [{
    name: 'isolated-svelte',
    resolveId(id, importer) {
      if (id === 'fixture-entry' || id === 'fixture-component') return id;
      if (id.startsWith('.') && importer && !importer.startsWith('fixture-')) return path.resolve(path.dirname(importer), id);
      if (id === 'svelte') return path.join(path.dirname(require.resolve('svelte/package.json')), 'src/index-client.js');
      if (id.startsWith('svelte')) return require.resolve(id);
      if (!id.startsWith('.') && !path.isAbsolute(id)) { try { return require.resolve(id); } catch {} }
      return null;
    },
    async load(id) {
      if (id === 'fixture-entry') return "import { FrameTransport } from '"+path.resolve(dir,'../../terminal-handoff-spike/node/frame-transport.mjs')+"'; window.FrameTransport=FrameTransport; import { mount } from 'svelte'; import App from 'fixture-component'; mount(App,{target:document.body});";
      if (id === 'fixture-component') return compiled.js.code;
      if (path.isAbsolute(id)) return readFile(id, 'utf8');
      return null;
    }
  }]
});
const { output } = await bundle.generate({ format: 'iife' });
await mkdir(path.join(dir, '../dist'), { recursive: true });
await writeFile(path.join(dir, '../dist/index.html'), `<!doctype html><meta charset="utf-8"><title>Isolated Svelte native composition fixture</title><body><script>${output[0].code.replaceAll('</script', '<\\/script')}</script>`);
console.log('Compiled isolated Svelte fixture to native-host-spike/dist/index.html');
