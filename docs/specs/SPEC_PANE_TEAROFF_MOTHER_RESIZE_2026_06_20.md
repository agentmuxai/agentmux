# Pane Tear-Off — Mother Window Resize

**Date:** 2026-06-20
**Status:** Proposed
**Issue:** TBD
**Extends:** `SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md` (Windows), `SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md` (macOS/Linux)

---

## 1. Problem

When a pane is torn off, the mother window keeps its full size. The remaining panes
reflow to fill the vacated space, which changes their sizes unexpectedly. The tear-off
feels like a "move" rather than a "split."

The intended UX: tearing a pane off should feel like physically splitting the window
along the pane's boundary. The mother shrinks to exactly the space its remaining
content occupies; the floater takes the pane's original footprint. Remaining panes do
not change size.

---

## 2. Scope

**In scope:**
- Panes that span the full height of the layout container (top-to-bottom columns). These
  represent a clean vertical split — removing one shrinks the mother window horizontally.
- Cross-platform: Windows, macOS, Linux.

**Out of scope / no-op:**
- Panes that share vertical space with at least one sibling at the same row level (i.e.,
  the pane does NOT reach both the top and bottom of the layout container). The mother
  window is unchanged; remaining siblings reflow normally into the vacated slot. This is
  the existing behavior and it is intentionally preserved — a pane that shares a row with
  others cannot cleanly "split off" a column.
- Tab tear-off (spawns a new full instance — out of scope).
- Single-pane layouts (one leaf = the entire window; nothing to shrink to).
- The mother window would shrink below `MIN_MOTHER_WIDTH = 400` CSS px. Clamp to
  `MIN_MOTHER_WIDTH` in that case (no resize if clamping would still make the window
  too small to be usable).
- Magnified-node layouts (one pane covers the others — geometry detection unreliable;
  skip resize when `magnifiedNodeId` is set).

---

## 3. Full-Height Detection

A pane **spans full height** when its CSS DIP rect (as reported by
`getBoundingClientRect`) touches both the top and bottom of the layout container,
within a 2 px tolerance to absorb sub-pixel rounding:

```typescript
const EPSILON_PX = 2;

function spansFullHeight(paneRect: DOMRect, containerRect: DOMRect): boolean {
    return (
        paneRect.top - containerRect.top <= EPSILON_PX &&
        containerRect.bottom - paneRect.bottom <= EPSILON_PX
    );
}
```

`containerRect` is the bounding rect of the `TileLayout` display container (the element
whose ref is `model.displayContainerRef`). This excludes the title bar, tab strip, and
status bar — only the pane grid area matters.

The `paneRect` is obtained by:
```typescript
const el = document.querySelector(`[data-blockid="${blockId}"]`) as HTMLElement | null;
const paneRect = el?.getBoundingClientRect();
```

This MUST be called before `TearOffBlock` (identical timing constraint to
`measureSourcePaneSize` which already runs first).

---

## 4. New Mother Resize Width Computation

When `spansFullHeight` is true:

```typescript
const newMotherWidthCss = Math.round(
    containerRect.width - paneRect.width
);
const belowMinimum = newMotherWidthCss < MIN_MOTHER_WIDTH;
```

`newMotherWidthCss` is in CSS DIP pixels (same coordinate space as the floater
`width`/`height` parameters already sent to the host). The gap between panes (the resize
handle overlay) is part of the pane's own `getBoundingClientRect` footprint on whichever
side it absorbs it, so no gap adjustment is needed — the panes are flex-contiguous at
the CSS level.

If `belowMinimum` is true, omit `mother_resize_to_width` from the IPC (treat as
no-resize path).

---

## 5. IPC Changes

### 5.1 `open_floating_pane_window` — new optional field

Add to `OpenFloatingPaneArgs` (both the TypeScript call sites and the Rust struct):

```rust
/// New mother-window width in CSS/DIP pixels after the pane is torn off.
/// Present only when the pane spans the full height of the layout container
/// AND the resulting width would be ≥ MIN_MOTHER_WIDTH (400 CSS px).
/// Absent = no resize (partial-height pane, single pane, or too-narrow result).
#[serde(default)]
pub mother_resize_to_width: Option<i32>,
```

On Windows, the host converts this DIP value to physical pixels using the
**source window's** monitor DPI (same `MonitorFromPoint` + `GetDpiForMonitor` pattern
used for the floater's `width`/`height` conversion, but using the source window's
current top-left position rather than the cursor position).

### 5.2 TypeScript call sites

`CrossWindowDragMonitor.win32.tsx` and `CrossWindowDragMonitor.linux.tsx` — both call
`open_floating_pane_window` in `performTearOff`. Before that call, add the detection and
pass `mother_resize_to_width` when appropriate:

```typescript
// Snapshot BEFORE TearOffBlock (DOM element still present).
const containerEl = props.layoutModel.displayContainerRef.current;
const containerRect = containerEl?.getBoundingClientRect();
const paneEl = document.querySelector(`[data-blockid="${blockId}"]`) as HTMLElement | null;
const paneRect = paneEl?.getBoundingClientRect();
const motherResizeWidth =
    containerRect && paneRect && spansFullHeight(paneRect, containerRect)
        ? Math.round(containerRect.width - paneRect.width)
        : undefined;
const motherResizeWidthSafe =
    motherResizeWidth != null && motherResizeWidth >= MIN_MOTHER_WIDTH
        ? motherResizeWidth
        : undefined;

// ... then later in the invokeCommand call:
await invokeCommand("open_floating_pane_window", {
    pane_id: payload.blockId,
    workspace_id: newWsId,
    x: screenX,
    y: screenY,
    width: floaterWidth,
    height: floaterHeight,
    source_window_label: sourceWindowLabel,
    mother_resize_to_width: motherResizeWidthSafe,  // NEW
});
```

The `props.layoutModel` reference is available on the `CrossWindowDragMonitor`
component via the standard `getLayoutModelForStaticTab()` call that already exists
in these files (see line 348 of `.win32.tsx`). The snapshot must happen before
`TearOffBlock` (the pane is removed from the layout tree at that point).

---

## 6. Host Implementation

### 6.1 Rust struct update (`commands/floating_pane.rs`)

```rust
pub struct OpenFloatingPaneArgs {
    // ... existing fields ...
    #[serde(default)]
    pub mother_resize_to_width: Option<i32>,
}
```

### 6.2 Apply the resize after the floating pane IPC returns

The resize is applied at the very end of `open_floating_pane_window`, after the floating
pane creation IPC has been posted but before returning the response. The source window
resize is posted to the UI thread so it runs on the next frame (same pattern as
`promote_pool_window`'s `wrap_task!`).

#### Windows (`#[cfg(target_os = "windows")]`)

```rust
if let Some(target_dip_w) = parsed.mother_resize_to_width {
    if let Some(source_label) = parsed.source_window_label.as_deref() {
        if let Some(&source_hwnd) = state.window_hwnds.lock().get(source_label) {
            // Convert DIP → physical px using the source window's monitor.
            let dpi_scale: f32 = unsafe {
                use windows_sys::Win32::Foundation::POINT;
                use windows_sys::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST};
                use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
                use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
                let mut wr = std::mem::zeroed::<windows_sys::Win32::Foundation::RECT>();
                GetWindowRect(source_hwnd as *mut _, &mut wr);
                let pt = POINT { x: wr.left, y: wr.top };
                let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
                let mut dpi_x: u32 = 0;
                let mut dpi_y: u32 = 0;
                let hr = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
                if hr != 0 || dpi_x == 0 { 1.0 } else { dpi_x as f32 / 96.0 }
            };
            let new_w_px = (target_dip_w as f32 * dpi_scale).round() as i32;
            crate::ui_tasks::post_resize_window_width(state, source_hwnd as isize, new_w_px);
        }
    }
}
```

`post_resize_window_width` posts a UI-thread task that calls:
```rust
// ui_tasks.rs
unsafe {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SWP_NOMOVE, SWP_NOZORDER, SWP_NOACTIVATE,
    };
    // Preserve current height; only shrink width.
    let mut wr = std::mem::zeroed::<windows_sys::Win32::Foundation::RECT>();
    windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd as *mut _, &mut wr);
    let current_h = wr.bottom - wr.top;
    SetWindowPos(
        hwnd as *mut _,
        std::ptr::null_mut(),
        0, 0,
        new_w_px,
        current_h,
        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
}
```

#### macOS/Linux (`#[cfg(not(target_os = "windows"))]`)

CEF Views `set_bounds` runs on the UI thread. Post via `wrap_task!`:

```rust
if let Some(target_dip_w) = parsed.mother_resize_to_width {
    if let Some(source_label) = parsed.source_window_label.as_deref() {
        let label = source_label.to_string();
        let state_clone = state.clone();
        crate::ui_tasks::post_set_window_width(state_clone, label, target_dip_w);
    }
}
```

`post_set_window_width` wraps a UI task that looks up the CEF `Window` by label
(via `cef::WindowDelegate`'s stored label → window map, or the existing
`state.cef_windows` map if one exists), reads its current bounds, and calls
`window.set_bounds(Some(&Rect { x: old_x, y: old_y, width: target_dip_w, height: old_h }))`.

On macOS, CEF Views handles the `NSWindow` resize natively. On Linux (X11/Wayland via
GTK), CEF Views `set_bounds` calls through to `gtk_window_resize`. Both are tested
via the existing `set_bounds` usage in the pool-promote path (see `ui_tasks.rs:849`).

---

## 7. Layout Side Effect: Flex-Ratio Preservation

When the mother window shrinks by exactly the pane's width, the remaining panes'
**CSS pixel widths are preserved without any additional layout action**:

- Before: panes B (300 px) and C (500 px) in a 800 px column (after A's 400 px is
  removed). Their flex ratios: B = 300/800 = 37.5%, C = 500/800 = 62.5%.
- After mother resize to 800 px: layout container is now 800 px. Flex ratios unchanged.
  B = 300 px, C = 500 px. ✓

This holds for any ratio because the remaining panes' proportional sizes are
unchanged relative to the new container. The layout model already handles this
correctly — no additional flex recalculation is needed.

---

## 8. Platform Coordinate Summary

| Platform | `mother_resize_to_width` unit | Host converts? | API |
|---|---|---|---|
| Windows | CSS/DIP px (same as floater w/h) | Yes: × (source monitor DPI / 96) | `SetWindowPos` with `SWP_NOMOVE \| SWP_NOZORDER \| SWP_NOACTIVATE` |
| macOS | CSS/DIP px | No | CEF Views `window.set_bounds()` on UI thread |
| Linux | CSS/DIP px | No | CEF Views `window.set_bounds()` on UI thread |

---

## 9. Edge Cases and Guards

| Case | Behavior |
|---|---|
| Single-pane layout | `containerRect.width - paneRect.width ≈ 0` → below `MIN_MOTHER_WIDTH` → no resize |
| Pane is partial-height (shares a row) | `spansFullHeight` returns false → no resize; siblings expand to fill |
| Resulting width < `MIN_MOTHER_WIDTH` (400 px) | Omit `mother_resize_to_width` → no resize |
| `source_window_label` absent or unresolved | Log warn, skip resize silently; floater still opens normally |
| `magnifiedNodeId` set | Skip detection entirely (add `if (layoutModel?.treeState.magnifiedNodeId) return` before the resize measurement) |
| Multiple monitors (different DPI) | Windows: DPI lookup uses source window's monitor (not cursor monitor) via `GetWindowRect` + `MonitorFromPoint` |
| Window already at minimum size | `SetWindowPos`/`set_bounds` no-ops at the OS level; no crash risk |
| Rapid back-to-back tear-offs | Each invocation of `open_floating_pane_window` carries its own `mother_resize_to_width`; resize tasks are independent, last one wins |
| macOS fullscreen | CEF `set_bounds` in fullscreen is a no-op; acceptable — fullscreen pane tears don't need a resize |

---

## 10. Testing Checklist

- [ ] **2-column layout, left pane torn off** — mother shrinks by left pane width; right pane keeps exact pixel width
- [ ] **2-column layout, right pane torn off** — same, mirrored
- [ ] **3-column layout, center pane torn off** — center pane spans full height → mother shrinks by center pane width; outer two panes keep their widths
- [ ] **Horizontal split (2 rows)** — each pane is half-height → `spansFullHeight` = false → no resize
- [ ] **Mixed layout** (one tall left column, two stacked right panes) — left pane → full-height → mother shrinks; either right pane → partial-height → no resize
- [ ] **Single pane** — no resize (min width guard)
- [ ] **Narrow result (< 400 px)** — no resize
- [ ] **Windows HiDPI** — tear on a 150% DPI monitor: floater is correct size, mother shrinks to correct physical width
- [ ] **Cross-DPI** — source window on 100% monitor, cursor on 150% monitor: floater uses 150% DPI, mother resize uses 100% DPI (source window's monitor, independent)
- [ ] **macOS** — same layout scenarios; CEF Views `set_bounds` path exercised
- [ ] **Linux** — same layout scenarios
- [ ] **Retry path (H.7 gate)** — `open_floating_pane_window` retried after 350 ms; `mother_resize_to_width` persists through the retry (it's in the args object, not re-measured)
- [ ] **Pool path** — when pool promotes instead of cold-spawning, the mother resize still fires (it's in `open_floating_pane_window` args; the pool path is a fast-path INSIDE that handler, not a bypass)

---

## 11. Files to Change

| File | Change |
|---|---|
| `frontend/app/drag/CrossWindowDragMonitor.win32.tsx` | Add `spansFullHeight` detection before `TearOffBlock`; pass `mother_resize_to_width` |
| `frontend/app/drag/CrossWindowDragMonitor.linux.tsx` | Same (Linux/macOS share this path) |
| `frontend/app/drag/tear-off-pool-helper.ts` | Export `MIN_MOTHER_WIDTH`, `spansFullHeight` helper |
| `agentmux-cef/src/commands/floating_pane.rs` | Add `mother_resize_to_width: Option<i32>` to `OpenFloatingPaneArgs`; call resize task |
| `agentmux-cef/src/ui_tasks.rs` | Add `post_resize_window_width` (Windows) and `post_set_window_width` (non-Windows) |

No changes to `agentmux-srv`, the layout reducer, or the floating pane pool. The resize
is purely a host-side window-management operation triggered at tear-off time.

---

## 12. Open Questions

1. **Animation** — should the mother resize animate (slide) or snap? Initial proposal:
   snap (instant `SetWindowPos` / `set_bounds`). The floater appears at the same time,
   creating the visual illusion of the pane physically detaching. An animation could
   conflict with the OS window-resize animation on Windows 11.

2. **macOS CEF Window lookup** — the non-Windows path needs a label → CEF `Window`
   map. Verify that `state.cef_windows` (or equivalent) exists and is populated for
   all top-level windows; if not, a new map is needed (similar to `window_hwnds`).

3. **Wayland** — `gtk_window_resize` behavior under Wayland compositors varies. The
   `set_bounds` path is already used for pool-promote on Linux; if it works there, it
   should work here.
