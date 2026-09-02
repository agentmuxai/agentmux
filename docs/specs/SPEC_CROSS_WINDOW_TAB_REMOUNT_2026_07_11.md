# Spec: Cross-Window Tab Remount (Drag a Tab onto Another Window's Header)

**Date:** 2026-07-11
**Status:** Draft (v2 — open questions resolved against the Pillar 2 lifecycle refactor, §9)
**Repo state:** `main` @ `7642454a`
**Scope:** Dragging a TAB from window A's tab strip and dropping it onto window B's tab strip ("header part"), remounting the tab — with all its panes, layout, and ordering — into window B at a cursor-accurate position. Pane (tile) drags, touch/pen input, and non-AgentMux drop targets are out of scope.

---

## 1. Problem

Tabs can already leave a window: dragging a tab past the strip's bottom edge (`TEAR_PAST_PX = 5`) tears it off into a new window. But there is no *direct* way to move a tab from window A into window B. What a user naturally tries — grab a tab, drag it over window B's tab strip, release — either does nothing useful or, if they happen to pass the tear-off threshold on the way, spawns a throwaway intermediate window they never asked for.

The surprising finding from code investigation: **the hard part already exists.** A full cross-window merge is implemented and working on Windows — but only as the tail end of the tear-off state machine:

1. Drag tab down past the strip → `requestTearOff` spins up a new window (`TearOffTab` saga → warm-pool promote / `openWindowAtPosition` → `tearOffSCMoveHandshake`).
2. A Win32 `WH_MOUSE_LL` hook (`agentmux-cef/src/commands/tear_off_hook.rs`) then tracks the cursor globally. Hovering another AgentMux window emits `tearoff:hover-changed` to it — which already shows the **standard insertion-point indicator** in that window's strip (`tabbar.tsx` handler → `computeInsertionPoint(clientX)`).
3. Releasing over that window emits `tearoff:merge`, whose handler (`tabbar.tsx:503-580`) computes the cursor-X-accurate insert index and calls `WorkspaceService.RestoreTornOffTab(tabId, fromWsId, ownWsId, insertIdx)` — moving the tab (blocks + layout travel automatically, see §2.3) and closing the emptied dragged window.

So "drag a tab into another window's header" today means: **tear off into a temporary window first, then drag that window over the target strip**. The remount machinery is solid; the *gesture* is wrong. This spec is about giving the existing merge a first-class direct path — and extending it beyond Windows.

---

## 2. Current Architecture (What Exists — Reuse, Don't Reinvent)

### 2.1 The tear-off state machine (Windows, shipped)

Phases per `docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md` (whose "Phases 2-7 not started" header is stale — Phases 2/4/5/6 are in code):

- **Trigger** (`tabbar.tsx:196-310`): pragmatic-dnd monitor `onDrag`, latched by `tearOffFired`, fires `requestTearOff` when the cursor passes `TEAR_PAST_PX` below the strip.
- **Host hook** (`tear_off_hook.rs`): installed by `tear_off_sc_move_handshake` (`drag.rs:634-643`) *before* the SC_MOVE post. `WH_MOUSE_LL` + `WH_KEYBOARD_LL` on a dedicated thread. Per mouse-move: `WindowFromPoint` → `GetAncestor(GA_ROOT)` → match against the host browser registry (`candidate_label_under_cursor_locked`, `tear_off_hook.rs:521-556`) → emit `tearoff:hover-changed`/`hover-cleared` to the right window via `emit_event_to_window` (`events.rs:84-105`). On `WM_LBUTTONUP`: `tearoff:merge` (over another window), `tearoff:cancel-back` (over source strip, or ESC), or `tearoff:standalone` (over nothing).
- **Frontend handlers** (`tabbar.tsx:437-679`): every window listens. `hover-changed` drives the standard insertion-point gap indicator; `merge` computes `insertIdx` and calls `RestoreTornOffTab`; `cancel-back` restores the exact original index (`originalTabIndex`/`wasPinned` captured at drag start).
- **Strip hit-testing is frontend-side only.** The host resolves cursor → top-level window label; whether the cursor is over the *strip* (vs. content) is decided in each renderer by `cursorY` vs `tabBarScrollRef.getBoundingClientRect()` with physical→CSS px conversion. There is no host-side strip-rect registration.

### 2.2 Backend tab-move (shipped, platform-agnostic)

- `Command::MoveTab` reducer (`agentmux-srv/src/reducer/tab.rs:341-449`): removes the tab id from the source workspace's `tab_ids`, inserts into the destination's at a clamped index, reparents `tab.workspace_id`, emits `TabMoved`. **Blocks and layout state are children of the Tab object — they travel automatically.** Nothing about a tab move touches `blockids` or `layoutstate`.
- `RestoreTornOffTab` RPC (`workspace.rs:956-1015`) → `restore_torn_off_tab` saga: `MoveTab`, then best-effort `DeleteWorkspace` of the now-empty source. Exists specifically because `MoveTabToWorkspace` (`workspace.rs:849-950`) has a **last-tab guard** that rejects moving a workspace's only tab.
- `MoveTabToWorkspace` RPC: the general path for multi-tab source workspaces; honors `insertIndex` (clamped); same-workspace short-circuit.

### 2.3 The HTML5 cross-window drag path (legacy for tabs)

`CrossWindowDragMonitor.{win32,darwin,linux}.tsx` + `DragOverlay.tsx`: on a tab drag's `dragend` over another window, the target's `DragOverlay` calls `MoveTabToWorkspace(tabId, sourceWsId, myWsId)` — **no insert index, no strip hit-test** (drops anywhere on the window merges into the active workspace at the end of the strip). On Windows this path is dead for tabs: `requestTearOff` clears `currentDragPayload` at the tear-off threshold precisely so this pipeline doesn't double-process. On macOS/Linux it is the *only* cross-window tab path today.

### 2.4 Known constraint: SC_MOVE doesn't visually engage mid-drag

Per `SPEC_TAB_TEAROFF_NATIVE_DRAG_LOOP_2026-05-07.md` (spec-only, never built): during the HTML5/OLE drag, mouse capture belongs to the source webview, so the torn-off window does not visually follow the cursor — it materializes on mouseup. The `WH_MOUSE_LL` hook still tracks correctly (hover/merge events flow), but the user sees only the OS drag ghost mid-gesture. Any design here must not assume a live window under the cursor.

---

## 3. Goals

1. **Direct gesture**: drag a tab from window A's strip, hover window B's strip → B shows its standard insertion-point indicator live; release → the tab remounts into B at that index. No intermediate throwaway window is created when the drop lands on another window's strip.
2. Source window keeps working: A stays open if it has other tabs; if the moved tab was A's last, A closes (same policy as merge-today's `closeWindowByLabel` on the emptied dragged window).
3. Cancel semantics preserved: ESC or dropping back on A's own strip = plain reorder / no-op, never a cross-window move.
4. Works with the existing tear-off: dragging *down* past the threshold still tears off to a new window (unchanged); the two gestures compose because they share one state machine.
5. Cross-platform story stated explicitly (win32 first-class; darwin/linux at least reach the `MoveTabToWorkspace`-with-index path).

## Non-Goals

- Making the torn-off window visually follow the cursor (that's `SPEC_TAB_TEAROFF_NATIVE_DRAG_LOOP`'s scope; this spec works with the OS drag ghost).
- Pane (tile) drags between windows (covered by floating-pane redock + `SPEC_PANE_DRAG_TO_TAB_2026_07_10.md` for in-window).
- Merging into a *specific tab's layout* (that's a pane-level concept); a tab drop always lands as a sibling tab.
- Host-side strip-rect registry (nice-to-have hardening; the frontend-side hit-test is kept, see §4.4).

---

## 4. Design

The core idea: **run the tear-off hook for every tab drag, not just after a tear-off** — and defer the "which outcome?" decision to release time.

### 4.1 Deferred-outcome drag

Today the state machine commits to tear-off the moment the cursor passes `TEAR_PAST_PX` (spawns a window mid-drag). Keep that for the downward gesture, but add a parallel *cross-window intent* track:

- On tab-drag start (`DroppableTab.onDragStart`), ask the host to `start_tab_drag_tracking(sourceLabel, tabId, sourceWsId, originalTabIndex)` — a trimmed variant of `start_tear_off_tracking` that installs the same `WH_MOUSE_LL` hook but with **no dragged/destination window** (none exists yet).
- The hook's per-move logic is unchanged (candidate window under cursor → `tearoff:hover-changed`/`hover-cleared` to it). Window B's existing handler already renders the insertion indicator — zero frontend changes on the target side for hover.
- On `WM_LBUTTONUP` over a candidate window ≠ source: emit a new **`tabdrag:merge-direct`** event to the candidate, payload `{tabId, fromWsId: sourceWsId, sourceWindowLabel, cursorX, cursorY}`. (Distinct from `tearoff:merge` because the source workspace is a *normal multi-tab* workspace, not a single-tab tear-off orphan.)
- Over the source window or nothing: emit nothing — the normal in-window pragmatic-dnd reorder / tear-off threshold logic owns those outcomes, exactly as today. The hook must NOT interfere with in-window reorders.

### 4.2 The merge-direct handler (target window)

New branch alongside the `tearoff:merge` handler in `tabbar.tsx`, ~90% shared code:

1. Strip hit-test cursorY (same as `tearoff:merge` — release over B's *content* area is not a tab drop; ignore, and the source's dragend cleanup handles payload clearing).
2. `insertIdx = computeInsertionPoint(clientX)` → numeric index (shared helper with the existing merge handler — extract the `{beforeTabId, afterTabId}` → index conversion at `tabbar.tsx:537-553` into `tabbar-dnd.ts`).
3. Call **`MoveTabToWorkspace(tabId, fromWsId, ownWsId, insertIdx)`** — not `RestoreTornOffTab`: the source workspace is a live multi-tab workspace, and the last-tab guard is *desirable* here... with one amendment, §4.3.
4. Notify the source window (`tabdrag:merged` targeted event) so it can cancel its in-flight pragmatic-dnd bookkeeping (clear `globalDragTabId`, insertion point, payload) and, if it just lost its last tab, close itself.

### 4.3 The last-tab case

`MoveTabToWorkspace`'s guard rejects moving a workspace's only tab (it would strand an empty workspace/window). For a direct cross-window drag of window A's only tab, the right UX is: move the tab AND close window A — functionally identical to today's merge-after-tear-off. Two options:

- **(a) Reuse `RestoreTornOffTab`** for the single-tab case (it exists to bypass the guard and delete the emptied workspace). The frontend picks the RPC by `tabIds().length === 1` at drag start (carried in the hook payload).
- **(b) Add an `allowLastTab: bool` param to `MoveTabToWorkspace`** that routes through the same saga machinery.

**(a) is recommended** — zero backend changes, the saga already handles workspace deletion best-effort, and semantically "the source workspace ends up empty and gets deleted" is exactly the torn-off-restore contract.

**Closing the source window — be precise about the mechanism** (verified against the Pillar 2 lifecycle refactor, §9): there is **no** host-side "empty-workspace watcher" that closes windows automatically. The existing `tearoff:merge` handler closes the emptied window *explicitly* — `RestoreTornOffTab` (deletes the emptied source workspace) then `getApi().closeWindowByLabel(draggedWindowLabel)` (`tabbar.tsx:559-565`) — and that explicit call is required, not redundant: the host's `orphan_reconcile` has zero visibility into srv workspaces/tab counts and will never reap a live window because its workspace emptied. Copy that exact two-step pattern.

**Precondition: never `closeWindowByLabel("main")`.** If the dragged tab's source is a 1-tab `main` window, closing `main` feeds the host's last-window quit sequence (`CloseWindowTask` scopes its park/demote logic to non-main windows). Simplest rule: when the source is `main` AND it's the last tab, treat the drop as a no-op (or move-and-leave-main-open via option (b)) — decide at implementation time, but don't fall into quitting the app accidentally.

### 4.4 Hook lifetime + safety

- `start_tab_drag_tracking` installs on tab-drag start and MUST uninstall on: `WM_LBUTTONUP` (any outcome), ESC, or a `stop_tab_drag_tracking` call from the frontend's dragend (belt-and-suspenders — a hook leak eats global mouse events).
- If the drag crosses `TEAR_PAST_PX` and `requestTearOff` fires, the tear-off handshake *replaces* the tracking session (its own `start_tear_off_tracking` takes over with the dragged/dest window populated). One session at a time; `start_tear_off_tracking` already tears down a prior hook if present — verify and test this handover.
- The hook thread already exists and is proven (Phase 4); this adds a second entry point with fewer parameters, not a new mechanism.
- DPI: cursor coords are physical px; the frontend converts (existing `physicalToClientX/Y` helpers). Multiple past review fixes in this area — reuse the helpers, don't re-derive.

### 4.5 Platform strategy

- **Windows**: full design above (§4.1–§4.4).
- **macOS/Linux (interim)**: no global hook (Phase 7 stubs). But the HTML5 `CrossWindowDragMonitor` → `DragOverlay` path already fires for tab drags there. Upgrade `DragOverlay`'s tab branch from "append to active workspace, no index" to: forward the drop's cursor position to the tab bar (window-local hit-test), compute `insertIdx` when over the strip, and pass it to `MoveTabToWorkspace`. This gives darwin/linux a functional (if indicator-less mid-drag) version of the feature with purely frontend changes.
- **macOS/Linux (full)**: blocked on Phase 7 of the tear-off spec (CGEventTap / XQueryPointer trackers). Out of scope here; this spec's hook entry point is designed so those trackers plug into the same `tabdrag:*` events when they land.

### 4.6 UX details

- Hover indicator on B: the existing insertion-point gap (no new visuals). The `tearoff:hover-changed` handler already ignores non-strip hover — unchanged.
- The dragged tab in A keeps its existing reduced-opacity `tab-dragging` styling; A's own strip continues showing its in-window reorder gap while the cursor is over A. When the cursor is over B, A's insertion point clears (the hook's `hover-cleared` to the previous target covers the B→A transition; A-local pragmatic-dnd covers its own).
- Landing bounce: reuse `bouncingTabId` on the remounted tab in B (same as reorder), giving drop feedback.

---

## 5. Files Touched

| File | Change |
|---|---|
| `agentmux-cef/src/commands/tear_off_hook.rs` | New `start_tab_drag_tracking` entry (no dragged/dest window); `handle_button_up` branch emitting `tabdrag:merge-direct`; teardown-on-handover to `start_tear_off_tracking`; fix the stale `tearoff:finalize` header comment |
| `agentmux-cef/src/commands/drag.rs` (or new command) | IPC command pair `start_tab_drag_tracking` / `stop_tab_drag_tracking` exposed to the frontend |
| `frontend/app/tab/droppable-tab.tsx` | Tab-drag start/end: invoke start/stop tracking IPC |
| `frontend/app/tab/tabbar.tsx` | `tabdrag:merge-direct` handler (shared core with `tearoff:merge`); `tabdrag:merged` source-side cleanup handler; single-tab → `RestoreTornOffTab` routing |
| `frontend/app/tab/tabbar-dnd.ts` | Extract shared insertion-point→index conversion used by both merge handlers |
| `frontend/app/drag/DragOverlay.tsx` | darwin/linux interim: strip hit-test + `insertIndex` for the tab branch |
| `agentmux-srv` | **No changes** (option (a) in §4.3) |

---

## 6. Implementation Order

1. Extract + unit-test the insertion-point→index conversion from the `tearoff:merge` handler (pure refactor, no behavior change).
2. Host: `start/stop_tab_drag_tracking` + `tabdrag:merge-direct` emission; verify hook handover when `requestTearOff` fires mid-session (the one genuinely new race).
3. Frontend: wire drag start/end IPC + the `merge-direct`/`merged` handlers; multi-tab source path (`MoveTabToWorkspace` with index).
4. Single-tab source path (`RestoreTornOffTab` + source-window close).
5. darwin/linux interim upgrade in `DragOverlay`.
6. Manual matrix: A→B multi-tab, A→B last-tab (A closes), drop on B's content area (no-op), ESC mid-drag, drag down (tear-off unchanged), rapid A→B→A hover transitions (indicator cleanup), DPR ≠ 1 monitors.

---

## 7. Testing Guidance

- Unit: insertion-point→index conversion (extracted helper) — cursor before first tab, between tabs, after last, empty target strip.
- Unit (Rust): `handle_button_up` outcome matrix for the new tracking mode — over source / other window / nothing / ESC.
- Integration: `tabdrag:merge-direct` handler calls `MoveTabToWorkspace` with the computed index for a multi-tab source, `RestoreTornOffTab` for single-tab.
- Regression: existing tear-off flow (down-drag → new window), merge-after-tear-off, and cancel-back all unchanged — they share the hook, so cover the handover explicitly.
- Diag: both merge paths log under `dnd` — verify via `muxlog srv grep RestoreTornOffTab` / `MoveTab`.

---

## 8. References

- `docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md` — master tear-off spec (phase model; header stale re: shipped phases).
- `docs/specs/SPEC_TAB_TEAROFF_NATIVE_DRAG_LOOP_2026-05-07.md` — why the window doesn't follow the cursor mid-drag (spec-only).
- `docs/specs/SPEC_TAB_TEAROFF_POSITION_AND_PAINT_2026-05-07.md` — `TEAR_PAST_PX`, anchor math.
- `docs/specs/SPEC_TEAR_OFF_POOL_PATH_2026_05_06.md` — warm-pool window creation.
- `agentmux-cef/src/commands/tear_off_hook.rs` — the WH_MOUSE_LL hook this spec extends.
- `agentmux-srv/src/sagas/restore_torn_off_tab.rs`, `agentmux-srv/src/reducer/tab.rs` (`handle_move_tab`) — the move machinery reused as-is.
- `frontend/app/tab/tabbar.tsx:437-679` — the Phase-4 event handlers the new events slot into.
- `specs/SPEC_PANE_DRAG_TO_TAB_2026_07_10.md` — the in-window sibling feature (pane→tab); its §7 payload-clearing rules also apply to any new drop consumption added here.
- `docs/specs/SPEC_PILLAR2_SANITIZE_THEN_DECIDE_2026_07_11.md` — the quit/drain lifecycle this spec's window-close path now runs through (§9).

---

## 9. Resolved Questions (2026-07-11, reconciled against the Pillar 2 lifecycle refactor)

Pillar 2 ("sanitize-then-decide", PRs #2080/#2081/#2084/#2083) landed a major rework of
the host's quit/drain lifecycle while this spec was in review. Verified findings:

**File overlap: nil.** Pillar 2 touches only `client/lifecycle.rs`, `reducer/{quit,browsers}.rs`,
`ui_tasks/window.rs`, `commands/{orphan_reconcile,window_pool}.rs`, `wrr/win_event.rs`, `state.rs`.
None of this spec's files (`tabbar.tsx`, `drag.rs`, `tear_off_hook.rs`, `DragOverlay.tsx`) are
affected. The only shared surface is behavioral, below.

**Q1 — Is closing the emptied source window safe post-Pillar-2? YES.**
`closeWindowByLabel` is unchanged. Its WM_CLOSE lands in `CloseWindowTask` →
`unregister_after_parking_close` (`ui_tasks/window.rs:364-385`), which now consumes the
reducer's drain verdict. In a merge, the target window B is still live, so
`should_begin_drain` (`reducer/quit.rs:170-186`) returns `None` — no spurious quit, no race.
The new invariant we *rely on* rather than fear: if a remount ever leaves **zero** live user
windows, the host now definitively begins Stage-1 drain + WRR Stage-2 quit — correct behavior,
not a bug. Hence the `main`-window precondition in §4.3.

**Q2 — "start_tear_off_tracking unreachable on the warm-pool path" (ANALYSIS_PANE_TAB_TEAR_SMOKE_2026_06_19): STALE, verified false on Windows.**
`requestTearOff` calls `tearOffSCMoveHandshake` with `tabId`/`sourceWsId`/`destWsId` on *both*
the warm-pool and cold paths (`tabbar.tsx:1038-1055`), and `drag.rs:633-644` installs the hook
whenever those are non-empty. The "unreachable" flag conflated the Windows installer
(`tear_off_hook.rs:102`) with the `#[cfg(not(windows))]` no-op stub (`tear_off_hook.rs:236`),
which is legitimately inert off-Windows. §4.4's handover concern stands, but it's a
*new-code* concern (two installers, one hook slot), not an existing bug.

**Q3 — Does the new `orphan_reconcile` auto-close or resurrect an emptied window? NEITHER.**
Terminology collision: Pillar 2's `orphan_reconcile` reconciles the host's `browsers`↔HWND
projection (dead/hostless HWNDs) — it never inspects srv workspaces or tab counts, never
closes a live window, never adopts one. The explicit `closeWindowByLabel` in the merge
handler is therefore required and un-raced. This spec's "orphan *workspaces*" (windowless
srv rows left by `requestTearOff` failures, `tabbar.tsx:1071-1090`) are a separate srv-side
concern, untouched by Pillar 2; `RestoreTornOffTab`'s atomic move+delete means this feature
creates no new ones on the happy path and inherits the existing conservative
leave-the-orphan stance on failure.

**Q4 — Is there now a canonical "close this window, its workspace emptied" primitive we should
use instead? NO.** `BeginDrain`/`ReconcileQuit` are whole-host quit machinery, not per-window
closes. `RestoreTornOffTab` + `closeWindowByLabel` (§4.3) remains the sanctioned pattern —
Pillar 2 makes it *safer* (the close path's drain verdict is now consumed uniformly), not
different.

One classification note for implementers: a *promoted pool* window keeps its `window-pool-*`
label forever but IS a live user window (`BrowserKind::TopLevel { is_pool: false }`,
`reducer/quit.rs:130-132`). Any logic in this feature that reasons about the source/target
window must classify by browser kind, never by label prefix.
