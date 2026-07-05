# HANDOFF — SPEC_864 layout authority (Phases 2–3 done) + window-close leak (fixed) + remaining roadmap

**Date:** 2026-07-05
**From:** Agent2
**To:** whoever picks up SPEC_864 Phases 3.5–5 (site #6 → Phase 4 → Phase 5) and the #1969 follow-ups — expected: AgentA, continuing in the isolated environment
**Status of this doc:** information handoff; everything described as "merged" is on `main` as of `d40a702b` + #1971 (if approved by the time you read this)

---

## 1. What merged today (all on `main`)

| PR | What | Key knowledge |
|---|---|---|
| **#1968** | Floating-pane redock lands at the ghost's exact rect | `nodesizefraction` (0.5 inner / 0.2 outer) on `LayoutActionData`, applied against the target's **live** size in `layoutTree.ts::applySizeFraction` — carves from the target so the parent's flex pool is conserved and siblings never move. Analysis: `docs/analysis/ANALYSIS_FLOATING_PANE_GHOST_LANDING_DISCONNECT_2026_07_04.md` |
| **#1970** | **SPEC_864 Phase 2** — `UpdateObject`→`LayoutSetTree` reroute (the "sharp edge") | See §2 below — the mechanics matter for everything that follows |
| **#1969** | Window-close leak fixed via **round-6 pool demote** + `backend_close_window` auth fix | See §4 — six rounds of evidence; the negative results are load-bearing |
| **#1971** (open at handoff time) | **SPEC_864 Phase 3** — seeders reroute (sites #2 + #4) | CI green; merge on approval. See §3 |

## 2. Phase 2 mechanics you need to know (merged, #1970)

- `Command::LayoutSetTree` / `Event::LayoutTreeReplaced` gained `slices: Option<LayoutClientSlices>` (`agentmux-common/src/layout_types.rs`) — the frontend-owned columns (leaforder / focus / magnify / pendingbackendactions) with **REPLACE semantics**: absent/null leaforder or pending queue = CLEAR (pushing without processed actions is the frontend's ack). `slices: None` = tree-only, columns untouched (granular arms use this).
- The `UpdateObject` route (`server/service/object.rs::update_layout_via_reducer`) runs **snapshot + dispatch + SQLite apply (+ failure rollback) inside ONE `srv_state` mutex hold**. Reagent caught two real races here in review (dispatch-order vs persist-order inversion; stale pre-lock rollback snapshot) — both fixed by widening the critical section. **Any new dispatcher of layout commands must follow the same pattern** or it reintroduces the divergence. `seed_layout_via_reducer` (Phase 3) is the reusable embodiment.
- Owned-row parse-failure fallback keeps the pre-Phase-2 Option-A focus/magnify dispatch (reagent review 3; test `update_object_layout_parse_failure_falls_back_with_focus_dispatch`).
- Invariant test to preserve: `update_object_layout_push_single_write_and_coherent_reducer` — one version bump per push, `TabRecord.rootnode == db_layout.rootnode`, queue ack clears.

## 3. Phase 3 (PR #1971) — what it does and what it deliberately skips

- New `seed_layout_via_reducer` (`server/service/reducer_helpers.rs`, re-exported from `service`): LayoutSetTree + slices, one lock hold, empty-tree rollback.
- **Site #2** (CreateWindow three-pane seed, `service/window.rs`): reducer-routed. Tree shape shared with the pre-bootstrap first-launch seed via pure `default_three_pane_tree` (`backend/wcore/mod.rs`).
- **Site #3** (`ensure_initial_data` → `seed_default_layout`): **deliberately store-direct** — runs pre-bootstrap, reducer not hydrated. Sanctioned by the spec.
- **Site #4** (`setup_torn_off_block_layout`, now async, takes `&AppState`): reducer-routed; callers = TearOffBlock / RedockFloatingPane / PromoteBlockToTab handlers (`service/workspace.rs`) + floating `pane.open` (`app_api/pane.rs`). All run post-saga so the tab is reducer-known.
- **Site #8** (`wcore::dnd::tear_off_block`): **dead** — zero production callers (superseded by sagas). Delete with tree-shake; don't reroute.
- Tests: `layout_seeders_route_through_reducer_coherently`, `layout_seed_unknown_tab_errors`.

## 4. Window-close leak (#1969, merged) — the map and the residuals

**The wall:** CEF 148 Views parks the browser on EVERY close/destroy sequence; `on_before_close` never fires for Views secondary windows. Rounds 2–5 (all negative, all instrumented) are in `docs/retro/retro-window-lifecycle-leak-2026-07-04.md`:
- r2/r3: `close_browser(1)` after/before `window.close()` — parks
- r4: native `DestroyWindow` (strict HWND) — window dies, zero CEF callbacks; **Views browsers don't tear down with their HWND** (the #1957 WM_DESTROY cascade is `set_as_child`-panes-only)
- r5: arm (`close_browser(1)`) + `DestroyWindow` — `do_close`+`can_close→1` fire, `OnBeforeClose` still never. Break is inside BrowserView→Browser destruction.

**The fix (round 6, pool demote):** closing a promoted pool window = `demote_srv_cleanup` (imperative on_before_close-equivalent: `backend_close_window` → srv `CloseWindow` → `delete_workspace` cascade, with the #1965 retry, then unregister) + `demote_promoted_pool_window` (`commands/window_pool.rs`): strict-HWND-first (mutation-free failure), reducer `HostCommand::DemotePoolWindow` (is_pool flip + `unpromoted` re-insert), park offscreen + `set_taskbar_hidden(true)`, re-cache HWND/Views-window, **reload to the `pool=1` boot URL** — queue re-entry rides the normal `pool_window_ready` handshake. Demote cap = `POOL_TARGET_SIZE + 2`. Verified live in equilibrium (close→demote→reopen promotes the recycled window→close re-demotes).

**Latent bug fixed en route:** `backend_close_window` (`client/helpers.rs`) used `?authkey=` query-param auth — **disabled 2026-05-11 (audit C3)** — 401'd on every call since, invisible because its caller never fired. Now `X-AuthKey` header. Remember this class: hand-rolled HTTP in dead code paths rots silently.

**Residual follow-ups (all documented in the retro):**
1. **Non-pool secondary windows still park their renderer** — cold-path `window-{uuid}` (pool exhausted / pre-warm) and drag-tear-off windows. srv state IS cleaned (the imperative cleanup runs for all `window-*` closes), but the ~100MB renderer isn't reclaimed. Fix = **pool adoption for foreign labels** (`mark_pool_window_renderer_ready` and the ready-handshake key on the `window-pool-` prefix — an adopted-labels set or label-agnostic pool membership is needed).
2. **srv `CloseWindow` leaves the bare window ROW** (workspace/tabs/blocks cascade correctly; the `db_window`/reducer window record lingers — visible as empty-workspace-name entries in `/api/v1/windows`). Same dual-write class as #864; a candidate Phase-5-adjacent cleanup.
3. **Non-Windows platforms keep the Views `window.close()` path** — no parked-browser evidence collected there; verify before porting demote.
4. Subwindow children of a demoted window take their own close path.

## 5. Remaining SPEC_864 roadmap (the critical path to Pillar 1)

Per `SPEC_864_LAYOUT_SINGLE_WRITER_2026_06_30.md` + the 2026-07-02 weak-cutover scope note (DISCUSSION §7b — **intent-flip NOT required for Pillar 1**):

1. **Site #6 — delete_block layout prune.** The prune lives inside `wcore::delete_block`, called from `persist_subscriber.rs::apply_block_deleted` (the subscriber must NOT dispatch commands). Design sketch: give `sagas::delete_block` a `LayoutDeleteNode` dispatch — but the reducer arm takes `node_id`, and the saga only knows `block_id`; either add block-id resolution to the arm (reducer owns the tree, `find_node_by_block` exists in `backend/layout/`) or add a `LayoutDeleteNodeByBlock` command. Then strip the layout-prune from the `apply_block_deleted` path (keep the block-row delete).
2. **Phase 4 — `pendingbackendactions` queue writers** (3 sites: `layout_helpers.rs::queue_target_layout_{insert,split}`, `queue_source_layout_delete`, + `app_api/pane.rs` open_pane's inline queue write). Add a reducer action arm for the queue; note the reducer does NOT model the queue in `TabRecord` — slices-style pass-through on a dedicated command/event is the Phase-2-consistent shape.
3. **Phase 5 — delete the backstops** once no Path-B writer remains: `heal_layout` + its 2 callers (`main.rs:~1336`, `service/workspace.rs:349`), the relaxed `reorder_tabs_bulk` validation + its migration test (`reducer/tab.rs:263-279`), the resync carve-outs. **DoD** (spec §6): no `Store::update`/`update_raw` on `OTYPE_LAYOUT` outside the persist subscriber; CAS/version semantics check (`store.rs:396,418,618` still blind-bump — decide whether Phase 5 adds CAS); invariant tests.
4. After #864: **Pillar 2 Stage 2** (consume `reconcile_quit`) → **Pillar 1** (host reproject) → saga collapse. Sequencing per `docs/status/STATUS_LIFECYCLE_OOM_REFACTOR_2026_06_30.md`.

## 6. Environment notes (why this is being handed to the isolated env)

- **Interrupted `cargo test -p agentmux-srv` runs orphan real `agentmux-srv.exe` children** (spawned by `tests/integration_test.rs` / `subprocess_io.rs`), which then hold the `target/debug` binary lock and hang/fail the next build. Happened 3× this session. Identify by path (`...\agentmux\target\debug\agentmux-srv.exe`) and kill by PID — NEVER by image name (multi-instance rule). Cheap permanent fix: `kill_on_drop(true)`/Job-Object guard on the integration-test spawns — good first PR in the new env.
- **Flaky under parallel runs:** `agentmux-cef::browser_pane::hwnd::tests::allow_pane_focus_once_swap_returns_prev_and_clears` — global-static raced by the parallel test harness; passes in isolation on both main and feature branches. Not related to any of today's changes.
- **Live E2E recipe** (used for #1969/#1970 verification): `cmd //c "set AGENTMUX_DEBUG_CLOSE=1&& scripts\dev-agent.cmd TITLE=..."`; endpoints/token from `~/.agentmux/dev/<branch-slug>/<hash>/data/authkey.dev`; host IPC = `POST /ipc` with `Authorization: Bearer <ipc_token>` (`open_new_window` / `close_window`); srv = `X-AuthKey` header; close trace at `%TEMP%\agentmux-close-debug.txt`; instance-scoped renderer count via `Get-CimInstance Win32_Process` filtered on the dev-dir path + `type=renderer`.
- The dev data dirs for today's branches (`~/.agentmux/dev/agenta-window-close-force-browser-teardown/`, `.../agent2-864-phase2-updateobject-settree/`) accumulated stale srv window records from repeated test rounds — baselines in those dirs are polluted; use a fresh branch/dir for count-based verification.

## 7. Also in this PR

`docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_05.md` — the full repo health audit (architecture assessment, Rust+frontend tree-shake inventories, hygiene, docs audit, cross-repo triangulation with agentmux-cloud/-docs) that produced the prioritized 32-item action plan this session started executing (Tier-4 items #24–#26 are §5 above). It was produced this session and previously lived outside the tree after being flagged as scope-mismatch in #1969; an information PR is its right home. Note: `SPEC_PANE_COLOR_PANEL_TOPLEVEL_2026_07_01.md` (not my work) also sits untracked at the workspace root — left for its author.
