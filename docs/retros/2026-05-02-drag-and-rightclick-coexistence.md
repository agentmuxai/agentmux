# Retro: Left-Click Drag + Right-Click Context Menu Coexistence on CEF/Wayland

**Date:** 2026-05-02
**Outcome:** Both work. Native window drag from any HTCLIENT header element + standard contextmenu on the same element.

---

## The original problem

On Ubuntu/GNOME/Wayland (Mutter), AgentMux's window header needed two things at once:
- **Left-click drag** anywhere in the empty middle of the title bar should move the window.
- **Right-click** anywhere in the same area should fire the window-header context menu.

The Linux SCSS already had this comment, predating us:

> NOTE: do NOT set `-webkit-app-region: drag` on the whole header. That makes
> the header HTCAPTION on Wayland, and right-click on HTCAPTION is consumed
> by the compositor — the renderer never sees `contextmenu`, so the app
> header menu can't open.

CEF Patch A (`41802fe60`, "Patch A: Let right-clicks on HTCAPTION fall through to renderer") was added to libcef.so to fix the right-click side. The intent: in `WindowEventFilterLinux::HandleMouseEventWithHitTest`, return `false` for right-click on HTCAPTION instead of consuming it, so the event would propagate through the handler chain to the renderer's `contextmenu` pipeline.

## What we found

### Diagnosis #1 — empirical confirmation that Patch A is incomplete

I added `-webkit-app-region: drag` to `.tab-bar-fill` (the wide gap between tabs and right-side buttons). Result:
- ✅ Drag worked end-to-end. Window moved.
- ❌ Right-click on the same area showed no menu (not even the tab/widget contextmenu — nothing).

To prove the renderer wasn't seeing the events, I injected a document-level **capture-phase** listener via DevTools `Runtime.evaluate`:

```javascript
document.addEventListener('mousedown', e => window.__amxctlEvents.push({...}), true);
document.addEventListener('contextmenu', e => window.__amxctlEvents.push({...}), true);
```

User did three right-clicks: tab, drag area, widget. Captured events:
- Right-click on tab (`HTCLIENT`): `mousedown(button=2)` + `contextmenu` ✓
- Right-click on widget (`HTCLIENT`): `mousedown(button=2)` + `contextmenu` ✓
- Right-click on drag area (`HTCAPTION`): **zero events**

Verdict: **Chromium architecturally suppresses ALL events on `-webkit-app-region: drag` elements before they reach the renderer process.** Capture phase, JS handlers, none of it matters — the events don't enter the renderer's input pipeline at all. Patch A's `return false` from `WindowEventFilterLinux` is *necessary but not sufficient*; the events are gone before they reach that handler in the renderer-targeted path. So `-webkit-app-region: drag` and `contextmenu` are **mutually exclusive** on the same element.

### Diagnosis #2 — the dock occlusion red herring

Earlier sessions blamed Ubuntu's left-edge dock for occluding the existing `.window-drag.left` strip (60×27 at window x=0–60). That was a real issue (and the dock does cover that strip), but it explained "user can't drag from the visible header" — not the deeper question of "why does drag preclude right-click." The dock was a separate issue we eventually moved past once we made the wider `.tab-bar-fill` draggable.

### Diagnosis #3 — false starts on the FFI

Once I committed to the proper fix (JS-driven drag calling a new CEF API), I added a `BeginWindowDrag` method to `CefWindow`. First impl used `views::Widget::RunMoveLoop` (the documented public API for window move). Returned `2` from Rust's perspective.

I spent half an hour chasing struct-layout / ABI / cef-dll-sys version mismatch theories. After verifying:
- Field counts in cef_window_capi.h matched cef-dll-sys binding (43 methods + 1 `base`)
- Field ORDER matched
- Sizes matched (post my +896 byte patch from 888)
- Cef-dll-sys 146.7 + libcef.so 146.0.7680.179 + my appended `begin_window_drag` field

…I temporarily replaced the impl with `return true;` to isolate the FFI plumbing. Got `1` back — FFI was fine. So `RunMoveLoop` was returning some value I just didn't expect, and the real question was whether RunMoveLoop is even the right API on Wayland.

It isn't. On Aura/Wayland, the right path is `WmMoveResizeHandler::DispatchHostWindowDragMovement(HTCAPTION, point)`, which dispatches `xdg_toplevel.move` directly. RunMoveLoop is a synchronous nested message loop intended primarily for X11 (`_NET_WM_MOVERESIZE` initiated synchronously). On Wayland it apparently completes immediately with some unhelpful status.

## The fix that worked

### CEF source patch (`libcef/include/views/cef_window.h` + `window_impl.{h,cc}`)

Added `bool BeginWindowDrag()` as a new public method on `CefWindow`. Crucially, **placed at the end of the class** so the generated C struct (`_cef_window_t`) gets the new function pointer appended at the end of its vtable — this preserves all existing field offsets and keeps ABI compatibility with the downstream `cef-dll-sys` Rust bindings.

```cpp
bool CefWindowImpl::BeginWindowDrag() {
  CEF_REQUIRE_VALID_RETURN(false);
  if (!widget_) return false;
#if BUILDFLAG(IS_OZONE)
  auto* native_view = widget_->GetNativeView();
  if (!native_view) return false;
  auto* host = native_view->GetHost();
  if (!host) return false;
  auto* platform_host = static_cast<aura::WindowTreeHostPlatform*>(host);
  auto* platform_window = platform_host->platform_window();
  if (!platform_window) return false;
  auto* handler = ui::GetWmMoveResizeHandler(*platform_window);
  if (!handler) return false;
  handler->DispatchHostWindowDragMovement(HTCAPTION, gfx::Point());
  return true;
#else
  return false;
#endif
}
```

The trick was finding `aura::WindowTreeHostPlatform::platform_window()` — public accessor (unlike `views::DesktopWindowTreeHostPlatform::platform_window()` which is protected). Static-cast from `aura::WindowTreeHost*` to `aura::WindowTreeHostPlatform*` is safe on Linux/Ozone because the host always *is* WindowTreeHostPlatform there.

### Translator + Rust binding patches

Ran `tools/translator.py` to regenerate the C API headers + cpptoc/ctocpp wrappers.

The `cef` Rust crate has hardcoded bindings (NOT generated from headers — they ship pre-baked in `src/bindings/x86_64_unknown_linux_gnu.rs`). And `cef-dll-sys` has hardcoded bindings too plus compile-time size asserts. Two manual patches needed:

1. **`cef-dll-sys` `_cef_window_t`** — append `pub begin_window_drag` field at the end, mirroring the C struct. Done via a Python script (`/tmp/patch_cef_dllsys.py`) so it patches all 5 cached versions.
2. **`cef-dll-sys` size assert** — bumped `888` → `896` (one extra `*const fn` = 8 bytes).

The `cef` crate doesn't have a typed wrapper for our extension method, so the call goes through raw FFI on `*mut _cef_window_t`:

```rust
let raw_ptr = <cef::Window as ImplWindow>::get_raw(&window);
unsafe {
    if let Some(f) = (*raw_ptr).begin_window_drag {
        f(raw_ptr);
    }
}
```

(Disambiguating `get_raw` is needed because `Window` implements `ImplView`, `ImplPanel`, AND `ImplWindow`, all with their own `get_raw`.)

### Rust IPC + UI-thread task

`agentmux-cef/src/ui_tasks.rs` got a `StartWindowDragTask` that runs on the CEF UI thread. The IPC handler `start_window_drag` (already wired in `ipc.rs`) posts this task. Nothing fancy — single task, single FFI call, no state.

### Frontend (`useWindowDrag.linux.ts`)

The header element gets `data-drag-region="true"` (a marker, NOT `-webkit-app-region: drag`). Document-level mousedown listener:
- Left button (button === 0) only — right-click is left to standard `contextmenu` propagation
- Records press position
- On `mousemove` exceeding 4-pixel threshold (matches Chrome's drag threshold + Mutter's input-gesture threshold), sends ONE IPC `start_window_drag` and stops listening
- Compositor handles the rest of the drag until mouseup
- Double-click on drag region toggles maximize via existing IPC

The element stays `HTCLIENT` so right-click `contextmenu` propagates normally to `.window-header`'s `handleContextMenu` handler.

## Lessons

1. **`-webkit-app-region: drag` is total**. Don't trust upstream comments / patches that promise "events will fall through" without verifying with a capture-phase listener in DevTools. Chromium's drag region implementation suppresses events at the input-routing layer, well before any handler can intercept them.
2. **The right Wayland API is `DispatchHostWindowDragMovement`, not `RunMoveLoop`.** Aura's `RunMoveLoop` is X11-flavored — it kind of works but doesn't do the right thing on Wayland for our use case. The handler-based path is non-blocking and goes directly to `xdg_toplevel.move`.
3. **When extending CEF's vtable, append at the end of the class.** Never insert in the middle. Field offsets in the generated C struct match declaration order, and downstream Rust bindings (`cef-dll-sys`, `cef`) hardcode those offsets via field positions and size asserts. End-only growth = ABI-compatible extension.
4. **`aura::WindowTreeHostPlatform::platform_window()` is public.** I wasted time digging through `views::DesktopWindowTreeHost*` looking for protected accessors. The aura layer below has the public method we wanted.
5. **Capture-phase document listener is the gold-standard diagnostic** for "is this event reaching the renderer at all." Three lines of `Runtime.evaluate` answered a question that two days of speculation couldn't.
6. **When debugging unexpected FFI return values, hardcode the C++ side first.** I burned time speculating about ABI mismatches when the simplest experiment (`return true;`) would have isolated the FFI vs. impl in 2 minutes. (And it did — `RunMoveLoop` was just behaving weirdly, not the FFI.)

## Files changed

CEF fork (`agentmux/7680-drag-rightclick-and-transparency` branch on `github.com/a5af/cef.git`):
- `include/views/cef_window.h` — add `BeginWindowDrag()` at end of `CefWindow` class
- `libcef/browser/views/window_impl.{h,cc}` — implementation calling `WmMoveResizeHandler::DispatchHostWindowDragMovement`
- Auto-generated: `include/capi/views/cef_window_capi.h`, `cef_window_capi_versions.h`, `libcef_dll/cpptoc/views/window_cpptoc.cc`, `libcef_dll/ctocpp/views/window_ctocpp.{h,cc}`

`cef-dll-sys` cargo cache (one-time edit, 5 versions in registry):
- Append `pub begin_window_drag` field to `_cef_window_t` struct
- Bump `Size of _cef_window_t` assert from 888 → 896
- Patch script: `/tmp/patch_cef_dllsys.py`

agentmux:
- `agentmux-cef/src/ui_tasks.rs` — `StartWindowDragTask` + `post_start_drag` (Linux/macOS)
- `frontend/app/hook/useWindowDrag.linux.ts` — JS-driven mousedown threshold + IPC dispatch
- `frontend/app/window/window-header.linux.scss` — removed `.tab-bar-fill { -webkit-app-region: drag }`, replaced with explanatory comment pointing to this approach

## Next steps

- The cef-dll-sys cache patch is ephemeral (will be wiped by `cargo update` or registry refresh). Two long-term options:
  - Vendor the modified cef-dll-sys (`[patch.crates-io]` in workspace `Cargo.toml`)
  - Upstream the BeginWindowDrag API to chromiumembedded/cef so the `cef` crate eventually picks it up natively
- The `widget_->IsMoveLoopSupported()` check we tried was a red herring; not needed for the WmMoveResizeHandler path.
- Consider exposing X11/Windows pointer location for the `DispatchHostWindowDragMovement` call. Currently passes `gfx::Point()` which Wayland ignores but X11 may use for `_NET_WM_MOVERESIZE`. Forward the renderer's `mousedown.screenX/screenY` through the IPC if X11 reliability proves insufficient.
