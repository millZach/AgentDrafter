// The viewer's page is built into dist/, which the agentdrafter binary
// embeds and serves (docs/viewer.md). `npm run dev` serves it from Vite
// instead, with the commands sent on to `agentdrafter serve` on the local
// machine.
import { defineConfig } from 'vite';

export default defineConfig({
  base: '/',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // The WebAssembly module stays a file of its own, never inlined.
    assetsInlineLimit: 0,
    target: 'es2022',
    sourcemap: false,
  },
  server: {
    proxy: { '/commands': 'http://127.0.0.1:7316' },
  },
});
