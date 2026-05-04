# SPEC: srv Phase E.4.B — Layout Tree as Reducer State

**Date:** 2026-05-03
**Status:** Formal implementation spec. Final answers to all open questions; ready to drive PRs.
**Decision context:** Full E.4.B committed (~4–5 weeks) for **multi-window robust operation**. The half-version (typed tree only) is rejected — it doesn't fix the drag-resize-clobber and concurrent-split-data-loss bugs that motivate the project.
**Reads-this-first:**
- `docs/specs/SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md` — original Phase E.4 spec; deferred E.4.B as "the hard part"
- `docs/specs/srv-phase-e4b-implementation-plan-2026-05-03.md` — the planning doc this formalizes
- `agentmux-srv/src/reducer.rs:655–700` — E.4.A handlers (the pattern)
- `agentmux-srv/src/state.rs:69–88` — `TabRecord` struct (extended in this spec)
- `frontend/layout/lib/layoutTree.ts` — the 11 pure-ish tree mutators to port
- `frontend/layout/tests/layoutTree.test.ts` — test oracle from PR #686

## 1. Goals

This spec covers the work to:

1. **Replace `LayoutState.rootnode: Option<serde_json::Value>` with a typed Rust `Option<LayoutNode>` struct** (`agentmux-srv/src/backend/obj.rs:391–406`). Removes the opaque-blob smell on the most-touched UI surface.
2. **Add 11 reducer commands + 11 events** for tree mutations (Move, Swap, Resize, Insert×2, Delete, Replace, SplitH, SplitV, Clear, plus E.4.A's already-shipped Focus/Magnify which stay).
3. **Migrate the 5 existing rootnode writers** to dispatch through the reducer.
4. **Wire the frontend** to subscribe to `Layout*` events + apply granular mutations to its local state instead of replacing the whole tree on every wstore update (slice #8 of the frontend reducer roadmap).

The user-visible payoff is multi-window robust operation. Concretely:

| Scenario | Today | After E.4.B |
|---|---|---|
| Drag-resize in window B while window A flushes a tree change | Window B's drag clobbered | Window B's drag continues; remote command applies to non-overlapping subtree |
| Both windows split different panes simultaneously | Last writer wins; one user's split silently disappears | srv serializes both commands; both splits land |
| Tear off a tab from window A | Window B re-renders entire layout from blob | Window B applies a typed `LayoutTabReplaced` event |
| Audit "what tree mutations happened in the last 5 min?" | Invisible | Captured in the global event log (PR-C) per-command |

## 2. Non-goals

- **No migration of `leaforder` to the reducer.** It's a derived view of `rootnode`; let the existing computation stay wcore-side.
- **No migration of `pendingbackendactions`.** Re-evaluate after E.4.B has soaked. Likely subsumed by command-level events; defer the call.
- **No change to the frontend's local-first reducer** (`frontend/layout/lib/layoutTree.ts`). It keeps its existing 15 actions as the local optimistic-update path. Slice #8 only adds an srv-event subscription on top.
- **No multi-window state sync for state OUTSIDE the layout tree** (e.g. agent-pane state, tab-bar state). Those are separate concerns.
- **No collaborative-editing-style conflict resolution** (CRDT, OT, etc.). srv is the serializing authority; conflicts are resolved by who-arrived-first ordering at the reducer mutex.
- **No change to the wstore (SQLite) schema for `LayoutState`.** Existing columns suffice. The persisted JSON shape stays compatible with the typed Rust struct via serde.

## 3. Architecture

### 3.1 The two reducers

| Reducer | Where | Owns | Role in E.4.B |
|---|---|---|---|
| **Frontend layout reducer** | `frontend/layout/lib/layoutTree.ts` + `layoutModel.ts` | Local treeState; 15 actions; pure-ish handlers | UNCHANGED. Stays the primary local-write path for optimistic UI. Slice #8 adds an srv-event subscription that applies remote mutations as additional reducer dispatches. |
| **srv reducer** | `agentmux-srv/src/reducer.rs` | Workspaces, tabs, blocks, windows, focus/magnify | EXTENDED. Gains 11 new `Command::Layout*` arms operating on `TabRecord.layout_tree: Option<LayoutNode>` (new field). |

### 3.2 Sync shape (Q2 from prior plan — finalized)

**Optimistic-local-apply with srv serialization.** Each tree mutation:

```
[user clicks pane in window A]
  → A's frontend layoutTree reducer dispatches Move action (LOCAL, sub-ms)
  → A's UI updates immediately (no waiting on srv)
  → A's persistence layer dispatches Command::LayoutMoveNode to srv (debounced 100ms)
  → srv reducer arm validates + applies the command authoritatively
  → srv emits Event::LayoutNodeMoved with version
  → All connected windows (including A) receive the event via wstore subscription
  → Each window's slice-#8 subscriber:
       - if event matches a pending local optimistic-apply → confirm (no-op)
       - if event differs → apply the command to local treeState (granular!)
```

This preserves the optimistic-UI feel (no srv round-trip on user actions) while making srv the source of truth for the persistent tree.

### 3.3 Conflict resolution

Conflicts are resolved at the reducer mutex by arrival order:

1. Window A and B both dispatch overlapping commands (e.g. both delete the same node).
2. srv reducer mutex serializes them — first command applies, second command's invariant check (`find_node_by_id`) fails → second command emits `Event::Error { code: NodeNotFound, fatal: false }` instead of mutating.
3. Both windows receive the success event for command 1 and the error event for command 2; the error window can no-op or display a brief toast.

No CRDT, no rollback. The reducer's "validate then apply" pattern handles this naturally.

### 3.4 Optimistic apply discrepancy detection (slice #8 concern)

When window A applies an optimistic Move locally THEN dispatches to srv THEN receives the matching event back, the slice-#8 subscriber needs to detect "this event corresponds to my own optimistic apply" and no-op. Two options:

- **Option α — Echo-loop guard pattern** (matches `launcher-event-reducer.ts`): set `applyingRemote = true` while dispatching the local `Move` action that came from a remote event; the persistence layer checks this flag and skips the outbound RPC. Simple but doesn't distinguish "my own command echo" from "your command that I should apply."
- **Option β — Command correlation IDs** (matches saga pattern): each outbound command carries a `correlation_id`; events reference it; subscriber checks "did I issue this command?" If yes, no-op (already applied locally); if no, apply.

**Decision: Option β.** Echo-loop guard works for single-window mirror but in multi-window it's ambiguous. Correlation IDs scale cleanly. ~10 LOC overhead per command.

## 4. Data model

### 4.1 The typed `LayoutNode` struct

New types in `agentmux-srv/src/backend/obj.rs` (alongside the existing `LayoutState`):

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutNode {
    /// UUID, generated on creation. Stable for the lifetime of this node.
    pub id: String,
    /// Direction children flow within this node. None for leaves.
    #[serde(rename = "flexDirection", skip_serializing_if = "Option::is_none")]
    pub flex_direction: Option<FlexDirection>,
    /// Flex size; relative units. Default 1.0.
    #[serde(default = "default_size")]
    pub size: f32,
    /// Children nodes. Empty for leaves.
    #[serde(default)]
    pub children: Vec<LayoutNode>,
    /// Leaf-only payload (block reference). None for groups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<LayoutNodeData>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlexDirection { Row, Column }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutNodeData {
    #[serde(rename = "blockId")]
    pub block_id: String,
}

fn default_size() -> f32 { 1.0 }
```

`LayoutState.rootnode` becomes:
```rust
pub rootnode: Option<LayoutNode>,
```

Wire compat: serde-derived (de)serialization produces the same JSON the frontend already consumes (camelCase fields, optional skip-serialize). Round-trip tests against production tree blobs ensure compat (Phase 2 acceptance criterion).

### 4.2 `TabRecord` extension (`agentmux-srv/src/state.rs`)

Add one field:

```rust
pub struct TabRecord {
    // ... existing fields
    /// Phase E.4.B — the layout tree owned by this tab. `None` when the
    /// tab has no panes (initial state or last-pane closed). Bootstrap-
    /// loaded from `LayoutState.rootnode` at startup; mutated by the 11
    /// `Command::Layout*` arms.
    pub layout_tree: Option<LayoutNode>,
}
```

Bootstrap from SQLite: `state.rs` startup load reads `LayoutState.rootnode` per tab and clones into the in-memory `TabRecord.layout_tree`.

### 4.3 Pure helper functions

In `agentmux-srv/src/backend/layout/mod.rs` (new file):

```rust
pub fn find_node_by_id(tree: &LayoutNode, id: &str) -> Option<&LayoutNode>;
pub fn find_node_by_id_mut(tree: &mut LayoutNode, id: &str) -> Option<&mut LayoutNode>;
pub fn find_parent_by_child_id(tree: &mut LayoutNode, child_id: &str) -> Option<&mut LayoutNode>;

pub fn insert_node(tree: &mut LayoutNode, node: LayoutNode);
pub fn insert_node_at_index(tree: &mut LayoutNode, node: LayoutNode, index_arr: &[usize]) -> Result<(), LayoutError>;
pub fn delete_node(tree: &mut LayoutNode, node_id: &str) -> Result<bool, LayoutError>;  // returns whether-was-focused
pub fn move_node(tree: &mut LayoutNode, node_id: &str, new_parent_id: &str, index: usize) -> Result<(), LayoutError>;
pub fn swap_nodes(tree: &mut LayoutNode, node1_id: &str, node2_id: &str) -> Result<(), LayoutError>;
pub fn resize_nodes(tree: &mut LayoutNode, ops: &[ResizeOp]) -> Result<(), LayoutError>;
pub fn replace_node(tree: &mut LayoutNode, target_id: &str, new_node: LayoutNode) -> Result<(), LayoutError>;
pub fn split_horizontal(tree: &mut LayoutNode, target_id: &str, new_node: LayoutNode, position: SplitPosition) -> Result<(), LayoutError>;
pub fn split_vertical(tree: &mut LayoutNode, target_id: &str, new_node: LayoutNode, position: SplitPosition) -> Result<(), LayoutError>;

pub enum LayoutError { NodeNotFound { id: String }, RootCannotBeMagnified, RootCannotBeSwapped, SelfSwap, InvalidSize { size: f32 } }
pub struct ResizeOp { pub node_id: String, pub size: f32 }
pub enum SplitPosition { Before, After }
```

These mirror `frontend/layout/lib/layoutTree.ts` semantics 1:1. Test oracle: port the 30 tests from `frontend/layout/tests/layoutTree.test.ts` (PR #686).

## 5. Command surface

All 11 new commands in `agentmux-common/src/ipc.rs` Command enum:

```rust
LayoutInsertNode {
    tab_id: String,
    node: LayoutNode,
    /// Insert at root (None) or as child of a specific parent (Some).
    parent_id: Option<String>,
    /// Position within parent's children. None = append.
    index: Option<usize>,
    /// If true, sets focused_node_id to this node's id after insert.
    focus_after: bool,
    /// If true, sets magnified_node_id to this node's id after insert.
    magnify_after: bool,
    correlation_id: Uuid,
},
LayoutInsertNodeAtIndex {
    tab_id: String,
    node: LayoutNode,
    /// Index path through the tree. e.g. [0, 2] = root.children[0].children[2].
    index_arr: Vec<usize>,
    focus_after: bool,
    magnify_after: bool,
    correlation_id: Uuid,
},
LayoutDeleteNode {
    tab_id: String,
    node_id: String,
    correlation_id: Uuid,
},
LayoutMoveNode {
    tab_id: String,
    node_id: String,
    new_parent_id: String,
    index: usize,
    correlation_id: Uuid,
},
LayoutSwapNodes {
    tab_id: String,
    node1_id: String,
    node2_id: String,
    correlation_id: Uuid,
},
LayoutResizeNodes {
    tab_id: String,
    ops: Vec<ResizeOp>,
    correlation_id: Uuid,
},
LayoutReplaceNode {
    tab_id: String,
    target_id: String,
    new_node: LayoutNode,
    focus_after: bool,
    correlation_id: Uuid,
},
LayoutSplitHorizontal {
    tab_id: String,
    target_id: String,
    new_node: LayoutNode,
    position: SplitPosition,
    focus_after: bool,
    correlation_id: Uuid,
},
LayoutSplitVertical {
    tab_id: String,
    target_id: String,
    new_node: LayoutNode,
    position: SplitPosition,
    focus_after: bool,
    correlation_id: Uuid,
},
LayoutClear {
    tab_id: String,
    correlation_id: Uuid,
},
LayoutSetTree {
    /// Bulk replace the tree. Used during migration period and for tear-off
    /// where the whole subtree changes atomically.
    tab_id: String,
    new_tree: Option<LayoutNode>,
    correlation_id: Uuid,
},
```

All commands carry `tab_id` (selects the slot) and `correlation_id` (for slice #8's optimistic-confirm logic).

## 6. Event surface

Mirror commands 1:1 in `agentmux-common/src/ipc.rs` Event enum:

```rust
LayoutNodeInserted {
    tab_id: String,
    node: LayoutNode,
    parent_id: Option<String>,
    index: Option<usize>,
    correlation_id: Uuid,
    version: u64,
},
LayoutNodeInsertedAtIndex {
    tab_id: String,
    node: LayoutNode,
    index_arr: Vec<usize>,
    correlation_id: Uuid,
    version: u64,
},
LayoutNodeDeleted {
    tab_id: String,
    node_id: String,
    /// True if the deleted node was the focused one (subscribers may need to refocus).
    was_focused: bool,
    correlation_id: Uuid,
    version: u64,
},
LayoutNodeMoved {
    tab_id: String,
    node_id: String,
    new_parent_id: String,
    index: usize,
    correlation_id: Uuid,
    version: u64,
},
LayoutNodesSwapped {
    tab_id: String,
    node1_id: String,
    node2_id: String,
    correlation_id: Uuid,
    version: u64,
},
LayoutNodesResized {
    tab_id: String,
    ops: Vec<ResizeOp>,
    correlation_id: Uuid,
    version: u64,
},
LayoutNodeReplaced {
    tab_id: String,
    target_id: String,
    new_node: LayoutNode,
    correlation_id: Uuid,
    version: u64,
},
LayoutSplitHorizontal {
    tab_id: String,
    target_id: String,
    new_node: LayoutNode,
    position: SplitPosition,
    correlation_id: Uuid,
    version: u64,
},
LayoutSplitVertical {
    tab_id: String,
    target_id: String,
    new_node: LayoutNode,
    position: SplitPosition,
    correlation_id: Uuid,
    version: u64,
},
LayoutCleared {
    tab_id: String,
    correlation_id: Uuid,
    version: u64,
},
LayoutTreeReplaced {
    tab_id: String,
    new_tree: Option<LayoutNode>,
    correlation_id: Uuid,
    version: u64,
},
```

Errors flow through the existing `Event::Error { code, message, fatal: false, version }` shape — same as E.4.A.

## 7. Reducer arm semantics

Each handler follows the E.4.A template (`reducer.rs:655–700`):

```rust
fn handle_layout_<verb>(state: &mut State, ...args) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error { code: ErrorCode::InvalidCommand, message: format!("LayoutXxx: unknown tab {}", tab_id), fatal: false, version: v }];
    };
    // 1. Validate (find target nodes, check invariants)
    // 2. If invariant violated: return typed Error event (non-fatal)
    // 3. Apply pure helper to tab.layout_tree
    // 4. Update tab.focused_node_id / magnified_node_id if command requested it
    // 5. bump_version
    // 6. Return Vec<Event> with the success event
}
```

### 7.1 Per-handler specifications

#### `handle_layout_insert_node`
- If `tab.layout_tree.is_none()`: validate `parent_id.is_none()` AND `index.is_none()`; promote `node` to root.
- Else: find parent by id (O(n) walk). If `parent_id.is_none()`, use `findNextInsertLocation` heuristic (port from frontend). If parent not found → Error event.
- Insert node at parent.children[index] (or append if index None).
- If `focus_after`: tab.focused_node_id = node.id.
- If `magnify_after`: tab.magnified_node_id = node.id (also focused per cohesion rule).
- Emit `LayoutNodeInserted`.

#### `handle_layout_delete_node`
- Find node by id; not found → Error.
- If node IS root: tab.layout_tree = None.
- Else: find parent, remove from children. Collapse-empty-parents pass.
- Track `was_focused = (tab.focused_node_id == node_id)`. If true: tab.focused_node_id = "" (clear).
- Emit `LayoutNodeDeleted { was_focused }`.

#### `handle_layout_move_node`
- Find node by id; not found → Error.
- Find new_parent by id; not found → Error.
- Detach node from old parent. Insert at new_parent.children[index].
- Edge: moving node into its own descendant → reject (Error: InvalidMove).
- Emit `LayoutNodeMoved`.

#### `handle_layout_swap_nodes`
- Find both nodes; either missing → Error.
- Reject if either is root → Error.
- Reject if same id → Error.
- Swap positions in their respective parents (sizes swap with the nodes — matches existing semantic).
- Emit `LayoutNodesSwapped`.

#### `handle_layout_resize_nodes`
- For each op: validate `0.0 <= size <= 100.0`. If any invalid → Error (and reject the entire op array atomically — match the existing "first invalid op causes early return" semantic from PR #686 tests).
- For each op: find node, set size.
- Emit `LayoutNodesResized`.

#### `handle_layout_replace_node`
- Find target_id; not found → Error.
- If target is root: replace root, preserving size.
- Else: find parent, replace at index, preserving size.
- If `focus_after`: tab.focused_node_id = new_node.id.
- Emit `LayoutNodeReplaced`.

#### `handle_layout_split_horizontal` / `handle_layout_split_vertical`
- Find target; not found → Error.
- If target's parent has matching flex_direction (Row/Column): splice new_node before/after target. (No new group needed.)
- Else: wrap target + new_node in a fresh group node with the requested direction; replace target with the group in the grandparent (or root).
- If `focus_after`: tab.focused_node_id = new_node.id.
- Emit corresponding event.

#### `handle_layout_clear`
- tab.layout_tree = None.
- tab.focused_node_id = ""; tab.magnified_node_id = "".
- Emit `LayoutCleared`.

#### `handle_layout_set_tree` (bulk replace)
- tab.layout_tree = new_tree.
- If new_tree is None: also clear focused/magnified.
- Used by migration period + tear-off (see §10).
- Emit `LayoutTreeReplaced`.

## 8. Persist subscriber arms

For each `Event::Layout*`, an apply function in `persist_subscriber.rs`:

```rust
fn apply_layout_node_inserted(wstore: &WaveStore, tab_id: &str, node: &LayoutNode, parent_id: Option<&str>, index: Option<usize>) -> Result<...> {
    let Some(tab) = wstore.get::<Tab>(tab_id)? else { return Ok(()); };
    if tab.layoutstate.is_empty() { return Ok(()); }
    let Some(mut layout) = wstore.get::<PersistedLayoutState>(&tab.layoutstate)? else { return Ok(()); };
    // Apply the same helper the reducer used — idempotent because the same pure function.
    if layout.rootnode.is_none() {
        layout.rootnode = Some(node.clone());
    } else {
        let mut tree = layout.rootnode.as_mut().unwrap();
        backend::layout::insert_node_at_parent(tree, node.clone(), parent_id, index)?;
    }
    wstore.update(&mut layout)?;
    Ok(())
}
```

Idempotency rationale: same pure helpers as the reducer mean re-applying produces the same result. If wstore state diverges from reducer state (shouldn't happen post-migration), the next persist roundtrips back to the reducer's truth.

11 apply functions, ~25 LOC each = ~280 LOC.

## 9. serde compatibility

The current `rootnode: Option<serde_json::Value>` produces JSON like:

```json
{
  "id": "...",
  "data": { "blockId": "..." },
  "flexDirection": "row",
  "size": 1
}
```

The new typed `LayoutNode` must produce byte-identical JSON to maintain wire compat (frontend consumes the same shape).

**Key serde annotations** (per §4.1):
- `#[serde(rename = "flexDirection", skip_serializing_if = "Option::is_none")]`
- `#[serde(rename = "blockId")]` on `LayoutNodeData.block_id`
- `#[serde(rename_all = "lowercase")]` on `FlexDirection`
- `#[serde(default = "default_size")]` on size to match frontend's "missing → 1.0" default

**Round-trip test** (Phase 2 gate): seed a SQLite DB with 50+ real production layouts; deserialize each into `LayoutNode`; reserialize; compare bytes (modulo whitespace). Difference fails the build.

**Bootstrap migration**: on srv startup, the existing `LayoutState.rootnode: Option<serde_json::Value>` rows already in SQLite deserialize transparently into `Option<LayoutNode>`. No data migration script needed — just a serde-compat win.

## 10. Migration of the 5 existing writers

Before any new commands flow, **each existing writer must dispatch through the reducer** (§7) instead of writing the wstore directly. Done as Phase 7.

| Writer | Today | After migration |
|---|---|---|
| `service.rs:2306` (tear-off) | `layout.rootnode = Some(serde_json::json!({...}))` then `store.update(&mut layout)` | `dispatch(Command::LayoutSetTree { tab_id, new_tree: Some(node), correlation_id })` |
| `dnd.rs:248` (DnD consolidation) | Same direct write | Dispatch `LayoutSetTree` |
| `mod.rs:167` (first launch) | Same | Dispatch `LayoutSetTree` |
| `block.rs:83` (last-pane close) | `layout.rootnode = None` then `store.update` | Dispatch `LayoutClear` |
| Frontend persist via waveObject subscriber (`service.rs:240–290`) | Detects rootnode change → writes blob | Detects rootnode change → diffs against prior tree → dispatches granular `LayoutMoveNode` / `LayoutResizeNodes` / etc. — see §10.1 |

### 10.1 Frontend-flush migration (the trickiest sub-step)

Today the persist subscriber sees `rootnode` changed and writes the blob. Post-E.4.B it needs to translate "the tree changed from old to new" into a sequence of granular commands.

Two options:

**Option X — Diff-based dispatch on srv side.** Subscriber computes a tree-diff (old → new) and emits commands for each delta. Pros: no frontend change. Cons: tree-diff is itself non-trivial (~200 LOC).

**Option Y — Frontend dispatches commands directly.** Refactor `persistToBackend()` to enumerate the actions taken since the last flush (the frontend reducer ALREADY knows what actions ran) and dispatch those commands. Pros: no diff; trivial mapping (frontend action → srv command). Cons: requires frontend persistence-layer rewrite.

**Decision: Y.** The frontend reducer already has the action log; reusing it eliminates the diff. Frontend persistence layer change is ~150 LOC. Phase 7's frontend-side scope.

### 10.2 Migration sequencing within Phase 7

To keep the migration safe:

1. Land the typed tree (Phase 2) — no behavior change.
2. Land the command surface + reducer arms + persist subscribers (Phases 3–6) — commands DEFINED but not DISPATCHED from the existing writers yet. Tests pass; production behavior unchanged.
3. Phase 7a: migrate the 4 wcore-direct writers ONE at a time. Each is a small PR. After each, smoke that path (tear-off / DnD / first-launch / last-pane-close).
4. Phase 7b: migrate the frontend-flush subscriber. Single big PR. Feature flag.
5. Once 7b is stable for 1–2 versions: delete the wcore-direct rootnode write path entirely.

## 11. Tests

### 11.1 Pure helper tests (Phase 4)

Port the 30 tests from `frontend/layout/tests/layoutTree.test.ts` (PR #686) to Rust. Each frontend `test()` becomes a `#[test]` in `agentmux-srv/src/backend/layout/tests.rs`. Same scenarios, same assertions. The frontend test suite is the **behavioral oracle** — Rust pure helpers MUST produce identical state transitions for identical inputs.

### 11.2 Reducer arm tests (Phase 5)

Per command, test:
- Happy path (round-trip: dispatch → state mutated → event emitted with correct fields)
- Unknown tab → Error event, no state change
- Unknown target node → Error event, no state change
- No-op short-circuit where applicable (e.g. setting a value that's already current)
- Invariant violation (e.g. swap-with-root) → Error event
- Correlation ID round-trips into the event

11 commands × ~3 tests each = ~35 reducer tests.

### 11.3 Persist subscriber tests (Phase 6)

Per event:
- SQL round-trip: dispatch event → wstore row updated correctly
- Idempotent re-apply: dispatch same event twice → second is no-op
- Missing tab → silent no-op (matches E.4.A pattern)

11 events × ~3 tests each = ~35 subscriber tests.

### 11.4 Integration tests (Phase 7)

End-to-end through the dispatch + persist path. Frontend-driven scenarios:
- Tear-off path: dispatch from `service.rs:2306` site → reducer updates state → subscriber writes wstore → next read returns the new tree.
- Multi-window mirror: simulate two clients; client A dispatches Move; verify client B receives the event with the same correlation ID and applies it.
- Concurrent splits: clients A and B dispatch Splits to different targets simultaneously; verify both land.

## 12. Frontend slice #8 (Phase 9)

After srv E.4.B is stable in main, slice #8 wires the frontend mirror.

### 12.1 Scope

`frontend/app/store/layout-tree-store.ts` (new module, ~250 LOC):
- Subscribes to wstore `Layout*` events via the existing wstore subscription mechanism.
- For each inbound event:
  - Check correlation ID against `pendingOptimisticDispatches` map (set by frontend reducer's persistence layer when it dispatched this command).
  - If matched (own echo): clear the pending entry and no-op.
  - If unmatched (remote command): dispatch the equivalent local action to `frontend/layout/lib/layoutTree.ts` reducer.
- The local reducer applies the action via the existing pure helpers (no separate code path).

### 12.2 Conflict scenarios

When window B receives a remote event for a node it just optimistically modified locally:

- If B's optimistic modification matched the eventual srv outcome (most common case): no observable conflict.
- If B's modification was rejected by srv (lost the race): B receives no matching success event, but does receive an Error event. B rolls back the optimistic apply.
  - Rollback strategy: keep a small stack of "optimistic deltas" per pending dispatch. On Error, pop the top delta and apply its inverse.

### 12.3 Drag-resize specifically

Drag-resize is the most-frequent multi-window pain point. With slice #8:
- Window A user drags a resize handle.
- Frontend treeReducer applies `SetPendingAction` locally (already happens today).
- On `CommitPendingAction` (mouseup): dispatch `Command::LayoutResizeNodes` to srv.
- srv emits `LayoutNodesResized`.
- Window B receives the event; applies via local reducer; tree updates without re-rendering from blob.
- If window B was mid-drag of a different handle: only the affected nodes update; B's drag state is preserved.

This is the user-visible payoff.

## 13. Phase-by-phase plan (PRs in order)

| # | Phase | LOC | Days | PR scope | Acceptance |
|---|---|---|---|---|---|
| 1 | Spec finalization | (this doc) | 2 | Doc only — this spec | All open questions answered (§14) |
| 2 | Typed `LayoutNode` | ~250 + ~150 tests | 3 | `obj.rs` types + serde impls + round-trip tests against 50+ production blobs | Existing tests still pass; no behavior change |
| 3 | Command + Event types | ~200 | 1 | `agentmux-common/src/ipc.rs` additions; `LayoutError`, `ResizeOp`, `SplitPosition` helpers | Compiles; no runtime effect |
| 4 | Pure helpers in `agentmux-srv/src/backend/layout/` | ~700 + ~400 tests | 4 | New module; 11 helper functions; 30 ported tests from PR #686 | All ported tests pass; identical output to frontend |
| 5 | Reducer arms | ~400 + ~300 tests | 3 | 11 `handle_layout_*` functions + dispatch entries; 35 reducer tests | Each command round-trips; correlation IDs preserved |
| 6 | Persist subscriber arms | ~280 + ~200 tests | 3 | 11 `apply_layout_*` functions; subscriber dispatch; 35 SQL round-trip tests | Idempotent; no schema migration |
| 7a | Migrate 4 wcore-direct writers (one PR each) | ~50 each | 3 | tear-off, DnD, first-launch, last-pane-close — each PR is one writer | Smoke each path; behavior unchanged from user perspective |
| 7b | Frontend-flush migration | ~150 + ~100 tests | 3 | `persistToBackend()` rewrite to dispatch granular commands; feature-flagged | Multi-window smoke shows no clobber |
| 8 | Correctness pass | varies | 3–5 | Bug-fix PRs surfacing from real usage of 7a + 7b | No multi-window state-divergence reports |
| 9 | Frontend slice #8 | ~250 + ~150 tests | 5 | `layout-tree-store.ts` + correlation-ID logic + rollback | Drag-resize in window B not clobbered by window A's flush |

**srv-side total (1–8): 16–22 days = ~3–4 weeks.**
**Frontend (9): ~1 week.**
**Combined: 4–5 weeks of focused work.**

## 14. Open questions — answered

| # | Question | Decision | Rationale |
|---|---|---|---|
| Q1 | Typed tree vs `serde_json::Value`? | Typed (Option A) | Bounded migration cost; safety compounds over 11 handlers |
| Q2 | Sync shape (frontend-waits vs optimistic-local) | Optimistic-local (Shape B) | Preserves UX; reducer value (audit + invariants + mirror) achieved either way |
| Q3 | Error semantics for unknown node-id | Typed `Event::Error { code: NodeNotFound, fatal: false }` | Match E.4.A pattern; subscribers can react |
| Q4 | Migrate `pendingbackendactions`? | No (re-eval after E.4.B soaks) | Likely subsumed by command-level events; defer the call |
| Q5 | Migrate `leaforder`? | No | Derived from rootnode; stays wcore-side |
| Q6 | Feature flag the migration in Phase 7? | Yes; default-on after one minor version | Risk floor for the most-touched UI surface |
| Q7 | Drop frontend's per-action `treeReducer`? | No | Frontend reducer stays as the primary local-write path |
| Q8 | Echo-loop guard vs correlation IDs? | **Correlation IDs** | Multi-window scales cleanly; ~10 LOC overhead per command |
| Q9 | Diff-based subscriber dispatch vs frontend-direct? | **Frontend-direct (Option Y)** | No diff complexity; reuses frontend reducer's action log |

## 15. Risks (concrete)

| Risk | Severity | Mitigation |
|---|---|---|
| serde compat issues with existing JSON shape | Medium | Phase 2 round-trip tests against 50+ production blobs (gate before Phase 3) |
| Subtle behavioral divergence between Rust port and frontend `layoutTree.ts` | Medium-High | Port the 30 PR #686 tests verbatim; run against both implementations during Phase 7 to catch drift |
| Phase 7a breaks tear-off / DnD / first-launch | High | One writer per PR; smoke each before next; keep direct path as fallback during Phase 7a; remove only after Phase 7b stable |
| Phase 7b multi-window flush race | High | Feature flag; rollout to 1 version with default-off; flip to default-on only after 0 regressions |
| Correlation ID overhead pessimizes the dispatch path | Low | UUID generation is sub-µs; not a perf concern |
| Drag-resize edge cases not covered by current frontend tests | Medium | Add multi-window-specific integration tests in Phase 7 |
| Effort estimate underruns reality by 2× | Medium | Plan assumes contiguous focus; real schedule needs 30% buffer |

## 16. Rollout plan

### Phase 7 feature flag

Add `AGENTMUX_LAYOUT_REDUCER_E4B=1` env var. Default-off until proven; default-on after one minor version with no regression reports.

When off: existing wcore-direct writers run; no commands dispatched.
When on: writers dispatch commands; subscriber writes via reducer event path.

### Smoke checklist (Phase 7 readiness)

Before flipping default-on:
- [ ] Tear-off pane to new tab: tree replicated correctly to new window
- [ ] DnD pane within tab: new positioning persists
- [ ] First-launch fresh install: initial three-pane tree appears
- [ ] Close last pane: tree clears
- [ ] Drag-resize across multiple panes
- [ ] Multi-window same-tab: split in window A appears in window B without clobbering window B's drag
- [ ] Two-window race: simultaneous splits both land
- [ ] Restart srv: tree round-trips through SQLite intact

### Rollback strategy

If a critical bug surfaces post-flip:
1. Revert to default-off via env or code change.
2. Wcore-direct writers continue to function (kept as fallback during Phase 7a).
3. Only after E.4.B is stable for 2+ versions does the wcore-direct code get deleted.

## 17. Slice #8 spec teaser

This spec covers srv E.4.B in full. Slice #8 (frontend wire-up) gets its own spec written when E.4.B is in Phase 6 stable. The teaser covers:

- `layout-tree-store.ts` shape
- Correlation ID tracking
- Rollback-on-Error
- Subscription wiring
- Drag-resize specific handling

Estimated frontend spec writing: 1 day, in parallel with srv Phases 5–6.

## 18. What changes about the architecture roadmap

After E.4.B + slice #8 ship:

| # | Slice | Status |
|---|---|---|
| #1 | agent-document | ✅ Shipped |
| #2 | conventions | ✅ Shipped |
| #3 | source-tagging | ✅ Shipped |
| #4 | agent-pane-state | ✅ Shipped |
| #5 / E1 | frontend-layout (refactor-only) | ❌ Cancelled |
| #5 / E2 | frontend-layout + srv sync (focus/magnify) | 🟨 Subsumed by E.4.B + slice #8 |
| #6 | launcher-event convergence | ✅ Shipped |
| #7 | tab-state | ❌ Cancelled |
| **#8** | **pane-tree** | **✅ Shipped (was deferred; unblocked by E.4.B)** |

The frontend reducer migration becomes complete. All slices either shipped or explicitly cancelled with retros.

## 19. Definition of done

E.4.B is "done" when:

1. All 11 commands implemented + tested + dispatched
2. All 5 wcore-direct writers migrated (Phase 7a + 7b)
3. Slice #8 frontend mirror shipped
4. Multi-window drag-resize works without clobber (smoke verified)
5. Concurrent splits in two windows both land (smoke verified)
6. wstore JSON shape unchanged (production layouts still load on old + new)
7. Feature flag flipped to default-on for one minor version with no regressions
8. wcore-direct rootnode write path deleted

## 20. Reading order for implementers

1. This spec (you're here)
2. `docs/specs/SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md` — context for why E.4.B was deferred from E.4.A
3. `agentmux-srv/src/reducer.rs:655–700` — the E.4.A pattern this mirrors
4. `agentmux-srv/src/persist_subscriber.rs:666–707` — the E.4.A persist pattern
5. `frontend/layout/lib/layoutTree.ts` — the helper functions to port
6. `frontend/layout/tests/layoutTree.test.ts` — the test oracle
7. `agentmux-srv/src/state.rs:69–88` — `TabRecord` (will be extended)
