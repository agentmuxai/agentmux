# Phase F (host reducer) audit — 2026-05-06

**Purpose.** Reconcile the master spec's "Phase F not started" status with what's actually in the code. The work has been happening in-flight via the H.x submodules (`agentmux-cef/src/reducer/{panes,browsers,drag,pool,quit,top_level}.rs`) and is far more complete than the master ledger reflects.

**Authority.** The master spec ([`MASTER_REDUCER_STACK_STATUS_2026-05-05.md`](./MASTER_REDUCER_STACK_STATUS_2026-05-05.md)) §2 row for Host (Phase F) flips from 🟨 *"mirrors built (B.5), reducer migration not started"* to 🟨 *"4/6 H.x phases complete; H.1 panes + H.6 top-level scaffolded but not wired"* in the companion update PR.

The Phase H spec ([`SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md`](./SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md)) and 5-PR plan ([`SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02.md`](./SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02.md)) are the *operational* plan; this audit captures the actual landed state of each H.x phase.

## Status by H.x phase

| Phase | Field on `HostState` | Reducer arms | Production callers wired | Legacy state on `AppState` | Status |
|---|---|---|---|---|---|
| **H.1** panes | `browser_panes: HashMap<String, BrowserPaneEntry>` | yes (`reducer/panes.rs`) | **NO** — `state.browser_panes: BrowserPaneManager` still authoritative | `pub browser_panes: BrowserPaneManager` (`state.rs:594`) | 🟨 SCAFFOLDED |
| **H.2** browsers | `browsers: HashMap<String, BrowserHandle>` | yes (`reducer/browsers.rs`) | **YES** — sole source of truth post-PR-#H2.e | DELETED (state.rs:483 comment) | ✅ COMPLETE |
| **H.3** drag | `active_drag: Option<DragSession>` | yes (`reducer/drag.rs`) | **YES** — `state.active_drag` removed | DELETED | ✅ COMPLETE |
| **H.4** pool | `pool: PoolState` | yes (`reducer/pool.rs`) | **YES** — drift-storm fix (PR #708) added `just_promoted_labels`; pool snapshot helpers in `state.rs:813-880` | DELETED (`window_pool` / `unpromoted_pool_labels`) | ✅ COMPLETE |
| **H.5** quit | `quit_state: QuitState` | yes (`reducer/quit.rs`) | **YES** — `is_quitting: AtomicBool` replaced | DELETED (state.rs:218 comment) | ✅ COMPLETE |
| **H.6** top-level | `top_level_creation: TopLevelCreationState` | yes (`reducer/top_level.rs`) | **NO** — `EnqueueTopLevelWindow` / `TopLevelCallbackFired` have no dispatchers | n/a (greenfield) | ⏸️ DORMANT |

Plus the original Phase F field that pre-dates the H.x reframing:
- **F.1** `pending_window_creations` — ✅ COMPLETE. State on `HostState`, single mutation path through `host_dispatch`.

**Net: 5/7 fully migrated. 2/7 (H.1 panes, H.6 top-level) have reducer-side scaffolding but no production callers.**

## What "scaffolded but not wired" means

Each H.x phase follows the standard a→b→c→d→e ratchet from [`migration-pattern.md`](../retro/migration-pattern.md):

```
a) parallel writes — both legacy and reducer mutate
b) reads with fallback — readers prefer reducer, fall back to legacy
c) flip reads — reducer is authoritative; legacy is a passive shadow
d) drop legacy writes — only the reducer mutates
e) delete legacy field — single source of truth
```

H.2/H.3/H.4/H.5 reached step e. H.1 and H.6 are at step **a** at best — the reducer fields exist (`#[allow(dead_code)]` on `browser_panes` and `top_level_creation` annotations confirm) but no production code dispatches commands to them yet.

## H.1 — pane lifecycle (largest remaining item)

**Legacy:** `state.browser_panes: BrowserPaneManager` (`agentmux-cef/src/browser_panes.rs`, 806 lines). Actively written by 9+ call sites in `ipc.rs` and elsewhere.

**Target:** `HostState.browser_panes: HashMap<String, BrowserPaneEntry>` keyed by `block_id`, mutated only via `host_dispatch(EnqueuePaneCreate / CompletePaneCreate / EnqueuePaneClose / CompletePaneClose / AbortPaneCreate)`.

**Migration path:** the standard ratchet, but `BrowserPaneManager` has more behaviour than the other H.x fields (drain logic, navigation, focus routing, per-pane CDP wiring). A→e likely needs ~3 PRs of its own:
1. **Parallel writes** — every `state.browser_panes.create()` / `close()` etc. also dispatches the matching `HostCommand`.
2. **Drift compare** — on every reducer event, assert `state.browser_panes.contains(label) == HostState.browser_panes.contains_key(block_id)`.
3. **Flip reads + delete legacy** — readers shift to `HostState.browser_panes`; `BrowserPaneManager` becomes a thin wrapper that only emits commands; eventually deleted.

Estimated effort: 5-8 days wall-clock (matches the 4-5 day "high risk" budget the 5-PR plan put on PR #2 for browsers + panes combined; browsers turned out smaller than expected).

## H.6 — top-level creation runner (smaller, optional)

**Legacy:** `agentmux-cef/src/ui_tasks.rs::post_create_window` and friends — direct CEF Browser creation. No state to migrate — it's a function-call path.

**Target:** `HostState.top_level_creation` queues `EnqueueTopLevelWindow { request }`; an event-driven runner consumes the queue, calls CEF, dispatches `TopLevelCallbackFired { label }` on `on_after_created` to advance the in-flight slot.

**Why dormant:** the existing function-call path works; the reducer-driven version is a structural improvement (event log captures every top-level open + the timing per phase) but doesn't fix any current bug. Defer until H.1 lands and the host reducer is the authoritative state container — at which point H.6 becomes "the last legacy callback path."

**Estimated effort:** 2-3 days wall-clock.

## Recommendation

1. **This PR** — doc-only update to master spec §2 + §4 reflecting actual status. Plus this audit file linking from there.
2. **Next PR** — H.1 step (a) — parallel writes for pane lifecycle. Smallest forward step that makes meaningful progress on the largest remaining item.
3. **After H.1 reaches step (e)** — Phase F is "essentially done" for purposes of master spec §2; reduce H.6 to a tracked-but-deferred item.

## Cross-reference

- Master spec authority: [`MASTER_REDUCER_STACK_STATUS_2026-05-05.md`](./MASTER_REDUCER_STACK_STATUS_2026-05-05.md) §4
- Granular Phase H plan: [`SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md`](./SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md)
- 5-PR operational plan: [`SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02.md`](./SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02.md)
- Migration ratchet: [`migration-pattern.md`](../retro/migration-pattern.md)
- Long-term tracking: [discussion #707](https://github.com/agentmuxai/agentmux/discussions/707)
