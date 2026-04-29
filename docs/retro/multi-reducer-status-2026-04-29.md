# Multi-reducer architecture — status & roadmap

**Status:** Active reference. Synthesis of current state across launcher / host / srv reducers, what's been built, and what's left.
**Date:** 2026-04-29.
**Author:** AgentA.

**Read first** if resuming reducer-architecture work. After this:
- `phase-b-roadmap.md` — Phase B sub-PR-level state.
- `multi-reducer-proposal-2026-04-28.md` — long-term design rationale.
- `next-steps-2026-04-29.md` — earlier ordering snapshot (some items already shipped).
- `specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` — driving spec.
- `specs/ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md` — pre-migration state inventory.
- `b5-migration-architecture-2026-04-28.md` — why some maps can't follow the standard ratchet.
- `wrr-design-2026-04-28.md` — Window Reality Reconciliation (B.9 family).
- `b9-3-lifecycle-analysis.md` + `b9-3-quit-thread-analysis.md` — close-cascade design rationale.

---

## 1. The vision

Three reducers, each canonical for its domain, communicating via versioned events:

```
                   ┌────────────────────────────────────────┐
                   │  LAUNCHER REDUCER  (privileged owner)  │
                   │  windows · pool · instance#            │
                   │  monitors · hwnd state · lifecycle     │
                   │  processes · pending_hwnds             │
                   └─────┬───────────────────────┬──────────┘
                Events │                       │ Events
                         ▼                       ▼
        ┌─────────────────────────┐   ┌──────────────────────────┐
        │  HOST REDUCER (Phase F) │   │  SRV REDUCER (Phase E)   │
        │  browsers · pool maps   │   │  workspaces · tabs       │
        │  pre-create queue      │   │  panes · layouts · agents │
        │  win32 lifecycle scaffolding│
        └─────┬─────────────────┬─┘   └──────────────────┬───────┘
        Events │             ▲ │ Cmds                   │ Events
                                ▼                       ▼
                   ┌─────────────────────────────────────────────┐
                   │  RENDERER REDUCERS (one per CEF browser)    │
                   │  InstancePanel · pane state · UI state      │
                   │  fed by typed events via CEF JS bridge      │
                   └─────────────────────────────────────────────┘
```

**Properties** (from `multi-reducer-proposal-2026-04-28.md`):
1. **Canonical per domain.** Each reducer owns its slice; others hold read-only projections.
2. **Events as the only cross-reducer contract.** No reducer mutates another's state directly.
3. **Sagas for cross-reducer transitions.** Reducer-emitted corrective events drive multi-process flows (e.g., `HostShouldQuit`, `CorrectiveWindowMove`).
4. **Versioned events + snapshot/replay.** Subscribers detect gaps, request resync.
5. **Single-writer per state field.** Structurally enforced — projections only have `apply_event`, no `mutate_field`.
6. **Drift detection** between projections and canonical, fired per-transition.
7. **Pure functional core.** Reducers never block, await, or do I/O — synchronous mutex-locked for sub-millisecond duration.
8. **Pure event-driven.** No timers, no heartbeats. State transitions and corrective actions fire on the same OS-event-driven dispatch tick.

---

## 2. Where we are now

### 2.1 Launcher reducer — ✅ canonical for 9 projections

Built across **B.3 → B.4 → B.5 → B.6 → B.7 → B.9 → B.9.3 → B.8** (PRs #570–#605).

| State projection | Authority | Phase |
|---|---|---|
| `lifecycle` (Starting → Running → Quitting → Dead) | Reducer | B.3 |
| `processes` (PID → kind / state / version) | Reducer | B.3 |
| `windows` (label → kind, parent, hwnd, visible, iconic, last_rect, last_foreground_at_ms, foregrounded_since_open) | Reducer | B.4 + B.5 + B.9 |
| `pool` (pre-warmed pool labels) | Reducer | B.4 follow-up |
| `instance_registry` (label → instance num) | Reducer | B.5 |
| `backend_window_ids` (label → backend window id) | Reducer | B.5 |
| `monitors` (Win32 monitor topology) | Reducer | B.9 |
| `pending_hwnds` (unlinked Win32 HWNDs awaiting reconciliation) | Reducer | B.9 |
| `event_version` / `next_client_id` (monotonic counters) | Reducer | B.3 |

**Plumbing:**
- Named-pipe IPC server, single-instance via `first_pipe_instance(true)` (B.6).
- Saga pattern proven: `CorrectiveWindowMove` (B.9.2), `HostShouldQuit` + `OrphanInstance` (B.9.3).
- WRR observation: `SetWinEventHook` + `WM_WINDOWPOSCHANGED` — zero polling, zero heartbeats.
- 60 reducer tests (44 unit + 5 proptest invariants + B.9 + B.8 close-all integration test).

### 2.2 Host — partial. Subscribes; still owns scaffolding maps

The host **subscribes** to launcher events via `apply_event_to_shadow` and forwards them to renderers via the CEF JS bridge. It does not yet have its own reducer.

Three classes of host fields (per `b5-migration-architecture-2026-04-28.md`):

| Class | Examples | Status |
|---|---|---|
| **Canonical (retired)** | `WindowInstanceRegistry`, `window_id_map` | ✅ Migrated. Host fields deleted. |
| **Sync caches** (mirror launcher state, refreshed on event) | `shadow_instance_registry`, `shadow_window_meta`, `shadow_backend_window_ids`, `window_meta` (synchronous-lookup local) | ✅ In place. Sole writers are `apply_event_to_shadow` + `on_after_created` (single-canonical-write site). |
| **Scaffolding** (FFI + lifecycle, can't migrate via standard ratchet) | `browsers: HashMap<String, cef::Browser>`, `window_pool: VecDeque<String>`, `unpromoted_pool_labels: HashSet<String>`, `pending_window_creations: VecDeque<PendingWindowCreation>`, `is_quitting: AtomicBool` | ❌ Still host-owned. **Retired by Phase F's host reducer.** |

The scaffolding class is what holds **CEF FFI handles** + **synchronous lifecycle gates** that can't survive the launcher's async event lag.

### 2.3 Srv — no reducer yet

Workspace / tab / pane / layout / agent state still flows through srv's HTTP/WS RPC layer. **Phase E** promotes srv to its own reducer. Currently:
- srv connects to launcher pipe as `ClientKind::Srv` for lifecycle facts (B.3+).
- All app data (workspaces, tabs, layouts, agents) bypasses the reducer model entirely — reads/writes go straight against srv's DB.

### 2.4 Frontend — typed events authoritative for InstancePanel

After **B.7.3 (#602–#604)** — typed launcher events delivered via the CEF JS bridge are the SOLE source for the InstancePanel atoms (`openWindowLabelsAtom`, `openWindowEntriesAtom`, `windowCountAtom`). The bespoke `window-instances-changed` channel is fully retired.

Pattern at the renderer:
- `frontend/util/launcher-events.ts` — installs `window.__agentmux_launcher_event` dispatcher into a SolidJS signal.
- `frontend/app/store/launcher-event-reducer.ts` — `createEffect` over the signal, maintains in-memory `knownEntries` map, mutates atoms via `recomputeAtoms()`.
- `frontend/app-init.ts::initInstanceTracking` — seeds the reducer from an init RPC snapshot for renderers that join mid-session; tombstones pre-seed close events to avoid ghost rows (codex P2 #604).

Atoms / blocks consume the typed stream. No bespoke proxy events from host. No polling.

---

## 3. Phase B status — exit-ready except B.8

| Sub-phase | Scope | Status |
|---|---|---|
| B.1 (#570/571/572) | srv as launcher-spawned sibling | ✅ |
| B.2 (#573) | named-pipe IPC server | ✅ |
| B.3 (#574) | pure reducer skeleton | ✅ |
| B.4 + B.4a + B.4b (#576/577/578) | window mirror, pool tracking, drift detection | ✅ |
| B.5 (#579–#594) | 3 maps migrated (instance_registry, window_id_map, window_meta sync-cache); 2 deferred to F (browsers, pool maps) | ✅ |
| B.6 (#595/598/599) | per-data-dir mutex single-instance | ✅ |
| B.7.1 (#596) | entries-bearing window-instances-changed | ✅ |
| B.7.2 (#597) | re-emit on BackendWindowId events | ✅ |
| B.9 (#600) | WRR observation + pure-reducer self-heal | ✅ |
| B.9.3 (#601) | pool-refill drain + cefsimple-pattern quit | ✅ |
| B.7.3.1 (#602) | host outbound JS bridge for typed events | ✅ |
| B.7.3.2 (#603) | typed events authoritative for atoms; bespoke demoted | ✅ |
| B.7.3.3 (#604) | retire bespoke channel + 4 sync emit sites | ✅ |
| **B.8** (open) | proptests + `--diag wrr` + dead-code sweep + close-all integration test | 🟡 **In review (PR #605, awaiting reagent + codex)** |

After B.8 merges → **Phase B done**.

---

## 4. Issues parked for later

### 4.1 browser_panes lock-held-across-SetWindowRgn deadlock

**Surfaced** during v0.33.505 smoke (B.8 build): teared a tab → opened a window → opened a browser pane → tried opening another window → host UI thread froze (process alive, windows unresponsive).

**Root cause** (in code since `d2cef570` — pre-Phase-B):
- `agentmux-cef/src/browser_panes.rs:387` holds `state.browsers.lock()` through a loop of `SetWindowRgn` calls.
- `SetWindowRgn` synchronously sends `WM_WINDOWPOSCHANGED` to the target HWND, which the UI thread must process.
- When the UI thread is itself trying to take `state.browsers.lock()` inside `on_after_created` (`client.rs:171`), and a worker thread is holding that lock during `set_pane_overlay_clip` → classic lock-while-SendMessage deadlock.

**Fix path A (small, targeted)**: in `set_pane_overlay_clip`, snapshot `(label, HWND)` pairs into a `Vec` while holding the lock, then drop the lock before any `SetWindowRgn`. ~30 LoC.

**Fix path B (Phase F)**: host reducer eliminates this bug class by design — `browsers` lives inside the reducer's State (single-threaded), and Win32 work happens in subscribers AFTER snapshot reads. No shared mutex for any thread to deadlock on.

**Decision (2026-04-29)**: defer to Phase F. The pattern's a structural smell that reducer migration cleans up; a one-off fix would just paper over it.

### 4.2 CEF Views position bug (next-steps §1.1)

WRR's `CorrectiveWindowMove` saga currently masks a real underlying bug — new top-level `CefWindow`s land at the Win32 hidden sentinel `(-31970, -31970)` instead of the requested offset. Investigation hooks documented; ~50 LoC fix likely via `WindowDelegate::get_initial_bounds`. Independent of reducer architecture; can ship anytime.

---

## 5. What's next — Phase D / E / F

### Phase D — durability / resync (~3 PRs)

After Phase B exits. Builds on the reducer's monotonic `event_version`.

| Sub-phase | Scope |
|---|---|
| **D.1** | `Command::GetSnapshot { since: u64 }` + `Event::Snapshot { state, events_since }` reply. Lets a reconnecting subscriber catch up without missing events. |
| **D.2** | Persisted event log (ring buffer at `<data-dir>/launcher-events.log`). Survives launcher crash for forensics. Not authoritative state. |
| **D.3** | Subscriber-resync protocol: `Register` → `GetSnapshot` → `EventList` → live stream. Idempotent `apply_event` becomes a typed contract (Rust `IdempotentApply` trait). |

D.1 also subsumes the `--diag wrr` snapshot story (today: stream observation via Tool client; D.1 makes it a clean RPC).

### Phase E — srv reducer (~5–8 PRs)

**First validation point for the multi-reducer pattern.**

`agentmux-srv` already has its own state (workspaces, tabs, blocks, layout, identity accounts). Promoting it to a Redux-style reducer is the first place we exercise the cross-reducer events + sagas pattern.

Why srv first (not host first):
- srv state is **purer** — no FFI handles, no Win32 sync constraints.
- srv migration is **lower-risk**, validates the pattern before applying it to host's harder constraints.
- srv reducer's events (e.g., `WorkspaceCreated`, `TabAdded`) cleanly cross to the launcher reducer for cross-process sync.

Expected deliverables:
- `agentmux-srv::reducer` with arms for `CreateWorkspace`, `AddTab`, `MoveTabBetweenWorkspaces`, `AddBlock`, etc.
- Events serialized via the launcher pipe; launcher reducer holds projections of srv state for cross-process queries.
- Renderer subscribes to srv events the same way it subscribes to launcher events (CEF JS bridge dispatcher + reducer effect).

### Phase F — host reducer + retire scaffolding (~5–8 PRs)

After E proves the multi-reducer pattern. Retrofit the host.

| Step | Goal |
|---|---|
| F.1 | New `agentmux-cef::reducer` with arms for `OpenBrowser`, `CloseBrowser`, `PoolAdd`, `PoolPromote`, `PoolDestroy`. |
| F.2 | `state.browsers` + pool maps move INTO the host reducer's canonical state. Host's `apply_event` from launcher's events drives FFI work as a SUBSCRIBER, not as direct mutation. |
| F.3 | Sagas: when launcher emits `Event::WindowClosed`, host's saga consumer fires `Command::CloseBrowser` into its OWN reducer, which drives the FFI close synchronously. WRR's CorrectiveWindowMove generalizes to a saga-consumer surface for any launcher-emitted corrective. |
| F.4 | Retire scaffolding model. The "3-class taxonomy" (canonical / sync cache / scaffolding) collapses to "host-reducer-state vs launcher-reducer-state vs sync-projection." |

**The browser_panes deadlock (§4.1) is fixed structurally here** — `set_pane_overlay_clip` becomes a saga consumer that snapshots `(label, HWND)` from the host reducer's state, drops out of the reducer, then does Win32 work without holding any cross-thread lock.

### Phase 7 — cross-platform parity (parallel track)

Linux + macOS IPC (Unix domain sockets), WRR-equivalent observation (X11/Wayland for Linux, NSWindow notifications for macOS). Independent of reducer architecture; can sequence with D/E/F as appetite allows.

---

## 6. Decisions log (don't relitigate)

(From `phase-b-roadmap.md` plus this session's additions.)

| Decision | Rationale |
|---|---|
| Tokio in launcher | srv already uses it |
| No reducer state persistence (memory only) | spec default; workspaces persist via srv DB. Phase D adds optional event log. |
| Frontend ↔ launcher via host JS bridge | renderers stay sandboxed; host is trust boundary |
| Migrate incrementally (a→b→c→d→e ratchet) | per-map sub-PR sequence keeps app running through migration |
| Pure event-driven, no timers / heartbeats | every transition has an OS event we can hook |
| `WindowInstanceRegistry` migrates first (B.5) | smallest map, simplest semantics |
| `window_meta` keeps sync cache (not full delete) | open_subwindow + cascade-close need synchronous local state |
| `browsers` + pool maps deferred to Phase F | FFI handles + sync lifecycle scaffolding can't migrate via standard ratchet |
| Multi-reducer is the long-term architecture | per `multi-reducer-proposal-2026-04-28.md`; Phase E/F validates the pattern incrementally |
| Single-instance lives in launcher pipe bind | OS-owned handle → no stale-state path |
| WRR uses event-driven Win32 hooks (B.9) | `SetWinEventHook` + `WM_WINDOWPOSCHANGED` deliver every needed transition synchronously |
| Pool refill suppressed during host drain via `is_quitting` flag (B.9.3) | CEF's `quit_message_loop` is QuitWhenIdle — without suppression, pool windows keep state non-empty forever |
| Cross-thread "deliver work to UI thread" uses `cef::post_task` (or Win32 `PostMessage(WM_CLOSE)` as bypass) — NOT direct calls | direct calls from worker thread are CEF UB; PostThreadMessage(WM_QUIT) is ignored by CEF's pump |
| browser_panes deadlock fix waits for Phase F | structural fix via host reducer eliminates the pattern by design; one-off lock-snapshot-and-drop would paper over the smell |

---

## 7. Sequencing recommendation

1. **B.8 merges** — Phase B exits.
2. **CEF Views position bug (next-steps §1.1)** — small, removes a known masking case for B.9. 1 PR.
3. **Phase D** — durability / resync. Independently useful even if multi-reducer is deferred.
4. **Phase E — srv reducer.** First multi-reducer validation. The pattern that works here will work for host.
5. **Phase F — host reducer.** Retires scaffolding maps. Fixes the browser_panes deadlock by design.
6. **Phase 7** — cross-platform parity, parallel track at any point.

Total remaining LoC for Phase D + E + F is significant — probably 15–25 PRs across the three phases — but each individually shippable, each with property tests, each one-step at a time.
