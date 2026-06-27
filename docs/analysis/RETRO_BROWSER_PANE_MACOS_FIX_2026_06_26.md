# Retro: Browser Pane Black Screen + Mouse Freeze — macOS ObjC Fix Attempt
**Date:** 2026-06-26  
**Branch:** `fix/browser-pane-deferred-bounds-macos`  
**Status:** ✅ FIXED — browser pane renders correctly as of v17 (2026-06-26 18:36)

---

## Bug Description

Opening a browser pane on macOS (CEF Views / Alloy mode) produces:
1. **Black pane** — pane area shows solid black, no web content renders
2. **Mouse click freeze** — clicks anywhere in the main window (including the sidebar, outside the pane) are silently dropped; JS/keyboard still work

---

## Root Cause (Confirmed)

### 1. `set_size` / `set_position` are permanent no-ops on macOS
`CefOverlayController::SetSize()` and `SetPosition()` route to `NativeWidgetMac::SetBounds()`, which calls `[NSWindow setFrame:]` — but the overlay NSWindow does not yet exist at the time these are called (async native init). The readback from `controller.bounds()` always returns `{0,0,0,0}`.

### 2. `parent_window.layout()` makes things worse
Calling `layout()` schedules an async deferred Views layout pass. This fires **after** our `SetPaneBoundsViewsTask`, resetting the overlay to 0×0 (or full-window default), re-triggering the freeze.

### 3. CEF's Views hit-testing uses full-window default for CUSTOM overlays
When a `DockingMode::CUSTOM` overlay has no committed bounds (because set_size is a no-op), CEF's RootView hit-testing treats the overlay BrowserView as covering the **entire parent window**. Every mouse click in the parent window is routed to the overlay browser, which doesn't process them → the main window's SolidJS UI appears frozen to clicks.

### 4. `set_visible(0)` removes the overlay from `[NSApp windows]`
Calling `set_visible(0)` → `Widget::Hide()` → `[NSWindow orderOut:]` removes the overlay window from `[NSApp windows]`. From that point on, we cannot find it through standard Cocoa APIs.

### 5. The NativeWidgetMacNSWindow at (0, 1112, 0, 0) is NOT the overlay
There is always exactly **one** `NativeWidgetMacNSWindow` visible in `[NSApp windows]` at coordinates (0, 1112, 0, 0) with h=0 — even **before any pane is ever opened**. This is a different CEF-internal widget (possibly the overlay for a different feature). We have been resizing this wrong window in all previous attempts.

### 6. CEF does not use `addChildWindow:ordered:`
`[mainWin childWindows]` is always empty — CEF overlays are not parented to the main window via the Cocoa child-window mechanism.

---

## Upstream Fix That Was Already Merged (d25f04f9, Jun 24)

PR #1769 fixed an ordering bug:
- **Before:** `set_size → set_position → set_visible(1) → layout()`  
- **After:** `set_size → set_position → layout() → set_visible(1)`
- **Also added:** `host.set_focus(1)` on main browser after `add_overlay_view` to prevent focus steal

This partially fixed the issue on Linux (where set_size/set_position DO work). On macOS, set_size/set_position are still no-ops, so bounds remain wrong. The `set_focus(1)` call does help restore keyboard focus but does NOT fix CEF's Views mouse-event routing, which uses the uncommitted full-window bounds.

---

## What We Tried (v1–v14)

### v1–v4: CEF Views API path
Called `set_size → set_position → layout() → set_visible(1)` in a deferred task (one event loop tick after creation). Confirmed no-ops on macOS via `controller.bounds()` readback → always `{0,0,0,0}`.

### v5–v6: `set_visible(0)` to prevent initial freeze
Added `set_visible(0)` in the creation path before any layout, to keep the overlay hidden until we can size it correctly. This prevents the initial full-window freeze, but the overlay window disappears from `[NSApp windows]` — we can no longer find it.

### v7–v8: ObjC `setFrame` on wrong window
Found the NativeWidgetMacNSWindow at (0,1112,0,0) via `[NSApp windows]`. Called `setFrame:display:YES` to move it to the pane position. **This resizes the WRONG window** (the pre-existing CEF widget, not the overlay). The wrong window becomes black and visible at the pane position. The actual overlay is still hidden and its Views bounds are still full-window → freeze persists.

### v8: `setIgnoresMouseEvents:YES` diagnostic
Confirmed that the pre-existing NativeWidgetMacNSWindow is NOT the source of the freeze (setting ignoresMouse on it doesn't fix clicks). The freeze comes from CEF's Views hit-testing inside the main CefNSWindow, routing events to the hidden overlay's full-window BrowserView bounds.

### v9: `childWindows` search
Checked `[mainWin childWindows]`. Always empty — CEF does not use `addChildWindow:ordered:` for the overlay.

### v10–v12: `set_visible(1)` first, then find window
Called `controller.set_visible(1)` to make the overlay appear in `[NSApp windows]`, then found it and resized. Result: the pre-existing NativeWidgetMacNSWindow at (0,1112,0,0) becomes `is_key=1` after `set_visible(1)`. This suggests the pre-existing window IS the overlay, OR that `set_visible(1)` makes something steal key status indirectly. Either way, the window stays at h=0 — never grows to visible size.

Key problem: `set_visible(1)` steals key status from the main window → sidebar clicks don't work even if Views bounds were somehow fixed.

### v13: Remove h>0 filter
Accepted any NativeWidgetMacNSWindow (including h=0). Found the pre-existing window. Called setFrame successfully (`got_w=591, got_h=665`). But freeze still persists → confirmed this is the wrong window; setFrame does not fix CEF's Views bounds for the actual overlay.

### v14: `[NSWindow windowWithWindowNumber:]` scan
New approach: scan window numbers 1–512 via `[NSWindow windowWithWindowNumber:]` (which CAN return hidden/ordered-out windows). Any NativeWidgetMacNSWindow not already in `[NSApp windows]` would be the actual hidden overlay. **Not yet tested** — the app entered a network service crash loop before we could test.

---

## Current State of Code

### `creation_views.rs` (macOS path)
```rust
// On macOS: skip set_size/set_position/layout (all no-ops or harmful)
#[cfg(not(target_os = "macos"))]
{
    overlay_controller.set_size(...);
    overlay_controller.set_position(...);
    parent_window.layout();
}
overlay_controller.set_visible(0); // hide until task sizes it correctly
// Upstream fix: restore focus to main window
if let Some(main_browser) = state.get_browser(&window_label) {
    if let Some(mut host) = main_browser.host() { host.set_focus(1); }
}
// Post sizing task
post_set_pane_bounds_views(..., retry=0);
```

### `ui_tasks.rs` `SetPaneBoundsViewsTask` (macOS path, v14)
```rust
// Step 1: scan [NSApp windows] for known NativeWidgetMacNSWindow IDs + main CefNSWindow
// Step 2: scan windowNumber 1..=512 for hidden NativeWidgetMacNSWindow (the overlay)
// Step 3: setFrame on overlay → fires windowDidResize → CEF Views bounds update
// Step 4: controller.set_visible(1)
// Step 5: makeKeyAndOrderFront on main window
// Step 6: host.set_focus(1) on main browser
```

---

## Startup Crash Issue (Introduced After Rebase)

After rebasing onto latest main (v0.49.5), the dev build crashes on startup:
- `Network service crashed or was terminated` loop
- `No rendezvous client, terminating process (parent died?)`
- Main window never appears

This appears to be a new upstream regression, possibly related to the new `CFBundleIdentifier` injection in dev builds (commit faf754e6: "inject CFBundleIdentifier into dev builds for LaunchServices isolation") conflicting with something in the sandbox/Mach port setup. **This blocks testing of the v14 windowNumber approach.**

---

## Promising Untested Approaches

### A. `can_activate=0` for the overlay
Change `add_overlay_view(..., can_activate=1)` to `can_activate=0`. With `can_activate=0`:
- The overlay NSWindow cannot become key (cannot steal focus)
- CEF may handle the overlay differently (possibly not routing events to it)
- Eliminates the focus-steal root cause that d25f04f9 tried to patch around

### B. Find overlay window BEFORE `set_visible(0)` (synchronous)
In `create_browser_pane_view`, immediately after `add_overlay_view` returns and BEFORE `set_visible(0)`, call ObjC synchronously on the UI thread to enumerate `[NSApp windows]`. If the overlay NSWindow is created synchronously in `add_overlay_view`, it would appear here (before orderOut). We can call `setFrame` immediately, then `set_visible(0)`. When the user later triggers `set_visible(1)`, the native frame is already correct → Views bounds update correctly.

### C. Don't call `set_visible(0)` at all
Skip the `set_visible(0)` call. The overlay appears briefly at wrong bounds (full-window black flash, <16ms until our task runs). Our task finds the overlay in `[NSApp windows]` (it's now ordered-in), calls `setFrame` before the next display composite → no visible flash. Overlay appears at correct size.

Risk: The full-window Views bounds during the 16ms window might route events incorrectly, but since this is sub-frame, the user won't notice.

### D. Fix the startup crash first
The rebase introduced a crash that blocks all testing. Need to bisect whether it's from our code changes or from upstream faf754e6 / new CFBundleIdentifier injection.

---

## Next Steps (Priority Order)

1. **Fix startup crash** — determine if it's our code or upstream; try reverting to pre-rebase state or adding `--no-sandbox` flag
2. **Test windowNumber scan (v14)** — if startup is fixed, test whether `[NSWindow windowWithWindowNumber:]` 1..512 finds the hidden overlay
3. **Try `can_activate=0`** — simplest possible change; might fix focus steal without any ObjC needed
4. **Try synchronous ObjC in creation** — find overlay before `set_visible(0)`

---

## Actual Root Cause (Discovered v15–v17)

The NativeWidgetMacNSWindow at (0,1112,0,0) IS the overlay created by `add_overlay_view`.
The setFrame calls DID set the NSWindow to 591×665 (readback confirmed).
But the pane was still black because **CEF's layout kept resetting it to 0×0**.

### Why the layout resets to 0×0

`overlay_view_host.cc::SetOverlayBounds()` does:
```cpp
bounds_.Intersect(window_view_->bounds());
```
At creation time (and immediately after `set_visible(1)`), the parent `CefWindowView::bounds()` is empty on macOS due to deferred native layout. The intersection of any rect with an empty rect is empty → overlay shrinks to 0×0. Every `setFrame:display:YES` on the NSWindow fires `windowDidResize:` → CEF layout → resets to 0×0 → infinite loop.

### The Fix (v17)

Call `controller.set_bounds()` from the **deferred UI-thread task** (`SetPaneBoundsViewsTask::execute`), after `set_visible(1)`. By the time the task runs (next runloop tick), the parent `CefWindowView` has bounds {0,0,1200,800} (fully laid out). The Intersect now preserves our desired rect. Log evidence:

```
readback_x=604, readback_y=107, readback_w=591, readback_h=665  ← set_bounds sticks!
got_w=591.0, got_h=665.0  ← NSWindow also stays at correct size (no more 0×0 resets)
```

**Code in `ui_tasks.rs`** (after the ObjC setFrame block):
```rust
let dip = CefRect { x: task_dip_x, y: task_dip_y, width: task_dip_w, height: task_dip_h };
controller.set_bounds(Some(&dip));
```
where `task_dip_{x,y,w,h} = pane_{x,y,w,h}` from ObjC (pixel coords ÷ backingScaleFactor).

### Additional Fixes Along the Way

- **macOS 26 Tahoe**: `[NSWindow windowWithWindowNumber:]` removed → replaced with pre-hide scan of `[NSApp windows]`
- **wrap_task! macro**: rejects `///` doc comments, use `//` instead
- **`makeKey` ObjC selector**: crashes on macOS 26 (SIGABRT) — use `makeKeyAndOrderFront:` on main then `orderFront:` on overlay
- **Startup crash**: `ChromeWebAppShortcutCopierMain` SIGTRAP/SIGABRT — fixed by early exit in `lib.rs` for `--type=web-app-shortcut-copier` subprocess arg

## Key Log Patterns to Watch

```
# Fix confirmed working:
[browser-pane] controller.set_bounds (post-ObjC) and readback  readback_w=591  readback_h=665
[browser-pane] ObjC task overlay setFrame reaffirmed  got_w=591  got_h=665  # all tasks stable

# Regression indicator:
[browser-pane] controller.set_bounds (post-ObjC) and readback  readback_w=0  readback_h=0
```

---

## Open Issue: Mouse Clicks Unresponsive in Browser Pane (macOS)

**Status**: Known limitation as of 2026-06-26 — tracked for follow-up.

After the black-pane fix, the pane renders and scroll wheel events reach the Chromium renderer (user can scroll the web page). However, mouse button clicks (left-click / right-click) in the pane are unresponsive.

### Observed Behaviour

- **Scroll**: Works — scroll events reach the pane renderer regardless of key-window status.
- **Click in pane**: Silent — mouseDown is not dispatched to the Chromium renderer.
- **Click in sidebar**: Also broken while pane is open.

### Root Cause Investigation

The two-window macOS architecture is the core of the problem:

- Main window (`NSKVONotifying_CefNSWindow`) — hosts SolidJS sidebar via main BrowserView.
- Overlay window (`NativeWidgetMacNSWindow`) — hosts pane browser via CEF Views overlay.

**Why scroll works but clicks don't**: macOS routes `NSEventTypeScrollWheel` to the window under the cursor regardless of key-window status. Mouse button events (`NSEventTypeLeftMouseDown`) go to the frontmost window under the cursor, but Chromium's renderer checks whether its enclosing window is the key window (or `acceptsFirstMouse:YES`) before dispatching `mouseDown` to the JS layer. When the overlay is not KEY, Chromium silently drops the mouseDown.

**Competing constraints**:

| State | Sidebar clicks | Pane clicks |
|---|---|---|
| Main is KEY (`makeKeyAndOrderFront:main` called in task) | ✓ work | ✗ broken (overlay not KEY) |
| Overlay is KEY (no `makeKeyAndOrderFront:main`) | ✗ broken (main not KEY) | ✗ still broken (see below) |

When the overlay IS the key window, pane clicks STILL don't reach the renderer. Hypothesis: `host.set_focus(1)` on the main browser (called in `creation_views.rs`) tells CEF's internal focus routing to direct events to the main browser, overriding the OS key-window state for the overlay.

Confirmed diagnostic: after tasks run, `[NSApp keyWindow]` pointed to `NSKVONotifying_CefNSWindow` when `makeKeyAndOrderFront:main` was called (v18), confirming sidebar-focus-restore works. Removing that call (v19) let overlay retain key status but clicks still failed — pointing to a deeper CEF input-routing issue, not just key-window status.

### Approaches to Try

1. **`acceptsFirstMouse:YES` on overlay content view**: Swizzle or subclass the overlay's root `NSView` to return YES from `acceptsFirstMouse:`. This allows the first click to be processed without needing key-window status. Requires ObjC in the task after `set_visible(1)`.

2. **`can_activate=0` for the overlay**: Change `add_overlay_view(..., can_activate=1)` to `can_activate=0`. This prevents the overlay from becoming KEY, but CEF may then route mouseDown events differently (possibly accepting them unconditionally since the window cannot steal focus). Untested.

3. **`host.set_focus(1)` on pane browser**: Call `set_focus(1)` on the pane browser (not main) to tell CEF's internal routing to direct events to the pane renderer. Risk: sidebar clicks then go to pane renderer and may be swallowed.

4. **CEF `SetAccessibilityState` / `NotifyMoveOrResizeStarted`**: Some CEF calls force a re-sync of the internal widget focus state. Worth trying to see if any of these reset CEF's input routing to match the actual NSWindow key state.

### Current PR State

The PR ships with `makeKeyAndOrderFront:main` after `set_bounds()` (sidebar-first policy): sidebar clicks are restored, pane clicks remain non-functional. This matches the pre-pane UX (sidebar usable) while the pane is visible but click-limited.
