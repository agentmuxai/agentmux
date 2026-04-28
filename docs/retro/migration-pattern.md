# Migration pattern: moving state from host to launcher (Phase B)

**Status:** Active reference for Phase B sub-PRs (B.4, B.5, B.7).
**Author:** AgentA (extracted from PR #581 review-cycle conversation).
**Date:** 2026-04-28.

## TL;DR

Phase B moves OS-level state (windows, pool, instance numbers, lifecycle) from the host's ad-hoc `HashMap`s into the launcher's pure-reducer state machine. We can't atomically swap a state store that has ~5 read/write call sites, so each migration is a **5-step ratchet** (a → b → c → d → e). The "shadow" naming refers to the host-side projection of the launcher's authoritative state during the transition.

## End state: the Redux/state-machine model

The launcher owns the canonical OS-level state. It runs a **pure reducer**:

```
update(state, command, ctx) → Vec<Event>
```

* **Commands** flow client→launcher — "I did a thing" or "do a thing."
* **Events** flow launcher→clients — broadcast state changes.
* **State** lives only in the launcher. Other processes hold *projections* of state that they maintain by consuming events.

This is the Elm/Redux pattern: state is data, transitions are functions, side effects fire from events. Spec source: `specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md`.

The contract is enforced by:
* The reducer being pure (no I/O, no clocks, no env reads — context is injected).
* The `update` function being **total** — never panics on input.
* `proptest` over arbitrary command sequences (asserts version monotonicity, lifecycle invariants, etc.).
* Strict layering: only the reducer mutates state.

## The problem this solves

Pre-Phase-B, host's `AppState` (`agentmux-cef/src/state.rs`) was the de-facto authority for everything: `browsers`, `window_meta`, `window_id_map`, `window_instance_registry`, `window_pool`, `unpromoted_pool_labels`, `window_pool_respawn_in_flight`, etc. — 13 separate Mutex-wrapped fields per `ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md`. Each map was mutated by ~5 call sites scattered through CEF lifecycle callbacks, IPC handlers, drag/drop code.

Symptoms of this fragmentation (recurring bugs the spec analyzed):

| Bug | Why |
|---|---|
| Pool windows leak to taskbar after tear-off crash | `WindowInstanceRegistry::register()` and `unregister()` had non-symmetric call paths — `unregister` called from `on_pool_window_destroyed` for labels that were never registered → silent no-op + leaked instance number. |
| Burst tear-offs empty the pool | `window_pool_respawn_in_flight: AtomicBool` released on render-ready, not on spawn-complete; second spawn saw stale `false`. |
| Pool windows mixed into main map | `unpromoted_pool_labels: HashSet` set parallel to `browsers` — not a typed distinction. |

No invariants enforced centrally; bugs came from these maps falling out of sync. The fix isn't more careful coding — it's **eliminating the parallel maps**: one state, one mutator, one place to enforce invariants.

## The migration ratchet (per state field)

You can't atomically swap a state field that has 5 read/write call sites. So each field migrates in 5 steps:

| step | who's authoritative | who's the "shadow" | what changes | risk |
|---|---|---|---|---|
| **a** | host | (none) | launcher gains its own copy, fed by host's `Report*` commands. Pure addition. | low |
| **b** | host | launcher's projection in host `AppState` | host caches the launcher's events, drift-logs disagreements. Pure observation. | low |
| **c** | shadow (read), host (write) | host's old field | host's reads route through helpers like `state.instance_num()` which prefers shadow with host fallback for race window. Host still writes to its old field. | medium — read paths now depend on the launcher round-trip working |
| **d** | shadow | host's old field (vestigial — still mutated for fallback semantics, but reads go to shadow) | host stops mutating its old field. Shadow is the only source of truth for reads. The fallback in step (c) is removed. | medium — exposes any code that read host's field directly |
| **e** | launcher mirror | (deleted) | host's old field is removed. Read sites go through the launcher's events end-to-end. | low (mostly delete-only) |

For `window_instance_registry`, we just finished step **c** (PR #581).

## Why this naming is awkward

The field is called `shadow_instance_registry` because in steps **a**/**b**/**c** it shadows the host's authoritative copy. But once we hit step **d**, the shadow IS the read path and the "shadow" name is misleading.

The fix is to rename at step (e) when the old field is deleted. Until then we keep the name to signal that this is mid-migration code, not a permanent design.

## Concrete example: window_instance_registry

The instance-number registry assigns sequential numbers (main=1, second=2, …) for window-title display. Migration timeline:

* **B.5a** (PR #579, merged): launcher gained `State.instance_registry: HashMap<String, u32>` + `State.next_instance_num: u32`. `handle_report_window_opened` now also assigns a number and emits `Event::WindowInstanceAssigned`. **Pure addition** — host untouched.
* **B.5b** (PR #580, merged): host gained `AppState.shadow_instance_registry: Mutex<HashMap<String, u32>>`. `launcher_ipc::apply_event_to_shadow` consumes `WindowInstanceAssigned`/`Released` events into the shadow. On every event the launcher's value is compared to host's `window_instance_registry`; disagreements log to `target = "launcher-ipc:drift"`. **Pure observation** — host's existing code untouched.
* **B.5c** (PR #581, merged): added `AppState::instance_num(label)` and `AppState::instance_count()` helpers that prefer the shadow with host fallback. Switched 5 read sites (`get_instance_number`, `get_window_count`, count emits in `register_backend_window`/`on_before_close`/`drag.rs`/`window_pool.rs`) to use the helpers. Host's `register()`/`unregister()` mutations preserved for the race window.
* **B.5d** (next): drop host's eager `register()`/`unregister()` calls. Two open design questions:
  * Without host mutations, the fallback in `instance_count` returns the seed-only host count (1) — fallback is moot. Either drop fallback (race exposure) or drive count emits from shadow updates (more invasive).
  * Frontend's `get_instance_number` IPC could race the launcher round-trip on first window load. Mitigation options: frontend retry, host pre-seeds expected number, or accept a brief `unwrap_or(1)` flash.
* **B.5e** (after B.5d validates): delete `AppState.window_instance_registry` field + `WindowInstanceRegistry` struct entirely. Rename `shadow_instance_registry` → `instance_registry`.

The smoke test before/after each step is the key validation: PR #580's B.5b smoke test showed two cold-path tear-offs assigning 2 + 3, monotonic, zero `DRIFT` lines. That gave us confidence to proceed to read cutover (B.5c) without any behavior change.

## How the layers fit together

```
                 ┌─────────────┐
                 │   LAUNCHER  │
                 │  (reducer)  │
                 │  STATE = {  │  ← canonical state
                 │   windows,  │
                 │   pool,     │
                 │   instance_ │
                 │     registry│
                 │   processes,│
                 │   …         │
                 │  }          │
                 └─────┬───────┘
                       │ events (broadcast)
        ┌──────────────┼─────────────────┐
        ▼              ▼                 ▼
   ┌─────────┐   ┌─────────┐       ┌─────────┐
   │  HOST   │   │   SRV   │       │  TOOL   │
   │ (proj.) │   │ (proj.) │       │ --diag  │
   │ AppState│   │  ...    │       │  ...    │
   │ has no  │   │         │       │         │
   │ canonical│  │         │       │         │
   │ state — │   │         │       │         │
   │ just a  │   │         │       │         │
   │projection│  │         │       │         │
   └────┬────┘   └─────────┘       └─────────┘
        │ commands
        ▼
  (back to launcher reducer)
```

Once all 13 host maps complete a→e:
* Bugs from "host's `window_meta` and `window_instance_registry` fell out of sync" become structurally impossible — one state, one reducer.
* `agentmux.exe --diag` (Phase D) becomes a Tool client that just reads launcher state and prints it.
* Frontend stops polling — it subscribes to events via the JS bridge (Phase B.7).
* Phase D resync (versioned events + GetSnapshot) becomes implementable: clients detect missed events by version-number gap and replay from a snapshot.

## Why we did B.4 first as a "mirror"

B.4 (window mirror + pool tracking + drift detection) is the same idea applied to maps the host was already tracking. The launcher's `state.windows` and `state.pool` are projections built from `Report*` commands. `ReportHostCounts` → `DriftDetected` was the validation loop that proves the projection tracks reality before any code DEPENDS on it.

B.5 then says: now that the projection is trusted, let's invert the dependency — host READS from the projection (via shadow) instead of its own copy.

## Layer reference

| concern | location | mutates? |
|---|---|---|
| Wire events | `agentmux-common/src/ipc.rs` (`Command`, `Event`) | — |
| Canonical state | `agentmux-launcher/src/state.rs` | only via reducer |
| Pure reducer | `agentmux-launcher/src/reducer.rs::update()` | the function |
| IPC server | `agentmux-launcher/src/ipc/server.rs` | dispatches commands → reducer → broadcasts events |
| Host outbound | `agentmux-cef/src/launcher_ipc.rs::report_*` | sends commands |
| Host inbound | `agentmux-cef/src/launcher_ipc.rs::apply_event_to_shadow` | updates host shadows from events |
| Host shadows | `agentmux-cef/src/state.rs::AppState.shadow_*` | only by `apply_event_to_shadow` |
| Host old fields (being retired) | `agentmux-cef/src/state.rs::AppState.window_*` etc. | by host code; progressively shrinking surface |

## Anti-patterns to avoid

* **Don't read directly from `shadow_*` fields.** Always go through helpers like `state.instance_num()`. The helper encapsulates the prefer-shadow / fallback / drift-log policy. Direct reads bypass the policy and make the migration step (d→e) harder.
* **Don't add new mutations to host's old fields.** Once a field is in step (b)+, all new mutations should go via `report_*` commands. Direct mutation of e.g. `window_instance_registry` from a new code path resets the migration progress for that field.
* **Don't wait synchronously on launcher events.** The IPC channel can be slow under load and CEF callbacks run on the UI thread — blocking would freeze the renderer. Frontend retries are the correct race mitigation.
* **Don't conflate the migration steps.** Each PR should be exactly one step (a, b, c, d, or e) for one field. Cross-step combinations make rollback expensive.

## Related docs

* `specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` — the driving spec.
* `specs/ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md` — pre-migration inventory of host's 13 state stores.
* `docs/retro/phase-b-plan-2026-04-28.md` — sub-PR sequence (B.1–B.8) with line-by-line risk analysis.
* `docs/retro/audit-vestigial-types-2026-04-28.md` — pre-Phase-E cleanup audit.
