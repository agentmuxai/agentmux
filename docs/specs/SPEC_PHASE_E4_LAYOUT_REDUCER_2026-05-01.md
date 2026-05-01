# SPEC: Phase E.4 — Layout reducer migration

**Date:** 2026-05-01
**Status:** Draft — granularity decision pending review.
**Author:** AgentA
**Reads-this-first:**
- `docs/retro/next-steps-architecture-completeness-2026-05-01.md` — step 3.
- `docs/retro/reducer-architecture-gaps-2026-05-01.md` — §1 E.4 entry, §4 `handle_move_tab` migration tolerance.
- `docs/specs/SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` — original Phase E spec.

---

## 0. The decision this spec answers

**Question:** at what granularity should layout state route through the srv reducer?

**Answer:** ship Option A first (focused/magnified only), defer the rootnode/leaforder decision to a Phase E.4.B review after Option A has soaked.

The rest of this spec exists to make that decision concrete and justify the deferral.

---

## 1. State inventory — the layout fields

Source of truth: `agentmux-srv/src/backend/obj.rs::LayoutState` (lines 391-405).

| Field | Type | Mutation cadence | Reducer fit |
|---|---|---|---|
| `rootnode` | `Option<serde_json::Value>` | Every drag-resize tick (potentially 60Hz) | **Bad** — opaque JSON tree, no semantic command surface. |
| `magnifiednodeid` | `String` | Rare — toggle-magnify keystroke | **Good** — single-field set. |
| `focusednodeid` | `String` | Frequent — pane-click + tab-switch | **Good** — single-field set. |
| `leaforder` | `Option<Vec<LeafOrderEntry>>` | Per pane add/remove + reorder | **Medium** — list ops; needs node-path representation. |
| `pendingbackendactions` | `Option<Vec<LayoutActionData>>` | Queue of pending ops; drained by frontend | **Medium** — list ops; producer/consumer. |
| `meta` | `Option<...>` | Misc; reducer already passes through | Stays on existing meta path. |

**The core problem:** `rootnode` is the heaviest mutator. Its current shape is the frontend's serialized tree; it has no field-level semantic commands. If we route it through the reducer naively (one command per `rootnode` write), every drag-resize tick allocates a JSON blob, hits the mutex, and emits an event — that's 60Hz of churn for what is logically *one ongoing user gesture*.

The rest of the fields fit the reducer cleanly. The decision is whether to split rootnode out as a special case or to design a node-path representation that makes rootnode reducer-shaped.

---

## 2. Option A — Minimal slice (focused/magnified) ✓ recommended

**Scope:**
- Reducer commands: `SetFocusedNode { tab_id, node_id }`, `SetMagnifiedNode { tab_id, node_id }`.
- Reducer events: `FocusedNodeChanged { tab_id, node_id, version }`, `MagnifiedNodeChanged { tab_id, node_id, version }`.
- Persist subscriber writes the field in SQLite within the existing `LayoutState` row.
- `rootnode` / `leaforder` / `pendingbackendactions` keep their current wcore-direct paths.

**LOC estimate:** ~600 LOC (reducer arms + dispatch wiring + persist subscriber + 2 frontend send-sites).

**PR breakdown (tightened to 2):**
- **PR 1** — Reducer arms + persist subscriber + frontend wiring for focused/magnified. ~500 LOC.
- **PR 2** — `handle_move_tab` strict-mode flip (drop migration tolerance, reinstate workspace_id check). ~100 LOC. **Depends on:** every tab being known to the reducer at boot — Option A by itself doesn't guarantee this; we additionally tighten `handle_move_tab` only after Option A has soaked for one minor version.

**Why this slice:**
- Closes gap §1 E.4 (layout migration started) and gap §4 (`handle_move_tab` tolerance).
- Doesn't require designing the node-path representation (Option B's hard part).
- High-cadence mutators (`rootnode`) stay on their fast path until we have data on whether routing them through the reducer is actually expensive.
- Each follow-up (rootnode, leaforder) becomes its own scoped PR with its own measurement.

**Why not more:**
- Option B's node-path work is non-trivial and currently has no consumer demanding it. Speculative complexity.
- The remaining wcore-direct paths in `setup_torn_off_block_layout` and `queue_source_layout_delete` are bounded — they're not growing — and are scheduled for re-examination during step 6 (F.5, F.6 sagas).

---

## 3. Option B — Full layout migration

**Scope (in addition to Option A):**
- Node-path representation: a stable address for any node in the layout tree (e.g. `Vec<NodeStep>` where each step is "child N of magnify boundary K").
- Reducer commands: `PatchNodeSize { tab_id, node_path, axis, fraction }`, `InsertNode { tab_id, parent_path, kind, position }`, `RemoveNode { tab_id, node_path }`, `MoveNode { tab_id, from_path, to_path }`, `SetLeafOrder { tab_id, leaf_id, order }`, `EnqueueBackendAction { tab_id, action }`, `DequeueBackendAction { tab_id, action_id }`.
- Frontend serializer: emit node-path commands instead of `rootnode` blob writes.
- Reducer projector: rebuild `rootnode` JSON from the in-memory tree on demand (for SQLite + frontend snapshots).

**LOC estimate:** ~1500 LOC additional (~2100 total).

**Why deferred:**
1. **No consumer demands it today.** The drag-resize 60Hz problem is not actually visible — the writes are batched in the frontend and flushed at gesture-end. Routing through the reducer would *add* the per-tick cost.
2. **The node-path representation is a real design effort.** Get it wrong and every mutation site has to be rewritten.
3. **Phase G (event-sourced) will revisit this.** If the layout tree becomes event-sourced, the node-path question gets re-answered in that context.

**When to revive:** if we ship a feature that needs to atomically inspect+mutate the layout tree from the saga coordinator (e.g. "tear-off-tab while preserving exact pane sizes, derived from current rootnode state"), Option B becomes load-bearing.

---

## 4. Reducer arms (Option A only)

```rust
// agentmux-srv/src/reducer.rs

pub fn handle_set_focused_node(
    state: &mut State,
    tab_id: &str,
    node_id: &str,
) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(tab_id) else {
        return vec![Event::Error {
            message: format!("set_focused_node: unknown tab {tab_id}"),
            ..Default::default()
        }];
    };
    if tab.layout.focused_node_id == node_id {
        return vec![]; // no-op short-circuit
    }
    tab.layout.focused_node_id = node_id.to_string();
    let v = state.bump_version();
    vec![Event::FocusedNodeChanged {
        tab_id: tab_id.to_string(),
        node_id: node_id.to_string(),
        version: v,
    }]
}

pub fn handle_set_magnified_node(
    state: &mut State,
    tab_id: &str,
    node_id: &str,
) -> Vec<Event> {
    // ... identical shape; toggle/clear semantics handled by node_id == "" case
}
```

Invariants enforced:
- Tab must exist (errors otherwise).
- No-op short-circuit when value unchanged (drops a zero-info event).
- Version bumped only on real change.

---

## 5. Persist subscriber

Existing pattern from F1.A: subscriber consumes the new events and writes to `LayoutState.focusednodeid` / `LayoutState.magnifiednodeid` in the same SQLite transaction as other tab-scoped writes.

Required change: extend `apply_event_to_wstore` in `persist_subscriber.rs` to handle the two new event variants. Each is a single-field `UPDATE LayoutState SET ... WHERE oid = (SELECT layoutstate FROM Tab WHERE oid = ?)`.

No migration required — the columns already exist (lines 397-399 of `obj.rs`).

---

## 6. Frontend wiring

### 6.1 Send sites to convert

```
frontend/layout/layoutModel.ts — setFocusedNode, setMagnifiedNode
frontend/layout/lib/utils.ts — selectNode handlers
```

Today these call `LayoutSetFocusedNode` / `LayoutSetMagnifiedNode` RPCs that hit wcore-direct paths in `service.rs`.

After Option A: those RPCs route through `dispatch_to_reducer(SetFocusedNode { ... })`. Frontend code unchanged; the routing change is server-side only.

### 6.2 Event consumption

The renderer's E.6 multi-source dispatcher (step 2 of the architecture-completeness plan) consumes `FocusedNodeChanged` / `MagnifiedNodeChanged` and updates the corresponding atoms.

If E.6 hasn't shipped yet when E.4 PR 1 lands, the events are emitted but ignored by the frontend — the frontend stays on its existing direct-state-update path. No regression because the reducer is now the source of truth and the wstore reads will match.

---

## 7. The `handle_move_tab` strict-mode flip

Today (`reducer.rs`):
```rust
// Migration tolerance — lazy-import unknown tabs from SQLite.
// Drops the workspace_id check during migration window.
```

After Option A soaks, PR 2 of step 3:
```rust
// Strict mode — every tab must be present in state.tabs at boot.
// Workspace_id check reinstated.
```

**Pre-flip checklist:**
- [ ] Option A PR has soaked for one minor version.
- [ ] Smoke logs show no `lazy-import` warnings during normal operation.
- [ ] Crash dumps from this period show no orphan-tab patterns.

If any check fails, the flip waits. Migration tolerance is annoying-but-safe; tightening prematurely is a regression.

---

## 8. Risk register

| Risk | Severity | Mitigation |
|---|---|---|
| Option A breaks tab-switch focus restore (focusednodeid races between reducer and wcore) | Medium | Smoke test: open 4 tabs, switch back and forth, verify focused pane preserved per-tab. Already part of existing smoke suite. |
| Magnify-toggle flickers under fast keystrokes | Low | No-op short-circuit prevents the worst case; if it surfaces, add a 16ms debounce on the frontend send site. |
| `handle_move_tab` strict flip catches a real bootstrap edge case | Medium | The pre-flip checklist exists for this reason. Flip is reversible. |
| Option A PR scope creeps into Option B territory | Medium | Reviewers should reject any node-path representation work in Option A PRs; that's Phase E.4.B's domain. |

---

## 9. Exit criteria for step 3

- PR 1 landed: focused/magnified mutations route through reducer; SQLite writes through persist subscriber; events emitted on srv pipe.
- PR 2 landed: `handle_move_tab` strict mode reinstated; lazy-import code path removed.
- Gap §1 E.4 entry: marked partial-closed (Option A done; Option B deferred with clear "when to revive" criteria).
- Gap §4 `handle_move_tab` tolerance: closed.

---

## 10. What this spec does NOT close

- **Option B (full layout).** Explicitly deferred per §3.
- **Layout commands during sagas.** Today no saga touches layout state. If F.5 / F.6 (step 6) need to, that PR adds the relevant arms.
- **Layout history / undo.** Out of scope; not on any phase's checklist.
- **Phase G event-sourced layout.** Explicitly Phase G's domain.

---

## 11. Cross-references

- `next-steps-architecture-completeness-2026-05-01.md` — the plan calling this spec.
- `reducer-architecture-gaps-2026-05-01.md` — gap §1 E.4 + gap §4.
- `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §6.4 — original layout migration sketch (superseded by this spec).
- `SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` — sibling reducer spec; same testing infra applies.
