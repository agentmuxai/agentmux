# Retro: Chrome Zoom Cross-Platform Regression Chain

**Date:** 2026-03-19
**Severity:** High — chrome zoom broken on Windows
**Root cause:** Linux fix inadvertently changed Windows zoom behavior by moving CSS logic to JS
**Duration:** Multiple hours, 4 fix attempts across 3 agents

---

## The Working State

**PR #86 (`ec051a6`)** — Original chrome zoom implementation (working on all platforms):

```typescript
// zoom.ts — only sets --zoomfactor
function applyChromeZoomCSS(factor: number): void {
    document.documentElement.style.setProperty("--zoomfactor", String(factor));
}
```

```scss
// window-header.scss — pure CSS compensation
width: calc(100vw / var(--zoomfactor, 1));
zoom: var(--zoomfactor);
```

**How it worked:** CSS `zoom` on `.window-header` scales the element. At `zoom: 1.5`, the element's content area shrinks (browser divides available space by zoom factor). The `calc(100vw / var(--zoomfactor))` compensates by making the element wider, so after zoom it fills the viewport.

**This worked on Windows and macOS.** Linux was untested at this point.

---

## The Regression Chain

### Step 1: Linux Fix — PR #175 (`4687ab6`)

**Agent:** Linux agent (snowbark + Claude Sonnet)
**Problem:** On Linux/WebKitGTK, CSS `zoom` does NOT divide flex children's layout space. The `calc(100vw / zoomfactor)` compensation made the header too narrow, pushing right-aligned widgets left.
**Fix:** Move width logic from CSS to JavaScript. Branch by platform:

```typescript
// zoom.ts — NEW: sets --chrome-header-width from JS
function applyChromeZoomCSS(factor: number): void {
    document.documentElement.style.setProperty("--zoomfactor", String(factor));
    const headerWidth = (PLATFORM === PlatformLinux || factor <= 1)
        ? "100vw"                        // Linux: no compensation
        : `calc(100vw / ${factor})`;     // macOS + Windows: compensate
    document.documentElement.style.setProperty("--chrome-header-width", headerWidth);
}
```

```scss
// window-header.scss — changed from calc() to var()
width: var(--chrome-header-width, 100vw);  // was: calc(100vw / var(--zoomfactor, 1))
zoom: var(--zoomfactor);
```

**What changed for Windows:**
- Before: `width: calc(100vw / var(--zoomfactor, 1))` — CSS evaluates this with the live CSS variable
- After: `width: var(--chrome-header-width)` → set to `calc(100vw / 1.5)` in JS (literal number, not CSS var)

**Subtle difference:** The old CSS `calc(100vw / var(--zoomfactor, 1))` is evaluated by the browser's CSS engine in the context of the zoomed element. The new `calc(100vw / 1.5)` (JS-set) is a static string — the browser may evaluate `100vw` differently when it's in a CSS custom property vs inline calc.

**Additional risk:** `PLATFORM` defaults to `"darwin"` at module load time. If `applyChromeZoomCSS` runs before `setPlatform()` (unlikely for user-triggered zoom, but possible for init), Windows would take the macOS code path.

**Was Windows retested?** No — the PR only tested Linux.

### Step 2: macOS Widget Shift — PR #177, commit `7a26562`

**Agent:** macOS agent (asaf + Claude Opus)
**Problem:** At non-integer zoom levels on macOS, right-aligned widgets shift left due to sub-pixel rounding in `calc(100vw / factor)`.
**Fix attempt:** On macOS only, replace CSS `zoom` with `transform: scale()`:

```scss
.platform-darwin & {
    width: 100%;
    zoom: unset;
    transform: scale(var(--zoomfactor, 1));
    transform-origin: left top;
}
```

Also added `platform-{os}` CSS class to `document.body` in `initGlobal()`.

**Why it broke everything:** `transform: scale()` is NOT equivalent to CSS `zoom`:
- `zoom` affects the element's layout box — children compute sizes relative to the zoomed dimensions
- `transform: scale()` is purely visual — the layout box stays at the unscaled size, creating mismatches between visual appearance and clickable areas
- The header visually overflows/underflows its layout allocation

**Was Windows affected?** Unclear. The `.platform-darwin &` selector should only match on macOS. But the user reported Windows breakage. Possible causes:
1. The `platform-darwin` class was applied on Windows (unlikely — `initOpts.platform` comes from the actual OS)
2. The other changes in the same PR (`system-status.tsx` removing `isMacOS()` guard) affected layout
3. Windows was already broken from Step 1 (Linux fix) and this PR was blamed

### Step 3: Revert — commit `6764a27`

**Agent:** macOS agent
**Action:** Reverted the `.platform-darwin &` CSS block.
**Result:** Removed the transform:scale approach. The `platform-{os}` body class remained (harmless).

### Step 4: macOS-Only Width Fix — PR #178 (`a3aa6ba`)

**Agent:** macOS agent
**Problem:** Same macOS widget shift, but now approach it through `applyChromeZoomCSS` instead of CSS.
**Fix:** Added a macOS-specific branch using `100%` instead of `calc(100vw / factor)`:

```typescript
function applyChromeZoomCSS(factor: number): void {
    document.documentElement.style.setProperty("--zoomfactor", String(factor));
    let headerWidth: string;
    if (PLATFORM === PlatformLinux || factor <= 1) {
        headerWidth = "100vw";                    // Linux: no compensation
    } else if (PLATFORM === PlatformMacOS) {
        headerWidth = "100%";                     // macOS: avoid sub-pixel rounding
    } else {
        headerWidth = `calc(100vw / ${factor})`;  // Windows: compensate
    }
    document.documentElement.style.setProperty("--chrome-header-width", headerWidth);
}
```

**Windows path unchanged** from Step 1. If Windows was already broken, this didn't fix it.

---

## Current State

Windows chrome zoom uses `calc(100vw / ${factor})` set from JavaScript. This is different from the original working state which used `calc(100vw / var(--zoomfactor, 1))` purely in CSS.

**Possible fix:** Restore the original CSS-only approach for Windows:

```scss
// Option A: CSS-only, no JS variable needed on Windows
width: calc(100vw / var(--zoomfactor, 1));
```

Or make the JS set the same value the CSS had:

```typescript
// Option B: JS sets same formula CSS used
headerWidth = `calc(100vw / var(--zoomfactor, 1))`;
```

Or just use `100vw` on Windows too (the original-original behavior before any compensation was added):

```typescript
// Option C: simplest — no compensation on any platform
headerWidth = "100vw";
```

---

## Root Causes

1. **The Linux fix changed Windows behavior without testing Windows.** Moving from CSS `calc(100vw / var(--zoomfactor))` to JS `calc(100vw / ${literalNumber})` is NOT equivalent — CSS custom properties in calc() are evaluated differently than literal values in custom properties.

2. **No cross-platform testing gate.** Each agent tested only their platform. There's no CI that runs zoom tests on all platforms before merging.

3. **`PLATFORM` defaults to `"darwin"`.** Any code that reads `PLATFORM` before `setPlatform()` runs will take the macOS code path on ALL platforms. The zoom init function calls `applyChromeZoomCSS(DEFAULT_ZOOM)` — if this runs before platform is set, Windows gets the macOS behavior.

4. **Cascading fixes.** Each agent "fixed" the issue for their platform without fully understanding the cross-platform implications, leading to 4 commits that made things progressively worse.

---

## Lessons Learned

1. **CSS-only solutions are more robust than JS-driven platform branching** for layout properties. CSS custom properties + calc() works identically everywhere — the browser handles platform differences internally.

2. **Never move working CSS to JS "just to add a platform branch."** The Linux fix should have added a Linux-only CSS override (e.g., `.platform-linux .window-header { width: 100vw }`) rather than rewriting the width logic in JS.

3. **Every PR that touches shared layout CSS must be tested on ALL platforms.** Add a PR checklist item: "Tested chrome zoom on Windows / macOS / Linux."

4. **The `PLATFORM` default of `"darwin"` is a landmine.** Change the default to `undefined` and make `applyChromeZoomCSS` handle the undefined case explicitly, or move the platform check into a function that asserts platform has been set.

5. **Multi-agent development needs a shared regression test.** A simple manual test script ("zoom in 3 steps, verify widgets pinned right, zoom out, verify reset") should be run on each platform after any zoom-related change.

---

## Recommended Fix

**Restore the CSS-only approach for Windows and macOS. Keep JS branching only for Linux:**

```scss
// window-header.scss
.window-header {
    width: calc(100vw / var(--zoomfactor, 1));  // works on Windows + macOS
    zoom: var(--zoomfactor);
}

// Linux override: WebKitGTK doesn't divide flex space by zoom
.platform-linux .window-header {
    width: 100vw;
}
```

```typescript
// zoom.ts — remove --chrome-header-width entirely
function applyChromeZoomCSS(factor: number): void {
    document.documentElement.style.setProperty("--zoomfactor", String(factor));
    // Width compensation handled in CSS. No JS branching needed.
}
```

This returns Windows and macOS to the exact CSS that was working, and uses the platform body class (already in place) to handle the Linux exception.
