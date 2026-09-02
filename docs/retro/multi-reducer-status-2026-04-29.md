# Multi-reducer architecture — status & roadmap

**Status:** Active reference. Synthesis of current state across launcher / host / srv reducers, what's been built, and what's left.
**Date:** 2026-04-29 (post-#608 / Phase D done).
**Author:** AgentA.

**Read first** if resuming reducer-architecture work. After this:
- `phase-b-roadmap.md` — Phase B sub-PR-level state.
- `multi-reducer-proposal-2026-04-28.md` — long-term design rationale.
- `next-steps-2026-04-29.md` — earlier ordering snapshot (some items already shipped).
- `docs/specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` — driving spec.
- `docs/specs/ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md` — pre-migration state inventory.
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

## 3. Phase B + D status — both done

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
| **B.8** (#605) | proptests + `--diag wrr` + dead-code sweep + close-all integration test + IPC server broadcast bus + diag Goodbye | ✅ |

**Phase B complete.** Launcher reducer canonical for 9 projections; host subscribes via `apply_event_to_shadow`; renderer subscribes via CEF JS bridge; bespoke `window-instances-changed` channel retired.

### Phase D — durability / resync (done)

| Sub-phase | Scope | Status |
|---|---|---|
| **D.1** (#607) | `Command::GetSnapshot` + `Event::Snapshot` (one-shot, no replay). Diag prints state-now via the new RPC. | ✅ |
| **D.2 + D.3** (#608) | Event log: in-memory ring (4096 events) for replay, plus `<data-dir>/launcher-events.log` for crash forensics. `Command::GetEvents { since: u64 }` + `Event::EventList { events, version }`. Server intercepts before reducer (log query is I/O); reducer's GetEvents arm is unreachable no-op for match exhaustiveness. | ✅ |

**Phase D complete.** Subscribers can now do full resync: `Register` → `GetSnapshot` → `GetEvents { since }` → live broadcast stream. Disk persistence survives crashes for forensics.

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

### 4.2 CEF Views position bug (next-steps §1.1) — deferred to issue #606

WRR's `CorrectiveWindowMove` saga currently masks a real underlying bug — new top-level `CefWindow`s briefly appear at the Win32 hidden sentinel `(-31970, -31970)` before being snapped onto the primary monitor. End state is correct; the visible glitch is the only symptom.

Tried 2026-04-29: naive `WindowDelegate::initial_bounds` returning `Rect{0,0,0,0}` for the "no explicit bounds" case broke rendering (zero-area black window). A real fix needs a correct sentinel for "use Views' default placement" + multi-DPI / multi-monitor verification — more careful work than a 30-min drive-by. Branch abandoned.

Filed as **issue #606** for future pickup. Cosmetic-only; not blocking any phase.

---

## 5. What's next — Phase E / F

### Phase E — srv reducer + saga coordinator (~8 PRs)

**First validation point for the multi-reducer pattern.** Now in progress.

| Sub-phase | Scope | Status |
|---|---|---|
| **E.1a** (#609) | Saga coordinator framework: `Saga` trait, `SagaCoordinator` task, `SagaStarted/Completed/Failed` events. Plus durable event-log fsync + 2 codex P2 carryovers from #608 | ✅ |
| **E.1b** (#610) | srv reducer skeleton + new srv pipe + broadcast bus + event log. 4 review rounds: codex P1 EventList unicast + P1 reconnect-after-disconnect (synthetic Goodbye) + P2 ErrorCode alignment | ✅ |
| **E.2** (#611) | Workspace lifecycle arms (CreateWorkspace / DeleteWorkspace) + SQLite bootstrap into reducer's session-only projection. Persist subscriber descoped after codex flagged systemic bus-lag / HWM issues — deferred to E.2c. 5 review rounds: codex P1 cascade-delete, codex P1 HWM-freeze-on-lag (resolved by dropping subscriber), 4 reagent P2 stale-doc rounds | ✅ |
| **E.2b** (#612) | Tab + ActiveTab arms (CreateTab / DeleteTab / SetActiveTab) + DeleteWorkspace cascade. Bootstrap loads tabs alongside workspaces (via reverse-lookup against both `tabids` and `pinnedtabids`). 2 review rounds: codex P1 pinned-tab orphaning fixed in second push | ✅ |
| **E.3** (#613) | Block lifecycle arms (CreateBlock / DeleteBlock) + two-level workspace→tabs→blocks cascade. Bootstrap loads blocks. Block content (view, meta, runtimeopts) deferred to follow-up. **Skipped ahead of E.2c** because E.2c needs design input on persist-subscriber bus-lag handling. 2 review rounds: reagent+codex both flagged the same orphan-tab block-cascade P1, fixed by reverse-lookup against state.tabs | ✅ |
| **E.2c.1** (#614) | Persist subscriber plumbing — new `persist_subscriber.rs` module with idempotent apply handlers for Workspace/Tab/Block/ActiveTab events. On Lagged: warn-only (resync deferred to E.2c.2 where RPC migration makes it non-destructive). 9 unit tests against in-memory wstore. Subscriber is dead-code in production until RPC migration lands. 1 review round | ✅ |
| **E.2c.2** (#615) | Workspace RPC migration. CreateWorkspace forward+compensate (reducer → SQLite → on failure dispatch DeleteWorkspace to roll back). DeleteWorkspace SQLite-first (wcore → reducer dispatch). Resync is insert/update only (delete-phase removed — window flows still create workspaces outside the reducer). Fixed two carryovers from #614 (`wcore::delete_workspace` cascade through `pinnedtabids`; subscriber provisions LayoutState alongside Tab). 4 review rounds: 4 codex P1s + reagent P2 doc | ✅ |
| **E.2c.3** (#616) | Tab RPC migration (CreateTab + SetActiveTab; pinned/CloseTab/ReorderTab deferred to E.2c.3b). Plus codex P2 #615 carryover. 4 review rounds, 8 P1+P2 fixed in flight. PR merged at 09:54:56Z while a final round-3 codex P1+P2 was being addressed (auto-activate-after-pinned regression on `active_tab_id=None`; pinned-detection masked StoreError) — those fixes carry forward to E.2c.3b | ✅ |
| **E.2c.3b** (#617) | CloseTab (SQLite-first via wcore::delete_tab) + ReorderTab (new `Command::ReorderTab` + `Event::TabReordered` + apply_tab_reordered handles both `tabids` and legacy `pinnedtabids`). **Pinned-tab handling REMOVED** — pinning was a Waveterm feature dropped from AgentMux; CreateTab `args[3]` ignored, SetActiveTab pinned-detection branch removed. Bootstrap defensively merges legacy `pinnedtabids` into reducer's `tab_ids`. 2 review rounds: codex P2 (legacy-pinned reorder persistence) + P3 (u32 clamp) | ✅ |
| **E.2c.4 + E.2c.5a** (#618) | Block RPC migration (CreateBlock + DeleteBlock through reducer; `Command::CreateBlock` / `Event::BlockCreated` carry meta map) + Rust host bridge to srv pipe (new `agentmux-cef::srv_event_bridge` + `srv_ipc` modules; forward events to renderer via `window.__agentmux_srv_event`). 1 review round | ✅ |
| **E.2c.5b** | TypeScript renderer dispatcher — install `window.__agentmux_srv_event` handler, route events into atom domains | |
| **E.5.1+2** (#619) | Saga foundation: window state (`WindowRecord` + `state.windows` + 3 atomic commands + 3 events + bootstrap + subscriber + reducer arms with 9 tests) + design docs (saga design spec + 4-PR execution plan + tear-off bug analysis). DeleteWorkspace cascades to drop window mappings. Client.windowids stay in sync via subscriber. 3 review rounds: codex P1 client.windowids sync, codex P1 cascade-windows-on-delete-workspace | ✅ |
| **E.5.3+4** (#620) | Atomic single-step commands + RPC migration. 6 new commands (`ReorderTabsBulk`, `RenameWorkspace`, `RenameTab`, `Update{Workspace,Tab,Block}Meta`) + 6 events. 4 RPC handlers migrated (`UpdateWorkspace` → `RenameWorkspace`, `UpdateTabIds` → `ReorderTabsBulk`, `UpdateTabName` → `RenameTab`, `UpdateObjectMeta` → decompose by otype). New `merge_meta_patch` helper preserves `merge_meta`'s `section:*` clear semantics. `apply_tabs_reordered_bulk` drains legacy `Workspace.pinnedtabids` to prevent UI double-counting. `handle_delete_workspace` emits `SrvWindowClosed` for cascaded windows so the subscriber can prune Client.windowids. 3 review rounds: reagent P2 doc, codex P1 (pinnedtabids drain) + P2 (section-clear semantics) | ✅ |
| **E.5.5+6** (#621) | Tear-off + restore sagas — **fixes the smoke regression** (tear-off → "+" tab). New saga coordinator in `agentmux-srv/src/sagas/` (Path A — srv-side; see `saga-coordinator-location-analysis-2026-04-30.md` for the full reasoning). Three sagas: TearOffTab (CreateWorkspace + MoveTab), TearOffBlock (CreateWorkspace + CreateTab + MoveBlock; layout setup + auto-close stay wcore-direct as E.4 territory), RestoreTornOffTab (MoveTab back + conditional DeleteWorkspaceCascade). Saga `dispatch` / `compensate` / `state_lock` helpers, 5s timeout wrapper, lifecycle event emission. RPC handlers migrated (TearOffTab/TearOffBlock/RestoreTornOffTab). Plus `MoveTabToWorkspace` migrated through the reducer to keep state.tabs in sync. New wire types: `Command::MoveTab`, `Command::MoveBlock`, `Event::TabMoved` (with `new_src_active_tab_id` + `new_dst_active_tab_id`), `Event::BlockMoved`. Reducer's `handle_move_tab` is migration-tolerant (lazy-imports unknown tabs, drops workspace_id check) since wcore-direct paths (e.g. PromoteBlockToTab) leave reducer state stale. SQLite is the source of truth for saga + RPC pre-checks. 4 review rounds: reagent P1 (missing version bump), codex P1+P2 (MoveTabToWorkspace + dst active tab), codex P1 round-2 (SQLite-truth pre-checks + tolerant reducer) | ✅ |
| **E.5.7+8+9** (#622) | **Phase E.5 closeout.** Final wcore-direct → reducer migrations: `MoveBlockToTab` (single-step Command::MoveBlock), `PromoteBlockToTab` (new saga: CreateTab + MoveBlock; layout/SetActiveTab/auto-close stay wcore-direct in handler), `CreateWindow` (multi-step inline: optional CreateWorkspace + CreateTab + CreateWindow with compensation), `CloseWindow` (CloseWindowInternal + conditional DeleteWorkspaceCascade), `SwitchWorkspace` (single-step). `handle_create_tab` auto-generates `tabN` when name is empty, mirroring wcore behaviour (covers both new CreateWindow path + TearOffBlock saga). MoveBlockToTab same-tab no-op short-circuit preserves prior contract. CloseWindow gains the missing Event::Error check. Cleanup audit: only SQLite-first delete patterns remain (DeleteBlock / DeleteWorkspace / DeleteTab — intentional from earlier sub-phases). 2 review rounds: reagent P1 (CloseWindow missing error check), codex P2+P2 (default tabN naming + same-tab MoveBlockToTab no-op) | ✅ |
| **E.4** | Layout state arms | |
| **E.5** | Drag/tear-off sagas — first concrete consumers of E.1a's coordinator | |
| **E.6** | Renderer: per-source version tracking + saga-buffer; bespoke WaveObjUpdate retired | |
| **E.7 — exit** | Property tests + cross-reducer integration tests + `--diag srv` / `--diag sagas` | |

`agentmux-srv` already has its own state (workspaces, tabs, blocks, layout, identity accounts). Promoting it to a Redux-style reducer is the first place we exercise the cross-reducer events + sagas pattern.

Why srv first (not host first):
- srv state is **purer** — no FFI handles, no Win32 sync constraints.
- srv migration is **lower-risk**, validates the pattern before applying it to host's harder constraints.
- srv reducer's events (e.g., `WorkspaceCreated`, `TabAdded`) cleanly cross to the launcher reducer for cross-process sync.

Expected deliverables:
- `agentmux-srv::reducer` with arms for `CreateWorkspace`, `AddTab`, `MoveTabBetweenWorkspaces`, `AddBlock`, etc.
- Events serialized via the launcher pipe; launcher reducer holds projections of srv state for cross-process queries.
- Renderer subscribes to srv events the same way it subscribes to launcher events (CEF JS bridge dispatcher + reducer effect).

#### Phase E carryovers (codex P2s from PR #608, integrate when touching the related code)

- **`agentmux-launcher/src/ipc/server.rs:438`** — `Event::Registered` is appended to the in-memory replay ring BEFORE `patch_launcher_identity` runs, so stored events keep reducer sentinels (`launcher_pid: 0`, empty `launcher_version`). When `GetEvents` returns those entries via the `EventList`, identity-patching only touches the top-level event, not nested replay events. Replay data is inconsistent with the live stream. Fix: patch identity BEFORE the `event_log.append` call (move `patch_launcher_identity` into the broadcast loop above the append).
- **`agentmux-launcher/src/event_log.rs:121`** — `replay_truncated` computes `since + 1` directly. `Command::GetEvents { since: u64::MAX }` overflows: debug builds panic, release builds wrap. Wire input is externally reachable; should use `since.checked_add(1)` or `since.saturating_add(1)`.

Both flagged as P2 (not blocking) but should fold into Phase E's first PR that touches `server.rs` / `event_log.rs` to avoid leaving them as a separate cleanup item.

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
| Phase D bundled (D.2 + D.3 in one PR #608) | parts interlock: D.3's `GetEvents` reads from D.2's event log. Splitting would create intermediate states where the snapshot RPC exists but no client knows how to consume it. ~500 LoC was tractable for review. |
| Phase E + F stay multi-PR | est. 2500–5000 LoC each; reagent + codex review quality drops at scale (PR #604 reagent flagged 16 regressions on 500 LoC; mega-PRs would skim or timeout). Bisection breaks. Multi-reducer specifically benefits from incremental validation (srv vs host pattern). Use a "phase exit" PR per phase to bundle the smaller cleanups (proptests + dead code) — same pattern as B.8 / D.2+D.3. |

---

## 7. Sequencing recommendation

1. ~~**B.8 merges** — Phase B exits.~~ ✅ done
2. ~~**Phase D** — durability / resync.~~ ✅ done
3. **Phase E — srv reducer (next).** First multi-reducer validation. Pattern that works here will work for host. Fold the two codex P2 carryovers (server.rs identity patching + event_log.rs overflow) into the first Phase E PR that touches those files.
4. **Phase F — host reducer.** Retires scaffolding maps. Structurally fixes the browser_panes deadlock (§4.1).
5. **Phase 7** — cross-platform parity, parallel track at any point.
6. **Issue #606** (CEF Views position bug) — cosmetic; pick up anytime.

Phase E + F is roughly 10–16 PRs together — each individually shippable, each with property tests, each one-step at a time.
