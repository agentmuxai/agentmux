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

## Mouse Click Investigation — Session 2 (2026-06-27)

**Status**: Still unresolved. Significant new diagnostics gathered; root cause narrowed but not eliminated.

---

### Updated Observed Behaviour

- **Scroll**: Works ✓
- **Link clicks (navigation)**: Work ✓ — clicking a hyperlink navigates to the linked URL
- **Interactive clicks**: Unresponsive ✗ — form inputs don't focus, buttons don't activate, JS `click` handlers don't fire
- **Sidebar clicks**: Work ✓ (with `makeKeyAndOrderFront:main` in task)

The link-click finding is the most important new data: **mouse events DO reach the Chromium renderer** (otherwise links wouldn't navigate). The problem is not event delivery — it is **renderer interactivity state**.

---

### Diagnostic Chain (Confirmed Facts)

#### 1. Z-order confirmed correct
`windowNumberAtPoint:belowWindowWithWindowNumber:0` returns the overlay window number at the pane center → overlay IS frontmost at the click location. Clicks go to the overlay NSWindow, not the main window.

#### 2. hitTest returns RenderWidgetHostViewCocoa
`[overlayContentView hitTest:(paneCenter)]` → `RenderWidgetHostViewCocoa`. NSWindow's `sendEvent:mouseDown:` dispatches to this view (deepest hit-test result), not to `BridgedContentView`.

#### 3. RenderWidgetHostViewCocoa::acceptsFirstMouse: returned NO (0) before fix
Confirmed via `afm_before=0` (retry=0). NSView's inherited `acceptsFirstMouse:` returns NO. Replaced via `method_setImplementation` → now returns YES. This fix is correct but was not the final cause.

#### 4. can_activate=0 vs can_activate=1: no difference
Switching to `can_activate=1` (overlay can become key window) made no difference. Clicks still unresponsive for interactive elements, links still navigate. Key-window status is NOT the root cause.

#### 5. set_focus(1) on pane browser: no difference
After `makeKeyAndOrderFront:main`, a 200ms-delayed task calls `host.set_focus(1)` on the pane browser. Confirmed fired in logs. Still no interactive clicks.

---

### Active Bug: Pool Window Contamination at retry=1

At retry=1 (50ms reaffirm task), the main-window scan picks the WRONG window because `is_main` check was removed:

```
i=0: NSKVONotifying_CefNSWindow x=-32000 (pool, is_main=0)  ← becomes main_win candidate
i=2: NSKVONotifying_CefNSWindow x=51     (real main, is_main=1)  ← overwritten by i=4
i=4: NSKVONotifying_CefNSWindow x=-32000 (pool, is_main=0)  ← LAST → becomes main_win!
```

**Effect**: `makeKeyAndOrderFront:pool_window` fires on the off-screen window at retry=1. Confirmed: retry=1 shows `main_x=-32000` and `screen_x=-31395` (wrong position). `controller.set_bounds()` corrects the overlay position afterward, but the wrong key-window assignment corrupts state.

**Fix (ready to apply)**:
```rust
if cls.contains("CefNSWindow") {
    if is_main != 0 {
        main_win = win;  // Definitive: is_main=1 window
    } else if main_win.is_null() {
        let fr = get_frame(win, sel_frame);
        if fr.origin.x > -1000.0 {
            main_win = win;  // Fallback: on-screen CefNSWindow
        }
    }
}
```

This is a correctness fix but is NOT expected to fix pane clicks (the correct main window was already found at retry=0, which is when links started working).

---

### Working Hypothesis: Renderer Input Routing in Overlay Mode

Links navigate → `mouseDown:` reaches `RenderWidgetHostViewCocoa` → renderer receives the event → Blink processes it → link activation occurs. This entire path works.

What doesn't work: anything requiring the renderer to treat the click as an **interactive user gesture** (focus a form field, fire an `onclick` JS handler, etc.).

In Chromium, interactive gestures require the render frame to have **user activation** (`LocalFrame::NotifyUserActivation`). `NotifyUserActivation` is called by:
- `RenderWidgetHostImpl::ForwardMouseEvent()` when the event is a `mousedown`

BUT: `ForwardMouseEvent` may check `IsUserInteractionInputType()` and the renderer's **focus state** before calling `NotifyUserActivation`. If the renderer's `WebWidget` is not focused (`RenderWidget::is_focused_ == false`), some activation paths are skipped.

**Sub-hypothesis A (focus state)**: CEF marks the pane renderer as unfocused (via `RenderWidgetHostImpl::Blur()`) when the overlay window loses key status. `set_focus(1)` on the CEF host may be overridden by CEF's internal Views focus system before or after our call. The renderer stays blurred → interactive clicks silently drop user activation.

**Sub-hypothesis B (input routing)**: CEF's `NativeWidgetNSWindowBridge` for the overlay may be routing mouse events to the PARENT widget (main window's bridge) because the overlay was created with `params.parent = parent_widget`. The parent bridge may process them incorrectly (wrong coordinate space, or just drop non-scroll events).

**Sub-hypothesis C (missing gesture handler)**: The overlay's `BrowserView` or its delegate may have an `OnGestureEvent` handler that returns `true` (consumed) for click events, eating them before they reach the renderer's `InputRouter`. Scroll events wouldn't be consumed this way.

---

### Approaches Not Yet Tried

#### A. Swizzle RenderWidgetHostViewCocoa::mouseDown: to confirm it's called
```rust
// In task code, at setup time:
static ORIG_MD: AtomicUsize = AtomicUsize::new(0);
extern "C" fn swizzled_md(this: Id, cmd: Sel, event: Id) {
    tracing::info!("[PANE-CLICK] RenderWidgetHostViewCocoa mouseDown: CALLED");
    let f: extern "C" fn(Id, Sel, Id) = transmute(ORIG_MD.load(SeqCst));
    f(this, cmd, event);
}
// method_setImplementation on RenderWidgetHostViewCocoa::mouseDown:
// ORIG_MD.store(old_imp, SeqCst);
```
If this logs when clicking → `mouseDown:` IS called → issue is INSIDE it.
If it DOESN'T log → NSWindow::sendEvent: is NOT dispatching mouseDown despite acceptsFirstMouse:YES → check NativeWidgetMacNSWindow::sendEvent: override.

#### B. Check NativeWidgetMacNSWindow::sendEvent: override
`sendEvent:` appears as a string in CEF's framework binary. If `NativeWidgetMacNSWindow` overrides it, it may bypass `acceptsFirstMouse:` entirely and use custom dispatch logic that ignores non-key windows.

Diagnostic: check whether `class_getInstanceMethod(NativeWidgetMacNSWindow_class, sendEvent_sel)` returns a method whose `IMP` is different from `NSWindow`'s `sendEvent:`. If different → CEF has a custom override.

#### C. Try add_child_view instead of add_overlay_view
Instead of creating a separate NSWindow for the pane, add the pane BrowserView directly as a child view of the main window's CefWindowView. All clicks would go to the main window (key) and NSView hit-testing would naturally route them to the pane's `RenderWidgetHostViewCocoa`.

Tradeoff: requires the main BrowserView to NOT cover the pane area (needs layout split), and the SolidJS UI must not overlap the pane. This is a larger architecture change.

#### D. NSEvent addLocalMonitorForEventsMatchingMask (from Rust via block)
Intercept all `NSEventTypeLeftMouseDown` events at the app level. When the event is in the overlay window's bounds → directly call `[renderWidgetHostViewCocoa mouseDown:event]` on the pane browser's RWHVC. This bypasses all NSWindow routing.

Limitation: requires creating an Obj-C block from Rust (needs objc_blocks crate or manual block ABI).

#### E. Synthesize clicks via CefBrowserHost::SendMouseClickEvent
`CefBrowserHost::SendMouseClickEvent(CefMouseEvent, MBT_LEFT, false, 1)` manually injects a click event directly into the CEF input pipeline, bypassing NSWindow/AppKit entirely. This would confirm whether the issue is in AppKit→CEF routing or deeper in CEF's input processing.

If synthetic clicks work → the Appkit→CEF path is broken.
If synthetic clicks don't work → CEF's own input pipeline has an issue for overlay browsers.

---

### Code State (ui_tasks.rs, as of this session)

All of the following are active in the current dev build:

1. `acceptsFirstMouse:YES` via `method_setImplementation` on `BridgedContentView` — correct but not sufficient (RWHVC is the hit target, not BridgedContentView)
2. `acceptsFirstMouse:YES` via `method_setImplementation` on `RenderWidgetHostViewCocoa` — `afm_before=0` confirmed at retry=0; now returns YES
3. `hitTest:` logging confirmed `RenderWidgetHostViewCocoa` with `sub_count=2` at pane center
4. `set_focus(1)` on pane browser (200ms delay after retry=1) — fired but no effect
5. `makeKeyAndOrderFront:main` at retry=0 (correct) and retry=1 (wrong — pool window)
6. `can_activate=0` (reverted from diagnostic `can_activate=1`)

---

### Next Priority Steps

1. **Fix pool-window bug** (pool window contaminating `main_win` at retry=1) — correctness fix, low risk
2. **Swizzle `mouseDown:`** on `RenderWidgetHostViewCocoa` — definitive answer on whether `mouseDown:` is called when clicking interactive elements
3. **Try `SendMouseClickEvent`** — if swizzle shows `mouseDown:` IS called, try injecting clicks directly into CEF to bypass AppKit
4. **Investigate `NativeWidgetMacNSWindow::sendEvent:` override** — if swizzle shows `mouseDown:` is NOT called, this is the next suspect
