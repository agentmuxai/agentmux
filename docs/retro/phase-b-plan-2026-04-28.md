# Phase B Implementation Plan — Launcher-Owned State Machine

**Date:** 2026-04-28
**Author:** AgentA-asaf
**Status:** Draft for review — surface decisions, then execute
**Inputs:** `SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md`, `docs/retro/pr-570-implementation-plan-2026-04-27.md`, current code in `agentmux-launcher/`, `agentmux-cef/`, `agentmux-srv/`, `frontend/`.

---

## Locked decisions (to confirm before execution)

1. **Strict layering wins.** Launcher = state machine + Job Object. Host = thin executor + HWNDs. Srv = persistent app data only, stays window-agnostic. Frontend = passive mirror.
2. **Robust > performance.** Pure sync reducer, exhaustive matches, panic-on-invariant-violation (kernel reaps via Job Object).
3. **No heartbeats / no polling.** Liveness comes from kernel signals (`child.wait()`, pipe-EOF). Already locked in spec edits via PR #570.
4. **Truth in launcher.** Deliberate departure from Chromium pattern (which puts truth in the privileged window-owning process) — justified by "launcher survives host crashes."

---

## Phase B deliverable (one-line)

A pure Rust state machine in the launcher that owns all window/process runtime state, drives the host via named-pipe IPC, broadcasts versioned events to all renderers, and survives host crashes without state loss.

---

## Architecture (concrete)

### Process tree (post-Phase B)

```
launcher (agentmux.exe)
│   • Per-data-dir mutex
│   • Job Object (KILL_ON_JOB_CLOSE)
│   • Pure reducer: update(state, Command) → (state, Vec<Event>)
│   • Effects runner (Tokio runtime; out of reducer hot path)
│   • State: Map<WindowId, WindowState>, Map<ProcessId, ProcessRecord>,
│            WarmPool, LifecyclePhase, EventLog (ring buffer)
│   • Named-pipe IPC server
│       \\.\pipe\agentmux-{data_dir_hash}\command   (host + frontend writes)
│       \\.\pipe\agentmux-{data_dir_hash}\events    (one per subscriber)
│
├── host (agentmux-cef-{ver}.exe)
│   • CEF + HWNDs
│   • Connects to launcher pipes on startup
│   • Receives Commands (do this Win32 thing)
│   • Emits Facts (this HWND was created / destroyed)
│   • Forwards launcher events to renderers via CEF JS bindings
│
└── srv (agentmux-srv-{ver}-win.exe)
    • Workspace/Tab/Block SQLite DB (unchanged)
    • Connects to launcher pipe
    • Reports started/ready (no heartbeat — pipe-EOF is liveness)
    • Receives Quit { reason } command; ack with "done"
```

The big change vs today:
- **Srv moves from host-spawned to launcher-spawned.** Both are children of the launcher's Job Object. Srv survives host crashes; the launcher can restart the host without losing srv state.
- **Launcher gains substantial logic** — currently ~270 lines, will grow to ~2000 lines (reducer + IPC + state + tests).
- **Host loses ~13 state stores** (the HashMaps in `AppState`). Becomes a thin executor.
- **Frontend deletes the polling loop in `app-init.ts`** and subscribes to the launcher's event stream via the host's CEF JS bindings.

### IPC layout

**One pipe per concern**, named after the data_dir hash so multi-instance still works:

| Pipe | Purpose | Server | Clients |
|---|---|---|---|
| `\\.\pipe\agentmux-{hash}\command` | Commands → launcher; one bidi stream per client | launcher | host (1), frontend renderers (N), srv (1), tooling |
| `\\.\pipe\agentmux-{hash}\events-{client_id}` | Events ← launcher; one ordered stream per client | launcher | each host/frontend renderer/srv subscriber |

Per spec §5.3: "ordering is guaranteed only within a single pipe" (Mojo guidance). Each subscriber gets its own event pipe so order is preserved per-subscriber.

The frontend doesn't connect to launcher directly; it goes **through the host** via CEF's JS bindings (host has the launcher pipe open; host's JS bridge forwards both directions). This avoids exposing the launcher pipe to renderer-process untrusted code.

### Reducer types (sketch)

```rust
// agentmux-launcher/src/reducer/mod.rs

pub enum Command {
    OpenWindow {
        workspace_id: String,
        position: Option<(i32, i32)>,
        source: WindowSource,
    },
    CloseWindow { id: WindowId },
    PromoteWarm { warm_id: WarmId, become: WindowId, position: (i32, i32) },
    BringToFront { id: WindowId },
    HostReportsHwndCreated { id: WindowId, hwnd_handle: u64 },
    HostReportsHwndDestroyed { id: WindowId },
    HostReportsCrash { kind: ProcessKind, pid: u32, exit_code: i32 },
    Quit { reason: QuitReason },
    GetSnapshot { client_id: ClientId },
}

pub enum Event {
    WindowAdded { id: WindowId, source: WindowSource, version: u64 },
    WindowStateChanged { id: WindowId, from: WindowState, to: WindowState, version: u64 },
    WindowRemoved { id: WindowId, reason: CloseReason, version: u64 },
    ProcessSpawned { pid: u32, kind: ProcessKind, version: u64 },
    ProcessExited { pid: u32, code: i32, version: u64 },
    LifecyclePhaseChanged { from: LifecyclePhase, to: LifecyclePhase, version: u64 },
    WarmPoolChanged { ready: usize, spawning: usize, version: u64 },
}

pub fn update(state: &State, cmd: Command) -> (State, Vec<Event>) {
    // exhaustive match; no I/O; no panics on input; panics on invariant violation
}
```

Effects (NOT in the reducer; driven by reducer events):

```rust
pub enum Effect {
    SpawnHost,
    SpawnSrv,
    TellHostToCreateWindow { workspace_id, position },
    TellHostToCloseWindow { hwnd_handle },
    TellHostToShowSize { hwnd_handle, x, y, w, h, ws_visible: bool },
    BroadcastEvent(Event),
    QuitSrv { timeout: Duration },
    CloseJobHandle, // last resort
}
```

The effects runner converts events → effects, executes side effects on a Tokio runtime, and reports back via more `Command::HostReports*` calls.

---

## Migration plan — incremental, app stays working

The constraint: AgentMux must keep working after every PR. We can't have a "Phase B starts" cutover. Strategy: **parallel state during migration**, with the new launcher-side state machine watching what the host already does, gated behind a feature flag. Once the launcher's state matches reality across all observable scenarios, we cut the host's state stores out and the launcher becomes authoritative.

### Sub-PR sequence (estimated)

| # | Sub-PR | New code | Changed code | Risk |
|---|---|---|---|---|
| **B.1** | Move srv from host-spawned to launcher-spawned | ~150 LoC launcher | ~80 LoC host (delete srv-spawn) | Medium — process-tree restructure |
| **B.2** | Tokio runtime in launcher + named-pipe IPC server (commands only, no events yet) | ~400 LoC launcher | 0 host | Low — additive |
| **B.3** | `Command` + `Event` types + pure reducer skeleton (no real state yet, just routes commands to host's existing IPC) | ~600 LoC launcher | ~50 LoC host (new IPC client) | Low — additive |
| **B.4** | Add launcher state mirror (read-only): launcher subscribes to host's existing state events, builds parallel state. Launch with diagnostic logging that compares launcher state to host state every transition; alert on drift. | ~500 LoC launcher | ~30 LoC host (emit state transitions) | Low — read-only, can't break anything |
| **B.5** | Migrate one HashMap at a time to launcher-authoritative (host queries launcher instead of its own map). Order: `window_instance_registry` first (smallest), then `window_meta`, then `browsers`, then `window_id_map`, then `window_pool` + `unpromoted_pool_labels`. Each migration is its own PR. | ~100 LoC per migration | ~50 LoC per migration | Medium — touches active state paths |
| **B.6** | Per-data-dir mutex single-instance (now trivial because launcher already has the named-pipe IPC) | ~80 LoC launcher | -50 LoC host (delete port-file check) | Low |
| **B.7** | Frontend: delete polling loop in `app-init.ts`; subscribe to event stream via CEF JS bridge | -50 LoC frontend | ~80 LoC host (JS bridge for events) | Low — frontend gets simpler |
| **B.8** | Phase B exit criteria: full lifecycle scenarios pass with launcher as sole authority. Delete host-side state stores. | 0 | -300 LoC host | Medium — atomic cut-over |

**Total**: ~9 PRs. Each ships independently. App remains usable throughout. Estimated calendar time: 4–6 weeks at one PR every 2–3 days.

### Pre-Phase B work (already done or in-flight)

- ✅ Job Object on launcher with CREATE_SUSPENDED race fix (PR #570 — in review)
- ✅ Spec lock: no heartbeats / no polling
- ✅ Strict-layering decision

### Phase B exit criteria

- All 13 state stores from `ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md` either deleted from host or remain only as effects-runner local state.
- Reducer is a pure function tested with property-based testing (`proptest`) over arbitrary command sequences.
- Frontend's `app-init.ts` polling loop (`refreshLabels(true, retriesLeft)`) is gone; rows update via event push.
- `agentmux.exe --diag` prints the canonical state and a process inventory; CI runs it after a synthetic close-all and asserts equality.
- Six invariants (spec §7) checked at every transition; violations panic.

---

## Concrete decisions to make before B.1 starts

### Decision 1: Tokio runtime in launcher?

The launcher is currently sync (~270 lines, no async runtime). Phase B needs async I/O for named pipes + multiple subscribers. Options:

**(a) Full Tokio runtime in launcher.** ~3 MB binary growth (Tokio + dependencies). Standard pattern.
**(b) Hand-rolled threads + sync I/O.** Smaller binary (~50 KB growth). More code (one thread per pipe accept + read).
**(c) Async-std or smol.** Lighter than Tokio (~1 MB). Less mature, fewer deps.

**Recommendation**: (a) Tokio. Standard, battle-tested, srv already uses it (so it's already in the build pipeline). Binary growth is acceptable for a redesign.

### Decision 2: Reducer state persistence?

Should the reducer's state survive launcher restarts? E.g., after a crash, should we restore the window list?

**(a) No persistence.** State lives in memory only. Crash → fresh start. Spec's default per §11.1.
**(b) Event log persisted to disk.** Replay on restart. Forensics-friendly but adds I/O overhead.
**(c) Snapshot every N events.** Compromise.

**Recommendation**: (a) for Phase B. Add (b) in Phase D as a forensics tool, gated behind `--diagnostics` flag. The user's workspaces/tabs already persist via srv's DB, so window restoration is a separate UX feature (deferred per spec non-goal).

### Decision 3: Frontend ↔ launcher path

The frontend can talk to the launcher via:

**(a) Through the host (CEF JS bridge).** Frontend → host's JS function → host forwards to launcher pipe. Two hops.
**(b) Direct named-pipe access from renderer.** Renderer process opens the pipe directly. One hop, but renderers are sandboxed and exposing pipe access is a security smell.
**(c) Through srv's existing WS.** Srv subscribes to launcher events, forwards to frontend. Two hops; reuses srv's WS infrastructure.

**Recommendation**: (a) Through host. Renderers stay sandboxed. Host is the trust boundary. Two hops adds ~ms latency, irrelevant for state events.

### Decision 4: Migration vs greenfield

**(a) Migrate incrementally** (the sub-PR sequence above). 9 PRs, weeks of work, lower risk.
**(b) Greenfield rewrite** in a feature branch. Single big merge. Higher risk but cleaner.

**Recommendation**: (a) Migrate. Bugs in this codebase are rarely from architecture; they're from edge cases. Migration keeps the codebase running while the new model gets exercised.

---

## Risks

1. **Process-tree restructure (B.1)** — moving srv from host-spawned to launcher-spawned changes who's responsible for srv's lifecycle. Need to verify on every supported platform: portable Windows, dev Windows, dev Linux/Mac (the spec is Windows-target but we don't want to break dev workflows). Phase B.1 should include a smoke test.
2. **Frontend event subscription latency** — going through CEF JS bridge adds ~1ms per event. For typical use (open/close window, tear-off) this is invisible. For high-frequency events (mouse position during drag) it could matter — but those events are NOT in the reducer (they're host-internal). Verify this assumption when wiring B.3.
3. **Multi-instance handling in mutex name** — must be deterministic from data_dir. Confirm hash strategy: `SHA-256(canonical_lowercase(data_dir))[..16]` as hex.
4. **Reducer panic on invariant violation** — spec §7 says panic + Job Object reaps. Need to verify Job Object actually does reap when launcher panics (it should — process exit closes handles regardless of exit reason).
5. **Backward compatibility during migration** — sub-PRs B.4 / B.5 / B.7 deliberately keep two state systems alive simultaneously. This means a bug in either could be confusing. Mitigation: aggressive logging + diagnostic flag that pretty-prints both states.
6. **Performance** — reducer is sync; effects runner is async on Tokio. Reducer hot path must stay fast (sub-millisecond) so it doesn't block IPC. Property-based tests should include throughput assertions.

---

## Phase C / D preview (post-B)

**Phase C** (warm pool consolidation) becomes near-trivial after Phase B — the warm pool is just another piece of state in the launcher reducer. The state-machine invariant (spec §7.1: pool windows can't be in `main_registry`) is enforced by the type system: `WarmPool` and `Map<WindowId, WindowState>` are different types.

**Phase D** (IPC contract hardening) is real work but additive: versioned event types, GetSnapshot resync protocol, echo-loop guard in frontend store. ~1–2 weeks.

**Estimated total time to fully realized state machine**: 6–8 weeks calendar time, assuming one PR every 2–3 days. PR #570 (Job Object) is the only piece not part of Phase B; it stands alone.

---

## Open questions to confirm before B.1

1. **Tokio in launcher: yes?**
2. **Mutex name format: SHA-256 first 16 hex of canonical-lowercase data_dir?** Or use launcher exe path?
3. **PR cadence: one B sub-PR at a time, or stack 2–3 in flight?** Stacking lets bots review smaller surfaces but complicates rebase.
4. **B.1 priority: ship first standalone, or roll into B.2?** B.1 is a process-tree restructure; B.2 is the IPC server; both touch launcher main.rs.
5. **Test strategy: unit tests + integration tests + manual?** Property-based tests via `proptest` for the reducer; `tempfile`-based integration tests for the IPC layer; manual for full lifecycle scenarios.

I have opinions on all five (see "Recommendation" lines above). Want to confirm or change?
