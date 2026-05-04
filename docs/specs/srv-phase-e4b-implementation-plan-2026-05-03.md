# srv Phase E.4.B — Implementation Plan

**Date:** 2026-05-03
**Status:** Plan with grounded analysis. Forensic findings citations throughout.
**Scope:** `agentmux-srv/` — make `LayoutState.rootnode` reducer-shaped on the Rust side
**Reads-this-first:**
- `docs/specs/SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md` — original spec; deferred E.4.B as "the hard part"
- `agentmux-srv/src/reducer.rs:655–700` — E.4.A handlers (the pattern to mirror)
- `frontend/layout/lib/layoutTree.ts` — the 11 pure-ish tree mutators that already encode the operations we'd port

## Two surprises from the forensic pass that change the math

The original Phase E.4 spec framed E.4.B as urgent for perf reasons:

> *"every drag-resize tick allocates a JSON blob, hits the mutex, and emits an event — that's 60Hz of churn for what is logically one ongoing user gesture."*

**The forensic pass invalidated both halves:**

1. **Not 60Hz.** Frontend resizes call `treeReducer(SetPendingAction)` locally, then `persistToBackend()` debounces 100ms (`frontend/layout/lib/layoutPersistence.ts:261–279`). One drag gesture produces ~1 backend update, not 60.
2. **Nodes already have UUIDs.** `service.rs:2305` and `dnd.rs` both `Uuid::new_v4()` when building tree nodes; the JSON shape preserves them. Path representation isn't blocked on schema work — by-ID addressing is already the natural fit.

**Consequence for the plan:** E.4.B is **not** an urgent perf project. It's a correctness/architecture project. That changes the right framing from "fix this hot path" to "decide whether the audit + invariants + multi-window-sync benefits are worth the migration cost." The cost is real; the perf benefit is small.

## What's actually there today

### `LayoutState` shape (`agentmux-srv/src/backend/obj.rs:391–406`)

```rust
pub struct LayoutState {
    pub oid: String,
    pub version: i64,
    pub rootnode: Option<serde_json::Value>,        // ← OPAQUE — the E.4.B target
    pub magnifiednodeid: String,                    // ← Reducer-shaped (E.4.A)
    pub focusednodeid: String,                      // ← Reducer-shaped (E.4.A)
    pub leaforder: Option<Vec<LeafOrderEntry>>,     // ← Wcore-direct
    pub pendingbackendactions: Option<Vec<LayoutActionData>>,  // ← Wcore-direct
    pub meta: Option<MetaMapType>,
}
```

### Existing `rootnode` writers (4 sites)

| Site | What it does | When |
|---|---|---|
| `agentmux-srv/src/server/service.rs:2306` | Builds a fresh single-node tree on tab tear-off | User drags pane out to new tab |
| `agentmux-srv/src/backend/wcore/dnd.rs:248` | Builds new tree after drag-drop consolidation | Mid-DnD reparenting |
| `agentmux-srv/src/backend/wcore/mod.rs:167` | Initial three-pane tree on first launch | App first run |
| `agentmux-srv/src/backend/wcore/block.rs:83` | Sets `rootnode = None` when last block removed | Last-pane close |

All four use `store.update(&mut layout)` directly — no reducer dispatch.

The frontend's debounced flush (`layoutPersistence.ts:261–279`) is the **fifth and most frequent** writer in practice — it overwrites the entire `rootnode` blob via the waveObject update subscriber path on every tree mutation that survives the 100ms debounce.

### The pattern E.4.A established (`reducer.rs:655–700`)

E.4.A handlers are tiny and uniform:

```rust
fn handle_set_focused_node(state: &mut State, tab_id: String, node_id: String) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        return vec![Event::Error { ... }];   // unknown tab → typed error
    };
    if tab.focused_node_id == node_id {
        return Vec::new();                    // no-op short-circuit
    }
    tab.focused_node_id = node_id.clone();
    let v = state.bump_version();
    vec![Event::FocusedNodeChanged { tab_id, node_id, version: v }]
}
```

~25 LOC. Unknown-target errors typed. No-op short-circuit. Single event on real change. Tests at `reducer.rs:1500+` cover happy-path / no-op / unknown-target.

Persist subscriber arm (`persist_subscriber.rs:666–686`):

```rust
fn apply_focused_node_changed(wstore: &WaveStore, tab_id: &str, node_id: &str) -> Result<...> {
    let Some(tab) = wstore.get::<Tab>(tab_id)? else { return Ok(()); };
    if tab.layoutstate.is_empty() { return Ok(()); }
    let Some(mut layout) = wstore.get::<PersistedLayoutState>(&tab.layoutstate)? else { return Ok(()); };
    if layout.focusednodeid == node_id { return Ok(()); }
    layout.focusednodeid = node_id.to_string();
    wstore.update(&mut layout)?;
    Ok(())
}
```

Idempotent. Single SQL UPDATE. Schema columns already exist.

## The 15 frontend operations to port

`frontend/layout/lib/types.ts:65–82` defines the action surface. Each becomes a candidate srv command:

| Frontend action | Mutation shape | E.4.B command target |
|---|---|---|
| `Move` | reparent subtree (target by UUID) | `Command::LayoutMoveNode { tab_id, node_id, new_parent_id, index }` |
| `Swap` | swap two siblings | `Command::LayoutSwapNodes { tab_id, node1_id, node2_id }` |
| `ResizeNode` | apply N size deltas | `Command::LayoutResizeNodes { tab_id, ops: Vec<ResizeOp> }` |
| `InsertNode` | add via `findNextInsertLocation` heuristic | `Command::LayoutInsertNode { tab_id, node }` |
| `InsertNodeAtIndex` | add at exact index path | `Command::LayoutInsertNodeAtIndex { tab_id, node, index_arr }` |
| `DeleteNode` | remove + collapse empty parents | `Command::LayoutDeleteNode { tab_id, node_id }` |
| `ReplaceNode` | swap one node for another, preserve size | `Command::LayoutReplaceNode { tab_id, target_id, new_node }` |
| `SplitHorizontal` | wrap-or-splice in Row group | `Command::LayoutSplitHorizontal { tab_id, target_id, new_node, position }` |
| `SplitVertical` | wrap-or-splice in Column group | `Command::LayoutSplitVertical { tab_id, target_id, new_node, position }` |
| `ClearTree` | wipe rootnode + focused + magnified + leaforder | `Command::LayoutClear { tab_id }` |
| `FocusNode` | set focused | **Already exists (E.4.A)** |
| `MagnifyNodeToggle` | toggle magnified | **Already exists (E.4.A)** |
| `ComputeMove` | pure compute — returns a pending Move | **Stays frontend-side** (no state mutation; just produces a Move command) |
| `SetPendingAction` | stage a pending tree change | **Stays frontend-side** (transient UI state during drag) |
| `CommitPendingAction` / `ClearPendingAction` | commit/cancel the staged change | **Stays frontend-side** (or maps to the underlying Move/Resize) |

**11 commands** to add (the first 10 + ClearTree). Three actions stay client-side.

## Two design questions that drive the rest

### Q1 — Typed Rust tree, or keep `serde_json::Value`?

**Option A — Typed tree (recommended).** Replace `Option<serde_json::Value>` with a Rust struct:

```rust
pub struct LayoutNode {
    pub id: String,                    // UUID
    pub flex_direction: FlexDirection, // Row / Column
    pub size: f32,
    pub children: Vec<LayoutNode>,
    pub data: Option<LayoutNodeData>,  // blockId etc.
}
```

Pros: handlers operate on a typed structure (compile-time safety); can reuse Rust enum exhaustiveness checks; serde-derived (de)serialization handles wire compat.

Cons: serde compat with the existing JSON shape needs careful test coverage. One-time write of the type definitions + (de)serializer.

**Option B — Keep `serde_json::Value`, add path-walk helpers.** Each command operates on the JSON tree via path-by-id traversal helpers.

Pros: zero migration risk; existing JSON shape unchanged; old wcore code keeps compiling.

Cons: handlers manipulate untyped JSON; runtime errors instead of compile-time; duplicates the shape definition (already implicit in the JSON serializer on the frontend).

**Recommendation: Option A.** The migration cost is bounded (one-shot typing exercise, not architecture-wide), and the safety benefits compound as the 11 handlers land. Frontend already has the typed shape; mirroring it in Rust closes a long-standing semantic gap.

### Q2 — How does srv apply a command and report it back to frontend?

Two roundtrip shapes:

**Shape A — Frontend dispatches granular command, srv applies authoritatively, srv emits authoritative event:**

```
[user clicks pane] → frontend treeReducer (locally apply)
                  → frontend dispatches RPC LayoutMoveNode
                  → srv reducer LayoutMoveNode
                  → srv emits LayoutNodeMoved event
                  → frontend receives event, reconciles local state if differs
```

**Shape B — Frontend stays authoritative, srv mirrors:**

```
[user clicks pane] → frontend treeReducer (locally apply)
                  → frontend persists via existing waveObject path
                  → srv subscriber detects change
                  → srv reducer dispatches LayoutMoveNode synthetically
                  → srv emits LayoutNodeMoved
                  → other frontend windows receive event
```

Shape A is the redux-correct version. Shape B is the migration-friendly version that preserves the existing frontend code path.

**Recommendation: Shape B for the migration**, with a future flip to Shape A once tests prove the round-trip.

Why: Shape A requires synchronous waiting on srv before applying user actions locally — adds latency on every focus/click/drag. Shape B preserves the optimistic-local-apply UX. The reducer's value (audit + invariants + multi-window mirror) is achieved either way.

## Implementation phases

Total: ~3–4 weeks of focused work, broken into 8 phases. Ship phase by phase with main-merges between each.

### Phase 1 — Spec finalization + design decisions (2 days, doc-only)

- Lock in Q1 + Q2 above
- Decide handling of `pendingbackendactions` field (likely subsumed; needs a call)
- Decide `leaforder` migration (probably stays wcore-direct; it's derived from rootnode)
- Decide error semantics for unknown node-id targets (typed error vs silent no-op — match E.4.A's typed-error pattern)
- Define exact wire protocol for each of the 11 commands

**Deliverable:** updated spec doc, no code.

### Phase 2 — Typed `LayoutNode` Rust struct (~3 days)

- Define `LayoutNode`, `LayoutNodeData`, `FlexDirection` in `agentmux-srv/src/backend/obj.rs`
- Serde-compat layer with current JSON shape (round-trip tests against real tree blobs from production data)
- `LayoutState.rootnode` field migrates from `Option<serde_json::Value>` to `Option<LayoutNode>`
- 4 wcore-direct writers (service.rs:2306, dnd.rs:248, mod.rs:167, block.rs:83) update their construction to use the new type
- Frontend (de)serialization unchanged (still consumes/produces the same JSON shape via serde)

**Deliverable:** typed tree on srv, no behavior change. Single PR. Zero new commands yet.

### Phase 3 — Reducer command + event types (~1 day, all in `agentmux-common/src/ipc.rs`)

- Add 11 `Command::Layout*` variants
- Add 11 corresponding `Event::Layout*` variants
- Add helper types: `ResizeOp`, `SplitPosition` etc.
- No handlers yet — just the wire types

**Deliverable:** types compile; no runtime behavior change.

### Phase 4 — Pure helper functions (~3 days, in `agentmux-srv/src/backend/`)

Port the frontend `layoutTree.ts` mutators to Rust as pure helper functions on `LayoutNode`:

- `find_node_by_id(&LayoutNode, id) -> Option<&LayoutNode>`
- `find_parent_by_child_id(&LayoutNode, child_id) -> Option<&LayoutNode>`
- `insert_node`, `delete_node`, `move_node`, `swap_nodes`, `resize_nodes`, `replace_node`, `split_horizontal`, `split_vertical`, `clear_tree`
- Each takes `&mut LayoutNode` (or `&mut Option<LayoutNode>` for clear) and the operation parameters

These are **pure functions** — no SQL, no events, no I/O. Mirror the shape of `frontend/layout/lib/layoutTree.ts`.

**Tests for each** — table-driven, mirror the test suite I just shipped for the frontend (PR #686). The frontend tests act as the **behavioral oracle** — port them.

**Deliverable:** ~700 LOC of pure mutators + ~400 LOC of tests. Coverage matches what the frontend already has.

### Phase 5 — Reducer arms (~2 days)

For each of the 11 commands, write a reducer handler in `reducer.rs` mirroring the E.4.A shape:

```rust
fn handle_layout_insert_node(state: &mut State, tab_id: String, node: LayoutNode) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        return vec![Event::Error { ... }];
    };
    let Some(rootnode) = tab.rootnode.as_mut() else {
        // Empty tree — promote to root
        tab.rootnode = Some(node.clone());
        let v = state.bump_version();
        return vec![Event::LayoutNodeInserted { tab_id, node, version: v }];
    };
    backend::layout::insert_node(rootnode, node.clone());  // pure helper
    let v = state.bump_version();
    vec![Event::LayoutNodeInserted { tab_id, node, version: v }]
}
```

~25–40 LOC per handler × 11 = ~350 LOC. **Tests at the reducer level** — round-trip tests for each command (mirror E.4.A's test patterns).

**Deliverable:** all 11 handlers + tests. No persistence yet.

### Phase 6 — Persist subscriber arms (~2 days)

For each `Event::Layout*`, add an apply function in `persist_subscriber.rs`:

```rust
fn apply_layout_node_inserted(wstore: &WaveStore, tab_id: &str, node: &LayoutNode) -> Result<...> {
    let Some(tab) = wstore.get::<Tab>(tab_id)? else { return Ok(()); };
    let Some(mut layout) = wstore.get::<PersistedLayoutState>(&tab.layoutstate)? else { return Ok(()); };
    // Apply the same mutation the reducer applied (idempotent because both paths use the same helper)
    if let Some(root) = layout.rootnode.as_mut() {
        backend::layout::insert_node(root, node.clone());
    } else {
        layout.rootnode = Some(node.clone());
    }
    wstore.update(&mut layout)?;
    Ok(())
}
```

11 apply functions, ~20 LOC each = ~220 LOC. Idempotent by design (same pure helper as the reducer).

**Deliverable:** events round-trip through SQLite.

### Phase 7 — Migrate the existing 5 writers (3 days, the risky one)

This is where behavior actually changes.

| Writer | Migration |
|---|---|
| `service.rs:2306` (tear-off) | Replaces direct `store.update` with `dispatch(Command::LayoutInsertNode { ... })` (or a tear-off-specific composite command) |
| `dnd.rs:248` (DnD consolidation) | Same — dispatch instead of direct write |
| `mod.rs:167` (first-launch) | Initial tree built then dispatched as a single LayoutInsertNode |
| `block.rs:83` (last-pane close) | Dispatch `LayoutClear` |
| Frontend persist via waveObject subscriber | This is the big one — see below |

**Frontend-flush migration** (the trickiest sub-step): the existing waveObject subscriber detects rootnode changes and writes them directly. Two options:

- **Drop-in replacement**: subscriber detects what changed and dispatches the corresponding granular Layout commands. Requires a diff against the prior tree state. Complex — but preserves the existing frontend code path entirely (Shape B from Q2).
- **Frontend dispatches directly**: refactor `persistToBackend()` to enumerate the actions taken since the last flush and dispatch them as commands. Requires a frontend code change (new RPC paths) but is cleaner long-term.

Recommended: drop-in replacement first (no frontend changes, lowest blast radius), then frontend-dispatches-directly as a follow-up.

**Risk profile:** highest of any phase. Layout is the most-touched UI surface. Mitigations:
- Keep the wcore-direct path as a fallback behind a feature flag during the rollout
- Heavy test coverage from Phase 4–6 already in place
- Smoke before merge (multi-window, drag-resize, tear-off, all the gestures)

### Phase 8 — Reducer-side correctness fixes that surface during migration (~3 days)

Once the migration is in main, real-world usage will surface:
- Edge cases in tree mutations the frontend tests didn't cover (you can't write a frontend test for "what happens when wcore-direct write races a reducer dispatch")
- Subtle behavioral differences between the frontend's pure-mutation tree code and the Rust port (off-by-one, child-collapse semantics, etc.)
- Drift detection — events emitted vs. SQL state

Budget for this is the honest part — it's never zero.

### Phase 9 — Frontend slice #8 (~1 week, separate spec)

Once srv E.4.B is stable, slice #8 (frontend-pane-tree-reducer) becomes implementable. The frontend already has `frontend/layout/lib/layoutTree.ts` as its own reducer; the slice #8 work is wiring it to subscribe to srv's `LayoutNode*` events for the multi-window mirror case. **This is a separate spec.**

## Total effort

| Phase | LOC | Days |
|---|---|---|
| 1 — Spec finalization | (doc) | 2 |
| 2 — Typed `LayoutNode` | ~200 | 3 |
| 3 — Wire types | ~150 | 1 |
| 4 — Pure helpers | ~700 + ~400 tests | 3 |
| 5 — Reducer arms | ~350 + ~300 tests | 2 |
| 6 — Persist arms | ~220 | 2 |
| 7 — Migration (risky) | ~300 | 3 |
| 8 — Correctness pass | varies | 3 |
| **srv-side total** | **~1900 LOC + ~700 LOC tests** | **~3 weeks** |
| Phase 9 — Frontend #8 | (separate spec) | ~1 week |

## What this plan deliberately does NOT do

- **Doesn't change the wire format.** `rootnode` JSON shape stays compatible — typed Rust struct serializes/deserializes to the same bytes.
- **Doesn't change debouncing strategy.** Frontend keeps the 100ms debounce.
- **Doesn't change `leaforder` or `pendingbackendactions`.** Both stay wcore-direct (decided in Phase 1; flagged for re-eval if a real bug appears).
- **Doesn't migrate the frontend tree reducer.** Frontend `layoutTree.ts` keeps its existing shape; Phase 9 only adds the srv-event subscription.
- **Doesn't add multi-window focus sync.** That's slice #5/E2 territory; orthogonal to E.4.B.

## Open questions for product/architecture review

| # | Question | Default proposed |
|---|---|---|
| Q1 | Typed tree vs `serde_json::Value`? | Typed (Option A) |
| Q2 | Sync shape (A frontend-waits, B optimistic-local) | B (optimistic-local) |
| Q3 | Error semantics for unknown node-id | Typed error (match E.4.A) |
| Q4 | Migrate `pendingbackendactions`? | No — re-eval if needed |
| Q5 | Migrate `leaforder`? | No — derive-only, stays wcore |
| Q6 | Feature flag the migration in Phase 7? | Yes, default-on after one minor version |
| Q7 | Drop the frontend's per-action `treeReducer` once srv is authoritative? | No — frontend reducer stays; just adds srv subscription |

## Risks (honest)

| Risk | Severity | Mitigation |
|---|---|---|
| Subtle behavioral divergence between Rust port of `layoutTree.ts` and the original | Medium-High | Use frontend tests as oracle; port them; run in parallel during Phase 7 |
| Migration of the 5 write sites breaks tear-off / first-launch / DnD | High (this IS the most-touched UI surface) | Feature flag; smoke each gesture before merge; keep wcore-direct fallback |
| serde compat issues with existing JSON shape | Medium | Round-trip tests against production tree blobs |
| Effort estimate underruns reality by 2× | Medium | Plan assumes contiguous focus; budget buffer |
| Plan reveals Q1 (typed tree) was wrong call mid-Phase 2 | Low | Phase 2 is small and reversible |

## Three honest paths forward

| Path | When to pick | Cost |
|---|---|---|
| **Don't do E.4.B** | If the perf framing was the only motivation, and we now know it's debounced | $0 — but slice #8 stays blocked |
| **Do E.4.B as planned** | If audit + invariants + multi-window mirror have value worth the work | ~3–4 weeks srv-side + ~1 week frontend |
| **Do half of E.4.B (typed tree only)** | If we want the type safety without the reducer command surface | ~5 days; lets future commands land incrementally |

The half-version (typed tree only — phases 1+2 of this plan) is the **Pareto-best** option if you're not committed to multi-window state sync. It eliminates the opaque `serde_json::Value` (which is the ugliest single thing on the srv side) without committing to the 11-command surface.

## Recommendation

**Do phases 1+2 ("typed tree only") first as a discrete project.** Single PR. Low risk. Removes the opaque-blob smell. Doesn't commit to the rest. After that lands, re-decide whether to continue with phases 3–8 based on whether multi-window state mirroring becomes a real priority. If it does, the typed tree is exactly the foundation those phases need; if it doesn't, we got a real architecture improvement for ~5 days of work and stopped at a coherent point.

Don't commit to the full ~4-week project until the value is concrete. The forensic pass already invalidated half the original motivation.
