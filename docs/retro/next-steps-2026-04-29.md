# Next steps — toward the full launcher reducer

**Status:** historical — planning doc from 2026-04-29 (post-#600 / B.9). Captures
what was done, what was left, and the intended order at that time.
**Author:** AgentA.
**Date:** 2026-04-29 (post-#600 / B.9 merge).

---

## Where the launcher reducer is right now

After this session's PRs (#594 + #595 + #596 + #597 + #598 + #599 + #600) the launcher's reducer **is the canonical mutator** for nine distinct state projections. Every state change goes through `agentmux-launcher::reducer::update(&mut state, cmd, &ctx) -> Vec<Event>`:

| State | What it tracks | Authority |
|---|---|---|
| `lifecycle` | Starting → Running → Quitting → Dead | Reducer (B.3) |
| `processes` | PID → kind / state / version | Reducer (B.3) |
| `windows` | label → kind, parent, opened_at, **+ B.9: hwnd, visible, iconic, last_rect, last_foreground_at_ms, foregrounded_since_open** | Reducer (B.4 + B.5 + B.9) |
| `pool` | pre-warmed pool labels | Reducer (B.4 follow-up) |
| `instance_registry` | label → instance num | Reducer (B.5) |
| `backend_window_ids` | label → backend window id | Reducer (B.5) |
| `monitors` | Win32 monitor topology | Reducer (B.9) |
| `pending_hwnds` | unlinked Win32 HWNDs awaiting reconciliation | Reducer (B.9) |
| `event_version` / `next_client_id` | monotonic counters | Reducer (B.3) |

**Pure functional core.** The reducer never blocks, never awaits, never does I/O. It runs synchronously inside a `Mutex<State>` lock for sub-millisecond duration. Side effects are emitted as `Vec<Event>` and applied by subscribers (host's `apply_event_to_shadow`, future Tool clients, future srv).

**Pure event-driven.** Every reducer call is in response to an external Command. There is no clock task, no heartbeat, no timer. State transitions and corrective actions both fire on the same OS-event-driven dispatch.

**59 tests** cover invariants:
- Reducer-arm unit tests (44 pre-B.9 + 8 new B.9: idempotent dup, drift+corrective, sentinel suppress, post-foreground no-correct, no-monitors-known, orphan-destroy chain, dual-source race, double-link drift, rect math).
- 1 proptest generates arbitrary command sequences and asserts mirror invariants hold.

---

## What "full" means

The golden vision (per `docs/specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` and `docs/retro/multi-reducer-proposal-2026-04-28.md`) is:

```
                        ┌────────────────────────┐
                        │  LAUNCHER REDUCER       │
                        │  (canonical state)      │
                        └─────────┬───────────────┘
                                  │ Events
              ┌───────────────────┼─────────────────────┐
              ▼                   ▼                     ▼
        HOST PROJECTION    SRV PROJECTION       RENDERER PROJECTION
        (CEF, win32 ops)   (workspace data)     (frontend reducers)
              │                   │                     │
              └─→ Commands ←──────┴──────── Commands ←──┘
```

Three things make the model "full":

1. **Authority**: every shared state mutation goes through the reducer. No subscriber writes to canonical state directly.
2. **Subscription**: every subscriber receives the events it cares about, so the projection it maintains is provably consistent with the reducer's view (modulo the well-defined async lag between event emission and projection update).
3. **Sagas / corrective events**: cross-process transitions (e.g., "user closed main → cascade-close children → reap pool") are encoded as reducer-emitted events that subscribers act on, NOT as ad-hoc cross-process synchronization in subscriber code.

**Where we stand against this:**

| Property | Status |
|---|---|
| (1) Authority over windows / pool / instance_registry / backend_window_ids / monitors / hwnd state | ✅ |
| (1) Authority over `host.browsers` (CEF Browser handles) | ❌ Phase B "scaffolding" — host-only, retired in Phase F |
| (1) Authority over `host.window_pool` + `host.unpromoted_pool_labels` | ❌ Phase B "scaffolding" — same |
| (2) Host subscribes to launcher events | ✅ via `apply_event_to_shadow` |
| (2) Renderer subscribes to launcher events | ❌ B.7.3 — currently goes through bespoke `window-instances-changed` |
| (2) srv subscribes to launcher events | ❌ Phase D / E |
| (3) Cross-process correctives via reducer events | ✅ (B.9.2 `CorrectiveWindowMove` proves the pattern) |
| (3) Snapshot / replay for resync after disconnect | ❌ Phase D |

---

## What's left, concretely

The work below is ordered as I'd ship it. Each row is independently shippable and reviewable.

### Tier 1 — finish Phase B (small, well-scoped)

#### 1.1 Source-side fix: CEF Views position bug

**Why first**: B.9.2's self-heal masks a real underlying bug. `commands/window.rs::open_window_with_kind` computes a position via `get_offset_position()` and passes it to `post_create_window`. In practice the new top-level CefWindow lands at the Win32 hidden sentinel `(-31970, -31970)` instead of the requested offset. The corrective then snaps it onto the primary monitor, which works but is a band-aid. Fixing this means new windows appear where the user expects (offset from the active window) rather than centered.

**Investigation hooks**: `agentmux-cef/src/ui_tasks.rs::CreateWindowTask` is what actually runs on the CEF UI thread. Either CefWindow's bounds are being overridden by Views' default placement, or the `set_bounds` call we make is racing the initial paint and getting overwritten. Likely fix: pass the position through `WindowDelegate::get_initial_bounds` instead of post-create `set_bounds`.

**Estimate**: ~50 LoC, 1 PR.

#### 1.2 B.7.3 — CEF JS bridge for typed launcher events

**Why next**: closes (2) for the renderer subscription axis. After this lands, the renderer doesn't depend on the host's bespoke `window-instances-changed` event — it subscribes to typed launcher events directly. The synchronous count-only emits in `window.rs` / `drag.rs` / `window_pool.rs` / `client.rs` can finally be retired (they exist as fallbacks for the non-launcher-driven path).

**Plan**:
- `agentmux-cef/src/launcher_ipc.rs` adds an outbound JS-bridge fanout. When `apply_event_to_shadow` receives an event, it also serializes it to JSON and `Frame::ExecuteJavaScript("window.__launcher_event && window.__launcher_event(<json>)")` against every active CEF browser.
- New module `frontend/app/store/launcher-events.ts`: registers `window.__launcher_event` as a dispatcher into a SolidJS signal that block-level subscribers can listen to.
- Migrate `app-init.ts::initInstanceTracking` from `getApi().listen("window-instances-changed", ...)` to the new launcher-event subscription.
- Sync emits in `window.rs:655`, `drag.rs:385`, `window_pool.rs:559`, `client.rs:475` get retired (verified-redundant via the launcher-event path).
- `frontend` reducer fed by the typed event stream (the original B.7 vision).

**Estimate**: ~3 PRs, ~500 LoC total. The JS bridge plumbing is the bulk; the migration is straightforward once the bridge works.

#### 1.3 B.8.1 — `agentmux.exe --diag wrr`

**Why**: gives operators read-only access to the reducer state without grepping the launcher log. Connects as `ClientKind::Tool`, sends a new `Command::GetSnapshot` (B.8 also lays the groundwork for Phase D's resync), pretty-prints the windows table + recent drift events.

**Plan**:
- New `Command::GetSnapshot` + `Event::Snapshot { windows, monitors, drift_log_tail, ... }`.
- Reducer arm clones state into the snapshot.
- New `agentmux.exe --diag wrr` CLI surface in launcher (a separate entry point that connects to its own pipe and asks for a snapshot).
- Pretty-printer with a stable column layout.

**Estimate**: 1 PR, ~250 LoC.

#### 1.4 B.8.2 — property tests + CI smoke

**Why**: the reducer is the canonical state surface; property-based tests across arbitrary command sequences guard against regressions in state-machine invariants. CI smoke (synthetic open-N-windows-then-close-all) catches integration regressions.

**Plan**:
- New `proptest` strategies for each command family. Existing `arb_window_cmd` is the template.
- Invariants: every Open has exactly one matching Close; instance_registry numbers are monotonic; `windows.keys()` ⊆ instance_registry ∪ pool; etc.
- CI smoke: `agentmux.exe --headless --smoke close-all` script that opens N windows, closes them, asserts state.windows is empty.
- Delete obsolete defensive code (e.g., `app-init.ts::refreshLabels(retriesLeft)`'s retry loop — once B.7.3 lands, it's dead).

**Estimate**: 1 PR, ~300 LoC.

**Phase B exit criteria after 1.1–1.4**: the launcher reducer is the canonical authority for everything except the two scaffolding scratchpads (`browsers`, `window_pool`/`unpromoted_pool_labels`); subscribers (host, renderer) consume the typed event stream; operator surface (`--diag`) exists; CI catches regressions; the original spec's "delete all 13 host stores" goal is realized for the 11 maps that can move.

#### 1.5 B.9.3 — `OrphanInstance` drift + reducer-driven quit signal

**Why** (surfaced 2026-04-29 smoke of v0.33.490):

Smoke sequence: open AgentMux → tear off a tab → close every visible window → expect the host process tree to exit cleanly.

Actual: launcher tree (launcher + srv + 6 host PIDs) stays alive after close-all. Win32 enumeration shows two `Chrome_WidgetWin_1` HWNDs still owned by the host main thread, both parked at the `(-32000,-32000)` sentinel position. They're pool windows held warm for the next tear-off. The host's existing close path doesn't reap pool when the last user-visible window closes — so the process hangs around indefinitely with no UI.

**Why B.9 didn't catch it**: pool windows are filtered from `ReportWindowOpened` by design (the launcher's `state.windows` mirrors user-meaningful labels only). When the last `state.windows` entry is removed, the reducer's invariant set says "everything's fine — zero windows is a valid state". It has no concept of "the host process should be alive only as long as it has at least one user-visible window".

**The state machine ALREADY knows enough to detect this.** When `state.windows` is empty and the host's process record is in `Running` (per `state.processes`), and `pending_hwnds` only contains sentinel-positioned entries (= unpromoted pool inventory), that's the signature.

**Pure-reducer fix design** (no timers, mirrors B.9.2's pattern):

```rust
// agentmux-common::ipc
HwndDriftKind::OrphanInstance,    // Host alive, zero user-visible windows, only sentinel HWNDs remain
Event::HostShouldQuit { version }  // saga-style corrective: host subscribes, reaps pool, quits
```

Reducer trigger: at the END of `handle_report_window_closed` (the existing path that drops a label from `state.windows`), check the post-mutation invariant:

```rust
fn check_orphan_instance(state: &State) -> bool {
    state.windows.is_empty()
        && !state.pool.is_empty()           // there ARE pool windows (host is "warm")
        && state.processes.values().any(|p| {
              matches!(p.state, ProcessState::Running) && p.kind == ClientKind::Host
           })
}
```

When the predicate flips from false → true on a `WindowClosed` transition, emit `Event::HwndDriftDetected { kind: OrphanInstance, severity: Warn, ... }` AND `Event::HostShouldQuit { version }`. Host subscriber in `apply_event_to_shadow` handles `HostShouldQuit` by:
1. Reaping the warm pool (`window_pool::shutdown_all`).
2. Calling `quit_message_loop()`.
3. Tree exits cleanly via existing J0 reaping.

**Pure-reducer guarantee preserved**: the trigger is a state transition (the last `state.windows` entry being removed), not the passage of time. No timer evaluates "host has been window-less for N seconds" — the moment the predicate flips, the saga fires.

**Edge case — opt out for legitimate "headless"**: if a Tool client (`agentmux.exe --diag`) is the only registered client and there's no Host registered, the predicate is false (no Host process to quit). Tool sessions don't trigger `HostShouldQuit`.

**Scope**:
- New `Event::HostShouldQuit` + `HwndDriftKind::OrphanInstance` variant.
- Reducer arm at end of `handle_report_window_closed` checking the predicate.
- Host subscriber handling `HostShouldQuit` in `apply_event_to_shadow`.
- Two unit tests: trigger fires on transition; doesn't fire if `state.windows` was already empty when host registered (e.g., headless Tool session).

**Estimate**: 1 PR, ~150 LoC (mostly tests).

**Where this fits**: this is B.9.3 — a follow-up to B.9 in the same WRR family, ships before Phase B exit (1.4). It's part of "completing observation" before adding the bigger Phase D / E / F machinery.

---

### Tier 2 — Phase D (snapshot + resync + persisted log)

After Phase B exits, Phase D adds the durability story. The reducer's `event_version` counter is already monotonic — Phase D makes it useful.

#### D.1 GetSnapshot RPC

Lets a subscriber that disconnected and reconnected fetch the current state + the last N events, so it can rebuild its projection without missing anything between the snapshot and "now".

#### D.2 Persisted event log (ring buffer)

Fixed-size ring buffer of recent events on disk (`<data-dir>/launcher-events.log`). Survives launcher crash for forensics. After launcher restart, it's not authoritative state (state machine restarts fresh) but is invaluable for debugging.

#### D.3 Subscriber-resync protocol

Subscriber on (re)connect: `Register` → `GetSnapshot { since: 0 }` → reducer replies with `Snapshot { ... }` + `EventList { events: [...] }`. After applying, subscriber is caught up. Then it transitions to live event stream.

**Estimate**: ~3 PRs.

---

### Tier 3 — multi-reducer (Phase E + F)

Phase B ends with the **scaffolding model**: host owns `browsers` + pool maps directly (because they hold FFI handles + need synchronous lifecycle decisions). Phase E + F retire the scaffolding model in favor of multi-reducer, where each process has its own reducer over its own canonical state, and **sagas** — pure-reducer-emitted corrective events — handle cross-reducer transitions.

#### Phase E — srv reducer

`agentmux-srv` already has its own state (workspaces, tabs, blocks, layout, identity accounts). Promoting it to a Redux-style reducer is the **first validation point** for the multi-reducer model. If the srv reducer pattern works for tabs/panes/layout, the same pattern will work for host's CEF browsers in Phase F.

**Why Phase E first**: srv's state is purer (no FFI handles, no Win32 sync constraints), so it's a lower-risk first migration.

#### Phase F — host reducer

After E proves the multi-reducer pattern, retrofit the host. `state.browsers` becomes part of the host-side reducer's canonical state. Host emits a domain command (`OpenBrowser { label, kind }`) into its own reducer, which authorizes the FFI call and emits the event. The launcher reducer subscribes (via the event stream) and updates `state.windows`.

**Phase F deliverables**:
- New `agentmux-cef::reducer` with arms for `OpenBrowser`, `CloseBrowser`, `PoolAdd`, `PoolPromote`, `PoolDestroy`.
- Host-side reducer becomes the authority over `browsers` + pool maps.
- Sagas: when launcher emits `Event::WindowClosed { label, .. }`, host's saga consumer fires `Command::CloseBrowser { label }` into its OWN reducer, which drives the FFI close synchronously.
- WRR's `wrr/enforcer.rs` (the CorrectiveWindowMove apply path) generalizes to a saga consumer surface for ANY launcher-emitted corrective.

**Estimate**: 5–8 PRs across both crates, the largest single architectural shift remaining.

---

## Recommended order

1. **1.5 (B.9.3 — OrphanInstance + HostShouldQuit)** — small, surfaced by today's v0.33.490 smoke. Same WRR family; ships before the rest. ~1 day.
2. **1.1 (CEF Views position fix)** — small, removes a known masking case for B.9. ~1 day.
3. **1.3 (--diag wrr)** — small, gives us better visibility into 1.2's effect when we cut over. ~1 day.
4. **1.2 (CEF JS bridge / B.7.3)** — completes Phase B's frontend story. ~2-3 days across multiple PRs.
5. **1.4 (proptests + CI smoke)** — closes Phase B. ~1 day.
6. **Tier 2 (Phase D)** — durability story. Sequence with Tier 3 by appetite — Phase D is independently useful even if multi-reducer is deferred.
7. **Tier 3 (Phase E + F)** — multi-reducer. The biggest remaining architectural shift; only make sense after Phase D's resync protocol exists (multi-reducer makes the resync surface broader).

## Cross-cutting notes

- **Don't add timers**. The "no heartbeat" guarantee is load-bearing. If a corrective should fire, it should fire on the same dispatch as the transition that surfaces the bug. If a transition can't fire it, that's a missing event source, not a missing timer.
- **Don't relitigate the 3-class taxonomy**. `browsers` and pool maps stay scaffolding through all of Phase B + D. Phase F is the only acceptable retirement path.
- **Don't push code direct to main**. Per `feedback_pr_for_code_changes`. Even small fixes go through PR + reagent + codex.
- **Smoke before push for bug fixes**. Per `feedback_verify_before_push`. Bot reviewers don't run the binary; the user-visible bug is what matters.

## Reference

- Roadmap: `docs/retro/phase-b-roadmap.md`
- WRR design: `docs/retro/wrr-design-2026-04-28.md`
- B.5 architecture analysis: `docs/retro/b5-migration-architecture-2026-04-28.md`
- Multi-reducer proposal: `docs/retro/multi-reducer-proposal-2026-04-28.md`
- a→b→c→d→e migration ratchet: `docs/retro/migration-pattern.md`
- Spec: `docs/specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md`
- Inventory + gaps: `docs/specs/ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md`
