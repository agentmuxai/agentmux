# Secondary Windows CEF Views — Implementation Plan

**Date:** 2026-04-02
**Branch:** `agenta/secondary-windows-cef-views`
**Base:** `origin/main` (after PR #272)

## Pre-flight

1. `git checkout -b agenta/secondary-windows-cef-views origin/main`
2. Verify main window works: `task cef:package:portable` → test

## Step 1: Parameterize AgentMuxWindowDelegate

**File:** `agentmux-cef/src/app.rs`

Add optional initial bounds to the delegate so it can be reused for secondary windows with custom position/size:

```rust
wrap_window_delegate! {
    pub struct AgentMuxWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
        initial_bounds: Option<(i32, i32, i32, i32)>,  // (x, y, w, h) or None for 70% centered
    }
}
```

Update `on_window_created`:
```rust
fn on_window_created(&self, window: Option<&mut Window>) {
    // ... add_child_view ...
    
    if let Some((x, y, w, h)) = self.initial_bounds {
        window.set_bounds(Some(&Rect { x, y, width: w, height: h }));
    } else {
        if let Some((x, y, w, h)) = get_monitor_centered_70pct(window) {
            window.set_bounds(Some(&Rect { x, y, width: w, height: h }));
        }
    }
    // Do NOT show — deferred to on_load_end
}
```

Update existing call sites:
- `on_context_initialized`: `AgentMuxWindowDelegate::new(RefCell::new(browser_view), None)`
- `on_popup_browser_view_created`: `AgentMuxWindowDelegate::new(RefCell::new(popup_bv), None)`

**Test:** Main window still works, no regression.
**Commit:** `refactor: parameterize AgentMuxWindowDelegate with optional bounds`

## Step 2: Switch open_new_window to CEF Views

**File:** `agentmux-cef/src/commands/window.rs`

The function currently builds a `WindowInfo` and calls `browser_host_create_browser`. Replace with CEF Views.

**Before:**
```rust
let window_info = cef::WindowInfo { style: WS_POPUP | ..., ... };
browser_host_create_browser(&window_info, client, &url, &settings, Some(&ctx), None);
```

**After:**
```rust
let mut bv_delegate = AgentMuxBrowserViewDelegate::new(RuntimeStyle::ALLOY);
let browser_view = cef::browser_view_create(
    client, &url, &settings, None, Some(&ctx), &mut bv_delegate,
);
let mut wd = AgentMuxWindowDelegate::new(
    RefCell::new(browser_view),
    Some((pos_x, pos_y, win_w, win_h)),
);
cef::window_create_top_level(&mut wd);
```

**Key details:**
- `RequestContext` (`ctx`) passed to `browser_view_create` 4th arg (extra_info=None, request_context=Some)
- Position from `get_offset_position()`
- Size from `get_secondary_window_size()`
- Need to import `AgentMuxWindowDelegate`, `AgentMuxBrowserViewDelegate` from `crate::app`
- Remove the `#[cfg(target_os = "windows")]` / `#[cfg(not)]` branches — CEF Views is cross-platform

**Test:** Click "new window" button → window opens, resize works, no flash.
**Commit:** `fix: switch open_new_window to CEF Views for resize support`

## Step 3: Switch open_window_at_position to CEF Views

**File:** `agentmux-cef/src/commands/drag.rs`

Same pattern as Step 2. This function opens windows for pane tear-off.

**Before:**
```rust
let window_info = cef::WindowInfo { style: WS_POPUP | ..., bounds: cef::Rect { x, y, w, h }, ... };
browser_host_create_browser(&window_info, client, &url, &settings, Some(&ctx), None);
```

**After:**
```rust
let mut bv_delegate = AgentMuxBrowserViewDelegate::new(RuntimeStyle::ALLOY);
let browser_view = cef::browser_view_create(
    client, &url, &settings, None, Some(&ctx), &mut bv_delegate,
);
let mut wd = AgentMuxWindowDelegate::new(
    RefCell::new(browser_view),
    Some((x, y, w, h)),
);
cef::window_create_top_level(&mut wd);
```

**Test:** Tear off a pane → window opens at cursor, resize works, no flash.
**Commit:** `fix: switch open_window_at_position to CEF Views`

## Step 4: Simplify on_load_end

**File:** `agentmux-cef/src/client.rs`

Remove the Win32 `ShowWindow` fallback — all windows now use CEF Views:

```rust
// Show via CEF Views API — works for all windows now
let mut browser_cloned = browser.cloned();
if let Some(bv) = browser_view_get_for_browser(browser_cloned.as_mut()) {
    if let Some(window) = bv.window() {
        if window.is_visible() == 0 {
            window.show();
            if let Some(ref mut b) = browser_cloned {
                if let Some(host) = b.host() {
                    host.set_focus(1);
                }
            }
        }
    }
}
```

**Test:** All window types show correctly after content loads.
**Commit:** `refactor: remove Win32 ShowWindow fallback from on_load_end`

## Step 5: Remove secondary window native mode code from on_after_created

**File:** `agentmux-cef/src/client.rs`

Remove the `install_frameless_resize_hook` call for secondary windows and the `is_secondary` check. CEF Views handles frameless via its delegate.

**Test:** Secondary windows are frameless with resize.
**Commit:** `refactor: remove install_frameless_resize_hook for secondary windows`

## Step 6: Clean up dead code

**Files:** `agentmux-cef/src/client.rs`, `agentmux-cef/src/commands/window.rs`

Remove:
- `get_secondary_window_size()` if no longer referenced
- Win32 `WS_POPUP` style imports if unused
- Any `#[cfg(target_os = "windows")]` blocks that only served native secondary windows

Keep (still used by `--use-native` flag or potential future use):
- `install_frameless_resize_hook` function definition
- `setup_native_frameless` function definition
- `ORIGINAL_WNDPROCS` static

**Commit:** `refactor: remove dead native-mode code for secondary windows`

## Step 7: Bump version + build + test

```bash
bump patch -m "fix: secondary windows use CEF Views for resize" --commit
task cef:package:portable
```

Test:
- [ ] Main window: no flash, resize, snap, frameless
- [ ] New window: no flash, resize, snap, frameless
- [ ] Tear-off: no flash, resize, positioned correctly
- [ ] Close secondary: main unaffected
- [ ] Close main: all windows close
- [ ] Reopen after close: works (lockfile cleaned)

**Commit:** `chore: bump version to X.Y.Z`

## Step 8: Push + PR

```bash
git push -u origin agenta/secondary-windows-cef-views
gh pr create --title "fix: secondary windows use CEF Views for resize"
```

Wait for reagent review, address feedback, merge.

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| `browser_view_create` API mismatch | Low | Matches main window pattern exactly |
| `RequestContext` not accepted by `browser_view_create` | Medium | Check cef-rs API signature; may need different parameter position |
| Secondary window doesn't register in browser map | Low | `on_after_created` handles all browsers regardless of creation method |
| Tear-off position incorrect | Low | `initial_bounds` passes exact position |
| Import visibility (delegate types) | Low | Make delegates `pub` if not already |

## Rollback

If anything breaks: revert the branch. Main window is unaffected by these changes (only secondary window creation changes).
