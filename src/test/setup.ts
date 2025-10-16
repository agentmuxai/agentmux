import { expect, afterEach } from 'vitest';
import { cleanup } from '@solidjs/testing-library';
import '@testing-library/jest-dom/vitest';

// Cleanup after each test
afterEach(() => {
  cleanup();
});

// Mock Tauri API for tests
globalThis.__TAURI_INTERNALS__ = {
  invoke: async () => {},
  transformCallback: () => {},
  postMessage: () => {},
} as any;

// Mock window.matchMedia
Object.defineProperty(window, 'matchMedia', {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => {},
  }),
});
