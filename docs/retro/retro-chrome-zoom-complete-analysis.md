# Complete Analysis: Chrome Zoom Widget Shift Regression

**Date:** 2026-03-19
**Issue:** Widgets shift left when chrome zoom > 1.0 on Windows

---

## The Complete History

### Phase 1: Original Implementation (PR #86, `ec051a6`) — WORKED

```typescript
// zoom.ts — simple, only sets --zoomfactor
function applyChromeZoomCSS(factor: number): void {
    document.documentElement.style.setProperty("--zoomfactor", String(factor));
}
```

```scss
// window-header.scss — 100vw width, CSS zoom scales it
width: 100vw;
zoom: var(--zoomfactor);
```

**Result:** Widgets shifted left at zoom > 1.0. The inner coordinate space shrank by 1/zoom, pushing flex children toward center.

### Phase 2: Width Compensation Fix (`7d74960`) — WORKED ON WINDOWS

```scss
// window-header.scss — compensate for zoom shrinking inner space
width: calc(100vw / var(--zoomfactor, 1));
zoom: var(--zoomfactor);
```

**Why it worked:** `calc(100vw / var(--zoomfactor, 1))` is evaluated by the CSS engine in the zoom context. At zoom 1.5, `100vw` = viewport width, divided by 1.5 = smaller width. Then CSS zoom scales it back up by 1.5x = exactly fills the viewport. Widgets stay pinned right.

**This was the last known WORKING state for Windows.** Pure CSS, no JS platform branching, no `--chrome-header-width` variable.

### Phase 3: Linux Fix (PR #175, `4687ab6`) — BROKE THE ARCHITECTURE

**Problem on Linux:** WebKitGTK handles CSS `zoom` differently — it does NOT divide flex children's layout space by the zoom factor. So `calc(100vw / var(--zoomfactor))` made the header too narrow on Linux.

**Fix:**
```typescript
// zoom.ts — NEW: moved width logic to JS with platform branching
function applyChromeZoomCSS(factor: number): void {
    document.documentElement.style.setProperty("--zoomfactor", String(factor));
    const headerWidth = (PLATFORM === PlatformLinux || factor <= 1)
        ? "100vw"
        : `calc(100vw / ${factor})`;  // ← THIS IS THE BUG
    document.documentElement.style.setProperty("--chrome-header-width", headerWidth);
}
```

```scss
// window-header.scss — now reads from CSS variable
width: var(--chrome-header-width, 100vw);
zoom: var(--zoomfactor);
```

**THE CRITICAL BUG:** `calc(100vw / ${factor})` is NOT the same as `calc(100vw / var(--zoomfactor, 1))`.

- `calc(100vw / var(--zoomfactor, 1))` — The browser evaluates `100vw` and `var(--zoomfactor)` in the CSS cascade, in the context of the zoomed element. The CSS engine handles the relationship between the viewport unit and the zoom factor correctly.
- `calc(100vw / 1.5)` — A static string set via `style.setProperty()`. The value `1.5` is baked in. The browser evaluates `100vw` in the custom property context (`:root`), not in the zoomed `.window-header` context. The denominator is a literal, not a live reference to the zoom factor.

**Additionally:** `PLATFORM` is imported from `platformutil.ts` where it defaults to `"darwin"`. The import `import { PLATFORM } from "@/util/platformutil"` may not be a live binding after Vite/Rollup bundling. If the bundler inlines the value at build time, `PLATFORM` is permanently `"darwin"` (macOS) — causing Windows to take the macOS code path.

### Phase 4: macOS Agent Changes (PR #177) — Made It Worse

- Added `transform: scale()` override for macOS (`.platform-darwin &`)
- This was reverted in `6764a27` because transform doesn't affect layout box

### Phase 5: macOS Width Fix (PR #178) — Correct for macOS Only

- Added macOS-specific branch: `--chrome-header-width: 100%`
- Windows branch unchanged (still `calc(100vw / ${factor})`)

### Phase 6: Windows Fix Attempt (PR #179) — STILL BROKEN

- Changed Windows to `removeProperty("--chrome-header-width")` so CSS fallback kicks in
- CSS changed to `var(--chrome-header-width, calc(100vw / var(--zoomfactor, 1)))`
- **This should have worked** — on Windows, the CSS fallback is the original working formula

**BUT:** The `PLATFORM` variable issue may cause Windows to take the macOS branch (`100%`) instead of the Windows branch (`removeProperty`). If `PLATFORM === PlatformMacOS` is true on Windows (due to the default or bundler inlining), then `--chrome-header-width` is set to `100%`, and the CSS fallback never kicks in.

---

## Root Cause: `PLATFORM` Default Is "darwin"

```typescript
// platformutil.ts line 8
export let PLATFORM: NodeJS.Platform = PlatformMacOS; // "darwin"
```

`zoom.ts` imports `PLATFORM` directly:
```typescript
import { PLATFORM, PlatformLinux, PlatformMacOS } from "@/util/platformutil";
```

There are TWO possible failure modes:

### Failure Mode A: Bundler Inlining
Vite/Rollup may inline the `let` export as a constant during production builds. If so, `PLATFORM` is permanently `"darwin"` in the built bundle, regardless of `setPlatform()` calls at runtime.

### Failure Mode B: Module Evaluation Order
Even with live bindings, if `applyChromeZoomCSS` runs during `initChromeZoom()` (called during app init), and `setPlatform()` hasn't been called yet, `PLATFORM` is still `"darwin"`. The init call uses `DEFAULT_ZOOM = 1.0`, which takes the `factor <= 1` branch (`100vw`) — so this specific call is fine. But if any other code path triggers `applyChromeZoomCSS` before platform is set, it would take the macOS path.

### Failure Mode C: Live Binding Works But CSS Var Set Wrong
Even if `PLATFORM` is correct at runtime, the Windows branch calls `removeProperty("--chrome-header-width")`. If another code path (or the init call) previously set `--chrome-header-width` to `100vw` (via the `factor <= 1` branch), and `removeProperty` doesn't correctly unset it, the stale value persists.

---

## The Fix

Replace `PLATFORM` direct import with function calls `isLinux()` / `isMacOS()`:

```typescript
import { isLinux, isMacOS } from "@/util/platformutil";

function applyChromeZoomCSS(factor: number): void {
    document.documentElement.style.setProperty("--zoomfactor", String(factor));
    if (isLinux() || factor <= 1) {
        document.documentElement.style.setProperty("--chrome-header-width", "100vw");
    } else if (isMacOS()) {
        document.documentElement.style.setProperty("--chrome-header-width", "100%");
    } else {
        document.documentElement.style.removeProperty("--chrome-header-width");
    }
}
```

**Why this works:**
- `isLinux()` and `isMacOS()` are **function calls** — they read `PLATFORM` at call time, not import time
- The bundler cannot inline a function return value — it must be evaluated at runtime
- By the time the user zooms (the only time `factor > 1`), `setPlatform()` has definitely run
- The CSS fallback `calc(100vw / var(--zoomfactor, 1))` is the original working formula for Windows

---

## Verification

To verify this is the issue, add a temporary log:

```typescript
function applyChromeZoomCSS(factor: number): void {
    console.log("[zoom] PLATFORM =", PLATFORM, "isLinux =", isLinux(), "isMacOS =", isMacOS());
    // ...
}
```

If `PLATFORM` logs as `"darwin"` on Windows but `isMacOS()` returns `false`, the bundler is inlining the `PLATFORM` export and the function call fix is correct.

---

## Lessons Learned (Again)

1. **NEVER import mutable `let` exports directly for platform checks.** Always use accessor functions (`isLinux()`, `isMacOS()`, `isWindows()`). This is documented in platformutil.ts line 7 but ignored:
   ```typescript
   /** @deprecated Use getPlatform(), isMacOS(), isLinux(), isWindows() instead.
    * Direct reads at module scope capture the default "darwin" before setPlatform() runs. */
   export let PLATFORM: NodeJS.Platform = PlatformMacOS;
   ```

2. **The comment warning was already there** and was ignored by the Linux agent (PR #175) which imported `PLATFORM` directly.

3. **Moving working CSS to JS platform branching introduced this bug.** The original `calc(100vw / var(--zoomfactor, 1))` worked on Windows purely in CSS. The Linux fix should have used a CSS-only override (`.platform-linux .window-header { width: 100vw }`) instead of JS branching.

4. **The `PLATFORM` default of `"darwin"` is a recurring source of bugs.** It should be changed to `undefined` or `"unknown"` to make incorrect usage fail loudly.
