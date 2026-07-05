# SPEC — Strong Reducer-Authority for Layout (Intent-Driven srv Reducer)

**Date:** 2026-06-30
**Type:** Implementation spec (full analysis)
**Status:** Ready to schedule
**Owner:** asaf
**Supersedes scope of:** `SPEC_864_LAYOUT_SINGLE_WRITER_2026_06_30.md` (that spec's "weak" single-writer
becomes Phase 1 here; this spec is the committed end-goal)
**Depends on / relates to:** #864 (retire wcore-direct), Pillar 1 host-reproject
(`SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md`), the authority principle in
`DISCUSSION_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_06_29.md` §7a.

> **Goal:** make the **srv reducer the sole authority over layout** — the frontend sends *intents*
> ("split node X horizontally", "move A into B", "resize these"), the reducer **computes** the
> resulting tree, enforces all invariants, persists via one write path, and projects the result back.
> Layout *logic* leaves the disposable renderer and lives in the durable tier, where it belongs.

---

> **Scope note (2026-07-02):** this strong-authority work (frontend sends *intents*; layout algebra in srv)
> is **NOT strictly required for the disposable host (Pillar 1)**. Pillar 1 needs only a *coherent single
> writer* for `db_layout`, which the **weak cutover** achieves — route the frontend's full-tree push through
> `LayoutSetTree` + retire wcore-direct (see `SPEC_864_LAYOUT_SINGLE_WRITER` and
> `DISCUSSION_LIFECYCLE_AND_CRASH_ARCHITECTURE` §7b). This spec is the architectural-purity target (moving
> layout *logic* out of the disposable renderer); pursue it above-and-beyond disposability, not as a Pillar-1
> blocker.

## 0. TL;DR — the decision and the surprise

**Decision (asaf, 2026-06-30):** reducer-as-authority is **critical, not optional**. Weak authority
(frontend computes the tree, reducer rubber-stamps it via `LayoutSetTree`) leaves the layout algebra
in the disposable renderer — the same smell as the host-reducer problem, one layer up. We commit to
**strong** authority: the reducer owns the algebra.

**The surprise (from the source audit):** this is ~70% already built and dormant.
- The **IPC command surface already exists** — all 11 layout intents are defined in
  `agentmux-common/src/ipc.rs` (`LayoutMoveNode`, `LayoutSwapNodes`, `LayoutResizeNodes`,
  `LayoutReplaceNode`, `LayoutSplitHorizontal/Vertical`, `LayoutInsertNodeAtIndex`,
  `LayoutInsertNode`, `LayoutDeleteNode`, `LayoutClear`, `LayoutSetTree`) — `ipc.rs:506-588`.
- The **pure tree algebra is already ported to Rust**, with a 30-test oracle against the frontend
  (`backend/layout/mod.rs`): `insert_node`, `insert_node_at_index`, `delete_node`, `move_node`,
  `swap_nodes`, `resize_nodes`, `replace_node`, `split_horizontal`, `split_vertical`,
  `clear_tree_node` + traversal helpers. Intent helper types `ResizeOp`/`SplitPosition` exist in
  common (`layout_types.rs:98-111`).
- Only **4 of 11 reducer arms are wired** (`reducer/layout.rs`: clear/set_tree/insert/delete; the
  "4 of 11 shipped" comment at `reducer.rs:86-91`). The other 7 commands are defined but **un-dispatched**.

**What's genuinely missing** (the real work):
1. **`balanceNode` is not ported to Rust** — the tree-normalization keystone. Only
   `reverse_flex_direction` exists; no single-child hoist, no leaf-collapse, no `validateNode`, no
   `_slipAnchor` exemption. **Without it the reducer can't be authoritative** — it would produce
   un-normalized trees the frontend would then "correct," reintroducing a second writer.
2. **Wire the remaining 7 reducer arms** to the existing pure fns + emit events.
3. **The frontend must send intents, not full trees** — flip `persistToBackend`'s full-tree
   `UpdateObject` push to per-action intent RPCs, and apply results via the existing
   `pendingbackendactions` channel.
4. **The minimize/slip/dissolve algebra** (`layoutMinimize.ts`, ~500 lines, depends on **render
   pixel ratios**) — the hard part; needs a logical/geometric split (§6).
5. **Single write path + coherence-by-construction** (the #864 work: reroute wcore-direct callers;
   CAS + derived `leaforder` as defense-in-depth).

---

## 1. Current architecture (verified)

### 1.1 Where the algebra lives today — the frontend
`frontend/layout/lib/`: pure tree fns in `layoutTree.ts` (13 reducers) + `layoutNode.ts` (primitives
+ `balanceNode`); the stateful dispatcher `LayoutModel.treeReducer` (`layoutModel.ts:522`); minimize
in `layoutMinimize.ts`; persistence in `layoutPersistence.ts`.

**Flow today (full-tree push):** user action → app builds an action object → `treeReducer` mutates
`rootNode` in place → `updateTree()` runs `balanceNode` → `persistToBackend()`
(`layoutPersistence.ts:281-299`, debounced 100ms) copies the **entire tree** onto the `LayoutState`
WaveObject → `ObjectService.UpdateObject(value)` (`wos.ts:312`). The frontend pushes the *computed
result*, through the generic object RPC. There is **no** frontend→srv layout-intent RPC for in-tab ops.

**Backend→frontend channel (already intent-shaped):** `pendingbackendactions` on the same
`LayoutState`; the frontend's `onBackendUpdate` → `processPendingBackendActions` →
`handleBackendAction` (`layoutPersistence.ts:52-275`) dedupes by `actionid` and applies each via
`treeReducer`. Seven action types already have a wire-intent form (`LayoutActionData`,
`gotypes.d.ts:1088`) — but only backend→frontend, not the reverse.

### 1.2 Where the algebra already exists — Rust srv
- **Commands:** all 11, `ipc.rs:506-588`. **Events:** partial — `LayoutSplitHorizontalApplied`,
  `LayoutSplitVerticalApplied`, `LayoutCleared`, … (`ipc.rs:1462+`); the rest need adding.
- **Pure fns:** `backend/layout/mod.rs` — full set, oracle-tested against
  `frontend/layout/tests/layoutTree.test.ts` (30 tests, PR #686). Handles `extra` catch-all
  preservation, single-child collapse on delete, ancestor/descendant guards on move/swap, atomic
  resize validation.
- **Wired arms:** `reducer/layout.rs` — `handle_set_focused_node`, `handle_set_magnified_node`,
  `handle_layout_clear`, `handle_layout_set_tree`, `handle_layout_insert_node`,
  `handle_layout_delete_node`. The insert/delete arms already do focus/magnify reconciliation
  correctly (e.g. delete walks the post-delete tree to clear orphaned focus ids,
  `reducer/layout.rs:296-312`).
- **Shared type:** `LayoutNode` (`layout_types.rs:55-75`) — `id`, `flex_direction`, `size`,
  `children`, `data`, and a `#[serde(flatten)] extra` catch-all. **The minimize fields
  (`minimizedSize`, `slipMinimize`, `columnDissolve`, `_slipAnchor`) are NOT modeled** — they
  currently round-trip through `extra` untyped, so the Rust reducer is blind to them today.

### 1.3 The gap, exactly
The reducer can already *compute* every structural operation. It cannot yet be *authoritative*
because: (a) nothing normalizes its output (`balanceNode` missing), (b) 7 arms are unwired, (c) the
frontend doesn't send intents, and (d) minimize/dissolve is geometry-coupled and unmodeled.

---

## 2. Target architecture — intent in, normalized tree out, projected back

```
 user action (split/move/resize/minimize/…)
        │  frontend builds an INTENT (no local tree mutation)
        ▼
 frontend → srv RPC:  Layout<Intent> { tab_id, target ids, params, correlation_id }
        │
        ▼
 srv REDUCER (sole authority)
   1. resolve targets against tab.rootnode
   2. call the pure algebra fn (backend/layout/mod.rs)
   3. balance_node()  ← NEW: enforce normalization invariants
   4. reconcile focus/magnify
   5. emit Layout<Intent>Applied event (carries new tree or a structural delta)
        │
        ├─► persist subscriber  → ONE write to db_layout (rootnode+leaforder), version-CAS
        └─► wave_obj_bridge      → waveobj:update / pendingbackendactions → frontend projects result
        │
        ▼
 frontend RENDERER (disposable projection)
   - applies the result (already supported via handleBackendAction)
   - derives PIXEL geometry locally (sizes, minimize realization)
```

**Invariants of the target state:**
- The reducer is the **only** producer of layout trees. The frontend never computes a persisted tree.
- The persist subscriber is the **only** writer of `db_layout` (kills the double-write + wcore-direct).
- The renderer holds only **derived/geometric** state it can recompute on reproject (pixel sizes,
  minimize realization) — consistent with Pillar 1 disposability.

---

## 3. The `balanceNode` port (the keystone — §new Rust work)

The frontend runs `balanceNode` (`layoutNode.ts:200-232`) after **every** committed action via
`updateTree(balanceTree=true)`. The Rust reducer must run an equivalent `balance_node` at the end of
every structural arm, or it cannot own normalized output. The six invariants to port (oracle:
`frontend/layout/tests/` balance tests + the live `balanceNode`):

1. **Direction alternation** — any child with `flex_direction == parent.flex_direction` is flipped to
   the reverse. Sibling axes alternate Row/Column down the tree.
2. **Single-child-branch hoisting** — if a child has exactly one child that is itself a branch, hoist
   the grandchild's children up one level — **unless** the node carries `_slipAnchor` (the explicit
   opt-out for slip/dissolve subtrees).
3. **Empty-children pruning** — drop any node whose `children` is empty (and has no `data`).
4. **Undefined filtering** — compact the children vec after pruning.
5. **Single-child-leaf collapse** — a node with exactly one leaf child *absorbs* that child's `data`
   and `id`, dropping `children` (branch-with-one-leaf becomes the leaf). NB: `delete_recursive`
   already does a localized version of this (`backend/layout/mod.rs:273-281`) — `balance_node`
   generalizes it to the whole tree post-action.
6. **`validate_node` (leaf-XOR-branch)** — every node has *either* `data` or `children`, never both,
   never neither; no empty `children`. The Rust port should return a `LayoutError` (not panic) so a
   malformed intent is rejected with an `Event::Error`, not a crash.

**Critical detail — `_slipAnchor` and the minimize fields:** invariant #2's exemption keys off
`_slipAnchor`, which lives in `extra` today. To port balance faithfully the reducer must *read*
`_slipAnchor` — so either (a) promote `_slipAnchor` (and the other minimize fields) to typed
`LayoutNode` fields, or (b) have `balance_node` peek into `extra`. **Recommend (a)** — model the
minimize fields explicitly (see §6); untyped `extra`-peeking is the kind of by-convention coupling
this whole program is trying to delete.

**Test strategy:** the frontend `balanceNode` tests become the Rust oracle (same input tree → same
normalized output). Add a differential test harness: feed N random trees through both the TS and Rust
`balance` and assert byte-equal JSON (the `serialize_size_smallest` shim already guarantees integer
size round-trips, `layout_types.rs:81-93`).

---

## 4. The intent protocol (frontend → srv)

### 4.1 Commands — already defined, need RPC surfacing
All 11 exist in `ipc.rs`. The work is a thin RPC entrypoint that accepts a layout intent and dispatches
it to the reducer (mirroring how `UpdateObject` currently dispatches `SetFocusedNode`). Each intent
carries `tab_id`, the relevant target node id(s), params, and a `correlation_id` (for the frontend to
match the echoed result and dedupe — the `processedActionIds` path already exists,
`layoutPersistence.ts:74-98`).

### 4.2 Per-action migration table (the full surface)
| User action | Today | Target intent | Pure fn (exists) | Notes |
|---|---|---|---|---|
| Split H/V (Cmd+D / chord / menu) | local + full push | `LayoutSplitHorizontal/Vertical` | `split_horizontal/vertical` ✅ | events exist (`*Applied`) |
| Drag-to-dock move | `computeMoveNode`→`moveNode`, full push | `LayoutMoveNode` | `move_node` ✅ | drag *preview* stays frontend (ephemeral); only the **commit** is an intent |
| Swap (center drop) | local, full push | `LayoutSwapNodes` | `swap_nodes` ✅ | |
| Resize divider | `onResizeEnd`, full push | `LayoutResizeNodes` | `resize_nodes` ✅ | send on commit (drag-end), not per-mousemove |
| New pane / ephemeral | local `InsertNode`, full push | `LayoutInsertNode` | `insert_node` ✅ | arm wired |
| Insert at index | backend-only today | `LayoutInsertNodeAtIndex` | `insert_node_at_index` ✅ | |
| Replace | local, full push | `LayoutReplaceNode` | `replace_node` ✅ | |
| Close pane | local `DeleteNode` + `DeleteBlock` | `LayoutDeleteNode` (+ block delete) | `delete_node` ✅ | arm wired; keep block-destroy ordering |
| Magnify / Focus | local toggle, full push | `LayoutSetMagnifiedNode`/`SetFocusedNode` | n/a | arms wired |
| Clear | backend-only | `LayoutClear` | n/a | arm wired |
| **Minimize / slip / dissolve** | **entirely frontend, pixel-coupled** | **new intents (§6)** | **not ported** | the hard part |

### 4.3 Drag preview stays local (important)
Drag *preview* (`computeMoveNode` running on every dragover) must remain renderer-local for latency —
it's ephemeral view state, not a mutation. Only the **drop commit** becomes a `LayoutMoveNode` intent.
This respects the §7a rule: the renderer may run real-time coordination as long as the *authoritative*
mutation goes through the reducer.

---

## 5. Single write path + coherence-by-construction (the #864 fold-in)

This spec absorbs `SPEC_864_LAYOUT_SINGLE_WRITER`. Once intents flow through the reducer:
- **Persist subscriber becomes the only `db_layout` writer.** Add `apply_layout_*` arms for every
  layout event (`persist_subscriber.rs:187-298`) + bridge broadcast arms
  (`wave_obj_bridge.rs:440-443`). Reroute the ~9 wcore-direct callers (the `UpdateObject` raw-tree
  write `service.rs:495`, seeders, tear-off, heal, prune, redock queues) to dispatch intents.
- **Defense-in-depth (mechanical coherence):**
  1. Make `version` a **real CAS** — `store.rs:394` currently does `SET version = version + 1` with
     no `WHERE version = ?` guard (verified): it's a blind counter mislabeled "optimistic locking."
     Add the guard + reject/retry. Benefits every otype.
  2. Make `leaforder` a **pure derivation of `rootnode`**, computed once in the subscriber on write —
     drift becomes impossible by construction, removing the main reason `heal_layout` exists.
- These two are *necessary but not sufficient* (they fix mechanical coherence, not authority) — they
  sit **beneath** the single writer, never instead of it.

---

## 6. The hard part — minimize / slip / dissolve (`layoutMinimize.ts`)

This is the highest-risk surface and deserves its own design pass; this spec scopes the problem and
sets direction, but flags the sub-design as open.

**Why it's hard:** `layoutMinimize.ts` (~500 lines) is stateful on `LayoutModel`, has three paths
(Column-collapse → `minimizedSize`; Row-slip → `slipMinimize` + `_slipAnchor`; column-dissolve →
`columnDissolve`, which **restructures** the tree by re-inserting a column atop an adjacent one), a
cascade undissolve (A→B→C), and — critically — it reads **render pixel ratios**
(`pixelToSizeRatio`, `rect.height`, `MinNodeSizePx=40`) to compute the collapsed sizes. srv has no
pixel geometry.

**Recommended direction — split logical state from geometric realization:**
- **Authoritative (srv reducer):** the *logical* facts —
  - *which* nodes are minimized (a boolean/flag per node),
  - the *structural* dissolve relationships (`columnDissolve` is a real topology change → must be a
    reducer intent: `LayoutDissolveColumn` / `LayoutUndissolveColumn` computing the restructure),
  - the slip linkage (`slipMinimize` target column).
  These become **typed `LayoutNode` fields** (not `extra`), so `balance_node` and reproject see them.
- **Derived (renderer):** the *pixel sizes* a minimized/slipped node collapses to. The renderer
  recomputes these from its current viewport on each layout and on reproject — exactly the
  disposable-projection model. srv stays geometry-free.

**The subtlety to resolve in the sub-design:** today the size value and the restructure are computed
together. Separating them means the reducer performs the *restructure* with size values expressed in
**flex units / conventions** (e.g. a collapsed leaf gets a small fixed flex unit, sibling absorbs the
remainder), and the renderer maps that to pixels. This will not be byte-identical to today's
pixel-exact behavior — acceptable if the visual result matches within tolerance, but it needs its own
spec + visual verification. **Until that lands, minimize/dissolve can remain a frontend-computed
operation that still routes its *result* through `LayoutSetTree`** (weak authority for this one
operation) so it doesn't block the rest — explicitly the one place we tolerate weak authority
temporarily, logged as debt.

---

## 7. Phased plan

**Phase 1 — single writer (weak authority), low risk.** Land `SPEC_864` Phase 1–2: persist-subscriber
+ bridge arms for the 4 existing events; reroute `UpdateObject` to `LayoutSetTree`; add CAS +
derived-leaforder. *Outcome:* one write path, double-write gone, coherence-by-construction — but the
frontend still computes trees. Behavior-neutral, fully unit-testable. **This is the stepping stone.**

**Phase 2 — port `balance_node` to Rust (the keystone).** Implement + differential-test against the
TS oracle. No behavior change yet (not called on the hot path until Phase 3), but it unblocks
authority. *Gated on the differential harness passing.*

**Phase 3 — wire the 7 remaining reducer arms.** `LayoutMove/Swap/Resize/Replace/SplitH/SplitV/
InsertAtIndex` → existing pure fns → `balance_node` → emit event → subscriber persists. Add the
missing `*Applied` events. *Unit-tested per arm against the oracle.*

**Phase 4 — flip the frontend to intents (strong authority).** Per the §4.2 table, replace local
`treeReducer` + full-push with intent RPCs; apply echoed results via the existing
`handleBackendAction`/`pendingbackendactions` path; keep drag-preview local. **This is the
behavior-changing core — needs app-running E2E verification (split/move/resize/swap/insert/delete/
replace all round-trip through srv). Do NOT auto-merge on bot approval.**

**Phase 5 — model the minimize fields + dissolve intents** (§6). Promote `minimizedSize`/
`slipMinimize`/`columnDissolve`/`_slipAnchor` to typed fields; add `LayoutDissolveColumn`/
`Undissolve` intents with the logical/geometric split. *Own sub-spec + visual verification.* Until
done, minimize routes its result via `LayoutSetTree` (tracked debt).

**Phase 6 — delete the backstops.** Remove `heal_layout` + callers, the relaxed `reorder_tabs_bulk`
validation + its migration test, the resync layout carve-out. The payoff deletion.

---

## 8. Test strategy
- **Oracle parity:** the 30 `layoutTree.test.ts` cases + the `balanceNode` tests are the cross-language
  oracle. Rust arms must reproduce TS state transitions byte-for-byte (JSON-equal).
- **Differential fuzz:** random tree generator → run identical intent through TS and Rust → assert
  equal normalized output. The keystone safety net for `balance_node`.
- **Reducer unit tests:** per arm — happy path, target-not-found → `Event::Error` (not panic),
  focus/magnify reconciliation, root-as-target guards.
- **E2E (Phase 4+):** "every in-tab layout action mutates only via srv"; "close last pane ⇒ tree
  exits" (ties Pillar 2); "reproject restores identical topology" (ties Pillar 1).
- **Coherence:** assert exactly one `db_layout` write + one version bump per intent; assert
  `TabRecord.rootnode == db_layout.rootnode` mid-session (the new invariant).

---

## 9. Risks / honest caveats
- **`balance_node` fidelity is make-or-break.** If the Rust normalization diverges from TS even
  slightly, the frontend will "fix" the tree on receipt and push it back → a second writer returns
  silently. The differential fuzz harness is mandatory, not optional.
- **Minimize/dissolve geometry coupling (§6) is genuinely unsolved here** — scoped, directioned, but
  needs its own spec + visual verification. Do not let it block Phases 1–4; carry it as explicit debt
  with the `LayoutSetTree` escape hatch.
- **Latency:** every layout action becomes a round-trip to srv. Resize especially must send on
  drag-*end*, not per-mousemove, and drag-preview must stay local, or interaction will feel laggy.
  The 100ms debounce on the current push is the budget to respect.
- **Phase 4 is behavior-changing and broad** — touches every layout interaction. Gate on app-running
  E2E; land behind the already-built pure layer so the risk is wiring, not algebra.
- **Frontend `layoutTree.ts` becomes vestigial** after Phase 4 (except drag-preview math) — plan its
  deletion so two implementations don't drift in the interim.

---

## 10. Definition of done
1. Every in-tab layout mutation flows through a reducer intent; the frontend computes no persisted
   tree (except the minimize escape hatch, tracked).
2. `balance_node` ported, differential-fuzz-equal to TS.
3. All 11 reducer arms wired + evented; persist subscriber is the sole `db_layout` writer.
4. `version` is a real CAS; `leaforder` is a pure derivation; `heal_layout` + relaxed-validation
   shims + resync carve-out deleted.
5. `TabRecord.rootnode == db_layout.rootnode` invariant holds mid-session (test).
6. Minimize fields typed; dissolve is a reducer intent (or explicitly tracked as remaining debt).
7. Unblocks Pillar 1: reproject reads a single authoritative, coherent layout source.

---

## 11. Sources / evidence
- Rust: `agentmux-common/src/ipc.rs:506-588` (commands), `:1462+` (events);
  `agentmux-common/src/layout_types.rs:55-111` (`LayoutNode`, `ResizeOp`, `SplitPosition`);
  `agentmux-srv/src/backend/layout/mod.rs` (full pure algebra + oracle note);
  `agentmux-srv/src/reducer/layout.rs` (4 wired arms); `reducer.rs:86-91` ("4 of 11");
  `backend/storage/store.rs:394` (blind-increment version, not CAS).
- Frontend: `frontend/layout/lib/{types.ts,layoutNode.ts,layoutTree.ts,layoutMinimize.ts,
  layoutPersistence.ts,layoutModel.ts}`; oracle `frontend/layout/tests/layoutTree.test.ts`.
- Program docs: `SPEC_864_LAYOUT_SINGLE_WRITER_2026_06_30.md`,
  `SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md`,
  `DISCUSSION_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_06_29.md` §7a (authority principle),
  `srv-phase-e4b-formal-spec-2026-05-03.md` (the original E.4.B port spec this finishes).
```
