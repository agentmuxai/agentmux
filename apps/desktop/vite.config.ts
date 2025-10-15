import { defineConfig } from 'vite';
import solidPlugin from 'vite-plugin-solid';
import buildInfoPlugin from './vite-build-info-plugin.js';

export default defineConfig({
  plugins: [solidPlugin(), buildInfoPlugin()],

  // Tauri expects a fixed port for the dev server
  server: {
    port: 1420,
    strictPort: true,
  },

  // Build configuration for Tauri
  build: {
    target: 'esnext',
    minify: !process.env.TAURI_DEBUG,
    sourcemap: !!process.env.TAURI_DEBUG,
  },

  // Prevent vite from obscuring rust errors
  clearScreen: false,
});
