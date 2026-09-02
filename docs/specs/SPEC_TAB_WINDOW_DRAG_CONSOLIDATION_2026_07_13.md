# Spec: Tab / Window / Pane Drag — Consolidated Status & Roadmap

**Date:** 2026-07-13
**Status:** Consolidation (no new design — maps existing specs/issues/discussions to current `main`, verified by code inspection, and orders what's left)
**Repo state:** `main` @ `8c3d6ad4`
**Scope:** Every drag-driven interaction across tabs, panes, and windows — in-strip tab reorder, pane→tab drag, tab tear-off, cross-window tab remount, floating-pane tear-off/redock, and the underlying drag-session/persistence architecture. Supersedes nothing procedurally (the specs below remain the design record); this is the map of what's actually landed vs. still open, so work doesn't get re-diagnosed or re-forked.

**Why now:** this domain has ~7 active/recent specs, 1 long-running umbrella discussion (#1205), and 6+ open issues, several of which have follow-up comments that were never reconciled against later fixes. Before starting new work (specifically: cross-window tab drag, which prompted this pass), the state needed to be read in one place.

---

## 1. Executive summary

| Question | Answer |
|---|---|
| **Does dragging a tab between two windows work?** | **Yes, on Windows** — direct gesture, cursor-accurate insertion, shipped (#2086, stabilized #2095). **No, not properly on macOS/Linux** — falls back to an append-only path with no drop index. |
| **Does dragging a pane onto a different tab work?** | Yes (same window), field-tested and stabilized (§2.3). Cross-window pane dock (a *different*, harder feature — see §3.4) does not have live hover feedback yet. |
| **Does reordering tabs within the strip work?** | **No.** Confirmed broken in code — `onDragStart` never fires. This is a basic, currently-dead feature with no tracking issue until this spec. See §3.1 — **highest-priority open item**. |
| **Is the drag code trustworthy for further feature work?** | Not yet. The cross-tab/cross-window pane-drag subsystem went through 4 rounds of point-fixes in one day before the root cause was found; a structural refactor is designed but not started (§3.2). |
| **Is redock (floating pane → docked) reliable?** | Mostly — the highest-impact race (redock destroying the just-moved block) was fixed 2026-06-22 (§2.5) but never verified against the issue that reported it (#1662 — closed by this spec, see §5). The structural fix (atomic reducer-owned MOVE) that would close the *class* of redock races is still design-only (§3.3). |

---

## 2. Shipped & verified in code (as of `8c3d6ad4`)

### 2.1 Tab tear-off (drag a tab down past the strip → new window)
Pool-served on all platforms (#1605/#1606, closed 2026-07-11). Windows path: `WH_MOUSE_LL` hook (`tear_off_hook.rs`) tracks the cursor; `TearOffTab` → warm-pool promote, pre-painted off-screen for instant appearance.

### 2.2 Cross-window tab remount (drag a tab onto another window's header) — **Windows**
`specs/SPEC_CROSS_WINDOW_TAB_REMOUNT_2026_07_11.md`, shipped in #2086 (before the spec was even finalized — the spec was written to formalize/validate a feature already in code) and stabilized in #2095 (field-testing round 1). Verified present: `start_tab_drag_tracking` (`agentmux-cef/src/commands/drag.rs:730`, `tear_off_hook.rs:183`), `tabdrag:merge-direct` event + handler (`tabbar.tsx:656`). Direct drag, live insertion indicator on the target window, cursor-accurate drop index, last-tab-closes-source-window handled via `RestoreTornOffTab`.

**Not done:** macOS/Linux still route through the older `CrossWindowDragMonitor` → `DragOverlay.tsx` path, which calls `MoveTabToWorkspace(tabId, sourceWsId, myWsId)` with **no insert index** (verified — `DragOverlay.tsx:120`, only 2 args passed) — drops always append to the end, no live indicator. This is the spec's §4.5 "interim upgrade," not yet implemented. Full parity is additionally blocked on a global input hook for macOS/Linux (CGEventTap / XQueryPointer — "Phase 7," design-only).

### 2.3 Pane drag-to-tab (drop a tile pane onto a different tab, same window)
`specs/SPEC_PANE_DRAG_TO_TAB_2026_07_10.md` v3. Spring-loaded tabs (dwell → switch → real-layout ghost) shipped #2073/#2079. Field-testing round 1 (#2095) fixed: 3-layer dead-tab-overlay cleanup, Outer-direction ghost/landing mismatch, stale-geometry re-measure after tab switch, source-pane-lingering on cross-tab move, defensive `pruneDanglingLeaves`.

**Root cause of the "dead tab" found AFTER round 1** (not by another patch — by a proper hit-test probe) and fixed directly, verified in code: `.placeholder-container` (a purely decorative drag-ghost layer) lacked `pointer-events: none` on the container itself (only its children had it), and its "offscreen" parking position was computed from a possibly-zero `getBoundingClientRect()` when the source tab was `display:none` mid-cross-tab-drag — parking it exactly on top of the tab instead of offscreen. Both fixed: `tilelayout.scss:29-33` (`pointer-events: none` on `.placeholder-container`), `layoutModel.ts:450-457` (`Math.max(100000, ...)` parking floor, never purely rect-derived). This single defect explains every field symptom across all 4 point-fix rounds.

### 2.4 Registry hardening + persistence discipline (2 of 3 "immediate stabilization" items from the drag-session refactor spec)
From `specs/SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md` §4:
- **§3.4 registry hardening** — landed. `unregisterBlockComponentModel(blockId, owner?)` (`global.ts:536-548`) only deletes if the caller still owns the map entry, preventing a double-mount's earlier unmount from destroying the surviving later mount's registration.
- **§3.6 Phase A persistence** — landed, with a more careful implementation than the spec's literal wording. `persistToBackend` (`layoutPersistence.ts:405-416`) no longer writes a blind local copy of `pendingbackendactions`; it carries forward the live queue minus already-processed action ids, so a stale debounced persist can no longer erase a freshly queued backend action.
- **§3.5 prune de-fanging — NOT landed** (see §3.2 below; still gated on the pre-refactor `dragInFlight` singleton, not session-idle state).

### 2.5 Redock focus-orphan (global typing lock) — Defect A of #1461
Fixed by #1446 (merged): `destroy_hwnd()` now posts a focus-reclaim task so keystrokes aren't orphaned app-wide when a redocked pane's HWND is destroyed while it held Win32 focus.

### 2.6 Redock destroying the just-moved block (root cause of #1662, and R1 of #1681)
Fixed by #1685 (`f829e496`, 2026-06-22), verified in code (`layoutPersistence.ts:248-264`): a backend "delete this node" layout action means *remove from the tree*, not *delete the block* — every backend emitter of that action (tear-off, redock, promote) is a MOVE, and the block lives on in its new tab. Previously this path ran through `closeNode()` → `onNodeDelete` → `DeleteBlock`, destroying the block that had just been moved (empty-slot redock, "block not found"). Now it calls `DeleteNode` directly on the tree, bypassing block deletion entirely. The dedicated block-move guard this replaced was deleted as redundant.

**This is the same mechanism #1662 diagnosed** ("Deeper root cause" comment traces `onNodeDelete ENTER` → `RedockFloatingPane` → `DeleteBlock` destroying the redocked block) — but #1662 was never reconciled against this fix. See §5.

### 2.7 Second-window tear-off + fresh-pane tear-off/redock (#1681's two open symptoms)
Both closed by #1690 and #1691 (one root cause: blocks created store-only after bootstrap were invisible to the reducer's `srv_state`, which the tear-off/redock saga precondition-checks). User-verified per #1681's own thread. #1681 itself stays open only for R2 (see §3.5).

---

## 3. Confirmed still-open (verified by code inspection, not just doc claims)

### 3.1 🔴 P0 — Tab reorder within the strip does not work at all
`specs/SPEC_PANE_DRAG_TO_TAB_2026_07_10.md` Addendum A3 (2026-07-11), never filed as its own issue, **no code has touched the relevant files since** (verified: zero commits to `frontend/app/tab/` after the addendum was written). Log evidence at write-time: zero `tab-drag started` lines (logged unconditionally in `DroppableTab.draggable.onDragStart`) across a full dev session — the drag never initiates as a pragmatic drag, not a drop-side bug. Debug plan already written (check `onGenerateDragPreview` for a silent throw; check whether native `dragstart` fires at all on `.tab-drop-wrapper`; check for an intercepting overlay/window-drag-region conflict; check `Tab`'s own `onDragStart={() => {}}` prop for interference).

**This is a basic, user-visible, currently-broken feature with no tracking issue.** Filing one and fixing it is the highest-leverage single next step in this domain — small surface (drag initiation only), doesn't depend on the larger refactor, and is probably a one-line interference bug given the "zero native dragstart" signature.

### 3.2 🟠 P1 — Drag-session architecture refactor (structural, designed, not started)
`specs/SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md`. The FSM (`dragSession.ts`) that would own all cross-tab/cross-window pane-drag state does not exist (verified: file absent). `dragInFlight.ts` — the pre-refactor gate the spec says to delete once the FSM lands — is still the live gate for `pruneDanglingLeaves` (`layoutPersistence.ts:6`, `isTileDragInFlight` import). 2 of the spec's 3 "immediate stabilization" items already landed (§2.4 above); the third (§3.5, proper session-gated pruning) has not.

Not urgent in the sense of "currently broken" — §2.3's root-cause fix stopped the active bleeding — but it's the fix that prevents the *next* point-fix cycle. Recommended: land §3.5 (small, mechanical) opportunistically; treat the full FSM (§3.1–§3.3 of that spec) as its own scheduled effort, not a fire.

### 3.3 🟠 P1 — Atomic reducer-owned redock MOVE (structural, addresses the redock-race class)
Discussion #1205's own conclusion (2026-06-23, AgentA): "stop incremental patching... the real fix is the structural §9 atomic-MOVE... + the R5/R6 HWND/role registry." Verified not landed: no `PaneLocation` state machine, no `RedockAtomic`-style command in `agentmux-srv/src/reducer/` or `sagas/`. Redock is still frontend close-then-recreate. `docs/specs/SPEC_REDUCER_SSOT_CONSOLIDATION_2026_06_22.md` Track 2 (R5/R6 — HWND/role registry, the drag/redock cluster) is still `Status: Draft`.

§2.6 fixed the highest-impact *symptom* of this gap (block destruction on redock). This item is the deeper fix that would retire the whole class of "redock roulette" races, including whatever's left of #1662's mechanism under different timing and any future variant. Not urgent standalone; sequence after P0/P1-FSM.

### 3.4 🟡 P2 — Seamless cross-window PANE dock (not tabs — moving a docked pane between windows in one gesture)
`docs/specs/SPEC_SEAMLESS_CROSS_WINDOW_PANE_DOCK_2026_05_30.md`. Distinct from §2.2 (that's tabs). Phase-0 spike (throwaway branch, discussion #1205) ruled out the cheap approach: HTML5 `drag` events do not fire once the cursor crosses into another window (CEF/OLE limitation) — zero heartbeat, so no live per-leaf preview is possible via the event stream alone. Two remaining options never decided: **(B)** host-side cursor poll started on `dragleave`, or **(C)** convert pane DnD to a JS-driven pointer-drag (large regression surface). Today's behavior: the move itself works (`DragOverlay.tsx` `cross-drag-end` → `MoveBlockToTab`), but only resolves the target once, on release — no live feedback during the drag, unlike the tab-remount and same-window pane-to-tab paths which both now have real per-leaf hover previews.

### 3.5 🟡 P2 — Floater → CEF-Views conversion (R2 of #1681, the one item keeping that issue open)
Floaters are still a raw Win32 popup with the browser-pane handler grafted on, causing a cold first-paint flash on tear-off (vs. the pooled tab path's instant appearance). Fix is architectural: make floaters CEF-Views windows like the main window. No spike or design doc found beyond the one-line mention in #1681.

### 3.6 🟡 P2 — #768 Phantom browser pane (orphan + tear-off) — host/frontend registry divergence
Already has an accurate, current status comment (2026-07-11, on the issue itself) — no new information from this pass. Noted here only for completeness of the domain map; not primarily a tab/window-drag issue (it's browser-pane HWND lifecycle), so left for its own thread.

---

## 4. What NOT to re-litigate (settled, cited so nobody re-opens the question)

- **Cross-window pane drag cannot use an HTML5 `drag`-event heartbeat** — proven non-viable by the Phase-0 spike (discussion #1205); any future design must start from option B or C in §3.4, not re-attempt A.
- **`SPEC_TAB_TEAROFF_NATIVE_DRAG_LOOP_2026-05-07.md`'s premise stands**: the torn-off/dragged window cannot visually follow the cursor mid-gesture on Windows (`SC_MOVE` doesn't engage until mouseup, because the source webview holds mouse capture during the drag). Any cross-window drag design must work with the OS drag ghost, not assume a live window under the cursor.
- **The tear-off vs. redock split is intentional, not tech debt**: tab tear-off = full pooled instance promotion (instant); pane tear-off = floating child window (not pooled, hence the flash §3.5 addresses). Do not try to unify them into one mechanism — VS Code's same-process re-parenting model was explicitly evaluated and rejected (CEF here is process-per-window; panes include iframe/agent surfaces that can't move either).
- **Reduced-motion is intentionally ignored for drag UX cues** (product decision, addendum A1): the hover strobe, drop pulse, and insertion indicators carry meaning, not decoration; `prefersReducedMotionAtom` is hard-`false` app-wide, plumbing kept for revisit but not wired to any UI.

---

## 5. Issue / discussion consolidation

**Discussion #1205** ("Floating panes, tear-off, redock & cross-window docking — long-term tracking thread") — **stays open**, by design (it's the evergreen umbrella; its own header says "don't fork side threads"). This spec is exactly the kind of artifact meant to be linked *from* it, not a replacement. Action taken: posted a status-refresh comment (matching the thread's existing convention of periodic updates) pointing here.

**Issue #1662** ("Redock onto a pool-served main window leaves an empty spot") — **closed by this spec**. Its own final diagnosis (`onNodeDelete` → `RedockFloatingPane` → `DeleteBlock` destroying the just-redocked block) is the exact mechanism fixed by §2.6 (#1685, 2026-06-22, same day as the issue's last comment — the fix landed but was never reconciled back to this issue). Closed with a comment citing the fix location; flagged as *verified by code inspection, not a fresh live repro* — reopen if it reproduces on current `main`.

**Issues staying open** (each still has genuine unresolved scope, confirmed above — not closing):
- **#1681** — R2 (floater CEF-Views conversion, §3.5) is real, unscoped work.
- **#1461** — Defect B ("needs a live repro to confirm") and Defect C (overlaps #768) are explicitly unresolved in the issue's own tracking table.
- **#768** — has its own accurate, current status; genuinely unimplemented.
- **#871** — Part 2 (TOCTOU hardening) explicitly still open; tangential to this domain (window-HWND resolution, not drag).
- **#1190** — browser-pane keyboard shortcuts; unrelated to drag, left alone.

**New issue to file**: §3.1 (tab-reorder-in-strip totally broken) — no tracking issue exists for a confirmed-broken basic feature. Filed as part of this spec's follow-through (see below).

---

## 6. Recommended next steps, in order

1. **File + fix §3.1** (tab reorder in strip). Small surface, isolated to drag *initiation*, doesn't block on anything else. The addendum's debug plan is ready to execute as-is.
2. **Land §3.2's remaining stabilization item** (session-gated `pruneDanglingLeaves`, replacing the `dragInFlight` gate) opportunistically — small, mechanical, removes one more point-fix-era workaround now that the root cause (§2.3) is fixed.
3. **Decide macOS/Linux cross-window tab drag** (§2.2's gap): ship the "interim" `DragOverlay` insertIndex upgrade (frontend-only, no host changes) now, defer full parity (global input hook) to whenever Phase 7 lands generally.
4. **Schedule, don't fire-fight, the two structural items** (§3.2 full FSM, §3.3 atomic redock MOVE) — both are designed, both retire whole classes of future bugs, neither is blocking anything today.
5. **§3.4 (seamless cross-window pane dock)** — needs a product decision (B vs C) before any code; lowest urgency of the open items since the non-seamless two-step flow already works.

---

## 7. References (full document map for this domain)

**Specs (design):**
- `specs/SPEC_CROSS_WINDOW_TAB_REMOUNT_2026_07_11.md` — §2.2
- `specs/SPEC_PANE_DRAG_TO_TAB_2026_07_10.md` (+ addenda A1–A3) — §2.3, §3.1
- `specs/SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md` — §2.4, §3.2
- `docs/specs/SPEC_SEAMLESS_CROSS_WINDOW_PANE_DOCK_2026_05_30.md` — §3.4
- `docs/specs/SPEC_REDUCER_SSOT_CONSOLIDATION_2026_06_22.md` (Track 2) — §3.3
- `docs/architecture/ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md` — canonical architecture map, P1–P6 plan (§3.3 = its P3)
- `docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md`, `SPEC_TAB_TEAROFF_NATIVE_DRAG_LOOP_2026-05-07.md`, `SPEC_TAB_TEAROFF_POSITION_AND_PAINT_2026-05-07.md`, `SPEC_TEAR_OFF_POOL_PATH_2026_05_06.md` — tear-off phase model (§2.1, §4)
- `docs/specs/SPEC_FLOATING_PANE_DND_RETHINK_2026_06_22.md` — R1/R2/R3 (§2.6, §2.7, §3.5)

**Analysis:**
- `docs/analysis/ANALYSIS_TEAR_OFF_PERF_2026_06_13.md` — pane-vs-tab tear-off perf, VS Code comparison
- `docs/analysis/ANALYSIS_BROWSER_PANE_REDOCK_BLACK_TYPING_LOCK_2026_06_15.md` — #1461's defect triage
- `docs/investigations/INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md` — persistence clobber mechanism behind §2.4

**Tracking:**
- Discussion #1205 — umbrella (§5)
- Discussion #707 — reducer-stack architecture (adjacent umbrella, referenced by #1461)
- Issues: #1681 (open), #1461 (open), #768 (open), #871 (open, tangential), #1662 (closed by this spec)
