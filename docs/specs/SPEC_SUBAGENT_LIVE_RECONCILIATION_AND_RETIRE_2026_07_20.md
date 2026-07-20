# SPEC — live subagent reconciliation + Retire action (best-practices plan)

**Date:** 2026-07-20
**Status:** Phase A shipped (#2234, merged). Phase B (#2235, this PR) implements
the rest of this doc.
**Builds on:** `docs/specs/SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md`
(closes its Open Question 1; makes a fresh product decision on Open
Question 2). Diagnosis: `docs/specs/REPORT_SWARM_SUBAGENT_INTERRUPTED_STATUS_2026_07_20.md`.
**Scope:** `agentmux-srv/src/backend/blockcontroller/persistent.rs`,
`agentmux-srv/src/backend/subagent_watcher.rs` (backend); `frontend/app/view/swarm/swarm-model.ts`,
`swarm-view.tsx` (frontend).

---

## 1. Context

A long-running agent pane accumulates subagent records that never get
reconciled to a terminal status, because `reconcile_stale_subagents` only
runs when its pane is closed and reopened — never live, never on a timer.
The frontend's client-side "Interrupted" backstop (a deliberate,
already-shipped guard — not a bug in itself) then fires for every such
stuck-`Active` row every time the parent pane is idle between turns, which
is most of the time. On a pane like "Lzop" — open continuously, spawning
many subagents, never reopened — this reads as "everything is always
Interrupted," even for subagents that may have genuinely finished. See the
companion report for full root-cause detail and citations.

This is exactly the tradeoff the 07-12 spec's Open Question 1 named and
deliberately deferred ("recommend starting with the reopen-time-only
version... and evaluating whether the live case is common enough in
practice to warrant the real-time wiring as a fast-follow"). Tonight's
observation answers that question: yes, it's common enough — any
long-lived pane hits it, not an edge case.

## 2. Goals

1. Reconcile a subagent's status the moment its parent's turn actually
   ends, not just at the next pane reopen — closing Open Question 1.
2. Give the user an explicit way to remove a terminal-status
   (`Completed`/`Abandoned`) row from the Swarm pane view — a Retire
   action, addressing the user's direct request tonight and Open Question
   2 (never resolved either way in the 07-12 spec).

## 3. Non-goals

- Persisting subagent status across `agentmux-srv` restarts — out of
  scope, same as the 07-12 spec's own non-goals.
- Changing Workflow-level dispatch status/aggregation
  (`DispatchStatus`/`refresh_dispatch_status`) — unaffected by either
  change here.
- A bulk "retire all" action — start with per-row, revisit if the manual
  cost proves real in practice (see §7 open questions).
- Persisting *retired* state across `agentmux-srv` restarts or across
  devices — see §6's design rationale for why this should start
  client-local/ephemeral, matching the existing
  `memory-pressure-banner.tsx` dismiss precedent (`dismissedAt` is a plain
  `createSignal`, resets on reload, no backend write).

## 4. Phase A — real-time reconciliation on turn-end

**The hook point already exists and is already commented for this exact
purpose.** `agentmux-srv/src/backend/blockcontroller/persistent.rs:917-922`,
the one place `turn_active` flips back to `false` for a persistent
(never-exits-between-turns) agent process:

```rust
// Claude's turn-ending marker. Persistent mode never exits
// between turns, so this is the only place `turn_active`
// can go back to false without waiting for process exit —
// see `send_message`'s matching `set_active_turn(true)`.
if parsed.get("type").and_then(|v| v.as_str()) == Some("result") {
    health_read.set_active_turn(false);
    // Publish the flip so the Swarm view's live
    // ControllerStatus subscription reflects "turn
    // ended" immediately ...
    if let Some(ref broker) = broker_read {
        let status = { /* ... */ };
        super::publish_controller_status(broker, &status);
    }
```

Add one call here: `subagent_watcher.reconcile_stale_subagents(&block_id, &session_id)`
(the method already exists, `subagent_watcher.rs:1062`, `fn` not `pub fn` —
needs `pub(crate)` or `pub` to be callable from `blockcontroller`, a
one-line visibility change). This block already needs a
`session_id` in scope for the call — confirm it's available at this call
site (likely via the same `inner_read` lock already taken for the status
snapshot) or thread it in from wherever `persistent.rs` already tracks the
current session id per block.

This closes the exact gap: a subagent stuck `Active` now gets resolved to
`Completed` (if it has a `Result` line — it may have raced the turn-end
event, worth double-checking ordering) or honest `Abandoned` within the
same tick the parent's turn ends, not at the next reopen. No new state,
enum, or wire format — purely calling an existing function from a new,
better-timed call site.

**Broadcast note:** `reconcile_stale_subagents` doesn't currently broadcast
anything on its own (confirmed — it mutates `SubAgentStatus` in place with
no WS emission). At reopen time this doesn't matter (the reopening pane's
own `loadSubagents()` picks up the corrected status on its next fetch
anyway). Live, it does matter — a connected Swarm pane watching this block
needs a push, not just a corrected value sitting server-side until some
unrelated event triggers a reload. Add a broadcast (reuse the
`subagent:completed`/`dispatch:updated` pattern the frontend already
listens to and reloads on) for any subagent this pass actually downgrades.

## 5. Reopen-time edge cases — audited

Phase A adds a *second* reconciliation trigger (live, turn-end) alongside
the existing one (`scan_session_subagents` → `reconcile_stale_subagents`,
fired from `handle_reactive_register`, `server/reactive.rs:350`, on pane
(re)open). Before adding a second call site, the existing reopen path was
audited end to end for correctness, since Phase A's live trigger inherits
the same `reconcile_stale_subagents` logic and any bug there would now
fire twice as often. Each case below is marked **Confirmed correct**
(verified in code, often already covered by an existing test),
**Confirmed correct, but out of scope** (a real gap, deliberately not
fixed here), or **Needs verification** (plausible, not yet confirmed —
flagged for the implementing PR, not resolved by this doc).

1. **Backfill-before-reconcile ordering** — does a subagent that actually
   finished (has a `Result` line already on disk) risk being reconciled to
   `Abandoned` before its own completion gets parsed? **Confirmed
   correct.** `scan_session_subagents` (subagent_watcher.rs:1000-1038)
   calls `scan_subagents_dir` (which reads every file, including any
   `Result` line, setting `Completed`) at line 1033, THEN
   `reconcile_stale_subagents` at line 1034 — the ordering is right, no
   race. `reconcile_stale_subagents` only ever touches subagents still
   `Active` after that pass (subagent_watcher.rs:1097), so an
   already-just-`Completed` one is untouched.

2. **No controller registered at reconcile time.** `reconcile_stale_subagents`
   reads `get_block_controller_status(parent_block_id).map(|s| s.turn_active).unwrap_or(true)`
   (subagent_watcher.rs:1063-1066) — `None` (no controller in
   `CONTROLLER_REGISTRY` for this block yet, `blockcontroller/mod.rs:218-224`)
   is treated as "assume active, don't touch," a deliberately conservative
   default (never guess a false demotion). **Needs verification:** does
   `handle_reactive_register` (which triggers the backfill+reconcile pass)
   ever run before this block's controller has registered itself in
   `CONTROLLER_REGISTRY`? If so, that reconcile pass silently no-ops, and
   the stale subagents wait for either a live Phase A trigger (which also
   needs a registered, *running* controller to fire from at all) or the
   *next* reopen. Add a trace/test for this exact ordering before
   shipping Phase A — if the race is real, the fix is likely "run the
   backfill+reconcile pass after controller registration is guaranteed,"
   not a change to `reconcile_stale_subagents` itself.

3. **Workflow dispatch aggregates don't reflect member-level reconciliation.**
   `refresh_dispatch_status` (subagent_watcher.rs:1829-1838) computes a
   Workflow dispatch's own `status`/`members_done` from journal counters
   (`journal_started`/`journal_results`) and a 60s-quiet heuristic on
   `last_event_at` — entirely independent of individual `SubAgent.status`.
   Reconciling one member to `Abandoned` (live or at reopen) does **not**
   update the dispatch's own aggregate "N/M active" count. **Confirmed
   correct, but out of scope** — this is the exact boundary
   `SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md`'s own Non-goals
   drew ("Redesigning workflow-level status... already has its own
   (heuristic but functioning) staleness handling"). Not fixed here;
   flagging so it isn't mistaken for something Phase A was supposed to
   also close.

4. **Idempotency of repeated reopens.** Rapid close/reopen cycles, or a
   reopen immediately followed by a live Phase A trigger for the same
   subagent — does re-reconciling cause any flapping? **Confirmed
   correct.** `reconcile_stale_subagents` only demotes subagents currently
   `Active` (subagent_watcher.rs:1097); already-`Abandoned` or
   `Completed` entries are untouched on a repeat pass — covered by the
   existing `reconcile_stale_subagents_never_downgrades_an_already_completed_subagent`
   test. No new test needed, but worth re-running that suite once Phase A
   adds the second call site to confirm it still holds when reconciliation
   can now fire from two places instead of one.

5. **Pane merely switching tabs vs. actually closing.** `TermWrap.dispose()`
   (`termwrap.ts:414-433`) fires `unregisterAgent()` → backend
   `unwatch_agent()`, which doesn't just reconcile — it fully **removes**
   every subagent/dispatch/pending-activity record for that agent identity
   (subagent_watcher.rs:604-630, extended in #2233 to also prune
   `dispatches`/`pending_activity`, not just `sessions`). **Needs
   verification:** does switching between tabs in the same workspace
   actually unmount the previous tab's pane component (triggering
   `dispose()`), or does AgentMux keep inactive tabs mounted-but-hidden
   (the more likely design, given xterm.js/PTY reconnection cost)? If tab
   switches genuinely dispose+reregister every time, that's not a
   *correctness* bug (backfill reconstructs the same state fresh from the
   JSONL files on disk — the source of truth isn't touched), but it would
   mean every tab switch pays a full unwatch+rewatch+backfill cost. Worth
   a quick trace before/independent of this spec's two phases — it's a
   possible performance finding, not a blocker for either phase.

6. **Interaction with #2233's block-delete pruning.** A block that's
   actually *deleted* (not just closed) now gets its subagent state
   removed entirely via `SubagentWatcher::prune_block`
   (`#2233`, subagent_watcher.rs) rather than reconciled — the row
   disappears outright instead of showing `Abandoned`. **Confirmed
   correct** — this is the right outcome (nothing to reconcile toward if
   the block no longer exists) and doesn't interact badly with either
   Phase A or Phase B: a deleted block's rows are gone before Retire would
   ever apply to them, and Phase A's live reconciliation only fires for
   blocks whose controller is still registered, which a deleted block's
   isn't.

## 6. Phase B — Retire action

**Design: client-local, ephemeral, per-row dismiss set — not a backend
concept.** Precedent: `frontend/app/notification/memory-pressure-banner.tsx`'s
`dismissedAt` — a plain `createSignal`, no backend write, resets on
reload/restart. Retiring a row is "I've seen this, stop showing it to me
right now," not a durable data mutation; the underlying `SubAgent`/
`AgentDispatch` record is untouched server-side (still available via
`GetHistory` if ever needed, still counted correctly for any future
"how many subagents has this session run" reporting).

- `SwarmViewModel` gains `retiredRowKeys: Set<string>` (same `rowKey`
  concept introduced in #2232's `getDispatchDetail`/
  `toggleDispatchExpanded` refactor — `agent:${agent_id}` for a
  single-subagent row, `dispatchId` for a `WorkflowDispatchRow`) and a
  `retireRow(rowKey: string)` method that adds to the set and triggers a
  tree recompute (same `buildTree()`-invalidation path an expand/collapse
  toggle already uses).
- `buildDispatchBuckets()` (or its caller, `buildTree()`) filters out any
  row whose key is in `retiredRowKeys` before returning `agentToolRows`/
  `workflowRows` — same shape as the existing `parentIds` fallback
  filtering, just subtractive instead of additive.
- **Gate Retire to terminal status only** — a row still genuinely
  `"working"` has no Retire action available (nothing to dismiss; it's
  legitimately in progress). Available for `"idle"` (Completed) and
  `"interrupted"` (Abandoned, or the stuck-Active backstop case Phase A
  mostly eliminates but doesn't guarantee zero of).
- UI: a small "×" or "Retire" affordance on the row, symmetrical with the
  existing per-row expand chevron — mirrors `AgentToolRow`/
  `WorkflowDispatchRow`'s existing hover-affordance conventions in
  `swarm-view.scss`.
- **Un-retiring:** none needed for v1 — a retired row reappears
  automatically the moment the underlying subagent/dispatch produces new
  activity (spawns again isn't possible for the same `agent_id`, but a
  dispatch could in principle; simplest correct behavior is: retiring
  is keyed to the CURRENT state snapshot, and any subsequent
  `subagent:spawned`/`dispatch:updated` event for that same key implicitly
  un-retires it by virtue of `scheduleLoadSubagents()` producing a fresh
  object — the filter only suppresses what's already known-dismissed, it
  doesn't block new information from that same key). If this proves
  surprising in practice, revisit with an explicit "un-retire" affordance.

## 7. Open questions

1. **Does Phase A's new call site race the `Result`-line processing?**
   `set_active_turn(false)` fires from the stdout-reader loop the instant
   a `"type":"result"` line is seen on the *parent's own* stream — but a
   subagent's own completion is detected separately, reading the
   *subagent's* JSONL file via the filesystem watcher
   (`process_jsonl_change`). If the subagent's own `Result` line hasn't
   been read yet by the time the parent's turn-end fires (plausible if the
   fs-watcher's 200ms debounce hasn't ticked), `reconcile_stale_subagents`
   would (correctly, per its own logic) mark it `Abandoned` even though a
   `Completed` line arrives moments later — but `reconcile_stale_subagents`
   only touches subagents still `Active`, and once `Abandoned` is set, does
   anything downgrade it back to `Completed` if the `Result` line shows up
   right after? Confirmed no — `process_jsonl_change`'s completion check
   (`subagent_watcher.rs:1363-1374`) sets `Completed` unconditionally on
   seeing `Result`, regardless of current status, so this self-corrects if
   the late line does eventually get read. Worth an explicit test:
   reconcile-then-late-result-arrives ends at `Completed`, not stuck
   `Abandoned`.
2. **Retire scope** — per-row only, or does retiring a `WorkflowDispatchRow`
   also implicitly retire... nothing, since dispatch rows don't expose
   member rows anymore (SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19).
   No cascading concern — flagging only to confirm during implementation.
3. **Should Retire persist across app restart?** Recommended no for v1
   (§6's rationale) — but if user feedback says otherwise once this ships,
   the natural persistence layer would be `AGENTMUX_DEV`-style local
   settings, not a backend/wire concept, since retiring is a purely
   client-side display preference.
4. **§5.2's controller-registration race** and **§5.5's tab-switch-vs-close
   question** — both need a direct trace/test before Phase A ships, not
   just this doc's reasoning. Neither blocks Phase B.

## 8. Rollout

Two independent PRs, either order:

1. Phase A (backend): visibility change + one new call site + one new
   broadcast. Small, testable in isolation (extend the existing
   `reconcile_stale_subagents_*` test suite in `subagent_watcher.rs` with
   a "called from the turn-end hook, not just reopen" case).
2. Phase B (frontend): `retiredRowKeys` set + filter + UI affordance. No
   backend dependency — can ship before or after Phase A, though Phase A
   landing first means fewer rows need retiring in the first place (they
   self-correct to `Completed`/honest `Abandoned` live instead of
   requiring the user to manually dismiss stuck-`Active` "Interrupted"
   noise).
