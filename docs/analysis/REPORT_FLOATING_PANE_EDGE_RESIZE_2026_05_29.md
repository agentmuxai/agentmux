# Floating-pane edge-resize is a no-op — why, and what it takes

**Date:** 2026-05-29
**Author:** AgentA
**Status:** Investigation / implementation plan
**Symptom:** Hovering/dragging the edge of a torn-off floating pane does nothing — no resize cursor, no resize. It *looks* like it should work (the window is `WS_THICKFRAME` and the wndproc maps edge zones), but it's inert.

## TL;DR

The floater's **parent** window is fully wired for native edge-resize — but the **embedded CEF child fills the entire client area, including the 6px edge border**, so the child receives the edge mouse events and the parent's `WM_NCHITTEST` resize zones are never reached. The missing piece is the **`HTTRANSPARENT` `WM_NCHITTEST` forwarder** on the CEF child wndproc(s) — explicitly deferred in #1159 ("Out of scope: edge-resize … the HTTRANSPARENT WM_NCHITTEST forwarder — lands separately") and part of #1132's parked resize work.

## What's already in place (and correct)

- **`floating_pane.rs::floating_pane_wndproc`** handles `WM_NCHITTEST` and returns `HTLEFT/HTRIGHT/HTTOP/HTBOTTOM/HTTOPLEFT/…` for a `RESIZE_BORDER_CSS = 6` px border (DPI-scaled). Corner + edge zones all mapped.
- The floater is created `WS_POPUP | WS_THICKFRAME` (`create_owned_popup`) — `WS_THICKFRAME` is what lets Windows run the native resize modal loop once `WM_NCHITTEST` returns an `HT*` edge code.
- **`WM_SIZE` handler (fixed in #1173)** resizes the floater's frontend browser to the client rect on any outer-window resize, and the frontend reflows + repositions its web-content child. So once a resize *happens*, the content already follows correctly.

So the resize **response** path is done. Only the resize **initiation** is broken.

## Root cause — the CEF child swallows the edge

`WM_NCHITTEST` is dispatched to the window the cursor is over. A floater embeds its browser via `WindowInfo::set_as_child`, so the CEF child HWND covers the floater's full client area **including the 6px edge strip**. When the cursor is at the edge it is over the *child*, which hit-tests as `HTCLIENT` and consumes the mouse-down — the parent floater's edge `WM_NCHITTEST` zones are never reached, so the native resize loop never starts.

For a **browser** pane there are *two* covering children (see #1173): the frontend browser (fills the floater) and the native web-content window (content area, on top). Both can cover an edge depending on which edge, so the fix must apply to whichever child occupies the border.

## The fix — `HTTRANSPARENT` forwarder on the child wndproc

Subclass the CEF child's wndproc so that, for cursor positions inside the floater's resize border, it returns **`HTTRANSPARENT`**. `HTTRANSPARENT` tells Windows to continue hit-testing the window *beneath* — falling through to the parent floater, whose `WM_NCHITTEST` then returns the `HT*` edge code → native resize runs. Everywhere else the child returns its normal value (clicks reach CEF / the JS header-drag handler unchanged).

Prior art for both halves exists:
- **`client/wndproc.rs::install_frameless_resize_hook`** — subclasses a *secondary* (full) window's wndproc to return `HT{LEFT,…}` at edges. Same hit-test math; the floater needs the *child-forwarder* variant (return `HTTRANSPARENT`, not the `HT*` code, since the parent owns the resize).
- **`browser_pane/hwnd.rs`** — the established pattern for subclassing a CEF child's wndproc (focus-redirect today) with `SetWindowLongPtrW(GWLP_WNDPROC)` + `CallWindowProcW` delegation. The forwarder slots into the same hook.

The border math should reuse `floating_pane_wndproc`'s `RESIZE_BORDER_CSS` (6px, DPI-scaled) so the child's transparent strip and the parent's resize strip line up exactly.

## Pieces that compose with this

1. **`WM_SIZE` reflow** — DONE (#1173). The frontend browser resizes + repositions its web-content child. No change needed.
2. **`ReportNormalRect` (reducer, deferred)** — `reducer/pane_window.rs`'s module doc lists this as a later phase: a `WM_WINDOWPOSCHANGED`-debounced command that updates `pane_window_states[label].last_known_normal_rect`. **Edge-resize needs it**: after the user resizes, a later maximize→restore must restore to the *new* size, not the tear-off size. Without it, resize and maximize/restore don't compose. This is the natural moment to land `ReportNormalRect`.
3. **Resize dimension overlay** — `SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md` currently shows the `WxH` badge only while a **tile-layout splitter** is dragging (`layoutModel.isSplitterDragging()`). Extending it to also show during a floater edge-resize is a nice-to-have (the spec explicitly says the badge "never affects hit-testing", so it's orthogonal to the forwarder).

## Risk — redock hit-testing (why #1132 deferred this)

#1159 notes maximize was kept "independent of edge-resize: it does NOT install the HTTRANSPARENT WM_NCHITTEST child-subclass, so it cannot perturb the redock hit-testing the way #1132's resize work did." The forwarder changes what the child returns from `WM_NCHITTEST`, which is the same surface the floater header-drag (JS-driven) and the redock flow (`resolve_window_at_cursor` Z-order walk) interact with. **Mitigations:**
- Scope the transparent return strictly to the `RESIZE_BORDER_CSS` border; the header-drag region and content interior are untouched (still `HTCLIENT`).
- The redock resolver walks *top-level* windows by process and matches against `window_hwnds`; it doesn't depend on child hit-testing, so a child returning `HTTRANSPARENT` at the border shouldn't change which top-level the cursor resolves to. Verify explicitly during the tear-off↔redock smoke (the bug class we just stabilized).

## Specs / references
- `docs/specs/SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md`, `…_CROSS_PLATFORM_2026-05-26.md` — floater design.
- `docs/specs/SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md` — WxH badge.
- `docs/specs/SPEC_PANE_STATE_REDUCER_2026-05-28.md` + `reducer/pane_window.rs` module doc — `ReportNormalRect`/`ReportOSPlacementChange` deferred phases.
- #1132 (parked resize+maximize, the original edge-resize attempt), #1159 (maximize-only, deferred edge-resize), #1173 (WM_SIZE resizes frontend child).

## Proposed plan (phased)
1. **Forwarder** — child-wndproc `HTTRANSPARENT` forwarder for the resize border (shared border math with the parent). Install on the floater's CEF child(ren). → native edge-resize starts; `WM_SIZE` (done) handles the reflow.
2. **Reducer** — land `ReportNormalRect` so resize updates `last_known_normal_rect`; resize + maximize/restore compose.
3. **Overlay (optional)** — extend the WxH badge to floater edge-resize.

## Verification
- Live: edge-hover shows resize cursor; drag resizes; content reflows (header stays, web content follows — #1173); resize then maximize→restore returns to the resized size (needs step 2).
- Regression: tear-off → redock still works (forwarder must not perturb redock hit-testing); header-drag still works.
- Both browser and terminal/agent floaters.
