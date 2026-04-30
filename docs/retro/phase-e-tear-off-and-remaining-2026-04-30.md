# Reducer/wcore Consistency During E.2 Migration — Analysis + Plan

**Date:** 2026-04-30
**Trigger:** Smoke regression in #618 (`agentmux-0.33.520`) — tear off tab to new window, click "+" tab, nothing happens. Same workspace's "+" works in the original window.

---

## 1. The bug

**Repro:**
1. Right-click an existing tab → "Tear Off" → new window opens with the torn tab.
2. In the new window, click `+` to create another tab.
3. **Expected:** new tab appears.
   **Observed:** nothing — RPC returns an error from the reducer (`CreateTab: workspace not found: <uuid>`).

The original window's `+` continues to work because its workspace was loaded into the reducer at bootstrap.

## 2. Root cause

The srv reducer's state is **bootstrap-loaded once at process startup** from SQLite. After that, the only way new workspaces enter `state.workspaces` is via `Command::CreateWorkspace` dispatched **through the reducer's pipe** by a migrated RPC handler.

But several RPC handlers — including all the tear-off / move / window-create paths — still call `wcore::*` directly, writing SQLite without notifying the reducer. The reducer's view goes stale the moment any of those run:

```
       ┌──────────────────────┐
       │  agentmux-srv        │
       │                      │
       │  ┌──────────────┐    │
HTTP ──┼─→│ dispatch_svc │    │
       │  └──┬───────┬───┘    │
       │     │       │        │
       │  reducer  wcore-direct│ ← writes SQLite,
       │     ↓       ↓        │   reducer never sees it
       │  state.    SQLite    │
       │  workspaces  ▲       │
       │     ▲        │       │
       │     └────────┘       │
       │  bootstrap ONCE      │
       └──────────────────────┘
```

This is a fundamental inconsistency in the migration window: the reducer's claimed-authoritative state is actually a **partial projection** of SQLite. Any wcore-direct mutation creates a divergence that the reducer can't recover from on its own.

## 3. Audit — every wcore-direct path that mutates reducer-relevant state

Searched `agentmux-srv/src/server/service.rs` (handlers) and `agentmux-srv/src/backend/wcore/` (callees). Each row below is a code path that writes SQLite for a workspace/tab/block/layout WITHOUT dispatching through the reducer.

| Handler / call site | wcore call | Mutates | Severity |
|---|---|---|---|
| `("workspace", "TearOffTab")` | `wcore::tear_off_tab` | creates new workspace + window; moves tab between workspaces | **HIGH** — direct cause of smoke regression |
| `("workspace", "TearOffBlock")` | `wcore::tear_off_block` | creates new workspace + window + tab; moves block | **HIGH** — same root |
| `("workspace", "RestoreTornOffTab")` | `wcore::restore_torn_off_tab` | moves tab back to source workspace; deletes torn-off workspace | HIGH |
| `("workspace", "MoveTabToWorkspace")` | `wcore::move_tab_to_workspace` | moves tab between workspaces | HIGH |
| `("workspace", "MoveBlockToTab")` | `wcore::move_block_to_tab` | moves block between tabs (potentially across workspaces) | HIGH |
| `("workspace", "PromoteBlockToTab")` | `wcore::promote_block_to_tab` | block becomes new tab in same workspace | MEDIUM (workspace stays known) |
| `("workspace", "UpdateTabIds")` | direct `store.update::<Workspace>` | reorders/replaces `tabids` wholesale | MEDIUM |
| `("workspace", "UpdateWorkspace")` | direct `store.update::<Workspace>` | renames workspace | LOW (read-side only when stale) |
| `("object", "UpdateObject")` | direct `store.update` | bulk wave-obj update | MEDIUM |
| `("object", "UpdateObjectMeta")` | direct `store.update` | meta merge on workspace/tab/block | LOW |
| `("object", "UpdateTabName")` | direct `store.update::<Tab>` | renames tab | LOW |
| `("window", "CreateWindow")` | `wcore::create_window_full` | creates new workspace + window | **HIGH** |
| `("window", "CloseWindow")` | `wcore::close_window` | may delete orphan workspace | HIGH |
| `("window", "SwitchWorkspace")` | direct `store.update::<Window>` | window points at different workspace; could surface a workspace not in reducer | MEDIUM |
| `wcore::ensure_initial_data` (startup, first launch only) | inserts Client/Window/Workspace/Tab/Block | creates the starter workspace | LOW (covered by bootstrap on next pass; but the FIRST run after first-launch races) |

**Severity rubric:**
- HIGH = produces the bug class the user just hit (CreateTab / SetActiveTab / ReorderTab on the new workspace fails).
- MEDIUM = produces stale reads via reducer (currently masked because `GetWorkspace` / `ListWorkspaces` read wstore directly).
- LOW = no immediate user-visible effect.

**Count:** 14 paths bypass the reducer today. Tear-off and window flows are the biggest offenders for migration-window regressions.

## 4. Design options

### Option A — Lazy hot-load on demand

Before dispatching any reducer command that takes a `workspace_id`, check if `state.workspaces` has it; if not, read from wstore and insert (along with referenced tabs/blocks). New helper:

```rust
async fn ensure_workspace_in_reducer(state: &AppState, workspace_id: &str) -> Result<(), String>
```

Called at the top of CreateTab, SetActiveTab, ReorderTab, CloseTab, CreateBlock, DeleteBlock.

- **Pros**
  - ~30 LoC, surgical, ships immediately
  - Fixes the smoke regression cleanly
  - Doesn't require touching tear-off / window code
- **Cons**
  - The inconsistency remains a footgun: when E.5 sagas run, they'll hit the same class of bug from different directions
  - Requires every new reducer-dispatching handler to remember to call the helper (easy to forget)
  - Doesn't solve UpdateTabIds / UpdateWorkspace causing stale reads through the reducer
  - Saga coordinator (E.5) emits commands that might race with wcore-direct writes — lazy hot-load can't reconcile mid-flight

### Option B — Migrate every wcore-direct path through the reducer

Each handler in §3 becomes:
1. Build a `Command::*` (existing or new) with the parameters
2. Dispatch through reducer (gets event(s))
3. Subscriber writes SQLite
4. RPC returns

For multi-step operations (tear-off, move) this means new compound reducer commands or — better — multi-step **sagas** orchestrated by the E.1a saga coordinator.

- **Pros**
  - Truly authoritative reducer; no inconsistency window
  - Subscribers (renderer, future replication, --diag) see a coherent stream
  - Testable: reducer unit tests cover the full surface
- **Cons**
  - ~500-800 LoC across 14 handlers
  - Multi-step ops (tear-off, restore) are saga shape — depend on saga coordinator (E.5) being live
  - Schema-level commands (`Command::TearOffTab`, etc.) bloat the wire protocol unless modelled as sagas
  - Doesn't ship until E.5 lands; user UX broken for that long without a stopgap

### Option C — Pure event publisher, drop reducer state

Reducer no longer holds workspaces/tabs/blocks. `Command::CreateWorkspace` becomes "call wcore + emit event"; subscribers get the event and update their own state.

- **Pros**
  - No reducer/wcore divergence (reducer doesn't store anything to diverge)
  - Simpler reducer state shape
- **Cons**
  - **Violates the spec's "pure functional core" invariant** — reducer becomes I/O-bound (calls wcore)
  - Loses the ability to validate cross-entity invariants in-reducer (e.g., "tab must belong to a workspace") — those move to wcore
  - Renderer-side state reconstruction needs every event since boot, no `GetSrvSnapshot` shortcut
  - **Reject:** undoes the architectural choice that motivated Phase E

### Option D — Synthetic register-existing events from wcore-direct paths

New `Command::RegisterExistingWorkspace { workspace_id }` (and `RegisterExistingTab`, etc.). After every wcore-direct mutation, the handler emits the corresponding `Register*` command into the reducer so it picks up the new entity. Reducer's handler reads from wstore once and inserts the record.

Equivalent to Option B but **single-step, no saga coordinator dependency**.

- **Pros**
  - Reducer stays authoritative for state queries (subscribers / renderer trust it)
  - No multi-step saga shape needed; one extra dispatch per wcore-direct call
  - Doesn't bloat the wire protocol with TearOff* commands — those stay local SQL, just with a register-after side-effect
  - Bus event still emitted, so subscribers and the renderer dispatcher see the change
- **Cons**
  - Two-step pattern in every wcore-direct handler (call wcore, then register)
  - If wcore succeeds but Register dispatch fails (e.g., reducer mutex briefly unavailable — but it's tokio Mutex so this doesn't happen): same divergence we have today, but now bounded to a known failure mode
  - Slightly more wire traffic (one extra command per wcore-direct op)
  - Doesn't fully migrate; reducer is still partly downstream of wcore. Phase F could finish the migration cleanly.

### Hybrid recommendation

**Ship Option A immediately as the smoke fix** (~30 LoC, ~1 review round, unblocks user testing today).

**Plan Option D as a follow-up sweep** in E.2c.6 (or fold into E.5) (~200 LoC, touches every wcore-direct handler in §3 to add the register-after dispatch).

**Defer Option B** to Phase F when the saga coordinator can absorb the multi-step ops cleanly.

Why this layering:
- **A right now** because the user is mid-smoke, the fix is small, and it's the bare minimum to unblock.
- **D as the durable resolution** because it preserves the reducer's authoritative property without requiring sagas to land first.
- **B is the long-term shape** because Phase F (host reducer) and Phase 7 (cross-platform) both benefit from "every state mutation goes through the reducer." But Phase F hasn't started; we shouldn't gate the smoke unblock on it.

If A introduces inconsistencies under sustained tear-off/restore activity (e.g., the `tab_ids` snapshot in the reducer drifts from wstore), we accelerate D.

## 5. Recommended Option-A implementation

```rust
// In service.rs, near the dispatch_to_reducer helper:

/// Phase E.2c hot-fix — ensure the reducer knows about a workspace
/// before dispatching commands that reference it. Wcore-direct paths
/// (tear-off, window create, etc.) write SQLite without notifying
/// the reducer; this helper picks up those workspaces on demand.
///
/// The proper fix is Option D in
/// `docs/retro/phase-e-tear-off-and-remaining-2026-04-30.md` — every
/// wcore-direct path emits a Register* command after its write. This
/// helper is a transitional patch.
async fn ensure_workspace_in_reducer(
    state: &AppState,
    workspace_id: &str,
) -> Result<(), String> {
    {
        let s = state.srv_state.lock().await;
        if s.workspaces.contains_key(workspace_id) {
            return Ok(());
        }
    }
    let ws = match state.wstore.get::<Workspace>(workspace_id) {
        Ok(Some(ws)) => ws,
        Ok(None) => return Err(format!("workspace not found: {}", workspace_id)),
        Err(e) => return Err(format!("SQLite read failed: {}", e)),
    };
    // Hot-load tabs that this workspace references so dispatches
    // touching tab_ids work too (CreateBlock validates tab presence).
    let mut tabs_to_load = Vec::new();
    for tid in ws.tabids.iter().chain(ws.pinnedtabids.iter()) {
        if let Ok(Some(tab)) = state.wstore.get::<Tab>(tid) {
            tabs_to_load.push(tab);
        }
    }
    let mut s = state.srv_state.lock().await;
    s.workspaces.entry(workspace_id.to_string()).or_insert_with(|| {
        crate::state::WorkspaceRecord {
            workspace_id: ws.oid.clone(),
            name: ws.name.clone(),
            // Pinned-then-regular concat matches bootstrap convention.
            tab_ids: ws.pinnedtabids.iter().chain(ws.tabids.iter()).cloned().collect(),
            active_tab_id: if ws.activetabid.is_empty() {
                None
            } else {
                Some(ws.activetabid.clone())
            },
        }
    });
    for tab in tabs_to_load {
        s.tabs.entry(tab.oid.clone()).or_insert_with(|| {
            crate::state::TabRecord {
                tab_id: tab.oid.clone(),
                workspace_id: workspace_id.to_string(),
                name: tab.name.clone(),
                block_ids: tab.blockids.clone(),
            }
        });
    }
    Ok(())
}
```

Call sites — at the top of each handler, immediately after parsing `ws_id`:

- `CreateTab`, `SetActiveTab`, `ReorderTab`, `CloseTab`
- `CreateBlock` (gets `ws_id` indirectly via `tab_id`; needs a `ensure_tab_in_reducer` companion that walks back to the workspace if needed — simpler: just always call ensure_workspace if we can derive ws_id from the tab via wstore lookup)
- `DeleteBlock` (same as CreateBlock)

For CreateBlock/DeleteBlock the helper variant looks up the tab's parent workspace via `wstore.get::<Tab>` then ensures that workspace.

**No new wire-protocol changes. No new commands. Pure RPC-layer patch.**

## 6. Remaining Phase E work

State of play after #618 merge (`agentmux-0.33.520`):

| Sub-phase | Scope | Status |
|---|---|---|
| E.1a / E.1b | Saga coordinator + srv reducer skeleton | ✅ |
| E.2 / E.2b / E.3 | Workspace + Tab + Block lifecycle arms | ✅ |
| E.2c.1 — E.2c.5a | Persist subscriber + RPC migration (workspace + tab + block) + Rust host bridge | ✅ |
| **E.2c.5b** | TypeScript renderer dispatcher — install `window.__agentmux_srv_event`, route events into atom domains | ⏸ next |
| **E.2c.6 (NEW)** | Option D rollout — emit `RegisterExisting*` after every wcore-direct path enumerated in §3 | ⏸ proposed |
| **E.4** | Layout state — minimal slice (focused/magnified node tracking only) | ⏸ deferred |
| **E.5** | Drag/tear-off sagas (first concrete saga consumers; unblocks Option B path) | ⏸ deferred |
| **E.6** | Renderer multi-source consumption + saga buffering | ⏸ depends on E.2c.5b |
| **E.7 — exit** | Property tests + cross-reducer integration tests + `--diag srv` / `--diag sagas` | ⏸ exit |

### Recommended sequencing post-#618

1. **Hot-fix branch (this PR)** — Option A `ensure_workspace_in_reducer`. Unblocks smoke.
2. **E.2c.5b** — TypeScript renderer dispatcher. Brings the host bridge live for renderer consumption. Expected: ~200-300 LoC TypeScript.
3. **E.2c.6 (Option D rollout)** — Register-existing dispatch after every wcore-direct mutation in §3. Removes the inconsistency window. ~150-250 LoC.
4. **E.4** — Layout minimal slice (focused/magnified). Unblocks E.6 renderer-side drift detection.
5. **E.5** — Sagas. Tear-off / move become coordinated multi-reducer flows. Replaces parts of §3 that benefit from the saga shape.
6. **E.6** — Renderer per-source version tracking + saga-buffer. Bespoke `WaveObjUpdate` retired or demoted.
7. **E.7** — Property tests + integration tests + `--diag` Tools. Phase exit.

### Carryovers from prior PRs

Codex P3 from #617 (already addressed in #617): u32 clamp on reorder index — done.

Codex P2 from #613 (post-merge): ambiguous block-parent during bootstrap (multi-tab block ownership). Defensive-repair concern; defer to next persist.rs PR (likely E.2c.6 or E.4).

## 7. Open questions

1. **Should `ensure_workspace_in_reducer` also hot-load blocks?** Currently the proposed helper only loads workspace + tabs. If a CreateBlock dispatch comes in for a tab in a tear-off workspace, the tab record gets loaded but the tab's existing blocks don't. Does the reducer's CreateBlock care? Looking at handle_create_block — it validates `state.tabs.contains_key(&tab_id)` and appends to `tab.block_ids`. It doesn't iterate `state.blocks`, so missing block records don't break Create. But DeleteBlock looks up by block_id — would silent-no-op on a block the reducer didn't load. That's idempotent-ish but means a block that exists in wstore can't be deleted via the reducer until something else loads it. **Decision: load blocks too** when ensuring a tab.

2. **Race between `ensure_workspace_in_reducer` and a concurrent `CreateWorkspace`?** Two RPCs arrive simultaneously: one calls ensure (loads from wstore), one dispatches CreateWorkspace (assigns new uuid). Different workspaces, so no conflict. The lock is held only briefly by each. **Safe.**

3. **What if wstore says "not found"?** The helper returns an error; the caller surfaces it. That's the correct behaviour — the user's CreateTab against a non-existent workspace should fail clearly.

4. **When does Option A become a problem?** When the reducer's lazily-loaded `tab_ids` for a workspace falls behind a wcore-direct mutation that adds tabs to that workspace. E.g., a saga (E.5+) creates a tab via `wcore::create_tab_with_opts` and the reducer's hot-loaded snapshot is now stale. **Mitigation:** by E.5, Option D is shipped; saga producers always go through reducer → no wcore-direct writes for migrated entities.

5. **Should the helper also reconcile reducer state with current wstore (overwrite)?** No — that's resync territory and would clobber legitimate reducer-only mutations between two RPC calls in the same workspace. The `or_insert_with` is intentional: only loads if missing.

---

## 8. Decision

**Proceeding with Option A as the immediate fix** (this PR, hot-fix #619 or similar branch).

**Option D rollout planned as E.2c.6** — one PR after E.2c.5b lands.

This document committed to `docs/retro/phase-e-tear-off-and-remaining-2026-04-30.md` so the analysis survives the next session.
