# Retro: pane tear-off shows the OS "no-drop" cursor on Windows — a fix that shipped for tabs was never ported to panes

**Date:** 2026-07-27
**Severity:** Low — cosmetic, no functional breakage (the tear-off itself works), but reads as broken/unpolished on every single Windows pane drag.
**Observed by:** user, while testing the floating-pane resize-hit-target fix (PR #2332).

> **Update (2026-07-29):** the "Fix direction" below was implemented
> (`fix/pane-tearoff-cursor-windows`, PR #2335) and **found insufficient in
> live testing** — automated review at the time also flagged (P2, unaddressed
> before this update) that a window-level `dragover` listener setting
> `dropEffect="copy"` whenever the cursor is outside the tile layout would
> overwrite the tab strip's own drop-target `dropEffect` for in-app tab
> redock, a correctness bug on top of the cursor not actually landing. Root
> cause of the insufficiency, confirmed via direct research in a follow-up
> session: Chromium's own `IDropSource::GiveFeedback` calls `SetCursor`
> continuously during the OLE drag session this whole mechanism runs on top
> of, based on its own drop-target hit-testing — no amount of `dropEffect`
> tuning or cursor-polling can reliably out-race that. The follow-up session
> explored eliminating the OLE session entirely (native Pointer Events +
> `setPointerCapture`, no `draggable()`) instead — see
> `docs/retro/retro-native-pointer-drag-tearoff-shelved-2026-07-29.md` for
> that attempt's own outcome (also not shipped, but for different, unrelated
> reasons — drop-zone/ghost regressions from rebuilding pragmatic-dnd's
> functionality from scratch, not the cursor mechanism itself). That
> follow-up also **revived** the `set_drag_cursor`/`restore_drag_cursor` API
> this retro's own "Prevention" section says not to revive — worth flagging
> explicitly: reviving it was correct *given the native-pointer-capture
> context* (no OLE session left to fight, so a direct `SetCursor()` call
> should reliably stick, unlike this retro's `dropEffect` proposal or the
> earlier native `SetSystemCursor`-polling-thread attempt this retro also
> describes) — the original "don't revive" advice assumed the OLE drag
> session was staying in place, which the newer session's approach doesn't.
> The rest of this document (root cause of the *original* circle-slash bug,
> why it wasn't caught as "the same bug twice") is still accurate and worth
> reading; only the "Fix direction" is superseded.

---

## TL;DR

Dragging a pane header off the tile layout to tear it into a floating window shows the OS circle-slash ("not-allowed") cursor on Windows, instead of a cursor indicating "this will create a new window." This is **not a regression** in the strict sense — as far as mainline history shows, Windows pane tear-off has never had a working cursor. The identical problem for **tab** tear-off was diagnosed and fixed on 2026-07-16 (PR #2175, commit `a9d291b1`), but that fix was explicitly scoped to tabs only and the pane half of the same bug was never picked up.

## Root cause

This app uses `@atlaskit/pragmatic-drag-and-drop` (standard HTML5 DnD under the hood) for both tab and pane header dragging. HTML5 DnD shows the browser/OS "not-allowed" cursor whenever the pointer is over a region that isn't a registered drop target and nothing calls `preventDefault()` + sets `dataTransfer.dropEffect` during `dragover`.

- **macOS/Linux panes**: `preventUnhandled.start()` (pragmatic-dnd's own helper) is called on drag start, which makes every element a valid drop target for the session — so the cursor never goes to "not-allowed."
- **Windows panes and tabs**: `preventUnhandled` is deliberately *not* used (`tab-reorder.ts:97-99` explains why — Windows tear-off relies on a native window-move handshake, not the HTML5 snapback `preventUnhandled` provides). Without it, Windows needs its own `dragover` listener that manually calls `preventDefault()` and sets `dropEffect` whenever the cursor is outside the tile layout / tab strip's own drop targets.
- **Tabs got this listener** (`tab-reorder.ts:95-134`, PR #2175) — a Windows-gated `window.addEventListener("dragover", ...)` that sets `dropEffect = "copy"` once the drag leaves the tab strip's bounds.
- **Panes never got the equivalent** in `TileLayout.win32.tsx`. The file already has a bounds-checking `dragover`/`onWindowDragOver` handler (`checkForCursorBounds`, lines 148-175) for a different purpose (clearing a pending drop-target highlight) — it just never sets `dropEffect`.

## Why this wasn't caught as "the same bug, twice"

PR #2175's own commit message and PR description scope the fix explicitly to tabs ("kill drag circle-slash (Windows)" in the tab-reorder context) and don't cross-reference the pane drag path, even though both live under `frontend/layout/lib/` and share the exact same underlying DnD library and Windows constraint. There was also an earlier, unrelated attempt at this exact problem in March 2026 (`origin/agenta/fix-tab-drag-cursor` branch, commits `71603a88`/`acb8ab17`/`2c17fea3`) that covered *both* tabs and panes via a native `SetSystemCursor` polling-thread approach — but that branch was never merged, and a partial backend port of its API (`agentmux-cef/src/commands/drag.rs`'s `set_drag_cursor`/`restore_drag_cursor`, `cef-api.ts`'s wrapper) survived as dead code that nothing calls today. Neither the abandoned branch nor the dead backend API should be revived — the July fix's approach (pure `dropEffect`, no native cursor override) is simpler and already proven to work for tabs.

## Fix direction (not yet implemented)

Port `tab-reorder.ts:120-134`'s pattern into `TileLayout.win32.tsx`: a Windows-gated `dragover` listener that sets `e.preventDefault(); e.dataTransfer.dropEffect = "copy"` whenever a pane drag is in flight and the cursor is outside the tile layout's own bounds — most naturally folded into the existing `checkForCursorBounds`/`onWindowDragOver` handler at lines 148-175, which already does the bounds check for a different purpose. Use `"copy"` (not `"move"`), matching the tab fix and correctly signaling "this creates a new window" rather than "this moves within a container."

## Prevention

When a fix addresses "gesture X shows the wrong cursor/behavior on platform Y," check whether the app has a sibling gesture using the same underlying library/constraint (tabs and panes both use pragmatic-dnd; Windows both times needed the same workaround) — and either fix both in the same PR or file a tracked follow-up issue immediately, rather than leaving the sibling case to be independently rediscovered.

## Files

- `frontend/layout/lib/TileLayout.win32.tsx:148-175` (`checkForCursorBounds`/`onWindowDragOver`) — where the fix belongs
- `frontend/app/tab/tab-reorder.ts:95-134` — the working reference implementation for tabs
- `agentmux-cef/src/commands/drag.rs:253-282`, `frontend/util/cef-api.ts:788-793` — dead `setDragCursor`/`restoreDragCursor` API, do not revive
