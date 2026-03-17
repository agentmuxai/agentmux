# Spec: Port AppImage Launch Fixes to SolidJS

**Status:** Required — app does not open on Linux AppImage
**Date:** 2026-03-14
**Component:** `frontend/tauri-bootstrap.ts`, `frontend/wave.ts`, `frontend/app/store/contextmenu.ts`
**Tracks:** Issues first fixed for React in PR #116, now regressed after SolidJS migration (PR #120)

---

## 1. Background

Three inter-related bugs were fixed for the React build in PR #116. The SolidJS migration (PR #120) rewrote the bootstrap sequence and reintroduced all three. The result: the AppImage silently fails to show a window on Linux/WebKitGTK.

---

## 2. Root Causes

### 2.1 Dynamic `import()` hangs in WebKitGTK (PRIMARY — window never opens)

**File:** `frontend/tauri-bootstrap.ts` line ~159

```typescript
// CURRENT — BROKEN:
await import("./wave");
```

WebKitGTK's JS engine cannot resolve dynamic `import()` calls over the `tauri://` protocol in production builds. The Promise hangs indefinitely, so `wave.ts` never loads, `initBare()` never runs, the window stays hidden, and the app appears to not open.

**Fix (React version, PR #116):**
Changed to a static top-level import, and called `initBare()` explicitly:

```typescript
// tauri-bootstrap.ts
import { initBare } from "./wave";   // static — resolved at bundle time, no protocol issue
// ...
await initBare();                    // called explicitly after setupTauriApi()
```

`initBare()` must be exported from `wave.ts` and called by `tauri-bootstrap.ts` instead of being triggered by the `DOMContentLoaded` event guard at the bottom of `wave.ts`.

---

### 2.2 Module-level `getApi()` calls in `wave.ts` crash on static import

**File:** `frontend/wave.ts` lines 33–35

```typescript
// CURRENT — will crash when statically imported (before window.api exists):
const platform = getApi().getPlatform();
const appVersion = getApi().getAboutModalDetails().version;
document.title = `AgentMux ${appVersion}`;
```

These execute immediately when the module is evaluated. With a dynamic import (current broken state) this runs after `setupTauriApi()` so `window.api` exists. With the static import fix (2.1), the module evaluates before `setupTauriApi()` runs — `window.api` is null, `getApi()` throws, module evaluation aborts, and nothing loads.

**Fix:** Move these into `initBare()` so they run after `setupTauriApi()`:

```typescript
// wave.ts — module level: no getApi() calls
let platform: string;
let appVersion: string;

export async function initBare() {
    // NOW safe — window.api exists, set by setupTauriApi() in tauri-bootstrap.ts
    platform = getApi().getPlatform();
    appVersion = getApi().getAboutModalDetails().version;
    document.title = `AgentMux ${appVersion}`;
    // ... rest of initBare
}
```

All downstream code that uses `platform` or `appVersion` must work with the deferred assignment. Since they are only accessed from within `initBare()` and functions it calls (`initTauriWave`, `initWave`, etc.), this is safe.

---

### 2.3 `ContextMenuModel` calls `getApi()` at module instantiation

**File:** `frontend/app/store/contextmenu.ts` lines 9–11, 57

```typescript
// CURRENT — broken:
class ContextMenuModelType {
    constructor() {
        getApi().onContextMenuClick(this.handleContextMenuClick.bind(this)); // ← module-level getApi()
    }
}
const ContextMenuModel = new ContextMenuModelType(); // ← instantiated at module level (line 57)
```

When `wave.ts` is statically imported (fix 2.1), its transitive imports are also evaluated before `setupTauriApi()`. `contextmenu.ts` is a transitive import of `wave.ts` via `global.ts`. The constructor call on line 57 fires during module evaluation, calling `getApi()` when `window.api` doesn't exist yet. This throws and aborts module loading.

**Fix (React version, PR #116):**
Add an `init()` method, defer the `getApi()` call, and call `init()` from `initBare()` after `setupTauriApi()`:

```typescript
// contextmenu.ts
class ContextMenuModelType {
    handlers: Map<string, () => void> = new Map();

    // No getApi() in constructor — safe to instantiate at module level
    constructor() {}

    // Called from wave.ts:initBare() after window.api is ready
    init() {
        getApi().onContextMenuClick(this.handleContextMenuClick.bind(this));
    }

    // ... rest unchanged
}

const ContextMenuModel = new ContextMenuModelType(); // safe — no getApi() in ctor
```

In `wave.ts:initBare()`, after `setupTauriApi()` has run (guaranteed because `tauri-bootstrap.ts` calls it before calling `initBare()`):

```typescript
import { ContextMenuModel } from "@/app/store/contextmenu";

export async function initBare() {
    // window.api is guaranteed to exist here
    ContextMenuModel.init();
    // ...
}
```

---

## 3. Dependency Order

The three fixes must be applied together. They form a chain:

```
tauri-bootstrap.ts
  1. setupTauriApi()          ← installs window.api
  2. initBare()               ← imported statically from wave.ts (Fix 2.1)
      ├── ContextMenuModel.init()  ← registers handler now that window.api exists (Fix 2.3)
      ├── platform = getApi().getPlatform()   ← safe now (Fix 2.2)
      ├── appVersion = getApi()...version     ← safe now (Fix 2.2)
      └── ... rest of init sequence
```

Applying only Fix 2.1 (static import) without 2.2 and 2.3 will crash on module evaluation.
Applying only 2.2 and 2.3 without 2.1 leaves the dynamic import hang in place.

---

## 4. Key Rule for All Future Development

> **No module imported (directly or transitively) by `wave.ts` may call `getApi()` at module evaluation time.**
>
> All `getApi()` calls must be inside functions, and those functions must only be called after `initBare()` runs.

This constraint is enforced by the static import: any violation will throw immediately on startup and produce a visible error, making violations easy to catch.

---

## 5. Additional Context: `DOMContentLoaded` Guard in `wave.ts`

Currently at the bottom of `wave.ts`:

```typescript
if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", initBare);
} else {
    initBare();
}
```

With the static import fix, `tauri-bootstrap.ts` calls `initBare()` directly. This guard becomes dead code for Tauri builds. It should be removed or conditioned on `isTauri` to avoid double-calling `initBare()`.

---

## 6. Window Centering (Already Present — Verify Survives)

`wave.ts` already has the Linux window centering fix:

```typescript
await currentWindow.show();
if (platform === "linux") {
    await currentWindow.center();
}
await currentWindow.setFocus();
```

After Fix 2.2 moves `platform` into `initBare()`, verify this code still has access to `platform`. Since it runs inside `initTauriWave()` which is called from `initBare()`, and `platform` is assigned at the start of `initBare()`, it will be in scope.

The `core:window:allow-center` capability in `src-tauri/capabilities/default.json` must also remain present (it was added in PR #116 and should still be there).

---

## 7. `WEBKIT_DISABLE_DMABUF_RENDERER` (AppRun — Already Present — Verify)

The Taskfile.yml custom AppRun script sets `WEBKIT_DISABLE_DMABUF_RENDERER=1`. Verify this is present in the built AppImage:

```bash
unzip -p target/release/bundle/appimage/AgentMux_*.AppImage AppRun | grep WEBKIT
```

Should print `export WEBKIT_DISABLE_DMABUF_RENDERER=1`. If missing, the window will never appear regardless of the JS fixes.

---

## 8. Files to Change

| File | Change |
|------|--------|
| `frontend/tauri-bootstrap.ts` | Replace `await import("./wave")` with `import { initBare } from "./wave"` (static); call `await initBare()` explicitly after `setupTauriApi()` |
| `frontend/wave.ts` | Export `initBare`; move `const platform` and `const appVersion` inside `initBare()`; remove/guard `DOMContentLoaded` self-start at bottom of file; call `ContextMenuModel.init()` inside `initBare()` |
| `frontend/app/store/contextmenu.ts` | Remove `getApi()` from constructor; add `init()` method that registers the click handler |

---

## 9. Verification

After applying fixes:

1. `task package` → `task desktop`
2. Launch AppImage — window must appear within 3 seconds
3. Right-click any pane header → context menu must appear at cursor position
4. Backspace in terminal must work (Canvas renderer — separate fix, already present)
5. Window must be centered on first launch (Linux)
6. Dev build (`task dev`) must continue to work — `initBare()` must still be callable from both `tauri-bootstrap.ts` (Tauri) and the `DOMContentLoaded` path (non-Tauri/dev)

---

## 10. References

- PR #116: Linux AppImage fixes for React — commits `f5bf097`, `5ec9d58`, `703e10d`
- PR #120: SolidJS migration — commit `f030661`
- Memory: `/home/snowbark/.claude/projects/-home-snowbark/memory/` — `AppImage Window Not Appearing — ROOT CAUSE FOUND`
- Tauri WebKitGTK dynamic import restriction: protocol `tauri://` does not support ES dynamic import in production WebView2/WebKitGTK
