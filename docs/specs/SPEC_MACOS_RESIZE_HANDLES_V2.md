# macOS Window Resize Handles — Root Cause Analysis v2

**Date:** 2026-03-16
**Status:** PR #131 fix did not resolve the issue
**Related:** PR #131 (`fix/macos-resize-handles`), Tauri Issue #8519, #7900, #3040

---

## 1. What PR #131 Tried

PR #131 hypothesized that the SolidJS migration left `draggable={true}` on all tile
nodes unconditionally, intercepting pointer events at window edges before NSWindow
resize hit-testing. The fix gated `draggable` on `dragReady` (only true when the
user presses on the block header).

**Result:** Resize handles still don't work. The hypothesis was wrong — or at least
incomplete. The `draggable` attribute is not the root cause.

---

## 2. The Real Root Cause (from Tauri/tao source code)

The issue is fundamental to how Tauri handles frameless windows on macOS:

### 2.1 — `startResizeDragging()` is NOT SUPPORTED on macOS

In `tao` (Tauri's windowing library), `drag_resize_window()` explicitly returns
`ExternalError::NotSupported` on macOS:

```rust
// tao/src/platform_impl/macos/window.rs:963
pub fn drag_resize_window(&self, _direction: ResizeDirection) -> Result<(), ExternalError> {
    Err(ExternalError::NotSupported(NotSupportedError::new()))
}
```

The tao CHANGELOG confirms: "Add `Window::drag_resize_window` — **Supported on
Windows and Linux only.**"

### 2.2 — Tauri's in-client resize JavaScript is DISABLED on macOS

Tauri PR #8537 added JavaScript-based resize handling (mousemove hit testing +
cursor changes + `startResizeDragging()`) for undecorated windows. However, this
code is **conditionally excluded on macOS** because `startResizeDragging` doesn't
work there.

### 2.3 — Native resize edges exist but are nearly invisible

When tao creates a frameless window, it uses:
```
NSWindowStyleMask::Borderless | NSWindowStyleMask::Resizable
```

macOS does provide resize edges with this mask, but they are **extremely thin**
(~1-2 physical pixels on Retina displays). Combined with `transparent: true`,
the resize targets are virtually impossible to hit.

### 2.4 — Summary

| Platform | Resize mechanism | Status |
|----------|-----------------|--------|
| Windows | `WM_NCHITTEST` below WebView layer | Works |
| Linux | X11/Wayland native + Tauri in-client JS | Works |
| macOS | Thin native NSWindow edges only | Broken (too thin) |

The resize problem on macOS has **nothing to do with `draggable` attributes**. It's
that Tauri provides no working resize mechanism for frameless windows on macOS.

---

## 3. Available Solutions (ranked)

### Solution A: NSWindow styleMask — keep `.titled`, hide titlebar (Recommended)

Instead of `decorations: false` (which uses `Borderless`), use the `.titled` mask
with a transparent hidden titlebar. This preserves native resize handles, rounded
corners, and traffic lights while giving a frameless appearance.

**Rust implementation** (in `src-tauri/src/lib.rs`, after window creation):

```rust
#[cfg(target_os = "macos")]
{
    use objc2_app_kit::{NSWindow, NSWindowStyleMask, NSWindowTitleVisibility};
    use objc2_foundation::MainThreadMarker;

    if let Ok(ns_window) = window.ns_window() {
        let ns_window: &NSWindow = unsafe { &*(ns_window as *const NSWindow) };
        unsafe {
            let mask = NSWindowStyleMask::Titled
                | NSWindowStyleMask::Resizable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::FullSizeContentView;
            ns_window.setStyleMask(mask);
            ns_window.setTitlebarAppearsTransparent(true);
            ns_window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        }
    }
}
```

**Pros:**
- Native resize handles with proper width (~8px hit target)
- Rounded corners preserved
- Traffic light buttons preserved (can position with `setTrafficLightsInset`)
- No JavaScript resize logic needed
- Works with `transparent: true` and `window-vibrancy`

**Cons:**
- Titlebar area technically exists (though invisible)
- Need to handle traffic light positioning
- `decorations: false` stays in tauri.conf.json (the Rust code overrides at runtime)

**Alternatives:**
- Use `titleBarStyle: "Overlay"` in tauri.conf.json instead of runtime manipulation
  (simpler but less control)
- Use `tauri-plugin-decorum` (third-party, does the same thing)

### Solution B: Custom JavaScript resize zones (Fallback)

Implement manual resize by tracking mouse position near edges and calling
`setSize()` / `setPosition()` via the Tauri window API.

**How it works:**
1. On `mousemove`, detect if cursor is within 8px of any window edge
2. Change cursor to appropriate resize cursor (n-resize, se-resize, etc.)
3. On `mousedown` in a resize zone, track mouse movement and call
   `window.setSize()` / `window.setPosition()` on each frame
4. On `mouseup`, stop tracking

**Pros:**
- Works on all platforms
- No native code changes

**Cons:**
- `setSize()` is async — causes jitter/lag during resize
- Complex coordinate math with scale factor
- Resizing from left/top edges requires simultaneous position + size changes
- Not as smooth as native resize

### Solution C: `titleBarStyle: "Overlay"` in tauri.conf.json

The simplest possible change — use Tauri's built-in overlay titlebar:

```json
{
  "windows": [{
    "titleBarStyle": "Overlay",
    "hiddenTitle": true,
    "transparent": true,
    "decorations": true
  }]
}
```

**Pros:** One-line config change, native resize, no code.
**Cons:** Less control over titlebar behavior, macOS traffic lights always visible,
behavior varies by macOS version.

---

## 4. Recommendation

**Solution A** (NSWindow styleMask override) is the right approach. It:
- Fixes resize with native macOS behavior (no jitter, proper cursors)
- Preserves the frameless aesthetic
- Only affects macOS (Windows/Linux unchanged)
- Is the same technique used by Electron's `titleBarStyle: 'hidden'`
- Is what `tauri-plugin-decorum` does under the hood

The key insight is: **don't fight macOS by removing decorations entirely**. Instead,
keep the native `.titled` mask and make the titlebar invisible. macOS was designed
to resize `.titled` windows — fighting this with `Borderless` is a losing battle.

---

## 5. What to Do with PR #131

The `dragReady` gating in PR #131 is not wrong — it's a minor optimization that
prevents unnecessary `draggable` attributes. But it doesn't fix the resize issue.

Options:
1. **Close PR #131**, open a new PR with Solution A
2. **Amend PR #131** to include Solution A alongside the dragReady change

---

## 6. macOS Tahoe (26.x) Warning

There is a reported regression in macOS Tahoe (26.3+) where removing `.titled`
from the styleMask at runtime can break resize entirely. Since Solution A **adds**
`.titled` rather than removing it, this should not be affected. But worth testing
on Tahoe specifically.

---

## 7. References

- [tao source: drag_resize_window returns NotSupported on macOS](https://github.com/tauri-apps/tao/blob/dev/src/platform_impl/macos/window.rs)
- [Tauri Issue #8519 — V2 custom titlebar unable to resize when decorations=false](https://github.com/tauri-apps/tauri/issues/8519)
- [Tauri Issue #7900 — data-tauri-drag-resize-region (closed: Not Planned)](https://github.com/tauri-apps/tauri/issues/7900)
- [Tauri Issue #3040 — Frameless windows difficult to resize on high-DPI](https://github.com/tauri-apps/tauri/issues/3040)
- [Tauri PR #8537 — Fix undecorated window resizing (excludes macOS)](https://github.com/tauri-apps/tauri/pull/8537)
- [tauri-plugin-decorum — overlay titlebar approach](https://github.com/clearlysid/tauri-plugin-decorum)
- [Electron frameless windows docs](https://www.electronjs.org/docs/latest/tutorial/window-customization)
- [Apple NSWindow.StyleMask docs](https://developer.apple.com/documentation/appkit/nswindow/stylemask)
- [macOS Tahoe resize regression](https://noheger.at/blog/2026/02/12/resizing-windows-on-macos-tahoe-the-saga-continues/)
