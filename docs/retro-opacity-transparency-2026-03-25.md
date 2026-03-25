# Retro: Window Opacity / Transparency — 2026-03-25

## TL;DR

Opacity does not work on Linux. The flashing-in-another-instance bug is a
broadcast loop: every config change is sent to all windows, and every window
re-applies the native transparency call even when it didn't initiate the change.
The CSS approach that was supposed to power Linux transparency is architecturally
correct but the WebKit RGBA path is unreliable in practice. Recommend two
targeted fixes.

---

## What Was Attempted (Chronological)

### 1. Initial transparency wiring (commit 5b0c721 — 2026-03-06)
`feat: enable full window transparency across all platforms`

- Added `"transparent": true` and `"backgroundColor": "#00000000"` to
  `src-tauri/tauri.conf.json`.
- Created `set_window_transparency` Tauri command in
  `src-tauri/src/commands/window.rs`.
  - macOS: `NSVisualEffectView` / vibrancy via `window-vibrancy` crate. ✓
  - Windows: Mica (Win11) / Acrylic (Win10) fallback. ✓
  - Linux: **log only** — relied entirely on CSS + RGBA visual.
- Frontend `AppSettingsUpdater` (app.tsx) added: sets `--window-opacity` CSS
  var on `<body>`, adds `is-transparent` class to `#main`, sets
  `document.documentElement.style.background = "transparent"`, then invokes
  `getApi().setWindowTransparency(…)`.
- CSS: `body { background: rgba(34,34,34,var(--window-opacity)); }`
- Result: worked on macOS/Windows, worked some of the time on Linux.

### 2. Startup flash fix (commit 8ab87c9 — shortly after)
`fix: prevent startup flash while keeping transparency support`

Addressed the brief white-flash before the window was ready. Not directly
related to ongoing opacity issue.

### 3. Windows-specific fixes (baead62 / 1982f53)
`fix: enable window opacity/transparency on Windows`

Windows-specific fine-tuning. Linux unchanged.

### 4. Opacity submenu added (5f1fbfd — 2026-03-22)
`feat: opacity submenu in widget bar right-click menu`

`createOpacityMenu()` in `frontend/app/menu/base-menus.ts` — radio items
100% → 35% in 5% steps. Calls:
```ts
RpcApi.SetConfigCommand(TabRpcClient, {
    "window:opacity": value,
    "window:transparent": value < 1.0,
});
```

### 5. Current WIP (2026-03-25, this session — uncommitted)
Working on v0.32.82.

**Zoom ghost-pixel fix** (kept, working):
- `zoom.linux.ts`: sets `--zoom-transition-dur: 80ms` in `initChromeZoom()`.
- `block.scss`, `StatusBar.scss`, `window-header.linux.scss`: add
  `transition: zoom var(--zoom-transition-dur, 0ms) ease-out`.
- Forces WebKitGTK to invalidate ghost pixels left behind on zoom-out.

**Linux transparency attempts** (iterated, reverted):
1. Added `window.set_background_color(Some(Color(0,0,0,0)))` — injects GTK CSS
   provider `window { background-color: rgba(0,0,0,0); }` at APPLICATION
   priority (600). **Broke the CSS opacity path.** Reverted.
2. Added `gtk_window.set_opacity(opacity)` — GTK widget opacity via
   `_NET_WM_WINDOW_OPACITY`. Reverted in favour of understanding root cause.
3. Final state: reverted `window.rs` to HEAD (log-only for Linux).

---

## Root Cause Analysis

### Bug 1: Opacity has no effect on Linux

**Why the CSS approach should work:**
- `tauri.conf.json` sets `transparent: true` → tao allocates an RGBA X11
  visual and installs a Cairo draw handler that fills the GTK window surface
  with `rgba(0,0,0,0)` (transparent black, `Operator::Source`) on every repaint.
- wry creates the WebKitWebView with `webkit_web_view_set_background_color(0,0,0,0)`.
- `AppSettingsUpdater` sets `--window-opacity: 0.8` on `<body>` →
  `body { background: rgba(34,34,34,0.8) }`.
- If the GTK surface is truly transparent, WebKit's RGBA pixels should reach the
  Mutter compositor (confirmed: `_NET_WM_WINDOW_OPACITY` in `_NET_SUPPORTED`,
  Xwayland depth-32 visuals available, `WEBKIT_DISABLE_DMABUF_RENDERER=1` forces
  SHM renderer which supports alpha).

**Why it probably doesn't work:**
- `body` has `transform: translateZ(0)` and `backface-visibility: hidden` in
  `app.scss`. These create a GPU compositing layer inside WebKit. WebKit may
  mark a layer as *opaque* if its backing element has an explicit background
  colour (even `rgba(34,34,34,0.8)`). Once marked opaque the alpha channel is
  dropped before the surface is handed to GTK.
- This is a known WebKitGTK limitation: hardware-accelerated layers containing
  opaque-looking CSS backgrounds are often promoted to opaque tiles, silently
  discarding alpha.

**Simple diagnostic**: temporarily remove `transform: translateZ(0)` from
`body` in `app.scss` and check if transparency appears. If yes, that is the
culprit.

### Bug 2: Setting opacity in one window flashes other instances

**Architecture:**
- Settings (`window:opacity`, `window:transparent`) are stored in a single
  global `settings.json` shared across all instances.
- Backend `SetConfigCommand` handler calls `broadcast_event()` with **no scope
  filtering** — every connected WebSocket client receives every config change.
- Frontend `global.ts` subscribes to "config" events with **no scope** field →
  every instance receives every broadcast.
- `AppSettingsUpdater` reacts via `createEffect()` and immediately calls
  `getApi().setWindowTransparency()` — a Tauri invoke that modifies the OS-level
  window.

**Result:** setting opacity in Window A →
1. backend broadcasts to Window A *and* Window B
2. both run `AppSettingsUpdater`
3. both call `setWindowTransparency(true, false, 0.8)`
4. Window B flashes: it changes from its current state → new opacity → (no
   restore, it just stays at new value)

The "flashing on and off" the user sees is probably Window B toggling between
the old and new state because the effect runs twice (once for the old atom value
still in flight, once for the new) — a React/SolidJS double-render artefact
combined with the Tauri invoke being async.

---

## What Was NOT Tried

- Removing `transform: translateZ(0)` / `backface-visibility: hidden` from
  `body` (most promising Linux fix, low risk).
- Using `gtk_widget_set_opacity()` *without* the double-apply problem: would
  need the frontend to skip the CSS `--window-opacity` path on Linux (or set it
  to 1.0) and rely solely on the GTK compositor-level opacity.
- Debouncing / equality-checking before calling `setWindowTransparency` so
  unchanged-opacity windows don't re-invoke the native API.
- Per-window opacity settings (complex, probably not desired).

---

## Best Practices (Research)

### WebKitGTK transparency
- Call `webkit_web_view_set_background_color` with RGBA(0,0,0,0) at creation
  (wry already does this when `transparent=true`).
- Avoid CSS properties that trigger GPU layer promotion on elements that need
  alpha (`transform`, `will-change`, `filter`, `backface-visibility`). Put those
  on child elements, not `body`.
- Test with `WEBKIT_DISABLE_DMABUF_RENDERER=1` — forces SHM renderer which
  preserves alpha. (Already required on this system anyway.)
- `_NET_WM_WINDOW_OPACITY` via `gtk_widget_set_opacity()` is the most reliable
  fallback: it bypasses WebKit's internal compositing entirely and lets Mutter
  handle dimming. Trade-off: all window content (text, UI) dims uniformly, same
  as alacritty/kitty/gnome-terminal opacity behaviour.

### Multi-window config broadcast
- Reactive effects that invoke native OS APIs should guard with a prev-value
  equality check (or use SolidJS `on()` with deferred option).
- Alternatively, the native API call can live in a separate, deduplicated
  effect: only fire the Tauri invoke when the values differ from last time.
- Config broadcasts that affect per-window OS state (transparency, window size,
  title) ideally should carry a `scope: windowId` so only the target window
  reacts. Current "config" events are deliberately unsroped for simplicity —
  either add per-window config namespacing or add the guard on the call site.

---

## Recommended Fix Plan

### Fix A — Linux opacity (try in order, stop when working)

**A1 (try first, 5 min):** Remove `transform: translateZ(0)` and
`backface-visibility: hidden` from `body` in `frontend/app/app.scss`. These
are not needed for correctness (they were added as rendering hints). If alpha
renders correctly afterwards, the CSS approach is all that is needed.

**A2 (if A1 doesn't fix it):** Switch Linux to `gtk_widget_set_opacity()` and
suppress the CSS `--window-opacity` path on Linux:
- In `window.rs` Linux block: call `gtk_window.set_opacity(opacity)`.
- In `AppSettingsUpdater` (app.tsx): detect Linux via `PLATFORM === "linux"`
  (already available) and skip setting `--window-opacity` on body — leave body
  background at full opacity and rely solely on GTK to dim the window.
- This is reliable, matches alacritty/kitty UX, and is supported by Mutter.

### Fix B — Multi-window flash (20 min)

In `AppSettingsUpdater` (`frontend/app/app.tsx`), add a prev-value ref and only
call the Tauri invoke when the relevant values actually changed:

```ts
let prevTransparent: boolean | undefined;
let prevBlur: boolean | undefined;
let prevOpacity: number | undefined;

createEffect(() => {
    const isTransparentOrBlur = ...;
    const isBlur = ...;
    const opacity = ...;

    // CSS updates always (cheap, idempotent)
    ...

    // Native API only when values changed for THIS window
    if (
        isTransparentOrBlur !== prevTransparent ||
        isBlur !== prevBlur ||
        opacity !== prevOpacity
    ) {
        prevTransparent = isTransparentOrBlur;
        prevBlur = isBlur;
        prevOpacity = opacity;
        getApi().setWindowTransparency(isTransparentOrBlur, isBlur, opacity);
    }
});
```

This means the native OS call fires only once per actual change, not once per
broadcast. Since all windows receive the same config, they all still update —
but the second-window "flash" disappears because the invocation is no longer
re-applied when nothing changed.

---

## Files to Touch

| File | Change |
|---|---|
| `frontend/app/app.scss` | Remove `transform: translateZ(0)` / `backface-visibility: hidden` from `body` (Fix A1) |
| `frontend/app/app.tsx` | Add prev-value guard in `AppSettingsUpdater` (Fix B) |
| `src-tauri/src/commands/window.rs` | Add Linux `gtk_window.set_opacity()` (Fix A2 only if A1 fails) |

---

## Current WIP Diff Summary (what is uncommitted right now)

| File | Change | Status |
|---|---|---|
| `frontend/app/store/zoom.linux.ts` | Set `--zoom-transition-dur: 80ms` in `initChromeZoom()` | Keep — ghost fix works |
| `frontend/app/block/block.scss` | `transition: zoom var(--zoom-transition-dur, 0ms)` | Keep |
| `frontend/app/statusbar/StatusBar.scss` | Same transition | Keep |
| `frontend/app/window/window-header.linux.scss` | `transition: zoom 80ms ease-out` | Keep |
| `src-tauri/src/commands/window.rs` | Added Windows comment only (no behaviour change) | Harmless |
