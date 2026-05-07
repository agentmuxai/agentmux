# Plan: srv reducer modularization

**Date:** 2026-05-07
**Status:** Draft — operational plan
**Companion PR:** #716 (originally an empty-tree-explicit-parent fix; expanded to absorb this refactor per user direction)

## Why

`agentmux-srv/src/reducer.rs` is **4389 lines** with **28 handlers + tests** in a single file. The other two reducers in the architecture are already modularized:

| Reducer | Layout | Lines |
|---|---|---|
| `agentmux-cef/src/reducer/` | mod + panes / browsers / drag / pool / quit / top_level / tests | 871 in mod.rs |
| `agentmux-launcher/src/reducer/` | mod + window / pool / wrr / tests | 586 in mod.rs |
| `agentmux-srv/src/reducer.rs` | **single file** | 4389 |

Srv is the outlier. Symptom: PR #715 added 4 layout arms (~250 LOC) to a file that was already over 4000; the next 7 layout arms (Phase 5 follow-up) plus Phase 6 persist arms plus Phase 7 wcore-migration would push it past 6000.

## Target shape

Mirror `agentmux-cef/src/reducer/` — one submodule per Command domain. Top-level `mod.rs` keeps the public `update()` dispatch + shared types.

```
agentmux-srv/src/
  reducer.rs                ← DELETE (becomes the directory below)
  reducer/
    mod.rs                  ← public: update(), Ctx, ErrorCode shared by all
    lifecycle.rs            ← Register, Goodbye, Ping
    snapshot.rs             ← GetSrvSnapshot, GetEvents (no-op)
    workspace.rs            ← CreateWorkspace, DeleteWorkspace, RenameWorkspace, UpdateWorkspaceMeta
    tab.rs                  ← CreateTab, DeleteTab, SetActiveTab, ReorderTab, RenameTab, UpdateTabMeta, MoveTab, ReorderTabsBulk
    block.rs                ← CreateBlock, DeleteBlock, UpdateBlockMeta, MoveBlock
    window.rs               ← CreateWindow, CloseWindowInternal, SwitchWorkspace
    layout.rs               ← E.4.A (SetFocusedNode, SetMagnifiedNode) + E.4.B (LayoutClear, LayoutSetTree, LayoutInsertNode, LayoutDeleteNode, ...future 7 arms)
    tests.rs                ← keep all existing tests in one file initially; per-domain split is a follow-up
```

**Estimated LOC per module:**
- mod.rs: ~250 (dispatch match + Ctx + use statements)
- lifecycle.rs: ~120 (Register has the most logic)
- snapshot.rs: ~50
- workspace.rs: ~250
- tab.rs: ~700 (largest — 8 arms incl. complex MoveTab + ReorderTabsBulk)
- block.rs: ~250
- window.rs: ~150
- layout.rs: ~600 (E.4.A 80 + E.4.B 4 arms 400 + room for 7 more)
- tests.rs: ~2900 (kept whole)

## Scope

**Pure code-move + minimal mechanical adjustments.** No behaviour change, no test edits, no algorithmic improvements.

Mechanical adjustments expected:
- `pub(super) fn` instead of `fn` for handlers called from the dispatch match in `mod.rs`
- `super::ErrorCode` / `super::Ctx` imports inside each domain module
- Test module stays as `#[cfg(test)] mod tests` but inside `mod.rs` so `use super::*` continues to reach all handlers
- A few helper fns (e.g. `merge_meta`) currently `fn` may need `pub(super)` if shared; otherwise duplicate-into-domain

## Validation

After the move:
- `cargo check -p agentmux-srv` — clean, no new warnings
- `cargo test -p agentmux-srv --bin agentmux-srv` — same pass count as pre-refactor (864 tests)
- `git diff --stat` shows mostly file deletions + new file creations + minor `use` changes; no algorithmic deltas

## Risks

1. **Cross-domain helper sharing.** Some handlers may call helpers defined inline in another section. Each one needs to be elevated to `mod.rs` with `pub(super)` visibility OR copied per-domain (only if truly small). Audit by grepping for fn calls that don't resolve once split.

2. **Test reach.** `mod tests` uses `super::*` — currently that pulls in all 28 handlers from one file. After split, `super::*` in `mod.rs::tests` reaches re-exports from the public `update()` only, NOT the per-domain `pub(super) fn handle_*` symbols. Two options:
   - (a) Tests stay in `mod.rs::tests` and call `update(...)` instead of `handle_xxx(...)` directly. Most tests already do this. Per-handler tests need rewriting.
   - (b) Re-export handlers from `mod.rs` with `#[cfg(test)] pub(super) use foo::*;` so tests reach them.

   **Plan: (a) — drive tests through the public `update()` dispatch.** Cleaner test contract; some 1-line test refactors needed. The 856 existing tests already follow this pattern at a glance; will verify per-domain during the move.

3. **Reagent / codex review surface.** A 4389-line move is hard to review byte-for-byte. **Mitigation:** keep the diff structure as "delete reducer.rs, create reducer/{mod,lifecycle,...}.rs"; reviewers can confirm character-equality by section. Do NOT mix in any other change.

4. **The empty-tree-explicit-parent fix already in #716.** Code-wise it lives inside `handle_layout_insert_node`. After the move that lives in `reducer/layout.rs`. Diff hygiene: ship the move FIRST in this PR's commit history, then a separate commit for the fix, so reviewers can see both cleanly. Currently the fix is on the branch; rebase to put the move commit first.

## Execution

Single PR, multi-commit:

1. **Commit 1 — pure move.** `git mv reducer.rs reducer/` impossible (different basenames); instead:
   - `git rm src/reducer.rs`
   - Create `src/reducer/{mod,lifecycle,snapshot,workspace,tab,block,window,layout,tests}.rs`
   - Each new file contains the verbatim handlers from the old reducer.rs (plus their doc-comments)
   - Update `mod.rs` to declare submodules + retain the `pub fn update()` dispatch match
   - `cargo check && cargo test` — expect 864 pass

2. **Commit 2 — re-apply the empty-tree fix** (already on branch; `git cherry-pick` the existing fix commit on top of the move). Diff is small + readable.

3. **Commit 3 — version bump** (existing pattern via `bump patch --commit`).

## Out of scope (follow-ups)

- Splitting `tests.rs` per-domain. Big test file is fine; per-domain co-location is gravy.
- Cross-reducer helper extraction (none anticipated for srv; launcher reducer has its own modules).
- Any algorithmic change to reducer logic. Strictly a move.

## Acceptance

- `cargo test -p agentmux-srv --bin agentmux-srv` passes 864/864 (matching pre-refactor count after #715/#716 merged)
- `git diff main..HEAD --stat` shows the move shape (deletions of old reducer.rs, additions of new reducer/*.rs)
- Reagent + codex reviews on the move commit treat it as a refactor (LGTM expected; any P1/P2 should be on the empty-tree fix in commit 2)
