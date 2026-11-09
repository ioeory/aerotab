import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

for (const [backend, field] of [['webdav', 'cs-url'], ['git', 'cs-repo'], ['s3', 's3-endpoint'], ['drive', 'drive-client']]) {
  test(`fresh ${backend} component exposes storage before entering authorization`, async () => {
    const server = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true, include: [] }, ssr: { optimizeDeps: { noDiscovery: true, include: [] } }, server: { middlewareMode: true, hmr: false, ws: false }, plugins: [
      { name: 'initial-backend-fixture', enforce: 'pre', transform(source, id) {
        if (id.endsWith('/ConfigSyncSection.svelte')) return source.replace("$state<Backend>('webdav')", `$state<Backend>('${backend}')`);
      } }, svelte(),
    ] });
    try {
      const { default: Component } = await server.ssrLoadModule('/src/components/settings/sections/ConfigSyncSection.svelte');
      const { render } = await server.ssrLoadModule('svelte/server');
      const { body } = render(Component, { props: { rpc: { call: async () => null }, onError() {} } });
      const fieldAt = body.indexOf(`id="${field}"`);
      assert.ok(fieldAt > 0, 'storage input renders at the actual empty initial state');
      // The shared provider storage region must not be hidden at initial step zero.
      assert.ok(body.includes('id="cs-backend"'));
      const before = body.slice(0, fieldAt);
      const ancestors = [];
      for (const match of before.matchAll(/<div\b[^>]*>|<\/div>/g)) {
        if (match[0] === '</div>') ancestors.pop(); else ancestors.push(match[0]);
      }
      assert.ok(ancestors.length > 0);
      assert.ok(ancestors.every(tag => !/\bhidden\b/.test(tag)), 'no storage ancestor hides fresh input');
    } finally { await server.close(); }
  });
}
