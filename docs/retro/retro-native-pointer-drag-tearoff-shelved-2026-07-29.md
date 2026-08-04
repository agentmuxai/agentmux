# Retro: native pointer-capture tab/pane drag rewrite — shelved 2026-07-29

**Status:** shelved, not merged. Branch `feat/native-pointer-drag-tabs` (local,
uncommitted at time of writing) is a working-but-incomplete spike, not a
mergeable PR.

## Why this exists

This is the **N-th** attempt at Windows tab/pane tear-off issues in this
repo's history — see "Related history" below for the full list of prior
branches/PRs. The user's explicit ask (2026-07-29): stop broadening scope,
shelve this attempt, and start a **fresh, narrowly-scoped discussion focused
specifically on the Windows circle-slash tear-off cursor** — the original,
single complaint that kicked off this whole session's work. Everything else
this attempt touched (drop-zone precision, ghost fidelity, spring-switch,
redock latency) should be treated as separate, pre-existing threads with
their own history, not re-litigated inside the cursor fix.

## What this session tried

Root cause (confirmed via direct research this session, not assumption):
Windows `WM_SETCURSOR` is not delivered to **any** window — not even the one
holding capture — while a mouse-capture session is active. Whatever cursor
was showing at the instant capture engaged just stays frozen until something
calls `SetCursor()` explicitly. Two different capture mechanisms were tried
against this:

1. **OLE `DoDragDrop` (existing mechanism, `draggable()` via
   `@atlaskit/pragmatic-drag-and-drop`).** Chromium's own
   `IDropSource::GiveFeedback` calls `SetCursor` continuously during an OLE
   drag session, based on its own drop-target hit-testing — this fights any
   external cursor override every mouse-move, which is why the earlier
   `fix/pane-tearoff-cursor-windows` branch's 2ms `SetCursor` polling thread
   didn't win the race in live testing.
2. **Native Pointer Events + `setPointerCapture()` (this session's new
   approach, no OLE session at all).** Implemented for both tabs
   (`droppable-tab.tsx` + new `frontend/app/drag/native-pointer-drag-tracker.ts`)
   and panes (`TileLayout.win32.tsx`). Confirmed this genuinely eliminates the
   circle-slash mechanism (OLE never engages), **but** a follow-up fix was
   needed: the frozen-cursor mechanism above still applies with no OLE
   involved — `agentmux-cef/src/commands/drag.rs`'s `set_drag_cursor`/
   `restore_drag_cursor` were changed from `SetSystemCursor(OCR_NO, ...)`
   (redefines a system cursor *resource* — has no effect if nothing ever
   loads that resource, which turned out to be the case once OLE was
   removed) to a **direct `SetCursor()` call** — since nothing else can
   override it during capture, one call should stick for the whole gesture.
   **This specific fix was never conclusively demoed working live** — testing
   moved on to a perceived "crash" (see below) before it was isolated and
   confirmed.

## What broke / regressed along the way (all found via live task-dev testing)

- **Reorder-update perf**: raw `pointermove` fires far more often than the
  `dragover` events it replaced (HID poll rate vs. Chromium's internal HTML5
  DnD throttling) — caused visible slowdown until coalesced through a single
  rAF-gated "latest event wins" slot in `native-pointer-drag-tracker.ts`.
- **Ghost never visible (panes)**: the dragging tile has `.dragging { filter:
  blur(8px) }`; CSS `filter` creates a new containing block for
  `position:fixed` descendants, so the custom drag-ghost `<img>` rendered
  relative to the (clipped, blurred) tile instead of the viewport. Fixed via
  `<Portal mount={document.body}>`.
- **SolidJS disposal bug (panes, "blank window")**: a successful pane
  tear-off runs `LayoutTreeActionType.DeleteNode` on the exact node being
  dragged, unmounting that `DisplayNode` component instance. The end-of-drag
  cleanup was still calling that instance's own `createSignal` setters
  (`setIsDragging`, `setGhostPos`) afterward. Guarded by checking
  `layoutModel.leafs().some(l => l.id === node.id)` before touching local
  signals; global/`layoutModel`-level cleanup still always runs. **Root
  cause of the blank window itself is NOT actually this** — per
  `docs/analysis/ANALYSIS_AGENT_PANE_CRASH_2026_05_28_PM.md` §B, writing to
  a disposed owner's signal is a silent no-op in SolidJS (the write
  succeeds, no render fan-out happens, no error), so it cannot itself have
  caused a blank window. The guard is a reasonable defensive cleanup either
  way, but the retro's original claim that it explained/fixed the blank-
  window symptom was speculative and is now known wrong; the actual root
  cause of the blank window was never identified this session.
- **Drop-zone precision regression (flagged by user, not yet fixed when
  shelved)**: the original per-pane hit-testing used the browser's real
  nested-element hit-test (`dropTargetForElements` on each `.overlay-node`)
  to find the innermost matched pane; the "nearest leaf by distance-to-
  center" logic (`computePaneHoverAndDispatch` in `tilelayout-shared.tsx`)
  was originally **only** the dead-spot fallback for gaps between panes.
  Reusing it as the *only* path for every hover position (no native drag
  session left to give precise nested hit-testing) means hovering squarely
  inside a pane near a boundary can now pick a different pane/quadrant than
  before. Needs an actual-rect-containment check inserted before the
  nearest-center fallback to restore parity.
- **Ghost size/DPI mismatch (flagged by user, not yet fixed when shelved)**:
  the DOM ghost renders `DragPreviewWidth`/`Height` as CSS pixels; the
  original native OS drag image was a device-pixel-resolution bitmap shown
  1:1, so on any HiDPI display the new ghost renders visibly larger than the
  old one.
- **Pre-existing floating-pane redock latency (NOT introduced this session,
  but newly reachable)**: once a pane tears off cleanly (no circle-slash),
  the natural next action — grabbing the new floating window to redock it —
  hits the *already-known-broken* `set_floating_redock_target`/
  `clear_floating_redock_hover` hover-tracking path (`Win32BeginMoveTask`'s
  per-move IPC), confirmed via `muxlog` showing IPC latency climbing from
  ~30ms to 200ms+ and dragging down unrelated calls (`get_user_home_dir`,
  `get_platform`, etc.) at the same time — a classic queue-backup signature.
  This is the same issue tracked by the retro/spec pair below; the user's
  perceived "crash" this session is very likely this latency spiral getting
  bad enough that the window stops repainting, not a process crash (no crash
  dump, no Windows Application Error event, host UI-thread liveness probes
  kept answering throughout).

## Verification state at shelve time

- `npx tsc --noEmit`: clean.
- `cargo check -p agentmux-cef`: clean.
- `task dev` boots and runs stably (confirmed via `muxlog`/launcher
  UI-liveness probes across ~5 restarts this session).
- Tab tear-off: mechanically works (reorder, tear-off-to-new-window,
  cross-window merge all exercised live); cursor-icon fix added but **not
  yet demoed live** for tabs specifically.
- Pane tear-off: works, circle-slash confirmed gone; cursor-icon fix added
  but **not yet demoed live** in isolation (testing moved to the blank-window
  investigation before it could be confirmed); drop-zone precision and ghost
  DPI regressions confirmed present and not yet fixed.
- **Not covered at all**: Escape-abort, cancel-back-into-strip, pin/close
  regression checks, multi-monitor — see the Phase 1 spec's own test
  checklist (§ Test plan) for the full list this never got through.

## Related history (read before starting the fresh discussion)

This repo has a long, repeated history of tear-off/redock work across many
agents — don't re-derive from scratch. In rough chronological/topical order:

- `docs/specs/PLAN_TAB_TEAROFF_PHASE1_WIN32_2026-05-07.md` + siblings
  (`SPEC_TAB_TEAROFF_NATIVE_DRAG_LOOP_2026-05-07.md`,
  `SPEC_TAB_TEAROFF_POSITION_AND_PAINT_2026-05-07.md`,
  `RESEARCH_TAB_TEAROFF_CROSS_PLATFORM_2026-05-07.md`) — the original
  `setPointerCapture` spike this session's tab work extended. Never merged.
- `docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md`,
  `SPEC_TEAR_OFF_POOL_PATH_2026_05_06.md`,
  `docs/specs/tearoff-pane-size.md` — sizing/pool-window history.
  `docs/retro/retro-pane-tearoff-full-window-regression-2026-06-20.md`,
  `docs/retro/2026-06-20-linux-pane-tearoff-anywhere.md`,
  `docs/retro/phase-e-tear-off-and-remaining-2026-04-30.md`,
  `docs/retro/BUG_WINDOW_DRAG_CURSOR_DRIFT_2026-05-07.md` — assorted
  regression retros from earlier tear-off work.
- macOS/Linux tear-off + redock parity:
  `docs/specs/SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md`,
  `SPEC_MACOS_TEAROFF_STABILITY_2026_05_29.md`,
  `SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24.md`,
  `SPEC_LINUX_FLOATING_PANE_TEAROFF_2026_05_30.md`,
  `SPEC_LINUX_TEAROFF_HEADER_ONLY_2026-06-20.md`.
- **PR branch `fix/pane-tearoff-cursor-windows`** (this session, earlier) —
  the cursor-polling-thread attempt against OLE's `GiveFeedback`, found
  insufficient in live testing. Its code is likely worth deleting once a
  real fix ships, not carrying forward.
- **PR branch `docs/pane-tearoff-and-redock-retros`** (this session,
  earlier) — `retro-pane-tearoff-cursor-never-fixed-2026-07-27.md`,
  `retro-redock-ghost-landing-reliability-2026-07-27.md`,
  `SPEC_REDOCK_FRAMEWORK_HARDENING_2026_07_27.md`,
  `SPEC_FLOATING_PANE_MULTI_MONITOR_TASKBAR_2026_07_27.md` — the redock
  latency issue hit again this session is exactly what
  `SPEC_REDOCK_FRAMEWORK_HARDENING_2026_07_27.md` already scopes fixing.
- **PR branch `docs/native-pointer-drag-tearoff-spec`** (this session,
  earlier) — `SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28.md`, the design
  doc this session's `feat/native-pointer-drag-tabs` branch implemented.
  Still the most relevant design reference for a narrower re-attempt.
- Many other historical branches touching this area (not read this session,
  listed for completeness — search their PR history before assuming a clean
  slate): `agenta/feat-tearoff-match-source-size`,
  `agenta/feat-tearoff-position-and-paint`,
  `agenta/macos-floating-pane-redock(-report)`,
  `agenta/macos-floating-pane-tearoff-phase-a`,
  `agenta/macos-linux-suppress-tearoff-not-allowed-cursor`,
  `agenta/macos-tearoff-suppress-snapback`,
  `agenta/redock-black-defer-fix`,
  `agentc/floater-independence-tearoff-jam`,
  `agento/fix-pane-tearoff-pool-full-window`,
  `agentu/linux-floater-jsdrag-redock`,
  `agentu/linux-floater-redock-ghosts`,
  `agentx/tearoff-splash-cover`,
  `feat/macos-tab-redock-cgeventtap`,
  `feature/phase-4b-redock-ghost-size`,
  `fix/resize-handle-triggers-tearoff`,
  `fix/tab-tearoff-commit-on-release`,
  `fix/tearoff-smoke-2026-06-19`.

## Suggested scope for the fresh discussion

1. **Isolate to just the cursor.** Don't touch drop-zone hit-testing, ghost
   rendering, or redock — those are legitimately separate, already-tracked
   problems. Confirm the cursor fix (already written, in
   `agentmux-cef/src/commands/drag.rs`'s `set_drag_cursor`/
   `restore_drag_cursor` on this branch) actually shows a crosshair cursor
   live, for **tabs only** first (smaller surface than panes). Not a plain
   `SetCursor()` call — `set_drag_cursor` replaces the *system* no-drop
   cursor resource (`SetSystemCursor(copy, OCR_NO)`) with a crosshair, and
   `restore_drag_cursor` resets ALL system cursors back to defaults
   (`SystemParametersInfoW(SPI_SETCURSORS)`) — a coarser, system-wide swap
   rather than a per-window/thread cursor set.
2. If native pointer-capture continues to look like the right mechanism
   (it does eliminate the circle-slash structurally, confirmed this
   session), consider shipping **tabs only** as a first PR, deliberately
   deferring panes to a second, separate PR once tabs are fully verified —
   this session's own spec already recommended this sequencing and this
   session skipped ahead anyway once the user asked to prioritize panes.
3. If native pointer-capture turns out to be too invasive for the value
   (this session's live feedback — "too broken with not much benefit" —
   suggests real doubt), it's worth explicitly weighing a narrower
   alternative before re-committing to the full rewrite: e.g., a low-level
   `WH_MOUSE_LL` hook (already used elsewhere in this codebase for tab-drag
   cross-window hit-testing, `agentmux-cef/src/commands/tear_off_hook.rs`)
   that calls `SetCursor()` on every hooked mouse-move during the *existing*
   OLE drag — untried this session, and unlike a plain polling thread it
   would be driven by real mouse events rather than a fixed-interval race
   against `GiveFeedback`, which may behave differently. Worth a quick spike
   before assuming the full drag-mechanism rewrite is the only path.

## Disposal

`feat/native-pointer-drag-tabs` (pushed, commit `416d3522`, **not** opened as
a PR — do not open one against it as-is, do not merge) has this retro's
worth of learning in it but is not close to mergeable as one PR. Pushed
rather than left local-only so a future narrower attempt has real code to
reference/cannibalize instead of starting blind, given this repo's long
history (§ Related history above) of tear-off/redock work getting
rediscovered from scratch across many agents. The fresh discussion should
still design its own scope rather than inherit this branch's wholesale —
see § Suggested scope above for what's actually worth reusing (the tab-side
mechanics and the cursor-freeze fix) versus what isn't (the pane drop-zone/
ghost rebuild).
