# Report: Robust dispatch attribution + swarm session lifecycle

**Date:** 2026-08-19
**Spec:** `docs/specs/SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md`
**Status:** Implemented, tested. No interactive/visual verification (see Verification below — same constraint as the prior report today).

## Context

Follow-up to the transcript dispatch-card work. The user reported a real symptom: a session with only "a couple" of real `Agent`-tool calls showed **48 entries** in the Swarm panel, including rows that visually read as duplicates of the same name (e.g. many rows sharing a slug like "flittering-booping-duckling"). The user also asked for the underlying restart/session lifecycle to be formalized — when an AgentMux instance closes and reopens, its subagents are dead and present only for historical record, and there was no way for a human to manage that.

Root-caused via two parallel research passes, then corrected mid-investigation: the exact-duplicate-name bug (task #44, 17 rows sharing one slug) turned out to already be fixed (`subagentDisplayLabel`'s short-id suffix) — the spec was updated to reflect this rather than re-fixing something already solved. The real causes were accumulation (unbounded historical retention, non-persisted dismissal) and a workflow-member spillage bug, plus a deeper gap: dispatches had no "dead" status of their own, and the reconciliation pass that should set one could silently no-op.

## What changed

**Phase A — attribution:**
- `buildDispatchBuckets` (`swarm-model.ts`) no longer spills a Workflow dispatch's members individually into the flat Agent-Tool bucket when `ListDispatches` is stale — they're grouped into one synthesized placeholder `WorkflowDispatch` row per `dispatch_id`, preserving the one-row-per-Workflow-call invariant even during a lag.
- Confirmed (not re-implemented) the naming-collision fix already in place; added no new code there, just corrected the spec's diagnosis.

**Phase B — dispatch-level lifecycle (Rust backend):**
- Added `DispatchStatus::Abandoned`. A Solo dispatch's status now mirrors its one member's `SubAgentStatus` directly (was previously collapsing `Completed`/`Abandoned` into one `Completed` bucket). A Workflow dispatch's status is set to `Abandoned` by `reconcile_stale_subagents` when every member is `Completed|Abandoned` and at least one is `Abandoned`.
- Found and fixed a real bug while wiring this up: `refresh_dispatch_status` (called by `list_dispatches()` on *every* read) recomputed status purely from member counts, silently overwriting a freshly-set `Abandoned` back to `Running`/`Completed` on the very next RPC call. Fixed with an early-return guard treating `Abandoned` as terminal.
- Fixed the flagged-but-never-confirmed race where `reconcile_stale_subagents` could run before a block's controller registers in `CONTROLLER_REGISTRY`, silently treating "unknown" the same as "confirmed active" forever. Now retries exactly once (2s delay) before giving up — bounded, not a loop, verified by a dedicated test for the exhausted-retry path.
- 6 new Rust unit tests covering the aggregation rule, the retry's success/exhaustion paths, and the terminal-status guard.

**Phase C — human-facing management:**
- `_retiredRowKeys` (the existing per-row dismiss mechanism) now persists to `localStorage` (local-machine scope, per the user's decision) instead of resetting on every reload — mirrors the existing `toolchain-view.tsx` localStorage pattern.
- New bulk "Clear completed (N)" button in the Swarm toolbar — retires every currently-visible terminal-status row in one click, only rendered when there's something to clear. Backed by a pure, directly-tested `collectClearableRows()` function.
- Descoped: a full Live/Historical collapsed-section UI split. This is real information-architecture work deserving its own design pass; the persisted bulk-clear already lets a human resolve a 48-row backlog in one click and have it stay resolved, which directly answers the reported complaint without rushing a larger UI change.

## Verification

- `cargo check`/`cargo test -p agentmux-srv`: full suite passes, 2472 tests (0 failed, 6 pre-existing ignores).
- `npx tsc --noEmit`: clean.
- `npx vitest run`: full frontend suite passes, 2801 tests across 190 files (2 pre-existing skips).
- New test coverage added this pass: 2 frontend tests (orphaned-member grouping), 4 frontend tests (retire persistence round-trip/corruption/overwrite), 6 frontend tests (bulk-clear eligibility), 6 Rust tests (dispatch abandonment aggregation, reconcile retry paths) — 18 new tests total, all passing alongside the pre-existing suite.

**Not done:** interactive verification (triggering a real restart, watching 48 rows collapse to a manageable state, clicking "Clear completed," confirming persistence survives an actual app reload). Same constraint as before — this machine runs shared, live AgentMux instances I shouldn't disrupt, and this is a native desktop app rather than something screenshot-able via browser tooling.

## Files touched

```
Backend (Rust):
  agentmux-srv/src/backend/subagent_watcher/types.rs
  agentmux-srv/src/backend/subagent_watcher/query.rs
  agentmux-srv/src/backend/subagent_watcher/scan.rs
  agentmux-srv/src/backend/subagent_watcher/jsonl.rs
  agentmux-srv/src/backend/subagent_watcher/tests.rs

Frontend:
  frontend/app/view/swarm/swarm-model.ts
  frontend/app/view/swarm/swarm-model.test.ts
  frontend/app/view/swarm/swarm-view.tsx
  frontend/app/view/swarm/swarm-view.scss

Spec (corrected in place during investigation, not just written once):
  docs/specs/SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md
```
