# SPEC: Internal-event → frontend WaveObjUpdate bridge

**Date:** 2026-05-14
**Author:** AgentX
**Status:** Draft
**Parent architecture:** [`MASTER_REDUCER_STACK_STATUS_2026-05-05.md`](./MASTER_REDUCER_STACK_STATUS_2026-05-05.md) — this work adds a peer subscriber to `srv_events_tx` alongside `persist_subscriber`, codified there as §8.15 (the srv-event subscriber idempotency contract — companion to §8.14 for launcher events).
**Discussion thread:** [#707 — Reducer-stack architecture: long-term tracking thread](https://github.com/agentmuxai/agentmux/discussions/707)
**Replaces / supersedes:** the per-handler "Option A" path proposed in `SPEC_REACTIVE_WORKSPACE_SYNC_2026-05-14.md`
**Naming note:** uses existing `WaveObj*` / `WaveStore` / `wos.ts` names; the Wave→Mux rename was deferred mid-implementation (issue #851).
**Motivating bug:** workspace renames don't propagate to the OS title or InstancePanel — `UpdateWorkspace` returned `success_empty()` and the response-broadcast loop had nothing to send. See §2 for the deeper class.

## How this fits the reducer architecture

The bridge is a **second consumer** of the existing srv reducer event bus (`srv_events_tx`), parallel to the persist subscriber:

```
reducer → emits Event → srv_events_tx ─┬─► persist subscriber → SQLite (existing, Phase E.2c.1–5a)
                                       ├─► disk writer → JSONL log (existing, Phase D)
                                       └─► wave_obj_bridge → WaveObjUpdate → broadcast (NEW, this PR)
```

This adds nothing to the reducer story — the reducer already emits the right events. The bridge just routes them to a new downstream the frontend cares about. Both subscribers must satisfy the §8.15 idempotency contract; this bridge satisfies it by construction (each broadcast is a fresh `wstore.get<T>()`).

**Phase E migration status caveat:** commands that haven't yet been migrated through the srv reducer (notably `UpdateObjectMeta` for `OTYPE_WINDOW`, `OTYPE_LAYOUT`, `OTYPE_CLIENT`) bypass `srv_events_tx` entirely — they go straight to `wcore` and never emit a reducer event. The bridge can't see them. PR #852 includes a temporary inline-update workaround for `OTYPE_WINDOW` in `service.rs:308`'s `_ => other` fallback arm; the proper fix is the Phase E.5.x migration of `UpdateWindowMeta` to go through the reducer (tracked separately).

---

## 1. Goal

Make the **internal sidecar event bus (`srv_events_tx`)** the single source of truth for "frontend should see this change", so that:

1. Every state mutation that appears on the bus reaches the frontend WOS cache, with no per-RPC-handler plumbing required.
2. Adding a new mutation RPC requires *only* writing the reducer event and the SQLite apply path — frontend reactivity is automatic.
3. The class of bug "I added a new RPC but forgot to attach `WaveObjUpdate` to the response" becomes structurally impossible.

This replaces the brittle current convention ("each mutation RPC must remember to call `success_with_updates(...)`") with an architectural guarantee.

---

## 2. Why this is more robust than per-handler fixes

| Property | Option A (per-handler) | Option B (bridge) |
|---|---|---|
| Fixes the reported workspace-rename bug | ✓ | ✓ |
| Prevents the next handler from reintroducing the bug | ✗ — relies on author remembering the convention | ✓ — convention is enforced by infrastructure |
| Symmetric with persist-subscriber | ✗ — persist subscribes to events; broadcast layer goes through a different path | ✓ — both subscribe to the same bus |
| New event types automatically wired up | ✗ — each handler updated separately | ✓ — bridge converts all known event types |
| Code review surface for new mutation RPCs | "Did you remember to add `success_with_updates`?" — easily missed | "Did you emit the event?" — harder to forget, since the event drives reducer state too |
| Multi-window correctness today | Already broadcasts to all clients via response loop | Same — preserved |
| Failure mode if bridge breaks | n/a | All mutations stop reaching frontend → loud, immediately visible (vs. current silent per-handler omissions) |

The most important property is the second row: **Option A leaves every future mutation RPC one missed call away from a silent reactivity bug**. Option B closes that hole at the layer where it can be closed once.

The robustness comes from collapsing two parallel notification paths (internal event bus + response broadcast) into one (internal event bus, with broadcast as a downstream consumer).

---

## 3. Current architecture

```
        ┌──────────────────────┐
        │  RPC handler         │  e.g. UpdateWorkspace
        │  (service.rs)        │
        └─┬──────────────────┬─┘
          │                  │
          │ dispatch         │ build response
          │ to reducer       │ with WaveObjUpdate(s)
          ▼                  ▼
   ┌────────────┐       ┌────────────────────┐
   │ reducer    │       │ WebReturnType      │
   │ emits      │       │ ::success_with_    │
   │ Event      │       │   updates([…])     │
   └─────┬──────┘       └──────────┬─────────┘
         │ publish_events                  │
         ▼                                 ▼
   ┌──────────────────┐             ┌─────────────────────┐
   │ srv_events_tx    │             │ response broadcast  │
   │ (internal)       │             │ loop (lines 39-52)  │
   └──┬────────────┬──┘             └──────────┬──────────┘
      │            │                           │
      ▼            ▼                           ▼
   persist     disk-writer            ALL WS clients receive
   subscriber  (JSONL log)            "waveobj:update"
   (SQLite)
```

**The defect:** the two paths (`srv_events_tx` and "response broadcast") are independent. The reducer always emits an event; the handler may or may not attach the update to its response. When the handler forgets, the frontend never hears about a real state change.

Also: the response-broadcast loop only knows about updates the handler explicitly attached. It doesn't see the internal event bus.

---

## 4. Proposed architecture

```
        ┌──────────────────────┐
        │  RPC handler         │  e.g. UpdateWorkspace
        │  (service.rs)        │
        └─┬──────────────────┬─┘
          │ dispatch         │ return success_empty
          │ to reducer       │   (no per-handler updates needed)
          ▼                  ▼
   ┌────────────┐       (response carries no updates)
   │ reducer    │
   │ emits      │
   │ Event      │
   └─────┬──────┘
         │ publish_events
         ▼
   ┌──────────────────┐
   │ srv_events_tx    │
   │ (internal)       │
   └──┬─────────┬──────────┬────────────┐
      │         │          │            │
      ▼         ▼          ▼            ▼
   persist  disk-writer  WaveObj-       (other future
   subscriber (JSONL)    Update         consumers)
   (SQLite)              Broadcast
                         Bridge  ◀── NEW
                            │
                            ▼
                  Per-event WaveObjUpdate(s)
                  → broadcast channel
                            │
                            ▼
                  ALL WS clients receive
                  "waveobj:update"
```

**Key changes:**

1. **NEW: WaveObjUpdate Broadcast Bridge** — a third subscriber to `srv_events_tx`. For each event, it:
   - Looks up the event-type → WaveObjUpdate mapping (§5)
   - Fetches the affected `WaveObj`(s) from the store at their post-event state
   - Builds `WaveObjUpdate` records and pushes them onto the same broadcast channel the response loop uses
2. **Response loop preserved** for backward compatibility (handlers that already attach updates keep working). Phase 3 (§7) optionally removes those calls once the bridge fully covers them.
3. **Frontend dedup** added (§6) so handlers that broadcast both via response AND bridge don't trigger redundant signal updates.

---

## 5. Event-type → WaveObjUpdate mapping

The reducer's `Event` enum (in `agentmux-srv/src/reducer/`) is the source of truth. Per §11.3, **a single user action can emit multiple ordered events, and a single event can imply changes to dependent WaveObjs** (e.g. `TabCreated` mutates the parent Workspace's `tab_ids`). The mapping must handle both.

### 5.1 Mapping table (Phase 1 + 2 coverage)

Event names below come from `agentmux-srv/src/reducer/{workspace,tab,block,layout,window}.rs`. Phase 1 ships the highlighted rows (workspace events — fixes the user-reported bug). Phase 2 fills the rest.

| Reducer event | WaveObjs to fetch + broadcast | `updatetype` per obj | Phase |
|---|---|---|---|
| **`WorkspaceRenamed { workspace_id, … }`** | workspace | `update` | **1** |
| `WorkspaceCreated { workspace_id, … }` | workspace | `update` | 1 |
| `WorkspaceDeleted { workspace_id }` | workspace | `delete` (oid only) | 1 |
| `WorkspaceMetaUpdated { workspace_id, … }` | workspace | `update` | 1 |
| `TabCreated { workspace_id, tab_id, … }` | tab + **workspace** ← dependent | `update` + `update` | 2 |
| `TabDeleted { workspace_id, tab_id, … }` | tab + **workspace** ← dependent | `delete` + `update` | 2 |
| `TabRenamed { tab_id, name, … }` | tab | `update` | 2 |
| `ActiveTabChanged { workspace_id, tab_id }` | workspace | `update` | 2 |
| `BlockCreated { tab_id, block_id, … }` | block + **tab** ← dependent | `update` + `update` | 2 |
| `BlockDeleted { tab_id, block_id, … }` | block + **tab** ← dependent | `delete` + `update` | 2 |
| `BlockUpdated { block_id, … }` | block | `update` | 2 |
| `LayoutUpdated { layout_id, … }` | layout | `update` | 2 |
| `WindowOpened { window_id, … }` | window | `update` | 2 |
| `WindowClosed { window_id }` | window | `delete` | 2 |
| `WindowMetaUpdated { window_id, … }` | window | `update` | 2 |
| `ClientUpdated { client_id, … }` (if any) | client | `update` | 2 |

> **Why `update` for both create and update:** The frontend's `updateWaveObject` (`wos.ts:259-279`) treats every `updatetype` other than `"delete"` identically — whether the local cache is empty (treat-as-create) or populated (treat-as-update). Sending `"update"` uniformly simplifies bridge logic and matches what the existing response-broadcast loop already emits for create-shaped events. The `"create"` discriminator is unused in this codebase (per ReAgent P2 on PR #852).

> **Authoritative mapping:** the actual `Event` enum is the source of truth. Phase 2's first task is `grep -nE "Event::" agentmux-srv/src/reducer/` and crossing the result against this table. Any variant in the enum but not in the table is either a (a) non-WaveObj event (saga, OS, window-pool) → `_ => vec![]` arm, or (b) a missing entry to add.

### 5.2 Dependent-object pattern

The "dependent" rows (TabCreated, TabDeleted, BlockCreated, BlockDeleted) reflect the §11.3 finding: when a child object is created/deleted, the parent's collection field (`workspace.tab_ids`, `tab.block_ids`) was also mutated by the reducer in the same dispatch. The bridge must therefore fetch the parent fresh and broadcast it too.

Why this works without double-counting: the parent's `version` is bumped exactly once in the reducer dispatch. The bridge's broadcast for the parent reflects that single bump. If a follow-up event (e.g. `ActiveTabChanged` after `TabCreated`) bumps the parent's version again, the second broadcast carries the higher version — frontend dedup (§6.1) absorbs no-op repeats automatically.

### 5.3 Mapping module

Implement as a single file, `agentmux-srv/src/server/wave_obj_bridge.rs`. **Each per-event dispatch is `async fn`** so it can offload the SQLite read into `tokio::task::spawn_blocking` — `WaveStore::get<T>()` acquires `std::sync::Mutex<Connection>`, and even though the hold is brief in steady state, a long reducer transaction would block the tokio worker thread otherwise (per ReAgent P1 on PR #852, §11.2):

```rust
async fn dispatch_event(event: Event, wstore: Arc<WaveStore>, event_bus: Arc<EventBus>) {
    match event {
        Event::WorkspaceRenamed { workspace_id, .. }
        | Event::WorkspaceMetaUpdated { workspace_id, .. }
        | Event::WorkspaceCreated { workspace_id, .. } => {
            let id = workspace_id.clone();
            let store = Arc::clone(&wstore);
            // Offload SQLite read to the blocking thread pool.
            let result = tokio::task::spawn_blocking(move || store.get::<Workspace>(&id)).await;
            match result {
                Ok(Ok(Some(ws))) => {
                    emit(&event_bus, "update", OTYPE_WORKSPACE, &workspace_id,
                         Some(wave_obj_to_value(&ws)));
                }
                Ok(Ok(None)) => log_warn_missing(&workspace_id),
                Ok(Err(e)) => log_err_fetch(&workspace_id, &e),
                Err(join_err) => log_err_panic(&workspace_id, &join_err),
            }
        }

        Event::WorkspaceDeleted { workspace_id, .. } => {
            // No fetch needed — frontend just needs the oid + delete tag.
            emit(&event_bus, "delete", OTYPE_WORKSPACE, &workspace_id, None);
        }

        // Compound: tab created + parent workspace.tab_ids was also
        // mutated. Phase 2 should fetch both and emit two broadcasts.
        // Event::TabCreated { workspace_id, tab_id, .. } => { … }

        // Saga events, OS facts, etc. — not WaveObj changes.
        _ => {}
    }
}
```

The actual implementation in `agentmux-srv/src/server/wave_obj_bridge.rs` includes the full error logging + the per-event panic isolation that wraps each `dispatch_event` call in `tokio::spawn(...).await` so a panic in one event can't kill the bridge loop. The pseudocode above shows the conceptual structure.

**Phase 2 implementors:** every new event-variant arm in `dispatch_event` MUST follow the `spawn_blocking` pattern for any `wstore.get<T>()` call, otherwise the tokio worker thread will block under contention. Don't write a synchronous `dispatch_event(&Event, &WaveStore)` shape — it's a footgun.

### 5.4 Events that should NOT broadcast

Some `srv_events_tx` events are internal-only and don't represent WaveObj changes:

- Saga lifecycle (`SagaStepStarted`, `SagaStepCompleted`)
- Launcher-domain facts (window-pool reuse, OS focus events that the host already mirrors via a different channel)
- Disk-writer ack events

For these, the mapping function returns an empty vec — the bridge silently skips them. The catch-all `_ => vec![]` arm in §5.3 makes this the default.

---

## 6. Dedup strategy

Risk: a handler that uses `success_with_updates(...)` AND triggers a bridge-broadcasted event would fire two `waveobj:update` events for the same change, with the same content but different broadcast timing.

Two layers of defense:

### 6.1 Frontend version-based dedup

Every `WaveObj` has a `version: i64` field that the reducer increments on each mutation (confirmed universal across all 6 types — see §11.4). When a `waveobj:update` arrives, the frontend's `wos.ts` `updateWaveObject()` checks:

```ts
function updateWaveObject(update: WaveObjUpdate) {
    const wov = getOrCreateWov(update.oid);
    const currentVersion = wov.data?.version ?? 0;
    const newVersion = update.obj?.version ?? 0;
    if (newVersion > 0 && newVersion <= currentVersion) {
        return;  // already-applied or stale; skip
    }
    wov.setData(update.obj);
}
```

The `newVersion > 0` guard preserves the "delete" path (`update.obj` is `None`/null for deletes; `version` is missing). Delete updates always pass through.

This makes duplicate broadcasts harmless and out-of-order broadcasts safe. Same-version arrivals from the response loop and the bridge are both no-ops on the second one.

### 6.2 Backend phased removal (optional, phase 3)

After the bridge is in place and proven, audit response-loop call sites and remove `success_with_updates(...)` for events the bridge already covers. The bridge becomes the *only* path for those updates.

This also reduces RPC response payload size — a small win.

---

## 7. Implementation phases

Each phase is independently mergeable.

### Phase 1 — Bridge alongside response loop (additive, low risk)

1. Land `agentmux-srv/src/server/wave_obj_bridge.rs` with the mapping function (§5.1) covering the **workspace events** at minimum (immediately fixes the reported bug).
2. Spawn a tokio task in `service.rs` startup that subscribes to `srv_events_tx` and calls `event_to_wave_obj_updates` per event, then pushes the resulting updates to the response broadcast channel.
3. Land frontend version-based dedup in `wos.ts` (§6.1) so the now-doubled broadcasts for `UpdateObjectMeta`-style handlers are coalesced.
4. **Outcome:** workspace-rename bug fixed; no per-handler changes; response-loop path still works for handlers that use it.

### Phase 2 — Mapping coverage expansion

1. Audit `Event::*` variants and add mapping table entries for every variant that affects a WaveObj (tab, layout, block, window).
2. Add Rust unit tests asserting the bridge produces the right `WaveObjUpdate` shape for each event variant.
3. Add a frontend integration test that mocks the WS channel and asserts reactive consumers re-derive on a synthetic event.

### Phase 3 — Response-loop deprecation (optional, opportunistic)

1. Identify `success_with_updates(...)` call sites whose updates are now fully covered by the bridge.
2. Per call site, change to `success_empty()`. Verify the frontend still updates (proves the bridge is doing the work).
3. Eventually remove the response-loop path entirely if no handlers need it. (The success-confirmation semantics of the RPC return are preserved by the empty-success status.)

---

## 8. Test plan

### 8.1 Phase 1

**Manual:**
- [ ] Rename workspace via UI → OS title and InstancePanel update immediately.
- [ ] Rename window via InstancePanel double-click → still works (regression).
- [ ] Open two windows, rename workspace → both windows' titles + both panels update (multi-client).

**Automated (Rust):**
- [ ] Unit test: `event_to_wave_obj_updates(Event::WorkspaceRenamed { … }, &wstore)` returns one update with the right shape.
- [ ] Integration test: dispatch `RenameWorkspace`, observe the broadcast channel emits one `waveobj:update` for the workspace.

**Automated (frontend):**
- [ ] Unit test for `updateWaveObject` dedup: feed two updates with the same version → second is a no-op.
- [ ] Unit test: feed two updates with v1 then v2 → both apply, v3 then v2 → v2 is no-op.

### 8.2 Phase 2

- [ ] One test per `Event::*` variant added to the mapping.
- [ ] Frontend manual sweep: tab rename, tab add, tab remove, layout reorder, block create/update — every action causes other windows' UIs to update.

### 8.3 Phase 3

- [ ] Per response-loop removal: confirm no behavioral regression.

---

## 9. Risks (after §11 research)

| Risk | Status | Notes |
|---|---|---|
| Mapping table gaps — events not translated | Medium | Phase 2 audit (§5.1 grep recipe); one test per variant |
| Double-broadcast triggers duplicate signal updates | **Mitigated** | §6.1 version dedup; all 6 WaveObj types have `version: i64` per §11.4 |
| Bridge crashes on malformed event → all subsequent updates dropped | Low | Mapping is exhaustive `match` with a `_ => vec![]` arm; impossible to panic on a missed variant |
| Performance: extra channel subscriber + per-event work | **Negligible** | `srv_events_tx` already has 2 subscribers; one more is cheap. Per-event work is one `wstore.get::<T>()` (sync SQLite query against the in-memory store, ms-scale per §11.2) |
| Persist-subscriber and bridge race — bridge broadcasts before SQLite is committed | **RESOLVED** (§11.1) | The reducer mutates the in-memory store and `apply_event_to_wstore` writes SQLite **synchronously inside the RPC handler**, both completing before `srv_events_tx.send()`. The bridge sees post-event state on every event. |
| Frontend dedup breaks legitimate reapply (e.g. force-reload after disconnect) | Low | Dedup compares by `version` field; explicit `reloadWaveObject` uses a different code path that always re-fetches from the server |
| Bridge broadcasts to disconnected clients | Negligible | Broadcast channel has bounded capacity; with no consumers, messages are dropped |
| `.await` while holding the `WaveStore` mutex would deadlock | **Mitigated** (§11.2) | `WaveStore` uses `std::sync::Mutex<Connection>` (blocking, not async). Mapping function is synchronous from `get<T>()` to `Vec` collection; broadcast happens outside the lock. Code review must enforce: no `.await` inside `event_to_wave_obj_updates` |

---

## 10. Out of scope

- Removing `srv_events_tx` and replacing with a single bus — the existing bus is fine; we're just adding a consumer.
- Cross-instance broadcast (events from instance A reaching instance B's frontend) — handled at a different layer (LAN peer / shared state), unrelated.
- Throttling / coalescing rapid same-object updates (e.g. drag-resize firing 60 events/sec) — possible follow-up if it matters; SolidJS effects already coalesce within a microtask.
- Schema versioning — `WaveObj.version` already exists and is incremented by the reducer; no change needed.

---

## 11. Resolved questions (research log)

These were open in the first draft of this spec; resolved by reading the sidecar source. Kept here so the next reader doesn't have to re-investigate.

### 11.1 Subscribe ordering — RESOLVED ✓ Safe

**Question:** When a bridge task subscribes to `srv_events_tx` and reads from the in-memory `WaveStore` on each event, will it see post-event state, or will it race?

**Answer:** **Safe.** The RPC handler path is synchronous up to and including the `srv_events_tx.send()`:

1. `dispatch_to_reducer()` — reducer mutates in-memory state (e.g. `state.workspaces`) **before returning** (`agentmux-srv/src/server/service.rs:1283-1290`)
2. `apply_event_to_wstore()` — called synchronously from the RPC handler in a loop, persists each event to SQLite (lines 1297-1304); see `persist_subscriber.rs:171-182` (`crate-visible by design so handlers can apply synchronously`)
3. `publish_events()` — broadcasts to `srv_events_tx` (line 1305)

So when the bridge receives an event, both the in-memory store *and* SQLite are already up-to-date. No race. The bridge can read from the in-memory `WaveStore` without coordination.

### 11.2 `WaveStore` lock structure — RESOLVED ✓ Cheap reads

**Question:** Can a tokio task hold a brief read lock per event without deadlocking the reducer's writes?

**Answer:** **Yes, cheap and safe.** `WaveStore` is `agentmux-srv/src/backend/storage/mstore.rs:24-31`:

```rust
pub struct WaveStore { conn: Mutex<Connection>, ... }
```

That's a **`std::sync::Mutex<rusqlite::Connection>`** (blocking, not async). Lock acquires are brief (one SQLite query per event). The reducer's writes already take the same lock for short periods. No async-aware lock means **the bridge must NOT `.await` while holding the lock** — but each fetch is a synchronous SQL query that completes immediately, so this is naturally satisfied.

**Implementation note:** the bridge's per-event work is `recv() → match event → wstore.get<T>() → build update → broadcast`. The lock is held only across the `get<T>()` call.

### 11.3 Compound events — RESOLVED ✓ Mapping refined (see §5.3 below)

**Question:** When a tab is added to a workspace, does the reducer emit one combined event or multiple?

**Answer:** **Multiple, ordered.** A single user action can fire 2+ events:

- `handle_create_tab` (`agentmux-srv/src/reducer/tab.rs:20-78`) pushes `Event::TabCreated` (line 64), **then** optionally `Event::ActiveTabChanged` (lines 70-76) if the new tab is activated.
- `handle_delete_tab` (`tab.rs:87-157`) pushes `Event::TabDeleted` (line 150), **then** optionally `Event::ActiveTabChanged` (lines 155-157) if the deleted tab was active.
- Workspace renames emit just `Event::WorkspaceRenamed` (single event).

This has **two implications** for the bridge:

1. **Some events imply dependent-object changes.** When `TabCreated` lands, the workspace's `tab_ids` field was also mutated by the reducer. The bridge's mapping for `TabCreated` must therefore fetch *both* the Tab (the new object) and the parent Workspace (whose `tab_ids` changed) and broadcast updates for both.
2. **Sequence matters; ordering is preserved.** Tokio's broadcast channel delivers in send-order to a single subscriber. Frontend will see `TabCreated` (with workspace update) followed by `ActiveTabChanged` (with workspace update again, version bumped). Both arrive — the second is a no-op via version dedup (§6.1) since the workspace's version was already updated by the first.

**Mapping table refined** to handle this — see §5.3 below.

### 11.4 Version field universality — RESOLVED ✓ Universal

**Question:** Does every `WaveObj` type have a `version` field?

**Answer:** **Yes, all six.** From `agentmux-srv/src/backend/obj.rs:305-461`:

| Type | `version` field | File:line |
|---|---|---|
| `Client` | `version: i64` | `obj.rs:308` |
| `Window` | `version: i64` | `obj.rs:327` |
| `Workspace` | `version: i64` | `obj.rs:348` |
| `Tab` | `version: i64` | `obj.rs:376` |
| `LayoutState` | `version: i64` | `obj.rs:401` |
| `Block` | `version: i64` | `obj.rs:450` |

All implement `WaveObj::get_version()` / `set_version()` via the `impl_wave_obj!` macro (`obj.rs:142-168`). The reducer calls `state.bump_version()` on every mutation (e.g. `reducer/workspace.rs:29,83,99`; `reducer/tab.rs:63,71,...`). The frontend dedup in §6.1 is universally applicable — no per-type carve-outs needed.

---

## 12. Why this beats Option A long-term

> Option A: ship a fix to one handler. The next person writing a mutation RPC has to remember a convention. Six months later someone adds `UpdateBlockTitle` and forgets — same bug, new place. Repeat.
>
> Option B: ship a one-time architectural fix. The convention becomes "emit the right event in the reducer", which the developer has to do anyway because the reducer drives state. Frontend reactivity becomes a property of the system, not a per-handler responsibility.

That's the whole argument for paying the up-front cost of the bridge. Option A is faster to land; Option B is the right architecture.

---

## 13. Implementation readiness checklist

Pre-flight, before opening the Phase 1 PR:

- [x] Subscribe ordering verified safe (§11.1 — bridge reads post-event state)
- [x] Lock structure understood (§11.2 — `std::sync::Mutex<Connection>`, sync, brief)
- [x] Compound-event pattern documented (§11.3 — dependent-object fetches in §5.2)
- [x] Version field universality confirmed (§11.4 — all 6 types have `version: i64`)
- [x] Mapping skeleton has runnable Rust pseudocode (§5.3)
- [x] Dedup logic handles deletes (§6.1 `newVersion > 0` guard)
- [ ] Find concrete `Event::*` enum (likely `agentmux-srv/src/reducer/types.rs` or similar) and reconcile §5.1 table against actual variants
- [ ] Identify the broadcast channel the response loop uses (the bridge needs to push to the same channel) — `agentmux-srv/src/server/service.rs:39-52` will reveal the type
- [ ] Confirm `OTYPE_WORKSPACE`, `OTYPE_TAB`, `OTYPE_BLOCK`, `OTYPE_LAYOUT`, `OTYPE_WINDOW`, `OTYPE_CLIENT` constants exist or define them

The two unchecked items are 5-minute code lookups, not decisions — verify during the Phase 1 PR rather than as a blocker on this spec.
