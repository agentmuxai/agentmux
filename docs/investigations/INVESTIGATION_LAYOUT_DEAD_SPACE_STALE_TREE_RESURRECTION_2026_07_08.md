# Investigation: Dead-space layout leaf referencing a deleted block

**Date:** 2026-07-08
**Status:** Root cause confirmed by code (not live-reproduced through the CEF frontend);
Mechanism A fixed — `agentmux-srv/src/sagas/delete_block.rs` Step 2b now queues a
`pendingbackendactions` delete via `queue_source_layout_delete`, same channel tear-off
already uses. Mechanism B (saga-step non-atomicity) and the missing CAS on `Store::update`
remain open, tracked as follow-ons in §Recommended fix.
**Component:** `agentmux-srv` reducer/layout persistence, `frontend/layout/lib/layoutPersistence.ts`
**Reported by:** user, dev instance (`v0.51.0`, built 2026-07-07 11:13:02, HEAD `8bea4a52`)
**Relation to Pillar 1 (host reproject, `SPEC_PILLAR1_*`):** not causal — Pillar 1's reproject
code doesn't write layout data, it reads `db_layout` as ground truth on window
open/crash-recovery. But Pillar 1's own design doc
(`SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md:85`) treats layout-tree coherence as a
**hard prerequisite**, marked "Done... confirmed live, not just claimed" on the strength of
SPEC_864. This investigation shows that claim has a real gap (Mechanism A) — any window whose
`db_layout` has this corruption will have it faithfully reprojected/re-rendered on every
crash-recovery or normal open, not just a one-off. Worth flagging to whoever owns Pillar 1's
reliability guarantees, since they inherit this data-integrity assumption without having
introduced or worsened it themselves.

---

## Symptom

A window's pane layout has a "dead" region — a layout leaf occupying real screen space
that renders nothing. Only present in the specific window whose layout was loaded across
the dev instance's restart; brand-new windows are unaffected (confirming this is corrupted
*persisted data*, not a bug in window creation/reproject).

## Direct evidence

The window's `db_layout` row (`objects.db`, oid `751cd177-2352-4f94-a6c3-842ccb18d9c5`) has
3 top-level leaves. One (node id `e0f8400d-227f-446f-8e32-f5a0d9af2608`) has
`data.blockId = "245c8762-af43-44ea-ad96-5b12f780a825"`. That block id **does not exist**
in `db_block` — it's gone, but the layout leaf referencing it survived. No block metadata
→ nothing to render into that leaf → dead space.

Tracing that block id through `srv-events.log` (a JSONL reducer-event stream, distinct from
the `db_layout`/`db_block` persisted-state tables):

- `block_created`, `meta.view = "swarm"`, in tab `993c511a-...`.
- Several `layout_tree_replaced` events (**full raw tree replacement** — the whole tree JSON
  pushed wholesale) on that tab.
- `block_moved` → tab `41ecfa06-...`. A `layout_tree_replaced` on the destination tab whose
  `new_tree.id` is `e0f8400d-227f-446f-8e32-f5a0d9af2608` — the exact node id that is the
  dead leaf today.
- `block_moved` back to tab `993c511a-...`.
- More `layout_tree_replaced` events (22 total across the block's history). Eventually one
  tab's tree root id (`0e68f000-46be-40f7-a919-5a5331a88d18`) becomes identical to the
  *window-level* `db_layout` root id today — that tab's content was promoted to a top-level
  window pane at some point (this app's tear-off/promote flow).
- **The block never appears in any deletion-flavored event.** There is exactly one
  `layout_node_deleted` in the whole trace, and it targets a different, sibling node
  (`cf306e33-...`), not this block's own node.

## Root cause

Two confirmed mechanisms, both present and unguarded on current `main` (`c9911ebe`,
2026-07-08). The first is the direct cause; the second is a compounding, structurally real
gap not yet matched to this specific incident.

### Mechanism A — `delete_block`'s layout prune bypasses the frontend-notification channel every sibling call site already uses (highest confidence)

The team's own code documents this exact risk class, in a comment written for a *different*
call site:

> `agentmux-srv/src/server/service/layout_helpers.rs:70-77` — "Why this and not direct
> rootnode/leaforder writes? The frontend's `LayoutModel` maintains its own in-memory tree
> state and doesn't auto-sync from external `LayoutState` WaveObj updates — so a backend
> `store.update` to the rootnode lands in the WOS cache but **the LayoutModel never picks it
> up**, and the next frontend-initiated `object.UpdateObject` **overwrites the backend
> version with the LayoutModel's stale tree.** The pending-actions queue
> (`onBackendUpdate` in `layoutPersistence.ts:50`) is the canonical channel for 'backend
> wants the frontend to mutate its layout tree'."

Every backend-driven layout mutation site built around this understanding — tear-off,
redock, promote (`layout_helpers.rs:28-187`, `queue_target_layout_insert` /
`queue_target_layout_split` / `queue_source_layout_delete`) — appends to the
`pendingbackendactions` queue instead of writing `rootnode` directly, specifically so the
frontend applies the mutation through its own reducer rather than being blind to it.

**`sagas::delete_block::run`'s Step 2 (`agentmux-srv/src/sagas/delete_block.rs:168-198`,
SPEC_864 site #6, PR #1973) does not follow this pattern.** It dispatches
`Command::LayoutDeleteNodeByBlock` directly — a raw `rootnode` mutation, exactly what the
comment above warns against — and enqueues no `pendingbackendactions` entry. Its own comment
leans on a now-incorrect assumption:

> `delete_block.rs:180-183` — "an orphaned node is exactly what the frontend's own
> delete-push converges away (SPEC_864 Phase 5 deleted the `heal_layout` backstop this
> comment used to reference — no Path-B writer remains to produce the orphans it swept)."

This is backwards relative to the team's own documented understanding elsewhere in the same
codebase: the frontend's next push does not converge the orphan away — **it is the
mechanism that resurrects it** — because, per `layoutPersistence.ts`, the frontend was never
told to remove the node in the first place.

Confirmed client-side. `frontend/layout/lib/layoutPersistence.ts:52-68`
(`onBackendUpdate`, the *only* function that reacts to an incoming backend `LayoutState`
update):

```ts
export function onBackendUpdate(model: LayoutModel) {
    const waveObj = model.getter(model.waveObjectAtom);
    if (!waveObj) return;
    if (!model.treeState.rootNode && waveObj.rootnode) {
        initializeFromWaveObject(model);
        return;
    }
    const pendingActions = waveObj?.pendingbackendactions;
    if (pendingActions?.length) {
        fireAndForget(() => processPendingBackendActions(model));
    }
}
```

For an already-loaded window/tab (the common case), a bare `waveObj.rootnode` change with no
accompanying `pendingbackendactions` entry is a **complete no-op**. The frontend's own
`treeState.rootNode` is left untouched, unaware the backend just pruned a node.

`layoutPersistence.ts:283-301` (`persistToBackend`, debounced 100ms) then fires on **any**
subsequent tree-affecting action in that tab — resize, focus, split, anything, not
necessarily something touching the deleted block — and pushes the frontend's stale,
unmodified tree (still containing the pruned leaf) straight into
`handle_layout_set_tree`'s unconditional overwrite (Mechanism A's write-side gap, below).

**No tight timing race is required.** The divergence is durable — it persists until that
tab's frontend session reloads and re-fetches a fresh snapshot — and fires on the very next
unrelated edit. This is exactly the "it happens sometimes" character the user described:
intermittent because it depends on whether *any* other edit lands in that tab before a
reload, not on a narrow timing window.

### Write-side gap that lets the resurrection stick: no staleness/CAS check anywhere in the layout write path

- `agentmux-common/src/ipc.rs:607-613` — `Command::LayoutSetTree` carries no version /
  expected-prior-tree field.
- `agentmux-srv/src/reducer/layout.rs:86-126` (`handle_layout_set_tree`) unconditionally
  does `tab.rootnode = new_tree.clone();` — no comparison against what it's replacing.
- `agentmux-srv/src/backend/storage/store.rs:381-424` (`Store::update` / `update_raw`) runs
  `UPDATE {table} SET data = ?1, version = version + 1 WHERE oid = ?2` — no
  `WHERE version = ?`. The surrounding comment calls this "optimistic locking"; it isn't —
  it blind-overwrites and blind-increments regardless of what version the caller last read.
- `agentmux-srv/src/persist_subscriber.rs:848-913` (`apply_layout_tree_replaced`) persists
  `new_tree` with zero referential-integrity check against `db_block` — a tree containing a
  leaf whose `blockId` points at a long-gone block persists exactly as happily as any other
  tree.

So: **any full-tree push that reaches the reducer after a delete, no matter how stale,
silently wins and gets persisted.** Nothing in the stack can reject it.

### Why this explains "never appears in any deletion event" exactly

`LayoutDeleteNodeByBlock` ran once and correctly emitted `layout_node_deleted` for this
node — but a later stale `layout_tree_replaced` from a frontend that never got the
`pendingbackendactions` memo silently overwrote `db_layout.rootnode` back to a tree
containing it. That overwrite is logged as a `layout_tree_replaced` event, not a second
delete — so it wouldn't show up as a `layout_node_deleted` entry, only as one of the 22
`layout_tree_replaced` entries already in the trace. The sibling node `cf306e33`'s correctly
logged deletion is unremarkable — a different block whose deletion simply didn't race a
stale push.

### Mechanism B — saga steps aren't atomic against a concurrent block move (moderate confidence, not matched to this specific incident)

A second, structurally real gap, found while reading the same code, that the team fixed for
one call site but not generically:

- `agentmux-srv/src/server/service/object.rs:452-465` documents that the team deliberately
  widened `update_layout_via_reducer`'s critical section to hold the state lock across both
  dispatch *and* SQLite persist in one hold, specifically because the reducer's normal
  release-before-I/O contract lets two concurrent writes commit out of dispatch order.
- `SagaCtx::dispatch` (`agentmux-srv/src/sagas/mod.rs:146-224`) still uses the normal
  contract: acquire lock, run reducer step, **release lock**, persist to SQLite outside any
  lock (`sagas/mod.rs:184-207`).
- `sagas::delete_block::run` issues `DeleteBlock` then `LayoutDeleteNodeByBlock` as two
  separate `ctx.dispatch()` calls with an `await` gap between them and no locking spanning
  both. `handle_move_block` (`reducer/block.rs:77-145`) never touches `tab.rootnode` at
  all — cross-tab block moves relocate the layout node entirely via two independent,
  non-atomic, frontend-applied `pendingbackendactions`. The saga captures `tab_id` once and
  reuses it for both steps; if a `MoveBlock` relocates the block to a different tab in the
  gap between steps, Step 2's `LayoutDeleteNodeByBlock{tab_id: <stale>, ...}` searches the
  wrong tab, finds nothing, and **silently no-ops** (`layout.rs:354-371` returns an empty
  event vec — treated as success, not an error). Nothing then ever prunes the node from the
  tab it actually lives in.

This mechanism also predicts "never appears in any deletion event" — but I have not matched
it to this specific block's timestamps, and Mechanism A alone fully explains the observed
trace. Flagging as a real, separate follow-on risk, not claiming it caused this incident.

## Connection to drag-and-drop / tear-off / promote

The underlying defect (Mechanism A) is general — it can affect any block whose owning tab's
frontend has a stale loaded copy when the block is deleted, with or without tear-off
involved. But the tear-off/redock/promote flow is the most plausible way *this* block ended
up in the vulnerable multi-copy state: `MoveBlock` never updates `tab.rootnode` itself
(§Mechanism B), so a block that has moved between tabs and/or been promoted from a torn-off
tab to a top-level window pane has, at various points, had its layout node visible to more
than one frontend/window instance — the source window (possibly still holding a stale tab
tree from before the move) and the destination/promoted window. This block's own trace
(`block_moved` twice, tab root eventually matching the promoted window's root) fits that
shape. No explicit TODO/FIXME naming this exact race was found anywhere in
`agentmux-cef/src/browser_pane/` or `agentmux-cef/src/ui_tasks/` — the only place this risk
class is documented in writing is the `layout_helpers.rs:70-77` comment and SPEC_864's own
caveats (below), both srv-side.

## This is a known, already-(prematurely)-closed bug class

`docs/specs/SPEC_AGENT_SYSTEM_MANAGEMENT_API_2026_07_04.md` §8 (added 2026-07-07 00:01, PR
#1996) closes out a 2026-07-04 incident matching this one almost exactly: *"`sysinfo`/`swarm`
panes were pruned from a live layout tree ... leaving a stale-rendered empty cell"* — this
dead-space block's `meta.view` was `"swarm"`. That investigation found no surviving evidence
of the original incident, attempted and abandoned a live repro, and closed with a
reasoned-not-proven conclusion: *"this is very likely already fixed as a side effect of
SPEC_864's now-fully-merged single-writer cutover... There is nothing sysinfo/swarm-specific
... their appearance in the original report is most plausibly incidental."*

That closure named its own falsification condition (lines 177-182): *"If this class of
symptom (a pane's content gone, empty cell persists) recurs on the current (post-864) build,
that would be strong evidence against this conclusion and should be treated as a fresh,
still-open bug ... the capstone invariant test ... is the regression guard for exactly this
class going forward."*

**This finding is exactly that recurrence**, and two things about the prior closure are
worth flagging directly:

1. **Timeline rules out "pre-fix scar."** See table below — the corrupted data was created
   on code that already includes every SPEC_864 phase, and postdates the closure itself.
2. **The named regression guard doesn't cover this.** `layout_stays_coherent_across_full_mutation_lifecycle`
   (`agentmux-srv/src/server/tests.rs:838-`) only asserts `TabRecord.rootnode ==
   db_layout.rootnode` after a scripted sequence of *serial, single-actor* reducer
   dispatches — i.e. it re-verifies intra-srv writer coherence, the exact thing SPEC_864
   fixed. It has no frontend `LayoutModel` in the loop, no concurrent/stale-push simulation,
   and no assertion that a deleted block's id never survives as a dangling reference. It
   would not catch either mechanism above.

No other doc in `docs/retro/` or `docs/investigations/` describes this failure mode (broad
grep for dangling/orphan/stale tree/resurrect/dead pane/phantom pane found only the one hit
above).

## Timeline (all commit dates verified via `git show -s --format="%ci"`)

| Event | Date |
|---|---|
| SPEC_864 Phase 2 (`UpdateObject`→`LayoutSetTree`) | 2026-07-05 (#1970, `30d76df3`) |
| SPEC_864 Phase 3 (seeders) | 2026-07-05 (#1971, `46d37927`) |
| **SPEC_864 site #6 (`delete_block` prune — Mechanism A's direct cause)** | **2026-07-05 14:11:23** (#1973, `e319a95b`) |
| SPEC_864 Phase 4 (pendingbackendactions queue) | (#1977, `8647e283`) |
| SPEC_864 Phase 5 (deletes the `heal_layout` backstop) | **2026-07-06 11:58:25** (#1981, `0c7ea2e3`) — "weak cutover COMPLETE" |
| Prior investigation closed ("if recurs, treat as fresh bug") | 2026-07-07 00:01:23 (#1996, `582a5bb8`) |
| **This dev instance's binary last built** | **2026-07-07 11:13:02** |
| This investigation's HEAD | 2026-07-08 02:43:46 (`c9911ebe`) |

The affected binary was built **over 23 hours after** SPEC_864 fully landed — including
Phase 5's removal of `heal_layout`, a periodic backstop that (per SPEC_864's own reasoning)
used to sweep up orphans of a different kind ("Path-B writers"), and which may have
incidentally caught some instances of this failure mode too even though it was never
purpose-built for it. It was also built after the 2026-07-07 00:01 closure that explicitly
named this exact symptom as the thing to watch for.

**Conclusion: this is a live, currently-reproducible bug on `main`, not a fixed one.** Every
piece of Mechanism A is confirmed present and unguarded as of `c9911ebe`. A live repro
through the actual CEF frontend (cross-window move/delete sequence) was not attempted in
this investigation — that's the one remaining gap between "confirmed by code" and "confirmed
by reproduction."

## Fix implemented (Mechanism A)

`sagas::delete_block::run`'s Step 2 (`agentmux-srv/src/sagas/delete_block.rs`) now has a
Step 2b: after the direct `LayoutDeleteNodeByBlock` reducer dispatch, it also calls
`server::service::layout_helpers::queue_source_layout_delete` (widened from `pub(super)` to
`pub(crate)` — previously only reachable from `server::service`, now callable from
`sagas::delete_block` too) to queue a `"delete"` `pendingbackendactions` entry on the tab,
the same channel tear-off/redock already use successfully for this exact class of problem.
Any frontend with a stale loaded copy of that tab now converges on its next
`onBackendUpdate` poll instead of resurrecting the pruned node on its next unrelated edit.
Both steps remain best-effort (matching the existing Step 2's failure posture) — a queue
failure is logged, not fatal, since the block is already gone either way.

Also corrected the Step 2 comment, which previously claimed "an orphaned node is exactly
what the frontend's own delete-push converges away" — backwards, per Mechanism A above — and
added a regression test
(`sagas::delete_block::tests::saga_queues_a_pendingbackendactions_delete_for_a_stale_frontend`)
asserting the queued action's `actiontype`/`blockid` match what
`frontend/layout/lib/types.ts`'s `LayoutTreeActionType.DeleteNode` handler expects. This
directly closes Mechanism A, the highest-confidence and directly-matched-to-evidence cause.
All 133 pre-existing + new layout/saga tests pass.

Mechanism B (saga-step non-atomicity against a concurrent `MoveBlock`) and the missing
CAS/version check on `Store::update`/`db_layout` writes are real but architecturally deeper.
Both are things SPEC_864's own docs already flag as open/undecided (see
`docs/handoff/HANDOFF_864_AUTHORITY_AND_WINDOW_LIFECYCLE_2026_07_05.md:58` on the CAS
question specifically) rather than novel findings here — worth tracking as follow-ons, not
blockers for the Mechanism A fix.
