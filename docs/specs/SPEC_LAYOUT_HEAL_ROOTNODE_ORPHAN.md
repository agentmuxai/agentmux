# SPEC: Layout Healer Misses Rootnode-Is-Orphan Case

Status: draft
Date: 2026-04-18
Owner: AgentA
Reported by: user on v0.33.263 (dev) after extended pane open/close cycling
Related: PRs #314/#325 (original layout healing), commit `b0eb316a`
("fix: prune orphaned layout nodes on block delete and tab activation")

## 1. Symptom

> "We still get dead spots on panes."

Clicking on an area where a pane should be does nothing — no focus, no
context menu, no block renders. The layout tree still has a node at
that position, but there is no block backing it. The pane appears as
empty visual space that rejects all interaction.

## 2. Evidence (live DB dump at v0.33.263)

`~/.agentmux-dev` doesn't exist on this machine; the CEF host stores
its data at `%APPDATA%\ai.agentmux.cef.v0-33-263\db\wave.db`.
Inspection of the running dev instance, no extra user interaction:

### `db_tab`
```
oid  = a1575f57-ac83-48ad-800c-8885aa4edff2
name = Untitled1
blockids    = []          ← no blocks in this tab
layoutstate = 2f79da2b-fdc5-4a87-8556-766fed3a258a
```

### `db_block`
```
(zero rows)                ← no blocks exist anywhere
```

### `db_layout` (id = `2f79da2b-…`)
```json
{
  "focusednodeid": "29a6b3bc-d6a8-4c5c-bbbf-3f7d708bdf7e",
  "leaforder": [
    {
      "blockid": "c037a478-6286-4bcb-a5ed-f2cdb50c4ce4",
      "nodeid":  "29a6b3bc-d6a8-4c5c-bbbf-3f7d708bdf7e"
    }
  ],
  "rootnode": {
    "data": { "blockId": "c037a478-6286-4bcb-a5ed-f2cdb50c4ce4" },
    "flexDirection": "row",
    "id": "29a6b3bc-d6a8-4c5c-bbbf-3f7d708bdf7e",
    "size": 10
  }
}
```

Both `leaforder` and `rootnode` point at a block (`c037a478…`) that
does not exist in `db_block`, and is not in `tab.blockids`. The layout
is rendering a phantom pane.

## 3. Why the healer missed it

`heal_layout(store, tab_id)` at
`agentmux-srv/src/backend/wcore/block.rs:122-165`:

1. Reads `tab.blockids` — here `[]`.
2. Builds `valid_blocks = {}` (empty).
3. Scans `layout.leaforder` for entries whose `blockid` is not in
   `valid_blocks` — finds `c037a478-…` as orphan.
4. Logs `"healing layout: removing orphaned block nodes"` (we don't
   see this log because the heal pass likely ran once and the phantom
   persisted — see §4).
5. Calls `prune_block_from_layout(&mut layout, orphan_id)` for each
   orphan.
6. Writes the layout back.

`prune_block_from_layout` at `block.rs:65-74`:
- `leaforder.retain(|e| e.blockid != block_id)` — **OK**, removes the
  leaforder entry.
- `prune_node(root, block_id)` — walks the rootnode tree.

`prune_node` at `block.rs:79-118`:
```rust
fn prune_node(node: &mut serde_json::Value, block_id: &str) {
    if let Some(children) = node.get_mut("children").and_then(|c| c.as_array_mut()) {
        children.retain(|child| {
            // … remove leaf children whose data.blockId == block_id
        });
        for child in children.iter_mut() {
            prune_node(child, block_id);
        }
    }
    // … single-child collapse
}
```

The function only handles the `children` array of split/row/column
nodes. It **never checks whether the node passed in IS ITSELF the
orphan leaf**. In a single-pane layout, the rootnode is the leaf —
no `children` — `prune_node` does nothing, and the orphan remains
in `rootnode.data.blockId` forever.

The next `heal_layout` pass again removes from `leaforder` (already
gone, no-op), tries to prune rootnode (no-op for same reason),
concludes "no orphans in leaforder" (because the first run removed
them), and never converges.

Net effect: a single-pane tab whose block was deleted leaves the
layout rootnode permanently orphaned, and the frontend keeps
rendering a dead space where the pane used to sit.

## 4. Why this wasn't caught earlier

- The L1 tests in #431 / #434 cover `BrowserPaneManager` and
  `PaneStateMachine` — nothing in `wcore::block::heal_layout` has unit
  tests today (`grep -r "heal_layout\|prune_node" --include="*.rs"`
  finds only the implementation, zero tests).
- The original healing PR (`b0eb316a`) was tested on multi-pane
  layouts where the orphan was a CHILD of a split node — the
  `children` pruning path works there.
- "Dead spots" only appear once the user closes the last pane in a
  tab, which isn't a default exercise path.

## 5. Fix

Minimal scope: make `prune_block_from_layout` handle the case where
`rootnode` itself is the orphan leaf.

```rust
fn prune_block_from_layout(layout: &mut LayoutState, block_id: &str) {
    // Prune leaforder
    if let Some(ref mut leaves) = layout.leaforder {
        leaves.retain(|entry| entry.blockid != block_id);
    }
    // Rootnode: if it IS the orphan leaf, drop it. Else walk its tree.
    let root_is_orphan = layout.rootnode
        .as_ref()
        .and_then(|r| r.get("data"))
        .and_then(|d| d.get("blockId"))
        .and_then(|id| id.as_str())
        .map(|id| id == block_id)
        .unwrap_or(false);
    if root_is_orphan {
        layout.rootnode = None;
    } else if let Some(ref mut root) = layout.rootnode {
        prune_node(root, block_id);
    }
}
```

`heal_layout` also needs a companion fix: after pruning, if
`rootnode == None`, `focusednodeid` should be cleared (it points at
the now-gone node):

```rust
if layout.rootnode.is_none() {
    layout.focusednodeid = String::new();
}
```

And for robustness, the healer should ALSO detect "the layout's
rootnode leaf refers to a block_id that's not in tab.blockids" as
an orphan source, not just `leaforder`. In the user's DB, both had
the orphan; but a future divergence between `leaforder` and
`rootnode` would skip past the healer today. Recommended: after the
leaforder pass, walk `rootnode` to collect any remaining leaf block
IDs that aren't in `valid_blocks`, and prune those too.

## 6. Forward repair for already-corrupted DBs

Since the healer's bug has been shipping for weeks, there will be
workspaces with stale orphan rootnodes on disk. Fixes above take
effect on the next `SetActiveTab`, which fires the healer. **No
manual cleanup required**, as long as the user opens a workspace
with a broken tab once after upgrading.

Optional belt-and-suspenders: on startup, walk every tab in the
workspace and run `heal_layout` against each. The code for "heal all
layouts on startup" existed at one point (commit `e33113e5` /
`9435c449`) — check whether it's currently wired in. If not, wiring
it in eliminates the latency of "tab must be clicked for the heal
to apply."

## 7. Tests

Write these *before* the fix, so they fail on current main:

### Rust L1 (`agentmux-srv/src/backend/wcore/block.rs` — new `#[cfg(test)] mod tests`)

| # | Test | Asserts | Catches |
|---|------|---------|---------|
| 1 | `prune_block_from_layout_removes_rootnode_leaf_when_it_is_orphan` | Layout with rootnode-only leaf pointing at orphan; after prune, rootnode=None | THE BUG |
| 2 | `prune_block_from_layout_removes_child_leaf` | Layout with row-of-two; after prune of one child, rootnode collapses to the remaining leaf | Regression of existing behavior |
| 3 | `prune_block_from_layout_noop_when_block_absent` | Layout with no matching orphan; rootnode and leaforder unchanged | Idempotency |
| 4 | `heal_layout_clears_focused_nodeid_when_rootnode_drops` | Layout with orphan rootnode + focusednodeid pointing at it; after heal, focusednodeid is empty string | Consequence of §5 companion fix |
| 5 | `heal_layout_catches_rootnode_orphan_missing_from_leaforder` | Layout where leaforder is clean but rootnode leaf is an orphan (malformed save); healer should still prune | Robustness |
| 6 | `heal_layout_idempotent` | Run heal twice; second run reports no orphans | Prevents log spam / infinite pass |

### Manual verification

- Open a fresh workspace, create one pane, close it. Check
  `wave.db`: `rootnode` should be `null` or absent, `blockids` empty,
  `leaforder` empty, `focusednodeid` empty.
- Create two panes, close one. `rootnode` collapses to the remaining
  leaf. Other pane continues to work.
- Start from an already-corrupted workspace (the one in §2 is
  captured): after the fix and `task dev`, clicking the tab or
  performing any interaction that fires `SetActiveTab` should heal
  the layout and the dead space disappears.

## 8. Not in scope

- Broader refactor of `prune_node` into a typed AST (it operates on
  `serde_json::Value` today). A proper `LayoutNode` enum would let
  the type system catch this class of bug; out of scope for an
  urgent fix.
- Frontend-side defensive rendering (when block lookup fails, render
  a placeholder with "block missing" error). Useful long-term, but
  the DB-side fix removes the cause.
- Renaming `leaforder` / `focusednodeid` etc. to something
  consistent with the rest of the codebase. Separate cleanup.

## 9. Delivery

1. Write the six tests first. Tests #1, #4, #5 will fail on main.
2. Apply the two fixes in `prune_block_from_layout` and
   `heal_layout`.
3. Tests go green.
4. Spot-check the running dev DB — the orphan should get pruned the
   first time `SetActiveTab` fires post-upgrade.
5. Document: tiny CHANGELOG entry, no user-visible breakage.
