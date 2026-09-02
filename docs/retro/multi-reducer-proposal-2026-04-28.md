# Multi-reducer architecture (accepted direction)

> **STATUS UPDATE (2026-04-28):** Originally written as a proposal; **direction accepted** later same day. This is now the agreed long-term plan. Sequencing: finish Phase B with the scaffolding model (per `b5-migration-architecture-2026-04-28.md`), then Phase D (snapshot/replay), then Phase E (srv reducer), then Phase F (host reducer — retires the scaffolding model). See `phase-b-roadmap.md` for current state.

**Author:** AgentA.
**Date:** 2026-04-28.
**Companions:**
* `b5-migration-architecture-2026-04-28.md` — the analysis that prompted this proposal
* `migration-pattern.md` — the a→b→c→d→e ratchet (single-reducer migration)
* `phase-b-roadmap.md` — phase B sub-PR sequence
* `docs/specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` — the driving spec

---

## TL;DR

The "host has scaffolding outside the state machine" framing in `b5-migration-architecture-2026-04-28.md` is pragmatic but conceptually awkward. A cleaner alternative: **three reducers** (launcher, host, srv), each canonical for its domain, communicating via versioned events.

This proposal sketches the design, the cross-reducer-sync patterns, and a recommended sequence for adopting it. Net recommendation: **adopt eventually, but not as part of Phase B**. Finish Phase B with the scaffolding model, then layer multi-reducer in Phase E (srv reducer, already planned) and Phase F (host reducer, new).

---

## The proposal

Three reducers, each owning a coherent slice of state:

| Reducer | Scope | Examples |
|---|---|---|
| **Launcher** | OS-level cross-process facts | process tree, window inventory, instance numbers, lifecycle phase |
| **Host** | CEF integration + lifecycle scaffolding | browsers, pool, pool-respawn-in-flight, pre-create handoff, taskbar/HWND state |
| **Srv** (planned Phase E) | App domain | tabs, panes, layouts, agents, workspaces |

The key flip: **`browsers` + pool maps stop being "scaffolding outside the state machine"** — they become host-reducer domain state with reducer-enforced invariants. Things like:

* "no label in both `unpromoted_pool_labels` and `window_pool` simultaneously"
* "pool size ≤ POOL_TARGET_SIZE"
* "every `window-pool-*` label is in either pool or promoted-windows, never both"

become reducer-enforced rather than convention-enforced. These are exactly the kinds of invariants that produced the bugs the spec calls out (instance count inflates after tear-off crash, burst tear-offs empty the pool, pool windows leak to taskbar).

---

## The cross-reducer-sync problem

This is well-trodden territory in distributed event-sourced systems. The patterns that work, applied here:

### 1. Local-canonical, global-eventually-consistent

Each reducer is **canonical for its domain**. Other reducers hold **projections** (read-only mirrors) updated via events. We already built this for launcher↔host with `shadow_*` fields. Generalizing to 3 reducers: each pair has the same relationship.

```
Launcher canonical: state.processes, state.lifecycle, state.windows (label set)
Host canonical:     state.browsers, state.pool, state.pre_create_queue
Srv canonical:      state.tabs, state.panes, state.layouts, state.agents

Projections each reducer holds:
- Launcher holds projections of host (window-set events) + srv (workspace events)
- Host holds projections of launcher (instance numbers, etc.) + srv (window→workspace mapping)
- Srv holds projections of launcher (window-set) + host (window labels)
- Frontend holds projections of all three via JS bridge
```

### 2. Events as the only cross-reducer contract

**Commands** stay within the issuing reducer's domain. **Events** are the only thing crossing boundaries. **No reducer ever directly mutates another's state** — they emit events, and the other reducer's `apply_event_from_X` handler decides what to do (typically: update a projection field).

This is the discipline that makes the system tractable. Without it, "reducer X writes to reducer Y's state through a back-channel" defeats the purpose.

### 3. Sagas for cross-reducer operations

A tear-off touches all three reducers:
- **Host**: pop pool window from queue, transition to user-visible.
- **Srv**: assign the workspace data to the new window.
- **Launcher**: register new instance number, update window inventory.

Express it as a **saga** — a state machine that lives OUTSIDE the reducers, sequences commands across them, waits for confirming events, handles failures with compensating actions. Pseudocode shape:

```
saga TearOff(workspace_id):
  1. cmd Host::PromotePoolWindow(workspace_id) → expect Host::PoolWindowPromoted{label}
  2. cmd Srv::AssignWindowToWorkspace(label, workspace_id) → expect Srv::WindowWorkspaceAssigned
  3. cmd Launcher::RegisterWindowInstance(label) → expect Launcher::WindowInstanceAssigned{num}
  4. (all three confirm) → saga complete

  on any step failure:
    compensate: emit Host::DepromoteWindow(label), Srv::UnassignWorkspace, …
```

Each step is observable; partial failures recover via compensating events. The user sees one logical operation; internally it's a sequenced multi-reducer transaction.

### 4. Versioned events + snapshot-replay

Each reducer's events are versioned within its scope. Subscribers detect gaps (`event.version > last_seen + 1`) and request a snapshot to resync. Phase D's `GetSnapshot` protocol generalizes naturally.

### 5. Single-writer per state field (enforced structurally)

Only the canonical reducer's `update()` can mutate its fields. Projections in other reducers are read-only — they only have `apply_event` handlers, no `mutate_field`. This is what we already do informally; multi-reducer makes it explicit and types it.

### 6. Drift detection between projections and canonical

Per-transition: subscriber reports its count/checksum; canonical compares to its own; emit drift events on mismatch. We built this for B.4 between host and launcher. Same pattern between launcher↔srv, host↔srv.

### 7. Bounded staleness + synchronous local caches

Projections lag by a bounded delay. Code requiring zero-lag uses synchronous local state (the "sync cache" pattern we landed for `window_meta`). This is unchanged in multi-reducer; it's just clearer that the cache is a property of the consuming reducer, not "scaffolding outside the model."

### 8. Idempotent event handlers

Every `apply_event` must be safe to apply twice. Critical for replay during resync. We've been doing this informally; should be a typed contract (Rust trait: `IdempotentApply`).

### 9. Per-reducer property tests + cross-reducer integration tests

Each reducer gets its own proptest battery (already done for launcher). Cross-reducer invariants tested via integration tests that drive sagas end-to-end against three running reducers.

### 10. Coordinator pattern for sagas

Sagas don't live in any reducer — they need a coordinator. Two options:

* **Centralized in launcher**: launcher hosts a saga runtime that drives cross-reducer flows. Heavier in launcher; easier to debug.
* **Distributed**: each saga has a "primary" reducer that owns its lifecycle; commands flow peer-to-peer. Lighter; harder to debug.

For our scale, centralized in launcher is simpler. The launcher already has Tokio + IPC + reducer; saga runtime is one more component.

---

## What this changes vs the current path

If we adopt multi-reducer:

* The `b5-migration-architecture-2026-04-28.md` framing of "scaffolding vs state" gets replaced with "host-reducer-state vs launcher-reducer-state vs srv-reducer-state." Cleaner.
* `browsers` + pool maps stop being awkward. They're host-reducer state with explicit invariants.
* B.5 finish becomes: define the host reducer, dispatch host state changes through it, expose its events to launcher.
* B.7 (frontend cutover): frontend subscribes to all three reducers' event streams via the CEF JS bridge.
* Phase E (srv reducer): natural fit — same pattern, third instantiation.

The existing single-reducer-in-launcher work stays valid. We don't retrofit `window_instance_registry` etc. into a host reducer; they genuinely are launcher domain (cross-process visibility was the goal).

---

## Trade-offs

### Pros

* Conceptually clean — every state has an explicit reducer enforcing its invariants.
* Better testability — each reducer property-tested in isolation; cross-reducer behavior tested via saga integration tests.
* Failure isolation — reducer bugs don't cross processes (panic in srv reducer doesn't corrupt launcher state).
* Aligns with Phase E plan — srv reducer was already going to happen; adding host reducer makes the system uniform.
* Eliminates the "scaffolding" exception that currently sits awkwardly in `b5-migration-architecture-2026-04-28.md`.

### Cons / costs

* **More boilerplate per process** (~500 LoC for host reducer infrastructure: types, dispatch, event apply, proptest harness). Same scale as launcher's reducer plumbing was.
* **Saga pattern is more complex than direct mutation** for cross-reducer ops. Mistakes here are subtle (compensating actions, partial failures). Need careful design + tests.
* **Three sources of truth for different aspects** → potential confusion about which reducer owns what. Mitigated by clear scope definitions and a single source-of-truth doc.
* **Performance: every state change goes through a reducer** — even host-internal ones. CEF callbacks now route through a dispatch instead of direct mutation. Probably fine (microseconds), but worth measuring.

---

## Sequencing recommendation

**Yes, adopt — but as Phase E+ work, not blocking Phase B exit.**

Phase B as planned can finish with the "scaffolding" framing — it's pragmatic and ships the launcher reducer cleanly. Phase E adds the srv reducer (already planned). After Phase E, retrofitting the host into a reducer is a smaller step because the patterns are already validated in two places.

Recommended sequence:

| Step | Goal | Validates |
|---|---|---|
| **Finish Phase B** (current scaffolding model) | Launcher reducer + host scaffolding | Single-reducer pattern at scale |
| **Phase D** (snapshot/replay) | Generic resync infrastructure | Versioned events + snapshot pattern |
| **Phase E** (srv reducer) | Second reducer + cross-process events | Multi-reducer pattern, saga skeleton |
| **Phase F** (host reducer) | Third reducer, retire scaffolding model | Generalization to N reducers |

Each step builds on validated patterns from the previous. Doing host-reducer right after Phase B would mean designing the multi-reducer infrastructure on the fly, against just one new reducer (no validation point).

---

## Why not now (defending the deferral)

The temptation is real: the architectural model gets cleaner and the "scaffolding exception" goes away. But:

1. **The scaffolding model isn't broken.** It works, ships, has invariants enforced via the launcher's drift detection. The cleanliness gap is aesthetic, not functional.

2. **We don't have a second reducer yet.** Designing N-reducer infrastructure with N=2 (launcher + host) is harder than designing it with N=3 (launcher + srv + host) because the patterns are clearer with three. Phase E gives us that third reducer for free, since it was already planned.

3. **Phase B exit unlocks downstream value (B.7 frontend cutover, Phase D snapshots) sooner.** Each session of "polish the architecture before shipping" delays user-visible improvements.

4. **The migration cost is bounded if we wait.** Adding a host reducer in Phase F is a localized refactor; the host's existing state is well-understood and the events it would emit are already half-defined (just the inbound `Report*` commands today, become outbound events post-reducer).

---

## What to write down before deferring

Even if we defer, capture the design intent now so future-us (or a Phase F PR reviewer) doesn't relitigate:

* This doc.
* A line in `phase-b-roadmap.md` (local) noting that "scaffolding" is provisional pending Phase F.
* A spec addendum in `SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` acknowledging the multi-reducer trajectory.

---

## Open design questions (for whenever we adopt)

* **Do reducers share a Tokio runtime or run separately?** Launcher has its own; srv does too. Host gets a third runtime? Or do we host-reducer-as-a-task in the existing host runtime? Cleanest: each reducer is a Rust struct, called from whatever runtime hosts the process. No new runtimes.
* **How are saga states persisted across launcher restarts?** If the launcher crashes mid-saga, do we resume? For Phase B/E, probably no — saga restart-on-launcher-restart is Phase D resync territory.
* **What's the granularity of events?** Per-field? Per-domain-action? Phase D's `GetSnapshot` performance depends on this.
* **How do we handle backward-compatibility when adding new event variants?** The serde tagged-enum strategy we use scales fine, but each reducer's event log on disk needs forward-migration tools.
* **Cross-reducer transactions vs sagas — when each?** Strict transactions need a coordinator with all reducers blocking. Sagas are eventually-consistent. Probably: sagas for everything; if we need a strict transaction, that's a sign the reducer scope is wrong.

---

## Bottom line

The multi-reducer pattern is the right long-term architecture. It generalizes from where we are. The "scaffolding" framing is a useful intermediate state, not a permanent design.

Sequencing: finish Phase B, do Phase D + E, then come back and do Phase F (host reducer). Total additional cost: 1 session for Phase F's infrastructure, 1-2 sessions for the actual host-reducer migration of `browsers` + pool maps.
