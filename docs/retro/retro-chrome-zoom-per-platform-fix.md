# Analysis: Chrome Zoom + DnD Keep Reverting on Windows

**Date:** 2026-03-19
**Problem:** Chrome zoom icons shift left + pane DnD broken on Windows — regresses every time Linux or macOS agent touches shared files

---

## Why It Keeps Reverting

Three agents (Windows, macOS, Linux) all edit the same files:
- `frontend/app/window/window-header.scss` — single file, shared width/zoom rules
- `frontend/app/store/zoom.ts` — `applyChromeZoomCSS()` has platform branching in JS
- `frontend/layout/lib/TileLayout.tsx` — `dragHandle` logic with platform branching

Each agent fixes their platform and breaks others because:
1. They can't test other platforms
2. The fix for one platform's rendering engine contradicts another's
3. The JS platform branching in `zoom.ts` is fragile (`PLATFORM` defaults to `"darwin"`)

### Specific Technical Conflicts

| Platform | CSS `zoom` behavior | Width compensation needed |
|----------|-------------------|-------------------------|
| **Windows** (WebView2/Chromium) | `zoom` divides flex space by factor | `calc(100vw / var(--zoomfactor, 1))` — **must be pure CSS, not JS-set literal** |
| **macOS** (WebKit) | `zoom` divides flex space by factor | `100%` avoids sub-pixel rounding at non-integer zoom |
| **Linux** (WebKitGTK) | `zoom` does NOT divide flex space | `100vw` — no compensation needed |

The JS-set `calc(100vw / ${factor})` (literal number) is NOT equivalent to CSS `calc(100vw / var(--zoomfactor, 1))` on Windows. The CSS engine evaluates the var() version in the zoom context; the JS literal is static.

### DnD Conflict

| Platform | HTML5 DnD behavior | dragHandle setting |
|----------|-------------------|-------------------|
| **Windows** (WebView2) | `draggable="true"` child inside `draggable="false"` parent works | `dragHandle: handle` (header ref) |
| **macOS** (WebKit) | Same as Windows | `dragHandle: handle` (header ref) |
| **Linux** (WebKitGTK) | `draggable="true"` child inside `draggable="false"` parent BROKEN | `dragHandle: undefined` (whole tile) |

---

## Solution: Per-Platform SCSS Files

Split `window-header.scss` into platform-specific files. Each agent owns their file. Shared structure stays in the base file.

### File Structure

```
frontend/app/window/
  window-header.scss           — shared structure (flexbox, height, cursor, etc.)
  window-header.windows.scss   — Windows: calc(100vw / var(--zoomfactor, 1))
  window-header.macos.scss     — macOS: width: 100%
  window-header.linux.scss     — Linux: width: 100vw
```

### How It Works

- Base file has everything EXCEPT `width` and `zoom` on `.window-header`
- Each platform file is scoped under `.platform-win32`, `.platform-darwin`, `.platform-linux`
- Body already gets `platform-${initOpts.platform}` class in `global.ts:194`
- `zoom.ts` `applyChromeZoomCSS()` simplified to ONLY set `--zoomfactor` — no more JS platform branching for width

### Same Pattern for StatusBar

`StatusBar.scss` also has `zoom: var(--zoomfactor)` — apply same split if needed.

### DnD Fix

`TileLayout.tsx` already handles platform branching correctly (PR #182). Verify the `dragHandleRef` polling is actually finding the handle on Windows — add a console.log to confirm.

---

## Implementation: Build-Time Platform File Selection

### Mechanism

1. **Taskfile** sets `VITE_PLATFORM` env var from `{{OS}}`:
   - `windows` → `win32`
   - `darwin` → `darwin`
   - `linux` → `linux`

2. **Vite plugin** (`platformResolve`) rewrites imports with `.platform.` suffix:
   - `import "./foo.platform.scss"` → `./foo.win32.scss` (on Windows build)
   - `import "@/store/zoom.platform"` → `@/store/zoom.win32`

3. **3 copies** of each platform-varying file — each contains ONLY that platform's logic, zero runtime branching.

### Files to Split

| Original | Win32 | Darwin | Linux | Import change |
|----------|-------|--------|-------|---------------|
| `zoom.ts` | `zoom.win32.ts` | `zoom.darwin.ts` | `zoom.linux.ts` | `@/app/store/zoom` → `@/app/store/zoom.platform` |
| `useWindowDrag.ts` | `useWindowDrag.win32.ts` | `useWindowDrag.darwin.ts` | `useWindowDrag.linux.ts` | `@/app/hook/useWindowDrag` → `@/app/hook/useWindowDrag.platform` |
| `window-header.scss` | `window-header.win32.scss` | `window-header.darwin.scss` | `window-header.linux.scss` | `./window-header.scss` → `./window-header.platform.scss` |
| `TileLayout.tsx` | `TileLayout.win32.tsx` | `TileLayout.darwin.tsx` | `TileLayout.linux.tsx` | `./lib/TileLayout` → `./lib/TileLayout.platform` |

### Key Differences Per File

**zoom.ts — `applyChromeZoomCSS()`:**
- Win32: only sets `--zoomfactor` (CSS handles width via `calc(100vw / var(--zoomfactor, 1))`)
- Darwin: sets `--zoomfactor` (same CSS calc works)
- Linux: sets `--zoomfactor` (CSS uses `100vw` — no zoom compensation)

**window-header.scss — `.window-header` width:**
- Win32: `width: calc(100vw / var(--zoomfactor, 1))`
- Darwin: `width: calc(100vw / var(--zoomfactor, 1))`
- Linux: `width: 100vw`

**TileLayout.tsx — dragHandle:**
- Win32: `dragHandle: handle` (header-only drag)
- Darwin: `dragHandle: handle` (header-only drag)
- Linux: `dragHandle: undefined` (whole-tile drag, WebKitGTK compat)

**useWindowDrag.ts — drag region:**
- Win32: `{ "data-tauri-drag-region": true }`
- Darwin: `{ "data-tauri-drag-region": true }`
- Linux: `{}` (drag handled by Rust GTK motion detection)

### Steps

1. Add `platformResolve` vite plugin to `vite.config.tauri.ts`
2. Add `VITE_PLATFORM` env to Taskfile `dev`, `quickdev`, `start`, `package` tasks
3. Create 12 platform files (3 per original)
4. Update imports to use `.platform.` suffix
5. Delete originals (or keep as `.template` reference)
6. Verify: `bump patch`, `task build:backend`, `task package:portable`
