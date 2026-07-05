# SPEC #864 — Collapse the Layout Split-Brain to a Single Writer

**Date:** 2026-06-30
**Type:** Implementation spec (sized)
**Status:** Ready to schedule
**Owner:** asaf
**Issue:** #864 (retire the wcore-direct layout path)
**SUPERSEDED IN SCOPE (2026-06-30):** This spec's "single writer" is now **Phase 1** of the committed
end-goal in `SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md` (strong reducer-authority). The
"weak authority" path described here (frontend computes tree, reducer persists) is the low-risk
stepping stone; the full intent-driven reducer is the target. Read both; this one for the Phase-1
mechanics, the other for the destination.

**Why now:** Promoted from "pay-down" to a **hard prerequisite for Pillar 1** by
`SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md` §4/§7 — host-reproject durability is incoherent
while two writers race one `db_layout` row. This must land before Pillar 1.

> Goal: make the **srv reducer the sole writer** of `db_layout` (`rootnode` / `leaforder` /
> `focusednodeid` / `magnifiednodeid` / `pendingbackendactions`), so the persisted layout record is
> coherent, the reducer's `TabRecord` is authoritative (not a passive shadow), and the
> reproject-from-srv read path reads a single, race-free source.

---

> **Scope note (2026-07-02):** for **Pillar 1 (disposable host)** the minimum is the **weak cutover** —
> retire wcore-direct and route the frontend's full-tree push through `LayoutSetTree` (the frontend keeps
> *computing* the tree) so there is ONE coherent writer of `db_layout`. The strong reducer-authority /
> intent-flip (`SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT`) is a separate, above-and-beyond goal, not a Pillar-1
> blocker. See `DISCUSSION_LIFECYCLE_AND_CRASH_ARCHITECTURE` §7b.

## 0. TL;DR

The layout tree is already persisted in srv (`db_layout`), so it survives host death. The problem
#864 fixes is **internal to srv**: that one `db_layout` row is written by **two paths** —

- **Path A (reducer):** writes only the focus/magnify slice (`SetFocusedNode` / `SetMagnifiedNode`),
- **Path B (wcore-direct):** writes the tree itself (`rootnode` / `leaforder` /
  `pendingbackendactions`) by calling `Store::update`/`update_raw` directly, bypassing the reducer.

The single `UpdateObject` RPC fires **both** on every frontend push (two SQLite writes, two `version`
bumps, per push), and Path B leaves the reducer's `TabRecord.rootnode` a stale shadow for the rest of
the session. The fix is **not greenfield** — the migration scaffolding already exists (Phase E.4.B:
dormant `Layout*` reducer arms, "4 of 11 shipped"; the deferred "Option B" decision in
`SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md`). #864 = **finish that migration**: complete the 7
remaining reducer arms, add the matching persist-subscriber + bridge arms, then reroute ~9
wcore-direct call sites through the reducer.

**Size:** Medium. ~7 reducer handlers + ~4–6 subscriber/bridge arms + ~9 caller reroutes, landed in
**5 incremental, independently-shippable phases** behind the already-present scaffolding.

---

## 1. The split-brain, concretely (verified against source)

**The persisted record** — `LayoutState` (`backend/obj.rs:398-416`), one row per tab, otype
`OTYPE_LAYOUT`, joined via `Tab.layoutstate`. Fields: `rootnode` (the tree), `leaforder` (parallel
flat leaf index), `focusednodeid`, `magnifiednodeid`, `pendingbackendactions`, `version` (optimistic
lock). Lowest-level writers: `Store::update<LayoutState>` and `Store::update_raw`
(`backend/storage/store.rs:381-424`).

**Read-back / reproject path:** `persist::bootstrap_state_from_wstore` (`persist.rs:141-152`) reads
`rootnode` + `focusednodeid` + `magnifiednodeid` into the reducer's `TabRecord` on startup. **This is
the path Pillar 1 fires on crash** — so its source must be coherent.

**Three in-memory copies** (audit's "~3", confirmed):
1. SQLite `db_layout.rootnode` — session-authoritative (`persist.rs:11-16`).
2. Reducer `TabRecord.rootnode` (`state.rs:106`) — **passive shadow**, written only at bootstrap;
   diverges the instant any Path-B write lands mid-session (acknowledged `state.rs:98-106`).
3. Frontend `LayoutModel` tree — pushes via `UpdateObject`; backend tree writes are invisible to it
   (the reason redock must use `pendingbackendactions`, `service.rs:2945-2955`).
   (`leaforder` is effectively a 4th parallel copy; `heal_layout` exists to repair its drift.)

**The double-write** (`service.rs:448-531`, verified): `UpdateObject` writes the **whole row**
(including `rootnode`) via `update_raw`, then on success dispatches `SetFocusedNode` +
`SetMagnifiedNode` to the reducer, whose subscriber **re-updates the same row's** focus/magnify
columns (`persist_subscriber.rs:671-716`). Two writes, two version bumps, per single push. Ordering
("DB write THEN dispatch, only on success") is hand-enforced for atomicity (codex P2 PR #632).

**Divergence backstops that exist *because* of the split-brain** (all deletable once collapsed):
- `heal_layout` startup + on-activation rewrite of `rootnode`/`leaforder` (`wcore/block.rs:122`,
  `main.rs:1320`, `service.rs:1517`) — "last defense" against drift.
- Relaxed reducer validation for tab order: `handle_reorder_tabs_bulk` drops set/length checks
  (`reducer/tab.rs:263-279`) + test `reorder_tabs_bulk_accepts_unknown_ids_during_migration`.
- `resync_workspaces` deliberately refuses to touch tab/block/layout on `Lagged`
  (`persist_subscriber.rs:108-128`).

---

## 2. Target end-state (the invariant)

> **Invariant:** every mutation of `db_layout` flows through exactly one channel — the reducer's
> `Layout*` command arms. The persist subscriber is the **only** code that writes `db_layout` to
> SQLite (mirroring reducer events). `wcore`/`service`/`app_api` never call `Store::update` /
> `update_raw` on an `OTYPE_LAYOUT` row.

Consequences:
- `TabRecord.rootnode` becomes **authoritative**, not a shadow → reproject reads one coherent source.
- The `UpdateObject` double-write collapses to one reducer dispatch (`LayoutSetTree`) → one write,
  one version bump.
- `heal_layout`, the relaxed-validation shims, and the resync carve-outs become removable.

The frontend stays the **editor** (user interaction lives in the renderer); it expresses intent via
`UpdateObject`, which the RPC translates into a reducer command. The renderer remains a disposable
projection — this spec does not move layout *editing* out of the frontend, only makes srv's
*persistence* single-writer.

---

## 3. Surface area to migrate (verified inventory)

### 3.1 Reducer arms — finish 7 of 11 (`reducer/layout.rs`)
Shipped (dormant): `LayoutClear`, `LayoutSetTree`, `LayoutInsertNode`, `LayoutDeleteNode`
(`reducer.rs:92-123`). Remaining 7 (`reducer.rs:86-91`): `insert_at_index`, `move`, `swap`, `resize`,
`replace`, `split_horizontal`, `split_vertical` — "structurally identical." Most callers can route
through `LayoutSetTree` (full-tree replace) initially; the granular arms are an optimization, not a
blocker (see §4 Phase ordering).

### 3.2 Persist-subscriber + bridge arms — add 4+ (`persist_subscriber.rs:187-298`)
Events `LayoutCleared` / `LayoutTreeReplaced` / `LayoutNodeInserted` / `LayoutNodeDeleted` currently
have **no** subscriber persist arms and **no** bridge arms (`wave_obj_bridge.rs:440-443`). Add
`apply_layout_*` writing `db_layout` via `Store::update<LayoutState>`, plus bridge broadcast arms so
the frontend still receives `waveobj:update`.

### 3.3 wcore-direct callers to reroute — ~9 sites
| # | Call site | Current write | Reroute to |
|---|---|---|---|
| 1 | `service.rs:495` `UpdateObject`/`update_object`→`update_raw` | full `rootnode`+`leaforder` (**highest frequency**) | `LayoutSetTree`; collapses double-write at `:501-519` |
| 2 | `service.rs:931` `CreateWindow` seed → `write_default_three_pane_layout` | seed `rootnode` | `LayoutSetTree` |
| 3 | `wcore/mod.rs:116` `ensure_initial_data` first-launch seed | seed `rootnode` | runs pre-bootstrap — may stay store-only or seed post-bootstrap via reducer (note existing two-path split `wcore/mod.rs:168-181`) |
| 4 | `service.rs:2912` `setup_torn_off_block_layout` | `rootnode`+`leaforder` | `LayoutSetTree` |
| 5 | `service.rs:2956`, `app_api.rs:448`, `app_api.rs:1057` `pendingbackendactions` queues (3 sites) | `pendingbackendactions` | reducer-routed insert/delete action arm |
| 6 | `wcore/block.rs:36` `delete_block` prune | `rootnode`+`leaforder` | `LayoutDeleteNode` (saga already partly reducer-driven) |
| 7 | `wcore/block.rs:122` `heal_layout` (2 callers: `main.rs:1333`, `service.rs:1517`) | `rootnode`+`leaforder` | route through `LayoutSetTree`, or keep as explicit reducer-bootstrap repair, then **delete** once drift source is gone |
| 8 | `wcore/dnd.rs:195` `tear_off_block` tree write | `rootnode`+`leaforder` | `LayoutSetTree` (reducer-state portion already in `sagas::tear_off_block`) |

**Single highest-leverage change: site #1** (`service.rs:448-531`). Routing `UpdateObject` through
`LayoutSetTree` collapses both the dual-writer-on-one-row hazard (§1) and copy #2's divergence in one
move. Do it first.

---

## 4. Phased plan (each phase independently shippable + testable)

**Phase 1 — close the persistence gap for the existing arms.** Add the 4 `apply_layout_*` subscriber
arms + bridge arms for the already-shipped `LayoutClear/SetTree/InsertNode/DeleteNode` events. No
caller changes yet → behavior-neutral, but now the dormant arms can actually persist. *Unit-testable:
dispatch each command, assert `db_layout` row + emitted `waveobj:update`.*

**Phase 2 — reroute `UpdateObject` (site #1).** Replace the `update_raw` full-row write + separate
focus/magnify dispatch with a single `LayoutSetTree` (carrying focus/magnify in the command). Delete
the double-write. *This is the load-bearing change — verify one write/one version bump per push, and
that `TabRecord.rootnode` now matches `db_layout` mid-session.* **Highest-value, highest-risk single
step.**

**Phase 3 — reroute the seeders + tear-off + delete (sites #2, #4, #6, #8).** Route
`CreateWindow`/`setup_torn_off_block_layout`/`delete_block` prune/`tear_off_block` through
`LayoutSetTree`/`LayoutDeleteNode`. *E2E: new window, tear-off, delete-block all persist via reducer.*

**Phase 4 — reroute `pendingbackendactions` (site #5, 3 sites).** Add a reducer action arm for the
backend→frontend command queue; route the 3 queue writers through it.

**Phase 5 — delete the backstops.** Once no Path-B writer remains: remove `heal_layout` + its 2
callers, the relaxed `reorder_tabs_bulk` validation + its migration test, and the resync carve-out
comments. *This is the payoff phase — a deletion, the safest kind of change. Assert the heal pass is
no longer needed by running a session and confirming no `rootnode`/`leaforder` drift.*

The 7 remaining granular arms (§3.1) are **not required** for the collapse — every caller can route
through `LayoutSetTree` (full-tree replace). Add the granular arms later as a write-amplification
optimization if `db_layout` row size makes full-tree writes costly. **Do not gate #864 on them.**

---

## 5. Risks / honest caveats
- **Phase 2 is the sharp edge.** `UpdateObject` is the highest-frequency layout writer; a regression
  here breaks every split/resize/move. Needs the app-running E2E pass, not just unit tests — treat it
  like Pillar 2 Stage 2: **do not auto-merge on bot approval.**
- **`ensure_initial_data` runs pre-bootstrap** (site #3) — the reducer/subscriber may not be live yet
  at first-launch seed. Likely must stay a direct store seed OR re-seed through the reducer
  immediately post-bootstrap. Resolve during Phase 3; don't force it through the reducer if it
  inverts the init order.
- **Frontend↔backend overwrite race is partly out of scope.** Single-writer *within srv* makes
  `db_layout` coherent, but the frontend `LayoutModel` still pushes full trees and is blind to
  backend writes (`service.rs:2945-2955`). This spec keeps `pendingbackendactions` as the
  backend→frontend channel; fully unifying cross-process layout authority is a Pillar 1 follow-on,
  not #864.
- **`version` optimistic-lock semantics** must be preserved through the reducer path — confirm the
  subscriber's `Store::update` still bumps/checks `version` so concurrent pushes can't silently
  clobber.
- **Migration-era shims have tests pinned to current behavior** (e.g.
  `reorder_tabs_bulk_accepts_unknown_ids_during_migration`) — Phase 5 must delete the test with the
  shim, not leave it asserting removed behavior.

---

## 6. Definition of done
1. No `Store::update`/`update_raw` on an `OTYPE_LAYOUT` row exists outside the persist subscriber.
2. `UpdateObject` produces exactly one `db_layout` write + one version bump per push.
3. `TabRecord.rootnode` equals `db_layout.rootnode` at all times mid-session (new invariant test).
4. `heal_layout`, the relaxed `reorder_tabs_bulk` validation, and the resync layout carve-out are
   deleted.
5. E2E: split / resize / move / tear-off / delete-block / new-window all persist via the reducer;
   app-running verification of Phase 2 passes.
6. Unblocks Pillar 1: reproject's `bootstrap_state_from_wstore` reads a single coherent writer.

---

## 7. Sources
- Internal map (this session): `reducer.rs:80-123`, `reducer/layout.rs`, `service.rs:448-531`,
  `persist.rs:130-165`, `persist_subscriber.rs:187-298,664-716`, `backend/obj.rs:398-416`,
  `backend/storage/store.rs:381-424`, `backend/wcore/{mod,block,dnd,tab,window}.rs`,
  `server/wave_obj_bridge.rs:184-244,440-443`, `main.rs:1320-1349`.
- `docs/specs/SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md` (the deferred "Option B" decision this
  finishes), `docs/specs/SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md` §4/§7 (why this gates
  Pillar 1), `docs/specs/SPEC_ARCHITECTURE_HEALTH_AND_REFACTOR_2026_06_29.md` §2.3 (layout
  split-brain finding).
