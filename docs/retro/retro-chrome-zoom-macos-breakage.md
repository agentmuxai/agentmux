# Retro: macOS Chrome Zoom Fix Broke Windows

**Date:** 2026-03-19
**Severity:** High — chrome zoom completely broken on Windows
**Time to detect:** ~30 minutes (user reported)
**Time to fix:** ~15 minutes (reverted)
**Commits:** `7a26562` (introduced), `6764a27` (reverted)
**PR:** #177

---

## What Happened

The macOS agent (PR #177, commit `7a26562`) attempted to fix a chrome zoom issue on macOS where right-aligned widgets in the window header shifted left at non-integer zoom levels due to sub-pixel rounding in the CSS `zoom` + `calc(100vw / factor)` width compensation formula.

The fix replaced CSS `zoom` with `transform: scale()` for macOS only, using a platform-specific CSS selector `.platform-darwin &`. A new `platform-{os}` class was added to `document.body` during `initGlobal()`.

**The fix broke chrome zoom on Windows.** The exact failure mode needs investigation, but the revert commit (`6764a27`) notes "caused worse layout offsets in multiple directions."

---

## Timeline

1. **`4687ab6`** — Linux agent fixes chrome zoom widget label shift on Linux (PR #175) — works fine
2. **`c92dc8b`** — macOS agent shows custom window buttons, removes `setMovableByWindowBackground`
3. **`7a26562`** — macOS agent adds `.platform-darwin &` transform:scale override — **breaks Windows**
4. **`02deea9`** — PR #177 merged (contains both `c92dc8b` and `7a26562`)
5. **User reports** — chrome zoom broken on Windows
6. **`6764a27`** — Revert: removes the transform:scale override

---

## Root Cause Analysis

### The Change

```scss
// Added in 7a26562:
.platform-darwin & {
    width: 100%;
    zoom: unset;
    transform: scale(var(--zoomfactor, 1));
    transform-origin: left top;
}
```

### Why It Should Have Been Safe

The `.platform-darwin &` selector should only match when `document.body` has the `platform-darwin` class. On Windows, the body gets `platform-win32`, so the rule should not apply.

### Possible Failure Modes

1. **PLATFORM default is "darwin"** — `platformutil.ts:8` has `PLATFORM = PlatformMacOS` as the default. If the platform CSS class is set before `initGlobal()` runs (or if there's a race), `platform-darwin` could briefly appear on all platforms. However, the class is added in `initGlobal()` using `initOpts.platform` (from the actual OS), not the default.

2. **CSS specificity interaction** — The `.platform-darwin &` selector has higher specificity than the base `.window-header` rule. If any other CSS or JS adds `platform-darwin` to the body (even transiently), the override activates. But this shouldn't happen on Windows.

3. **The `transform: scale()` approach itself is flawed** — Even on macOS, `transform: scale()` doesn't behave like `zoom`. Transform doesn't affect the element's layout box — the header would still occupy its original unscaled size in the document flow, while visually appearing scaled. This means:
   - The header's clickable area doesn't match its visual size
   - Adjacent elements (like the tile layout below) wouldn't account for the scaled size
   - At zoom > 1, the header visually overflows its layout box
   - At zoom < 1, there's a visual gap between the header and content below

4. **Build/deploy timing** — The portable build on Windows may have included a stale CSS bundle or had a Vite cache issue. Unlikely but possible.

### Most Likely Root Cause

**Theory 3 is most likely.** The `transform: scale()` approach fundamentally doesn't work as a drop-in replacement for CSS `zoom` because it doesn't affect layout. The revert commit confirms: "caused worse layout offsets in multiple directions."

Even if `.platform-darwin` only matched on macOS, the approach was incorrect. The fact that it reportedly broke Windows suggests either:
- The platform class wasn't being set correctly (body had `platform-darwin` on Windows), OR
- The user tested the macOS build on a Windows-connected session, OR
- There was a different Windows-specific issue exposed by the same PR's other changes (the `system-status.tsx` changes in `c92dc8b`)

---

## What Was Reverted

Commit `6764a27` removed only the `.platform-darwin &` CSS block. The other PR #177 changes remain:
- `platform-{os}` class on body (from `global.ts`) — kept, useful for future platform CSS
- Custom window buttons on macOS (from `c92dc8b`) — kept
- Removed `setMovableByWindowBackground` (from `c92dc8b`) — kept

---

## Lessons Learned

1. **`transform: scale()` is NOT a drop-in replacement for CSS `zoom`** — they have fundamentally different layout semantics. `zoom` affects the element's layout box; `transform` does not.

2. **Platform-specific CSS changes must be tested on ALL platforms before merging** — a `.platform-darwin` selector should be safe, but the underlying approach can still be wrong.

3. **Multi-agent PRs need cross-platform review** — when one agent works on macOS and another on Windows/Linux, changes to shared CSS (especially zoom/layout) need explicit cross-platform testing.

4. **The chrome zoom width compensation formula is fragile** — `calc(100vw / factor)` works on Windows but not Linux, and has sub-pixel rounding issues on macOS. Each platform's WebView handles CSS `zoom` differently. A more robust approach is needed.

---

## Open Issue

The original macOS problem (right-aligned widgets shift at non-integer zoom levels) remains unfixed after the revert. Potential approaches:

1. **Round the zoom factor** to avoid sub-pixel issues: `Math.round(factor * 100) / 100`
2. **Use `will-change: transform`** on the header to promote it to its own compositing layer
3. **Use JavaScript-based positioning** for the right-aligned widgets instead of CSS flex
4. **Accept the sub-pixel shift** on macOS as a minor visual imperfection
