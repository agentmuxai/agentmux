import { defineConfig } from 'vite';
import solidPlugin from 'vite-plugin-solid';

// Get version from package.json
import packageJson from './package.json';

// Generate build timestamp
const buildTime = new Date().toLocaleString('en-US', {
  timeZone: 'America/Los_Angeles',
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  hour12: true
}) + ' PT';

export default defineConfig({
  plugins: [solidPlugin()],

  // Define environment variables
  define: {
    'import.meta.env.VITE_APP_VERSION': JSON.stringify(packageJson.version),
    'import.meta.env.VITE_BUILD_TIME': JSON.stringify(buildTime),
  },

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
