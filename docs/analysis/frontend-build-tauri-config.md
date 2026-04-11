# Frontend Build Uses Tauri Vite Config

**Date:** 2026-04-02
**Status:** Known issue — not causing immediate breakage but worth addressing
**Discovered during:** Clipboard CEF implementation

## Finding

The `build:frontend` task in `Taskfile.yml` uses `vite.config.tauri.ts`:
```yaml
build:frontend:
    cmd: npx vite build --mode production --config vite.config.tauri.ts
```

There is only one vite config — no CEF-specific variant exists. Since the Tauri host is deprecated and CEF is the primary host, the frontend is being built with a config designed for a different runtime.

## Impact

- Tauri plugin imports (e.g., `@tauri-apps/plugin-clipboard-manager`, `@tauri-apps/plugin-fs`) are bundled but only used via dynamic `import()` with CEF fallbacks
- Tauri-specific Vite plugins may add unnecessary overhead
- No functional breakage — CEF mode is detected at runtime via `__AGENTMUX_IPC_PORT__`

## Recommendation

Either:
1. Rename `vite.config.tauri.ts` → `vite.config.ts` (it's the only config)
2. Or create a `vite.config.cef.ts` that strips Tauri plugins and update Taskfile

Low priority — works as-is.
