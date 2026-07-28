# Retro: floating-pane redock reliability — a 7-week-old structural fragility, not a single bug

**Date:** 2026-07-27
**Severity:** Medium — no data loss, but a core interaction (drag a floating pane back into the layout) that's supposed to be reliable has degraded to "sometimes."
**Observed by:** user, on the current dev instance, after previously experiencing this working well through a "whole set of PRs."

---

## TL;DR

This is not a pinpoint regression the way the resize-hit-target and tear-off-cursor issues were. Redock (dragging a torn-off floating pane back into the tiled layout, with a "ghost" preview showing where it will land) is a subsystem that **already has its own architecture doc** (`docs/architecture/ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md`), written specifically to stop a regression cycle that had already happened once by late May. That doc names 5 recurring structural themes and proposes 6 fixes (P1-P6) — only some of which were ever executed. The pipeline's last confirmed-working, deliberately-fixed state was **2026-07-05** (PR #1968, "redock lands at the ghost's exact rect"). Since then, no PR has touched redock on purpose, but several large unrelated refactors have touched code the pipeline depends on, and the two most architecturally significant proposed fixes — **replacing floaters' raw Win32-popup model** and **making redock an atomic backend operation** — were both explicitly deferred and never scheduled.

## What "working well" looked like, and when

The feature was built across ~20 commits from 2026-05-27 (PR #1112, MVP redock) through 2026-07-05 (PR #1968, the last functional fix — fraction-based landing-rect sizing so the ghost preview and the actual landing spot always agree). In between, three "durable fix" waves each closed out a specific failure mode:
- **R1** (PR #1685, 2026-06-22) — move-vs-close intent modeled at the root, replacing a racy ad hoc guard.
- **Phase 4b** (PR #1728, 2026-06-23) — fixed a ghost-size mismatch via a push-then-store direction hand-off between windows.
- **R3** (PR #1875, 2026-06-30) — pool-window relabeling so promoted pane-pool windows resolve identity correctly.
- **Phase 4c** (PR #1968, 2026-07-05) — the ghost's previewed rect and the actual landing rect were computed two different ways; unified them and fixed shared-pool sizing math so redocking one pane didn't dilute uninvolved siblings ("carve, don't dilute").

By 2026-07-05, per the architecture doc and the analysis doc that drove Phase 4c, this was considered closed.

## What's changed since, that redock depends on but nobody re-verified against

None of the following mention redock in their commit message — that's the point:

1. **PR #2079 (2026-07-11), "pane drag-to-tab reworked as spring-loaded tabs"** — introduces a *second* frontend caller of the exact same `RedockFloatingPane` RPC (dragging a pane onto another tab button, not just dragging a floating window). Two callers now share one RPC with different assumptions (one clamps certain directions the other doesn't need to). This is the same "shared thing, one caller's fix changes the other's behavior" shape that caused the resize-hit-target regression.
2. **PR #2111 (2026-07-12), "bind label→HWND at Views creation time"** — changes *when* the window-identity cache that `resolve_window_at_cursor` reads gets populated. That resolver function is the single most fragile piece in the whole pipeline (see below) and has broken from timing changes at least 4 times before (#1165, #1166, #1195, #1681). This PR's own purpose was unrelated (a different crash bug), so nobody had a reason to re-run the redock checklist against it.
3. **PRs #2178-ish (2026-07-16/17), "minimize is a display mode (i3 pattern)"** — a from-scratch rewrite of the shared-pool layout-sizing math that Phase 4c's "carve, don't dilute" fix depends on. Verified the specific function Phase 4c touched is still correct, but this was a large geometry-model swap in exactly the math redock's landing size relies on, landed 10 days ago, and was still fixing its own bugs as of the last commit in the series.
4. **PR #2181 (2026-07-16), "recover host bridge on reload for floating/pool windows"** — fixes a real bug where a floating window's IPC could silently go dead after certain reloads. If this half-applies mid-drag, every redock-hover/ghost-target call from that floater would silently no-op — a "ghost just doesn't show, no error" symptom.

## Diagnosis

Static analysis (this pass didn't have a live, bisectable repro) found the ghost-rendering and dock-RPC/layout-insertion code paths both **architecturally intact** — matching their last-fixed state, no obvious breakage. The **hover-detection / window-resolution step** (`resolve_window_at_cursor` in `agentmux-cef/src/commands/window/motion.rs`) is the standout suspect: it's a Z-order HWND walk with three layered special cases, a cache-independent fallback, and a *permanent diagnostic trace left in the code specifically because this function has broken repeatedly before*. Combined with the 2026-07-12 HWND-binding-timing change touching its inputs, this is the most likely source of the *intermittent* (not total) failures the user describes — "no longer works reliably" fits a timing race better than a clean break.

## The framework-level ask

The user explicitly asked for a framework-level fix, not another point patch — and that ask is already partially answered by two existing, unexecuted proposals:

- **`ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md`, P3**: model redock as a single atomic backend MOVE operation, instead of today's 5 independently-scheduled steps (block move, floater auto-close, pane close-in-floater, pane create-in-target, frontend `onNodeDelete`) that each have their own timing/ordering assumptions.
- **`ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md`, P4** / **`SPEC_FLOATING_PANE_DND_RETHINK_2026_06_22.md`, R2**: replace the floater's raw owned Win32-popup model with a proper CEF-Views window. The DnD-rethink spec calls this "the recurring footgun" and explicitly deferred it ("do last, gated on its own spec") — it has not been picked up since. Owned popups carry Z-order pinning, owner-destroy cascade behavior, and `GA_ROOT` quirks that every other fix in this history has had to work *around* rather than eliminate.
- **P6** (same doc): an automated regression harness for the redock gesture. This still doesn't exist — the only thing catching this class of bug today is a manual checklist that a large unrelated refactor has no reason to know it should run.

Other structural weaknesses this investigation surfaced, additive to what the architecture doc already named:

- No single source of truth for window identity — the HWND cache, the reducer's browser registry, an independent `EnumWindows` fallback, and the frontend's URL-param label are four sources that must agree, and disagreement here is the most common historical failure mode.
- The dwell/velocity hover gate is hand-implemented twice (once for the Windows native-move-loop path, once for the macOS/Linux JS-driven path) with near-duplicate state — any future gating tweak has to be applied symmetrically by hand or platforms silently diverge.
- The cross-process ghost hand-off is a single global `HashMap` keyed only by target window label, with no session/drag identifier — fine today (one cursor, one drag at a time), but a latent trap for any future feature enabling concurrent drags (this codebase has plenty of agent-driven RPC surface that could plausibly trigger one).
- `RedockFloatingPane`'s two independent frontend callers (floater-window redock, and now cross-tab pane drag since #2079) is new since the architecture doc was written and should be added to its list of recurring themes.

## Recommended next step

Not a code fix yet — this needs a scoping pass before implementation, per the user's own framing. Suggest: a new spec that (a) finally schedules P3 (atomic backend move) and P4/R2 (CEF-Views floaters, or at minimum a written decision to keep deferring it with a stated reason), (b) adds the two new-since-May themes above to the architecture doc's living list, and (c) stands up the P6 regression harness so the next unrelated refactor that touches `resolve_window_at_cursor`, the shared layout-sizing math, or the `RedockFloatingPane` RPC gets an automatic signal instead of relying on institutional memory.

## Files

- `agentmux-cef/src/commands/window/motion.rs` (`resolve_window_at_cursor`, `update_floating_redock_hover`, `set_floating_redock_target`/`get_floating_redock_target`) — hover-detection and cross-process ghost hand-off
- `agentmux-cef/src/ui_tasks/drag.rs` (`Win32BeginMoveTask`) — native Windows move loop, source of hover events during floater drag
- `frontend/app-init.ts` (`installFloatingRedockHoverListener`) — ghost/preview rendering
- `frontend/app/workspace/floating-pane-workspace.tsx` — drag state machine, dwell/velocity gate (duplicated per-platform), redock trigger (`tryRedockAtCursor`)
- `agentmux-srv/src/server/service/tear_off.rs` (`handle_redock_floating_pane`), `agentmux-srv/src/sagas/redock_floating_pane.rs`, `agentmux-srv/src/server/service/layout_helpers.rs` — backend RPC, saga, layout insertion
- `frontend/layout/lib/layoutPersistence.ts`, `layoutTree.ts`, `layoutGeometry.ts` (`applySizeFraction`) — frontend layout application
- `frontend/layout/lib/crossTabDrag.ts` — the second, newer `RedockFloatingPane` caller (#2079)
- `docs/architecture/ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md` — existing living doc, P1-P6 proposals (P3/P4/P6 still open)
- `docs/specs/SPEC_FLOATING_PANE_DND_RETHINK_2026_06_22.md` — R1/R3 done, R2 (CEF-Views floaters) explicitly deferred, never rescheduled
- `docs/analysis/ANALYSIS_FLOATING_PANE_GHOST_LANDING_DISCONNECT_2026_07_04.md` — the analysis that drove the last functional fix (#1968)
