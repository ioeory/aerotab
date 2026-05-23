import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';

// Tauri webview entrypoint. Builds the Svelte + Tailwind UI into dist/.
export default defineConfig({
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: 'es2022', sourcemap: true, outDir: 'dist', emptyOutDir: true },
  plugins: [svelte(), tailwindcss()],
});
