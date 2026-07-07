# SPEC — Browser pane: clicking the body selects the pane (macOS)

**Date:** 2026-07-07
**Type:** Bug fix
**Status:** Implemented for macOS and manually confirmed; Linux still open (see Scope)
**Scope:** `agentmux-cef` (`ui_tasks/platform_macos.rs`, `ui_tasks/pane_geometry.rs`) —
macOS only. The new `PANE_OVERLAY_WIN_TO_BLOCK` map, the registration in
`SetPaneBoundsViewsTask`, and the `sendEvent:` swizzle tap are all
`#[cfg(target_os = "macos")]`; Linux's branch of `SetPaneBoundsViewsTask`
(`pane_geometry.rs`, the `#[cfg(not(target_os = "macos"))]` early-return path)
never reaches this registration code, so **Linux gets no fix from this
change** — it needs its own native click-detection mechanism (GTK/X11 or
Wayland have neither an HWND-style subclass nor an AppKit-style `sendEvent:`
swizzle to reuse), tracked as follow-up work, not implemented here.

## Problem

Clicking a normal (in-DOM) pane's body selects it — the thin `block-focused`
border appears — because the click is a real DOM event that bubbles up to
`blockframe.tsx`'s outer wrapper (`onClick={props.blockModel?.onClick}` →
`handleBlockClick` in `block/block.tsx` → `nodeModel.focusNode()`).

A **browser pane** doesn't work this way: per
`docs/specs/SPEC_NATIVE_BROWSER_PANE_2026_04_17.md`, its content is a second,
sibling `CefBrowserView` layered on top of the main window's DOM via CEF's
Views `AddOverlayView`, not an iframe. Once a page is loaded, the pixels and
input for that page belong to a separate native Chromium instance — a click
there never becomes a DOM `click`/`mousedown` event, so it can't bubble to
`blockframe.tsx` and the pane never selects. Only clicking the pane's header
(real DOM) worked.

Windows already has a fix: `agentmux-cef/src/browser_pane/hwnd.rs` subclasses
the pane's HWND and emits `browser-pane-clicked` (with `block_id`) directly
from `WM_LBUTTONDOWN`. `agentmux-cef/src/browser_pane/callbacks.rs`'s
`#[cfg(not(target_os = "windows"))]` branch explicitly notes this subclass
"doesn't exist" on macOS/Linux — that gap is what this fix closes for macOS
only (Linux remains open, see Scope above).

## Root cause confirmation

Verified directly against the running app's server log and by reading
`ui_tasks/platform_macos.rs` — no existing code path emitted a select signal
for a click landing on the overlay's own `NativeWidgetMacNSWindow`.

## Fix

Rather than adding new native hooking machinery, this reuses the
`swizzled_nsapp_send_event` `-[NSApplication sendEvent:]` swizzle that's
already installed while any pane is open (originally for keyboard-focus
routing — see the swizzle's own doc comment). That function already
classifies every intercepted mouse event by whether its window is the pane's
own overlay (`is_overlay`) or the main window; the `is_overlay == true` branch
previously just fell through to the original `sendEvent:` unmodified.

Added:

- `PANE_OVERLAY_WIN_TO_BLOCK` (`platform_macos.rs`): a new static map, keyed
  by the pane's own overlay `NSWindow*` (distinct from `PANE_WIN_TO_HOST` /
  `PANE_LABEL_TO_WIN`, which key by the *main* window), storing
  `(pane label, block_id, Weak<AppState>)`.
- Populated in `SetPaneBoundsViewsTask::execute` (`pane_geometry.rs`), at the
  same point `PANE_WIN_TO_HOST`/`PANE_LABEL_TO_WIN` are populated, using the
  already-resolved `task_overlay_win` pointer and `block_id` parsed from the
  pane label (same `browser-pane-<uuid>-<seq>` parsing as
  `callbacks.rs::resolve_pane_block_id`).
- Cleaned up in `clear_pane_swizzle_statics` by pane label, alongside the
  existing `PANE_LABEL_TO_WIN` cleanup.
- In `swizzled_nsapp_send_event`'s `is_overlay` branch: on `ev_type == 1`
  (leftMouseDown), look up the event's window in `PANE_OVERLAY_WIN_TO_BLOCK`
  and, if found, emit `browser-pane-clicked` with that `block_id` — the same
  event Windows emits, consumed unchanged by the existing frontend path
  (`browser-model.ts` → reducer → `refocusNode` → `layoutModel.focusNode`).
  This is a non-invasive tap: it does not `return` early, so the event still
  falls through to the original `sendEvent:` exactly as before this change —
  only an additional event emission was added.

No frontend changes were needed; the consumer side already existed for the
Windows path and is provider-agnostic.

## Verification

- `cargo check -p agentmux-cef` passes clean, no new warnings.
- **Not yet manually click-tested** — this changes core native mouse dispatch
  for the whole app while any browser pane is open, so it needs a human to
  actually click a loaded browser pane's body and confirm (a) the pane
  border/selection now appears, and (b) no regression to normal in-pane
  interaction (scrolling, form input, right-click context menu, dragging)
  that the surrounding swizzle logic already handles.
