# B.5 migration architecture — what's possible, what's not, and how we finish

> **READER NOTE (2026-04-28, post-decision):** This doc analyzed the migration wall and proposed the "scaffolding model" as the Phase B end state. Direction was subsequently refined: scaffolding is **intermediate**, not permanent. The long-term destination is **multi-reducer** (host gets its own reducer in Phase F). See `multi-reducer-proposal-2026-04-28.md` for the accepted plan, and `phase-b-roadmap.md` for current sequencing. This doc remains valid as an analysis of WHY the standard ratchet hit a wall on `browsers` and pool maps.

**Status:** historical — analysis doc, partly superseded by
`multi-reducer-proposal-2026-04-28.md`. Not `superseded`: that status requires a
single successor that fully replaces this, and the READER NOTE above says the
direction was refined rather than replaced.
**Author:** AgentA.
**Date:** 2026-04-28, after PRs #579-#592.
**Companions:**
* `docs/specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` — the driving spec
* `docs/specs/ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md` — pre-migration inventory
* `docs/retro/migration-pattern.md` — the a→b→c→d→e ratchet
* `docs/retro/phase-b-roadmap.md` — sub-PR sequencing

---

## TL;DR

- **3 of 5 host maps fully migrated** through the standard a→b→c→d→e ratchet (window_instance_registry, window_id_map, window_meta — the last with a sync-cache refinement).
- **2 maps fundamentally cannot follow the same ratchet**: `browsers` (holds CEF FFI handles) and the pool maps (`window_pool` + `unpromoted_pool_labels` — coupled with CEF lifecycle scaffolding).
- The architectural insight: the original "host has no canonical state" framing was a useful simplification, but the practical end state has **two host residual classes**: synchronous caches that mirror launcher state, and pure scaffolding for FFI/lifecycle that never crosses process boundaries.
- Phase B can still complete, but the Phase B exit criteria document needs updating to acknowledge the residual scaffolding fields.

---

## What the original B.5 vision said

From the spec, §"Migration":

> Migrate the existing `browsers` / `window_meta` / `window_id_map` / `window_instance_registry` HashMaps into one `Map<WindowId, WindowState>` in the launcher reducer.

The implied final state: host's `AppState` shrinks to non-state infrastructure (CEF runtime objects, IPC handles, etc.). Launcher owns everything queryable.

This worked beautifully for 3 maps. For the other 2, it doesn't work, and the reason is structural — not a coding gap.

---

## What worked: 3 successful migrations

### `window_instance_registry` (PR #579-#584)

Sequential window numbering (`{ "main": 1, "window-abc": 2 }`). Pure data, no FFI coupling, no lifecycle scaffolding role. **Clean a→b→c→d→e**, ending with the host field deleted entirely.

### `window_id_map` (PR #585-#589)

Label → backend window UUID. Same shape as instance_registry — pure data, no FFI coupling. **Clean a→b→c→d→e**.

### `window_meta` (PR #590-#592, refined)

Per-window kind + parent_instance_id. Looked migrate-able (data is pure JSON), but step d ran into two same-process synchronous-lookup consumers:

1. `open_subwindow`'s parent-liveness check (must reflect "is this parent currently open" with no async lag).
2. Cascade-close enumeration during a parent's `on_before_close` (must include children opened just before).

The shadow-fed-from-launcher-events pattern lags by milliseconds. For these synchronous consumers, that lag was a correctness regression.

**Refinement**: kept `host.window_meta` as a synchronous local cache, written from a single canonical site (`on_after_created` from the popped `PendingWindowCreation` entry) and removed at `on_before_close`. The launcher's `state.windows` is still canonical for cross-process queries; `host.window_meta` covers same-process synchronous lookups.

This is the first migration where step e ≠ "delete the field." Step e became a doc-only acknowledgment.

---

## What hit a wall: 2 maps that can't follow the ratchet

### `browsers: HashMap<String, cef::Browser>`

Holds CEF `Browser` handles — FFI pointers to Chromium browser objects in the same process. Properties:

- **Cannot serialize over the IPC pipe.** A `Browser` is an opaque pointer to native CEF state. Sending it across processes is meaningless.
- **Must stay co-located with CEF.** Calls like `browser.host().close_browser()`, `browser.is_same()`, finding HWNDs from browser handles — all are in-process FFI calls. The launcher doesn't have a CEF runtime; it can't make these calls.
- **Lifetime owned by CEF.** The browser exists from `on_after_created` to `on_before_close`. Host inserts on creation, removes on close.

The KEY-SET role (label set queries) IS migrate-able — and is **already covered** by B.4's `state.windows: HashMap<String, WindowMirror>` + B.5's `shadow_window_meta`. Label-set queries can read from those instead of `browsers.keys()`.

What `browsers` is, post-migration: a thin host-side scaffolding map that wraps CEF FFI handles for in-process access. Not state. Not authoritative. Just a typed container for handles.

### Pool maps: `window_pool` + `unpromoted_pool_labels`

`window_pool: VecDeque<String>` is the queue of pre-painted ready-to-promote windows. `unpromoted_pool_labels: HashSet<String>` tracks "spawned but not yet ready." Both are intertwined with CEF lifecycle:

- `spawn_pool_window` writes both, kicks off CEF window creation.
- The renderer-ready handshake (an IPC from frontend) moves a label from `unpromoted_pool_labels` into `window_pool`.
- `promote_pool_window` pops from `window_pool`, removes from `unpromoted_pool_labels`, calls Win32 to reposition + show the window.
- `on_pool_window_destroyed` (from `on_before_close` for `window-pool-*` labels) cleans up.

Properties:

- **Pool decisions need synchronous local state.** "Should I refill?" answers depend on `window_pool.len() < POOL_TARGET_SIZE`. Race-tolerant async-mirrored state would produce incorrect refill decisions.
- **Renderer-ready handshake is host-internal.** The frontend sends an IPC to the host; host moves the label between maps. The launcher could be informed but it's a host-internal sequence.
- **List filtering needs synchronous reads.** `list_windows` filters out unpromoted pool labels — needs to know "right now" which labels are pool, not "as of the last launcher event."

The launcher already has `state.pool: HashSet<String>` (B.4) which mirrors the conceptual inventory. But the FINE-GRAINED states (pre-render-ready vs in-queue) and the synchronous decision points are inherently host-side.

What the pool maps are, post-Phase-B: synchronous lifecycle scaffolding. Launcher's `state.pool` is the cross-process projection. Host's two maps are the working set.

---

## The architectural insight

The original "host has no canonical state" framing held up under three migrations and broke on two. The pattern of breakage is consistent:

| Map | Role | Migrate-able? |
|---|---|---|
| `window_instance_registry` | Pure data, cross-process queryable | ✓ Fully |
| `window_id_map` | Pure data, cross-process queryable | ✓ Fully |
| `window_meta` | Pure data + synchronous lifecycle consumers | Partial (sync cache stays) |
| `browsers` | FFI handles | ✗ Host-only by nature |
| Pool maps | Synchronous lifecycle scaffolding | ✗ Host-only by nature |

The **refined model** for Phase B end state:

```
LAUNCHER (canonical, cross-process queryable):
  state.windows: HashMap<String, WindowMirror>     ← B.4
  state.pool: HashSet<String>                       ← B.4
  state.instance_registry: HashMap<String, u32>     ← B.5a
  state.next_instance_num: u32                      ← B.5a
  state.backend_window_ids: HashMap<String, String> ← B.5 (id_map)
  state.processes: HashMap<u32, ProcessRecord>      ← B.3
  state.lifecycle: LifecyclePhase                   ← B.3

HOST (synchronous caches — mirror of launcher state for hot-path lookups):
  shadow_instance_registry                           ← B.5b
  shadow_backend_window_ids                          ← B.5 (id_map step b)
  shadow_window_meta                                 ← B.5 (meta step b)
  window_meta                                        ← sync cache (refinement)

HOST (scaffolding — never authoritative, FFI/lifecycle only):
  browsers                                           ← CEF FFI handles
  window_pool, unpromoted_pool_labels                ← pool lifecycle
  window_pool_respawn_in_flight: AtomicBool          ← pool lifecycle
  pending_window_creations                           ← pre-create handoff
  sidecar_child, backend_pid, backend_endpoints      ← srv handles
  ipc_port, ipc_token, ...                           ← network handles
```

The taxonomy isn't "13 stores deleted" — it's "every host field falls into one of three classes, and only the **canonical** class moved." The other two classes are intrinsically host-internal, not bugs to migrate away.

---

## What "completely delivered" actually means

Refining the Phase B exit criteria from the original spec:

| Original criterion | Refined |
|---|---|
| "All 13 state stores from inventory either deleted from host or remain only as effects-runner local state." | All 13 state stores either: (1) deleted post-migration, (2) preserved as a synchronous host-side cache mirroring launcher state, or (3) preserved as host-internal scaffolding (FFI/lifecycle). Each surviving field has a comment classifying its role. |
| "Reducer is a pure function tested with property-based testing over arbitrary command sequences." | ✓ Already done (proptest in `agentmux-launcher/src/reducer.rs`). |
| "Frontend's `app-init.ts` polling loop is gone; rows update via event push." | B.7. Unchanged. |
| "`agentmux.exe --diag` prints canonical state." | B.8. Unchanged. |
| "Six invariants checked at every transition." | ✓ Already done (reducer panics on violations). |

---

## Concrete remaining work

### B.5 finish — small audit

1. Add a comment to `state.browsers` declaring it as scaffolding (not authoritative). Same for the pool maps.
2. Audit label-set reads of `browsers.keys()` and pool maps for cases where they're really asking "which labels exist in the launcher's view." Switch those to read from `state.windows` / `shadow_window_meta`. (~30-50 LoC, one PR.)
3. Update `phase-b-roadmap.md` (local) to reflect this refined understanding.

### B.6 — single-instance mutex (1 PR)

The named-pipe `first_pipe_instance(true)` already rejects a second launcher. Just need to:
- Surface a clear error when launch fails due to existing instance.
- Delete the old TCP port-file probe in `agentmux-srv` if any remains.
- Maybe show a dialog to the user: "AgentMux is already running (PID N)."

### B.7 — frontend cutover (2-3 PRs)

Replace `app-init.ts::refreshLabels(true, retriesLeft)` polling with subscription to launcher events via the CEF JS bridge. Frontend reducer fed by event stream.

### B.8 — Phase B exit (1-2 PRs)

- Property tests for the 13 invariants in `ANALYSIS_*.md`.
- `agentmux.exe --diag` Tool client that prints launcher state.
- CI synthetic close-all + assertion.
- Delete obsolete defensive code (e.g., the host-side fallbacks in `state.window_meta()` if no longer needed post-`task dev` reconciliation).

### Spec addendum

Update `SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` §"Migration" to reflect the three-class taxonomy. The original "single `Map<WindowId, WindowState>`" framing is partially impossible (WindowState can't carry FFI handles); needs to be split into a launcher-side `WindowState` for queryable data and a host-side `CefHandles<WindowId, Browser>` for FFI ownership.

---

## Implications for the golden vision

The "Redux/state-machine model" is intact. State changes flow through the reducer. Events flow out. Host/srv/frontend hold projections.

The refinement: **some host residuals are not state**. They're FFI-coupled scaffolding or synchronous lifecycle helpers. They don't violate the state-machine model — they're outside its scope (boundary infrastructure, not domain state).

A cleaner mental frame:

> The launcher reducer owns all **domain state** — everything that has meaning across processes and that any subscriber might want to query.
>
> Host owns **boundary scaffolding** — FFI handles, lifecycle helpers, synchronous local caches of domain state for hot-path reads.
>
> The state machine is intact for domain state. Boundary scaffolding never participated in the state-machine in the first place.

This is a useful refinement for any future B-equivalent migration: identify what's domain state vs boundary scaffolding before assuming everything is migrate-able. Boundary scaffolding doesn't need a ratchet; it just needs a comment explaining its role.

---

## Calendar implications

Original estimate (post-#582): "B.5 remaining: 3-5 sessions." Revised: **B.5 finish is one small audit PR, ~1 hour.** The "remaining maps" turned out to mean "audit existing read-paths" rather than "run the full ratchet on 2 more maps."

This accelerates the path to golden vision:

- B.5 finish: ~1 PR (~1 hour)
- B.6: 1 PR (~1 hour)
- B.7: 2-3 PRs (~1 session)
- B.8: 1-2 PRs (~1 session)

**Total to Phase B done: ~1.5-2 sessions.** Then Phase C/D become trivial; Phase E is parallel work.
