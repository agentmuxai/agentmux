# Vite Production Build Failure: Windows Path Case Sensitivity

**Status:** Open
**Date:** 2026-04-09
**Severity:** Build-breaking — portable builds cannot be produced
**Affects:** `task cef:package:portable`, `task build:frontend`

## Symptom

```
npx vite build --mode production --config vite.config.ts

Rollup failed to resolve import "@/util/startup-bench"
  from "C:/Systems/agentmux/frontend/tauri-bootstrap.ts"
```

Only 2 modules transform before the failure. The file exists on disk.
`task dev` (Vite HMR server) works fine.

## Root Cause

**Windows path case mismatch between Rollup and vite-tsconfig-paths.**

1. Rollup reports the importer path as `C:/Systems/agentmux/...` (capital S,
   matching the actual directory name `C:\Systems\agentmux`)
2. vite-tsconfig-paths loads tsconfig.json from `C:/systems/agentmux/...`
   (lowercase, as Vite normalizes it)
3. The plugin calls `path.posix.relative(configDir, importerFile)` which is
   **case-sensitive** even on Windows
4. Result: relative path becomes `../../Systems/agentmux/frontend/tauri-bootstrap.ts`
   instead of `frontend/tauri-bootstrap.ts`
5. The plugin's include check (`frontend/**/*`) fails on this path
6. Plugin returns "not applicable", Rollup can't resolve `@/util/*`, build fails

## Why Dev Works

The Vite dev server normalizes paths through its own middleware before
passing them to plugins, so the case mismatch doesn't occur. Rollup in
build mode passes the OS-native cased path directly.

## Fix Options

### Option A: Normalize paths in vite.config.ts (recommended, no dep change)

Add explicit `root` with a normalized path to force consistent casing:

```typescript
import { normalizePath } from "vite";
import path from "path";

export default defineConfig({
    root: normalizePath(path.resolve(".")),
    // ...
});
```

Or set `root` to the lowercase form:

```typescript
root: process.cwd().replace(/\\/g, "/"),
```

### Option B: Configure vite-tsconfig-paths with explicit root

```typescript
tsconfigPaths({ root: normalizePath(path.resolve(".")) }),
```

### Option C: Lowercase the drive path in index.html entry

This is fragile — don't do this.

### Option D: Rename C:\Systems to C:\systems

Nuclear option. Fixes all tools at once but requires updating all
references, PATH entries, and other configs.

### Option E: Patch vite-tsconfig-paths

The bug is in `node_modules/vite-tsconfig-paths/src/path.ts` where
`path.posix.relative` is used for paths that may have different casing
on Windows. A PR could fix this upstream, or a `patch-package` patch
could fix it locally.

## Recommended Fix

**Option A** — add path normalization in vite.config.ts. This is a
one-line change that doesn't depend on upstream fixes:

```typescript
// vite.config.ts line 95
root: path.resolve(".").replace(/\\/g, "/").toLowerCase(),
```

However, lowercasing the entire root may cause issues with other plugins
that expect the original casing. A safer approach:

```typescript
import { normalizePath } from "vite";
// ...
root: normalizePath(path.resolve(".")),
```

If that doesn't work (Vite's normalizePath may not lowercase), try
**Option B** — passing an explicit root to `tsconfigPaths()`.

## Verification

After fix:
```bash
npx vite build --mode production --config vite.config.ts
# Should complete with "X modules transformed" and produce dist/frontend/
```

Then:
```bash
task cef:package:portable
# Should produce dist/agentmux-cef-0.33.72-x64-portable.zip
```

## Impact on Build Pipeline

This bug means **no new portable builds can be produced** until fixed.
The existing `dist/frontend/` from April 9 15:20 is stale (pre-0.33.72)
and lacks the runtime controls, compact results, auto-grow input, and
Ctrl+P keyboard handler.

The Rust binaries (CEF host, backend, wsh) build independently and are
not affected — only the frontend bundle is broken.

## Files to Change

| File | Change |
|------|--------|
| `vite.config.ts` | Normalize root path or configure tsconfigPaths root |

## Related

- vite-tsconfig-paths: https://github.com/aleclarson/vite-tsconfig-paths
- Vite issue tracker: path normalization on Windows
- Node.js `path.posix.relative` is always case-sensitive
