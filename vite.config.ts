import { defineConfig } from 'vite';

const host = process.env.TAURI_DEV_HOST;

// Everything is bundled locally: no CDN, no remote fonts or icons.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // explicit IPv4: on Windows "localhost" may resolve to ::1 only and Tauri never connects
    host: host || '127.0.0.1',
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: {
    // WebView2 (Chromium), WKWebView (Safari 15+), WebKitGTK
    target: ['es2021', 'chrome105', 'safari15'],
    outDir: 'dist',
    sourcemap: false,
    cssCodeSplit: false,
  },
});
