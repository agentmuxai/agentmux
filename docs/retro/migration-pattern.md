# Migration pattern: a→b→c→d→e ratchet (with sync-cache exception)

**Status:** Reference. Read AFTER `phase-b-roadmap.md` if you're resuming Phase B work.
**Author:** AgentA.
**Date:** 2026-04-28 (post-#592, post-multi-reducer decision).

---

## TL;DR

Phase B.5 migrates host-owned state to launcher-owned state one field at a time using a 5-step ratchet (a→b→c→d→e). 3 fields completed; 2 deferred to Phase F because they require a host-side reducer (multi-reducer architecture). One field (window_meta) used a refined version of the ratchet with a synchronous host cache instead of step-e deletion.

---

## When to use the ratchet

Apply the a→b→c→d→e ratchet when:
- The state is queryable across processes.
- The state has well-defined commands that mutate it.
- The state doesn't carry FFI handles or other in-process-only objects.
- The state can tolerate eventually-consistent reads (with a synchronous cache where needed).

**Don't apply it when** the state holds FFI pointers (e.g., CEF `Browser` handles), or when synchronous lifecycle decisions depend on it without tolerance for round-trip lag. In those cases the field is **boundary scaffolding** (current Phase B framing) or **host-reducer state** (Phase F destination — see `multi-reducer-proposal-2026-04-28.md`).

---

## The five steps

| Step | Who is canonical | Who is the shadow | What changes | Risk |
|---|---|---|---|---|
| **a** | host | (none) | Launcher gains its own copy, fed by host's `Report*` commands. Pure addition. | Low |
| **b** | host | launcher's projection in host AppState | Host caches launcher events into a `shadow_*` field; drift logged. Pure observation. | Low |
| **c** | shadow (read), host (write) | host's old field | Reads route through `state.<helper>()` which prefers shadow with host fallback. Host still writes. | Medium — read paths now depend on launcher round-trip |
| **d** | shadow | host's old field (vestigial) | Host stops writing. Shadow is the only source for reads. | Medium — exposes any code that read the field directly |
| **e** | launcher mirror | (deleted) | Host field removed; helper fallback removed. | Low (mostly delete-only) |

---

## Worked example: `window_instance_registry` (PRs #579-#584)

* **a (#579)**: launcher gained `State.instance_registry: HashMap<String, u32>` + `State.next_instance_num: u32`. `handle_report_window_opened` assigns numbers and emits `Event::WindowInstanceAssigned`.
* **b (#580)**: host gained `AppState.shadow_instance_registry`. `apply_event_to_shadow` consumes the events; on every event compares to host's `WindowInstanceRegistry` and logs drift.
* **c (#581 + #582)**: `state.instance_num()` / `state.instance_count()` helpers added (shadow-first, host fallback). 5 read sites switched. Host writes preserved. Smoke test caught a pre-existing orphan-pool-window bug (#582) — drift detection earned its keep.
* **d (#583)**: 4 mutation sites removed. `instance_count` fallback removed (host's count is now seed-only and would be stale). Shadow-driven re-emit added so InstancePanel catches up.
* **e (#584)**: `WindowInstanceRegistry` struct + field deleted. Helper fallback removed. Drift compares removed. **Net: -161 LoC / +52 LoC.**

Same pattern worked for `window_id_map` (#585-#589).

---

## The sync-cache exception: `window_meta` (PRs #590-#592)

`window_meta` looked like a normal candidate but step d hit two synchronous-lookup consumers:

1. `open_subwindow`'s parent-liveness check — must reflect "is this parent currently open" with no async lag.
2. Cascade-close enumeration during `on_before_close` — must include children opened just before.

The shadow lags by milliseconds; for these consumers, the lag was a correctness regression (orphan subwindows on race).

**Refinement**: keep `host.window_meta` as a **synchronous local cache**, written from a single canonical site (`on_after_created` from the popped `PendingWindowCreation` entry) and removed at `on_before_close`. Step e becomes a doc acknowledgment, not a delete.

The data flow:
- Caller (`drag.rs`, `window.rs`, `window_pool.rs`) pushes `PendingWindowCreation { label, kind, parent }` to a queue.
- `on_after_created` pops the queue, writes host's `window_meta` (sync cache), and emits `ReportWindowOpened` to launcher.
- Launcher's reducer updates `state.windows` and broadcasts `Event::WindowOpened`.
- Host's `apply_event_to_shadow` updates `shadow_window_meta`.
- Both host's sync cache and the launcher's projection are populated.

**Lesson for future migrations**: before step e, check whether the field has any same-process synchronous lifecycle-checking consumer. If yes, expect step e to become a sync-cache acknowledgment.

---

## When the ratchet doesn't work at all: `browsers` + pool maps

`browsers: HashMap<String, cef::Browser>` holds CEF FFI handles. The handles can't serialize over IPC and must stay co-located with the CEF runtime. The KEY-SET role (label set queries) is migratable and is already covered by B.4's `state.windows` + B.5's `shadow_window_meta` — but the field itself stays.

`window_pool` + `unpromoted_pool_labels` are coupled with CEF lifecycle callbacks and synchronous pool decisions. Same story.

These fields are **boundary scaffolding** in the current Phase B framing. Long-term they become host-reducer state under the multi-reducer model — see `multi-reducer-proposal-2026-04-28.md`. Phase B is finishing without retiring them.

---

## Anti-patterns

* **Don't read directly from `shadow_*` fields.** Always go through helpers (`state.instance_num()`, `state.window_meta()`, etc.). The helper encapsulates prefer-shadow / fallback / drift policy. Direct reads bypass the policy.
* **Don't add new mutations to host's old field once you're in step b+.** All new mutations should go via `report_*` commands. Direct mutation resets migration progress for that field.
* **Don't wait synchronously on launcher events from CEF callbacks.** UI thread can't block. Frontend retries are the correct race mitigation.
* **Don't conflate steps.** Each PR should be exactly one step (a, b, c, d, or e) for one field. Combining steps makes rollback expensive.
* **Don't try to fully retire a field with synchronous lifecycle consumers.** Use the sync-cache pattern (`window_meta`) instead of pushing all the way to step e.
* **Don't try to retire `browsers` or pool maps via this ratchet.** Read `b5-migration-architecture-2026-04-28.md` first; they need a host-side reducer (Phase F).

---

## Layer reference

| Concern | Location | Mutates? |
|---|---|---|
| Wire events | `agentmux-common/src/ipc.rs` | — |
| Canonical state (launcher reducer) | `agentmux-launcher/src/state.rs` | Only via reducer |
| Pure reducer | `agentmux-launcher/src/reducer.rs::update()` | The function |
| IPC server | `agentmux-launcher/src/ipc/server.rs` | Dispatches commands → reducer → broadcasts events |
| Host outbound | `agentmux-cef/src/launcher_ipc.rs::report_*` | Sends commands |
| Host inbound | `agentmux-cef/src/launcher_ipc.rs::apply_event_to_shadow` | Updates host shadows from events |
| Host shadows | `agentmux-cef/src/state.rs::AppState.shadow_*` | Only by `apply_event_to_shadow` |
| Host sync caches (`window_meta`, etc.) | Same file | Single canonical mutation site (`on_after_created` / `on_before_close`) |
| Host scaffolding (`browsers`, pool maps) | Same file | Direct CEF lifecycle / saga code paths; NOT migrate-able under current model |
