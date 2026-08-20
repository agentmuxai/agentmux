# SPEC: Robust dispatch attribution + formalized session lifecycle for Swarm

**Status:** Implemented (Phases A, B, C — C's Live/Historical section split descoped as a follow-up, see §3.3).
**Date:** 2026-08-19

## 1. Problem statement

Reported live: a session with only "a couple" of real `Agent`-tool calls
showed **48 entries** in the Swarm panel's Agent Tool bucket, including
multiple rows sharing the exact same generated name (e.g. several rows all
titled the same kebab-case slug). The expectation — and this spec's goal —
is that **one real Agent/Workflow-tool call is always exactly one row,
which expands to one continuous stream of everything that call produced**,
with no duplicates and no unexplained accumulation.

A second, related problem: when an AgentMux instance closes and reopens (or
an agent respawns), its subagents are dead — no longer running, present
only for historical record. There is currently no formalized way to tell
"this dispatch is from a session that's still alive" apart from "this
dispatch's owning app instance no longer exists," and no way for a human to
manage (see, hide, or clear) that historical backlog.

## 2. Root cause (evidenced, not fully live-confirmed — see §2.5)

Two prior retros already fixed adjacent issues and are confirmed still
fixed in current code, so they're ruled out as the cause here:
- `retro-subagent-watcher-shared-dir-fanout-and-leak-2026-07-23.md`'s
  cross-pane misattribution bug — fixed (`session_belongs_to_block()` gate,
  `agentmux-srv/src/backend/subagent_watcher/mod.rs:399,447,644`), and
  wouldn't explain this symptom anyway since the frontend already filters
  by `parent_block_id` (`swarm-model.ts:1441`) — all 48 rows share one real
  block.
- `RETRO_SWARM_PHANTOM_ROWS_AND_STALE_TRACKING_2026_08_06.md`'s phantom-row
  hypothesis — the proposed fix (`hasRenderableBlock`) is present
  (`swarm-model.ts:304`). Establishes the general failure class (stale
  state outliving its real registration) but isn't the row-count cause.

True duplication of one real subagent process into multiple
`SubAgent`/`dispatch_id` entries is structurally hard to produce:
`agent_id` is deterministic from the JSONL filename, the in-memory dedup
key is `session.subagents` keyed by that same `agent_id`
(`jsonl.rs:87,114`), and `dispatch_id` is a deterministic function of it.
Re-scans of an already-known `agent_id` correctly see `is_new == false`.

The evidence instead points to **accumulation + non-unique naming**,
compounding into something that looks like duplication:

1. **Unbounded backend retention, no persisted dismissal.** The backend's
   `sessions`/`dispatches` maps hold every subagent/dispatch a block has
   *ever* produced (`query.rs:17,35`, `list_active`/`list_dispatches` fully
   unfiltered — filtering to one block happens client-side only). On pane
   reopen or after an `srv` restart (state is not persisted to disk —
   `SubagentWatcher::new` starts with empty maps, `mod.rs:147-158`;
   `bootstrap.rs:1164` spawns a fresh watcher every launch), the backfill
   scan replays up to `BACKFILL_MAX_FILES = 200`
   (`types.rs:177`) historical `agent-*.jsonl` files as `is_new` spawns —
   because from this fresh watcher's perspective, they genuinely are new.
   Meanwhile the only "dismiss" mechanism (`retireRow`, see §2 of
   `SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06.md`) is a client-local
   `Set`, never persisted — confirmed at that spec's §7.3 non-goal. So a
   long-lived pane's entire historical Agent-tool activity can resurface at
   once on reload, dwarfing what actually happened in the visible current
   turn.

2. **CORRECTED (previously misdiagnosed here) — exact-duplicate-name rows
   are already fixed, not an open bug.** Backfilled rows do skip eager
   naming and fall back to Claude Code's own raw `slug` field, and that
   slug genuinely is a **per-batch shared codename, not per-subagent-unique**
   (`swarm-model.ts:316-336` cites a prior live incident, task #44 — 17
   distinct agents sharing one visible slug, rendered as 17 apparently-
   duplicate rows). But that incident's fix already shipped:
   `subagentDisplayLabel()` (`swarm-model.ts:343-349`) appends a short,
   always-unique `agent_id` suffix (`${slug} · ${shortId}`) whenever
   `display_name` is absent, and `SubagentRow` (`swarm-view.tsx:757`) calls
   it for every row in `agentToolRows` uniformly — confirmed by reading
   both sites directly. So two genuinely-different backfilled subagents
   sharing a slug do **not** render as identical text today.
   What actually produces the *perception* of duplicates: many rows
   sharing the same slug's memorable portion (mechanism 1's accumulation
   makes this a large N), with only a small, easy-to-miss 7-char suffix
   distinguishing them — visually reads as "48 duplicate rows" even though
   each is technically unique. The real bug is accumulation (mechanism 1),
   not naming — no naming fix is needed here beyond what already shipped.

3. **Workflow-member spillage under stale dispatch data.** If
   `ListDispatches` is lagging/stale when the tree builds,
   `buildDispatchBuckets`'s `orphanedWorkflowMembers` fallback
   (`swarm-model.ts:224-239`) spills a Workflow's members **individually**
   into the flat Agent-Tool bucket instead of one grouped row — turning
   "1-2 Workflow-tool calls" into dozens of same-slug Agent-Tool rows.

4. **Reconciliation can silently no-op, so dead entries never even get
   marked stale.** `reconcile_stale_subagents` only acts when the parent
   block's turn is confirmed idle (`turn_active == Some(false)`) — unknown
   (`None`) defaults to "assume active, don't touch"
   (`scan.rs:104`, `unwrap_or(true)`). `SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20.md`
   §5 Open Question 2 (flagged, never confirmed) notes reconcile can fire
   before the block's controller registers in `CONTROLLER_REGISTRY`, in
   which case `get_block_controller_status` returns `None` and the whole
   pass no-ops. This compounds mechanism 1: even the cleanup path meant to
   catch this can silently fail to run at exactly the moment (startup)
   it's needed most.

5. **Even when a member IS correctly marked `Abandoned`, the dispatch
   itself is not.** `DispatchStatus` only has `Running`/`Completed` —
   `Abandoned` was "proposed but not yet implemented"
   (`types.rs:138-148`). `SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20.md`
   §5.3 confirms reconciling a member never updates the dispatch's own
   aggregate status. A dead dispatch can still read as ambiguous/running.

6. **Abandoned rows never auto-clear.** The 60s auto-linger countdown
   (`SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06.md`) only arms for a
   clean terminal `"idle"` completion, explicitly not for
   `"interrupted"`/`Abandoned` rows. Combined with #1's non-persisted
   dismissal, a dead-session row has literally no path to ever leaving the
   list on its own.

### 2.5 What's confirmed vs. inferred

Mechanisms 1, 3, 4, 5, 6 are confirmed directly from code (file:line cited
above, independently re-verified for the most load-bearing claims:
`types.rs`'s `DispatchStatus` enum, `scan.rs`'s `unwrap_or(true)` gate,
`swarm-model.ts`/`swarm-view.tsx`'s naming-suffix fix). Mechanism 2 was
investigated and found to be **already fixed**, not a live bug — corrected
above rather than left as an open item. **No one has reproduced the exact
48-row incident against srv logs** — proceeding on the strength of this
code-level evidence rather than blocking on a live repro (decided together
with the user); if the fixes below don't fully resolve a future recurrence,
capture `muxlog srv grep "backfilling session subagents"` /
`"capping cold-backfill replay"` around a fresh repro next.

## 3. Design

### 3.1 Robust one-call-one-stream attribution

- **Naming-collision fix already shipped** (`subagentDisplayLabel`'s short-id
  suffix, task #44) — no further work needed here; kept as a regression
  guard (a unit test asserting the suffix, see §5 verification) rather than
  re-implemented.
- **`orphanedWorkflowMembers` must not spill members into the flat
  Agent-Tool bucket.** When dispatch data is stale, either hold rendering
  in a loading state until `ListDispatches` catches up, or group orphaned
  members under a single synthesized placeholder row keyed by their shared
  `dispatch_id`/workflow id — never as N independent flat rows.
- **Reaffirm `dispatch_id` as the sole grouping/dedup key**, everywhere a
  row is built or a component decides "is this the same call." No code
  path should key off `slug`/`display_name` for identity, only for label
  text — the two are already conflated in mechanism 2 above and that's
  exactly the bug class to close off structurally, not just patch once.

### 3.2 Formalized session lifecycle

- **Add `DispatchStatus::Abandoned`** and propagate it: when
  `reconcile_stale_subagents` marks a dispatch's last/all live members
  abandoned, update the owning `AgentDispatch.status` too (aggregate rule:
  `Abandoned` iff all members are `Completed | Abandoned` and at least one
  is `Abandoned`; otherwise keep existing `Completed`-iff-all-members-done
  behavior). This closes the gap where a dead dispatch reads as ambiguous.
- **Make reconciliation's "unknown controller status" branch not a silent
  no-op.** Today `None` (controller not yet registered) is treated
  identically to "confirmed active." Distinguish them: `None` should either
  retry once the controller registers (if that's recoverable within the
  same process lifetime) or, at minimum, not be indistinguishable from
  "genuinely still running" — the current behavior is the single most
  likely reason dead entries survive a restart looking alive.
- **No separate "boot id" concept needed** (simplified from an earlier draft
  of this section) — once `DispatchStatus::Abandoned` exists (previous
  bullet) and reconciliation reliably runs (this bullet), "is this dead"
  is just `status ∈ {Completed, Abandoned}`. Nothing in this app persists
  `SubagentWatcher` state across a restart (§2, mechanism 1), so an entry
  surviving into a fresh process is either live (still being actively
  written to, will resolve to `Completed` on its own) or genuinely dead
  (will resolve to `Abandoned` once reconciliation runs) — no third state
  to track separately.

### 3.3 Human-facing management — implemented, scope note

- **Persist dismissal/retire state past reload — shipped.**
  `_retiredRowKeys` now round-trips through `localStorage`
  (`loadRetiredRowKeysFromStorage`/`saveRetiredRowKeysToStorage`,
  `swarm-model.ts`), local-machine scope per §6's decision. Every write
  path (`retireRow`, the retired-entry pruning pass) goes through one
  wrapped setter so persistence can't be bypassed by a future call site.
- **Bulk clear action — shipped**, not a "Historical" collapsed section.
  `collectClearableRows()` + `SwarmViewModel.retireAllCompleted()` +
  a "Clear completed (N)" button in the Swarm toolbar (only rendered when
  N > 0) retires every currently-visible terminal-status row in one click —
  directly answers "we need a way for humans to manage that" without a
  larger UI restructuring.
- **Visual distinction for Abandoned — already closer to done than this
  spec's draft assumed.** Re-checked `swarm-view.scss` directly: the status
  *dot* already differs by color (`--interrupted`: warning-orange,
  `--idle`: gray) — only the row/group-level dimming wrapper is identical
  (0.55/0.85 opacity for both). Left as-is; the dot is the primary status
  signal and a shared dimming level for "any terminal row" is reasonable,
  not obviously broken. No change made here.
- **NOT shipped: a full Live/Historical collapsed-section UI split.**
  Descoped deliberately — a real information-architecture change (default-
  collapsed section, its own retention bound, restructuring how
  `AgentTreeNode`'s buckets render) that deserves its own design pass and
  test coverage rather than being rushed alongside the backend lifecycle
  work in this same pass. The persisted bulk-clear above already directly
  addresses the reported symptom (a human can now clear a 48-row backlog in
  one click, and it stays cleared across reload); the section split is a
  further UX polish on top, tracked as a follow-up, not required to close
  the original complaint.

## 4. Non-goals

- Not attempting to fix the recursive/nested-subagent attribution gap
  discussed separately (`parentUuid`/`spawned_from_agent_id` not yet
  consumed) — out of scope here, tracked separately.
- Not persisting full historical dispatch data server-side across restarts
  beyond what already exists (JSONL files on disk remain the source of
  truth for backfill) — this spec formalizes *classification* (live vs.
  historical) and *display*, not a new persistence layer for raw activity.

## 5. Phasing

- **Phase A — stop the bleeding (attribution):** §3.1's orphaned-member-
  spillage fix (the naming fallback item is already fixed, kept only as a
  regression-test target).
- **Phase B — dispatch-level lifecycle:** §3.2's `DispatchStatus::Abandoned`
  + reconcile-ordering fix. Makes "is this dead" answerable as a direct
  `status` read instead of a timing-dependent heuristic.
- **Phase C — human management surface:** §3.3's persisted retire +
  Live/Historical split (driven by Phase B's `status`) + bulk clear +
  visual distinction.

B before C (C's Live/Historical split needs B's `DispatchStatus::Abandoned`
to be meaningful). A is independent and lands first.

## 6. Decisions (user delegated — "your choice, proceed", 2026-08-19)

1. **Proceeding directly to fixes**, no live repro captured first — the
   code-level evidence is strong enough per §2.5, and this environment
   runs shared production instances that shouldn't be used for a
   deliberate repro.
2. **Retired/dismissed state persists to local settings** (per-machine,
   not synced/cross-device) — lowest-risk option, consistent with how
   other per-machine UI state already persists in this app.
3. **Historical retention bound: cap at `BACKFILL_MAX_FILES` (200)
   equivalent for display** — reuse the existing constant's value rather
   than invent a new number; a bulk "Clear historical" action covers the
   rest.

## Critical files

- `agentmux-srv/src/backend/subagent_watcher/types.rs` — `DispatchStatus`,
  `SubAgentStatus`, `AgentDispatch`/`SubAgent` structs.
- `agentmux-srv/src/backend/subagent_watcher/jsonl.rs` — eager-naming gate
  (`live` param), `process_jsonl_change`.
- `agentmux-srv/src/backend/subagent_watcher/scan.rs` — `reconcile_stale_subagents`,
  `scan_subagents_dir`/backfill, the `unwrap_or(true)` gate.
- `agentmux-srv/src/backend/subagent_watcher/query.rs` — `list_active`/`list_dispatches`
  (unbounded, unfiltered).
- `frontend/app/view/swarm/swarm-model.ts` — `buildDispatchBuckets`,
  `orphanedWorkflowMembers`, `subagentDisplayLabel`, `_retiredRowKeys`.
- `frontend/app/view/swarm/swarm-view.tsx` / `swarm-view.scss` — status
  label mapping, `Abandoned`/`Completed` visual parity.
- Prior specs this builds on: `SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19.md`,
  `SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20.md`,
  `SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06.md`,
  `RETRO_SWARM_PHANTOM_ROWS_AND_STALE_TRACKING_2026_08_06.md`,
  `retro-subagent-watcher-shared-dir-fanout-and-leak-2026-07-23.md`.
