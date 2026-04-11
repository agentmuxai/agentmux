# Window Opacity Inconsistency — Investigation Report

**Date:** 2026-04-02
**Status:** Root causes identified, fix spec below
**Severity:** UX bug — opacity sometimes doesn't apply, worse with multiple windows

## Symptoms

1. Switching between opacity levels doesn't always take effect
2. Multiple windows: some get new opacity, others don't
3. May need to toggle multiple times for it to stick

## Code Audit

### Frontend → Host flow

1. `app.tsx:141` — `AppSettingsUpdater` effect reacts to settings changes:
   ```ts
   getApi().setWindowTransparency(isTransparentOrBlur, isBlur, opacity);
   ```
2. `cef-api.ts:391` — sends IPC:
   ```ts
   invokeCommand("set_window_transparency", { transparent, blur, opacity })
   ```
3. `ipc.rs` routes to `commands::window::set_window_transparency()`

### Host-side: `set_window_transparency()` (window.rs:179)

```rust
pub fn set_window_transparency(state, args) {
    let transparent = args["transparent"];  // bool
    let blur = args["blur"];               // bool
    let _opacity = args["opacity"];         // f64 (0.0-1.0)

    let hwnd = find_own_top_level_window();  // ← BUG 1: only finds ONE window
    if blur {
        apply_window_effects(hwnd, true, true);  // DWM backdrop
    }
    if transparent {
        apply_window_opacity(hwnd, _opacity);    // WS_EX_LAYERED + alpha
    }
    // ← BUG 2: no "else" to REMOVE opacity when transparent=false
}
```

### `find_own_top_level_window()` (window.rs:212)

```rust
EnumWindows(callback, ...);
// callback: find first visible window with our PID → return it, stop enum
```

Returns the **first** visible window it finds. In multi-window, this is arbitrary — could be any window.

### `apply_window_opacity()` (window.rs:292)

```rust
// Adds WS_EX_LAYERED + SetLayeredWindowAttributes(alpha)
let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_LAYERED);
SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
```

## Root Causes

### Bug 1: Only targets one window

`find_own_top_level_window()` returns a single HWND — the first visible window in enum order. With multiple windows, only one gets the opacity change. Which one is non-deterministic (depends on Z-order).

**Fix:** Iterate `state.browsers` and apply to every window's HWND.

### Bug 2: No opacity removal path

When `transparent` is `false`, the function does nothing — it doesn't remove `WS_EX_LAYERED` or reset alpha to 255. So once opacity is set, it can only be changed to a different value, never fully removed.

```rust
if transparent {
    apply_window_opacity(hwnd, _opacity);  // sets opacity
}
// Missing: else { remove_window_opacity(hwnd); }
```

**Fix:** When `transparent=false`, remove `WS_EX_LAYERED` from the extended style.

### Bug 3: DWM backdrop not removed

Same issue with `blur` — `apply_window_effects` is only called when `blur=true`. When `blur=false`, the DWM backdrop (Mica/Acrylic) is never reset to `DWMSBT_NONE`.

Actually — looking more carefully, `apply_window_effects` DOES handle the `!transparent && !blur` case at line 247:
```rust
if !transparent && !blur {
    let backdrop_type: i32 = 1; // DWMSBT_NONE
    DwmSetWindowAttribute(...);
    return;
}
```

But this is only reached when BOTH are false. If only blur changes from true to false while transparent stays true, the backdrop isn't cleared.

**Fix:** Always call `apply_window_effects` with the current state, not conditionally.

### Bug 4: CEF Views HWND mismatch

`find_own_top_level_window()` enumerates by process ID and returns the first visible HWND. In CEF Views mode, the window hierarchy is:
```
CefWindow (top-level, our target)
  └── BrowserView
       └── Chrome_WidgetWin_0 (CEF internal)
```

The enum might return the Chrome_WidgetWin child instead of the CefWindow parent. Setting `WS_EX_LAYERED` on the wrong HWND has no effect.

**Fix:** Use `state.browsers` → `browser.host().window_handle()` → `GetAncestor(GA_ROOT)` to find the correct top-level HWND for each window.

### Bug 5: Each window's frontend calls independently

Each window runs its own `AppSettingsUpdater`. When settings change, ALL windows send `set_window_transparency` — but the handler only applies to one HWND. The first IPC call works; subsequent calls from other windows target the same HWND (the first one found).

**Fix:** The IPC call should include the `windowLabel` so the handler can target the correct window. Or: apply to all windows unconditionally.

## Proposed Fix

### Option A: Apply to all windows (simplest)

```rust
pub fn set_window_transparency(state, args) {
    let transparent = args["transparent"];
    let blur = args["blur"];
    let opacity = args["opacity"];

    #[cfg(target_os = "windows")]
    {
        let browsers = state.browsers.lock().unwrap();
        for (label, browser) in browsers.iter() {
            if let Some(host) = browser.host() {
                let hwnd = host.window_handle();
                if !hwnd.0.is_null() {
                    let top = GetAncestor(hwnd.0 as _, GA_ROOT);
                    let target = if !top.is_null() { top } else { hwnd.0 as _ };
                    unsafe {
                        apply_window_effects(target, transparent, blur);
                        if transparent {
                            apply_window_opacity(target, opacity);
                        } else {
                            remove_window_opacity(target);
                        }
                    }
                }
            }
        }
    }
}

unsafe fn remove_window_opacity(hwnd: *mut c_void) {
    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_LAYERED as isize));
}
```

### Option B: Per-window targeting (cleaner)

Pass `windowLabel` from frontend, look up the specific browser in `state.browsers`. Requires frontend change to include the label.

**Recommendation:** Option A for now — fixes all bugs with minimal changes. Option B for follow-up if per-window opacity is desired.

## Files to Change

1. `agentmux-cef/src/commands/window.rs` — rewrite `set_window_transparency` to iterate all browsers, add `remove_window_opacity`, fix `apply_window_effects` to always set backdrop type
2. No frontend changes needed for Option A

## Estimated Complexity

Low — ~30 lines changed in one file.
