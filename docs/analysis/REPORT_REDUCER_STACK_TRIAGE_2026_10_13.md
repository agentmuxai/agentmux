# Reducer stack triage — where we are (2026-10-13)

**Status:** analysis
**Supersedes as the current snapshot:** `docs/specs/MASTER_REDUCER_STACK_STATUS_2026-05-05.md` §0, `docs/specs/REPORT_REDUCER_STACK_AUDIT_2026_07_26.md`
**Checked against:** `main` at 78f171c56

One place for the state of every reducer and state machine in AgentMux: what is done, what is still open, and which tracking docs have gone stale. Every claim below was checked against the code, not taken from a doc's own status line.

## 1. Summary

- The migration the April–July specs planned is essentially done. The srv, host (cef) and launcher reducers own their state, the sagas run on the two frameworks the 2026-09-07 decision describes, and the frontend has eight tested slice reducers plus several pure state machines.
- About half of the ~35 tracking docs still say `draft` or `proposed` for work that shipped months ago. That makes the real open items hard to see. §5 lists the status fixes.
- Real open work falls in four areas: agent-pane scroll-follow, pane-tab layout revisions, redock window resolution, and the frontend still pushing whole layout trees. There are also two small correctness bugs in the reducers themselves (§2).
- Reducer code is quiet. Of 10 commits touching `crates/*/src/reducer*` since 2026-09-01, only #4359 changed behaviour; the rest were comments, moves and removals.

## 2. Reducer code: findings

| | srv | host (cef) | launcher |
|---|---|---|---|
| Owns | lifecycle, processes, workspaces, tabs, block ids, window→workspace, layout tree + focus/magnify (`crates/srv/src/state.rs:133`) | pending creations, browser panes, browsers, drag, window pool, pane pool, quit, opacity, pane-window states (`crates/cef/src/reducer/mod.rs:57`) | lifecycle, processes, windows mirror, pool, instance registry, monitors, pending HWNDs (`crates/launcher/src/state.rs:173`) |
| Actions | shared `ipc::Command` (73); 44 handled | own `HostCommand` (30) | shared `ipc::Command`; ~30 arms reply "misrouted" |
| Effects | `Vec<Event>` → persist subscriber writes SQLite | `DispatchOutput` → caller applies Win32/CEF effects | `Vec<Event>` → saga coordinator |
| Tests | ~126 (incl. a layout proptest) | 79 | 89 (3 proptest blocks) |

### Bugs

- **P1 — File I/O inside the launcher reducer.** `apply_hwnd_position_changed` (`crates/launcher/src/wrr/mod.rs:523`, also `:213`) calls `crate::log`, which creates, rotates and appends to the log file. It runs inside the reducer on every window-position event.
- **P1 — The last-tab guard sits outside the reducer.** `MoveTabToWorkspace` (`crates/srv/src/server/service/tab_move.rs:313-338`) reads the source workspace's tab count from SQLite, then dispatches `MoveTab`. Two concurrent moves of a two-tab workspace can both pass and leave it empty. `sagas/tear_off_tab.rs:63-96` has the same shape. `DeleteTab` already closed this race with a guard inside the reducer (`reducer/tab.rs:99-125`). Do the same here: `MoveTab { allow_empty_source }`, false from the RPC and true for the draining sagas.

### Writes that bypass the reducer

- **P1 — Generic object update.** `server/service/object.rs:289` → `object_helpers.rs:73` writes whole Workspace/Tab/Block/Window rows without dispatching, including fields the reducer owns (`tabids`, `activetabid`, `blockids`, `workspaceid`). The only caller today sends layout objects (`frontend/.../layoutAtom.ts:29`), so this is latent. Restrict `update_raw` to object types the reducer doesn't own.
- **P2 — Layout fallback.** When a reducer-owned layout row fails to parse (`object.rs:237-310`), the tree is written directly, and the reducer's copy goes stale.
- **P2 — Host attended flag.** `crates/cef/src/background_audit.rs:424` writes `background_unattended` directly, and `start_hidden.rs:165` logs attended/unattended transitions without updating the reducer's flag.
- Documented exceptions, fine as they are: `window_close.rs:236` (a workspace the reducer doesn't know) and the pre-bootstrap layout seed (`wcore/mod.rs:185-201`).

### Latent bugs and dead code

- **P2 — `LayoutInsertNode` is half-built.** It isn't persisted (`persist_subscriber.rs:298`), and unlike every other node-adding arm, it skips the balance/prune/lock/validate steps (`reducer/layout.rs:172`). It has no producer today. Delete it, or finish it.
- **P2 — Focus/magnify changes aren't persisted** when a structural layout arm makes them (`persist_subscriber.rs:340-343`).
- **P2 — Nine dead srv layout commands.** `LayoutClear`, `InsertNode`, `MoveNode`, `SwapNodes`, `ResizeNodes`, `ReplaceNode`, `SplitHorizontal`, `SplitVertical` and `InsertNodeAtIndex` have no producer (`reducer.rs:89-256`). A pipe client sending one would leave memory and SQLite out of step.
- **P2 — Dead host gates.** `HostLifecyclePhase::ShuttingDown` is only ever set in tests, so the gates at `cef/src/reducer/mod.rs:1160`, `browsers.rs:23` and `panes.rs:47` never fire. `ConfirmDrained` and `PoolLeaveReason` are unused.
- **P3 — `crates/srv/src/backend/wcore/dnd.rs`** reimplements move/tear-off/promote, is unreachable, and is hidden by `#![allow(dead_code)]` at `wcore/mod.rs:1`.

### Purity and cleanup (P2–P3)

- The reducers generate ids and read the clock inside reduce. srv makes UUIDs (`block.rs:32`, `tab.rs:41`, `workspace.rs:19`). cef reads the clock (`browsers.rs:35`, `panes.rs:23`), uses a global label counter (`mod.rs:252`) and reads an env var in `Default` (`mod.rs:226`). Pass these in with the command.
- Duplicated logic:
  - `clear_dangling_focus_magnify` and `reconcile_focus_magnify` (`layout.rs:395`, `:708`);
  - the prune/snap/validate triplet (`:131`, `:955`, `:1039`);
  - 38 inline error-event copies in srv and 26 in launcher (`mod.rs:173-582`);
  - three `bump_version` copies, only one of which saturates.
- Inconsistencies:
  - The active-tab fallback differs between delete (next or previous) and move (first) (`tab.rs:127` vs `:422`).
  - cef clamps opacity to [0,1], the frontend slice to [0.35,1].
- Untested arms: `SetWindowOpacity`, `PopAndPromoteFrontPanePoolWindow`, `ReportHwndIconicChanged`, and srv's rename and meta-update arms.

## 3. Frontend reducers

**Slice reducers (`frontend/app/store/`), all tested:**
- `agent-document`
- `agent-pane-state`
- `agent-pane-layout`
- `browser-pane-state`
- `drone-run-state`
- `launch-flow-state`
- `launcher-event`
- `window-opacity`
- `editor-pane-state-store.ts`, a single-file slice

**Other pure state machines:** `doc-tabs`, `layout/lib/layoutTree.ts`, `view/term/predictive-echo.ts`, `workspace/redock-arming.ts`, `util/event-buffer.ts`, and `view/agent/scroll/{one-way-flow,follower}.ts`.

**State a spec said to move into a reducer, still outside one:**
- **Scroll-follow:** 28 uses of the old follow flags in `AgentDocumentVirtualList.tsx`.
- **The post-mount launch phase:** in `useAgentControllerStatus.ts:308-354` and `flows/launch-flow.ts`. That's 7 signals, 16 `setLaunchPhase` calls, and the `reloginInFlight` / `seedInFlight` / `loginCancelled` flags.
- **The layout tree:** the frontend still pushes the whole tree (`layoutPersistence.ts:448-470`).

**Left outside a reducer on purpose:** the hook-local auto-retry countdown, the view-local `documentState` (A6), and the local expand toggle in `CompactResult.tsx`.

## 4. Open work, deduplicated

| P | Item | Source | Issue |
|---|---|---|---|
| P1 | Launcher reducer file I/O (§2) | code | — |
| P1 | `MoveTab` last-tab guard into the reducer (§2) | code | — |
| P1 | Scroll-follow Phases 1–4: pure follow reducer behind `agent:followcontroller`, then delete the old flags. Fold in one-way-flow Phases 2–3 | SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24, SPEC_AGENT_PANE_ONE_WAY_FLOW | #3655 |
| P1 | Pane tabs Phase 2: `layout_revision`, so a stale whole-tree push can't drop stack members. Phase 1 leftovers: tab switch/close as commands with `actionid` dedup, invariant I1 | SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18 | #4860 |
| P1 | Redock window resolution: `cef/src/commands/window/motion.rs:390-437` still classifies by label prefix, and `window_hwnds` is a separate map (`app/mod.rs:149`). Likely root of #2873 | SPEC_REDUCER_SSOT_CONSOLIDATION R5/R6 | #2873 |
| P1 | Restrict generic `update_raw` to non-reducer types (§2) | code | — |
| P2 | Strong layout authority Phases 4–5: send layout intents, not the whole tree; minimize/dissolve in srv | SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30 | #4861 |
| P2 | Launch phase as a reducer, and one "log in and confirm it landed" core | REPORT_REDUCER_STACK_AUDIT_2026_07_26 items 9, 12 | — |
| P2 | `saga_dispatch.rs:351-364` counts user windows by prefix instead of `count_live_user_windows` | SSOT L3 | — |
| P2 | Pane tabs Phase 3: invariant reaper and loud logging | pane-tabs spec | #4860 |
| P2 | Persist focus/magnify from structural arms; delete or finish `LayoutInsertNode` and the other dead layout commands; drop dead cef gates | code | — |
| P2 | Pass time, ids and label sequence in from outside the reducers | code | — |
| P3 | `browser-pane-state/reducer.ts`: route ~18 inline tab edits through `replaceTab()` | audit 07-26 | — |
| P3 | Rename `launcher-event-reducer.ts` → `launcher-event-store.ts` | audit 07-26 | — |
| P3 | Merge `AuthSessionManager` and `OAuthSessionManager` (`identity/auth_session.rs:181`, `oauth_client.rs:175`) | audit 07-26 | — |
| P3 | Delete or wire `UserExpanded`/`UserCollapsed` (`agent-pane-layout/reducer.ts:185-189`), and the unused save-bundle auth states | layout reducer spec, launch-auth spec | — |
| P3 | Move the bundle-pick signals of `AgentLaunchModal.tsx` into the launch-flow slice | launch-modal spec | — |
| P3 | Deduplicate `layout.rs` and the error boilerplate; delete `wcore/dnd.rs`; fill the test gaps in §2 | code | — |

Related open issues not owned by a reducer doc: #3338 (process/turn-state consolidation), #768 (phantom browser pane), #2188 (macOS quit-chain parity), #942 (service supervision).

## 5. Tracking docs: verified status and what to do with them

**Mark superseded:**
- `SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01`, by Pillar 1 Step 6 (#2191).
- `frontend-reducer-architecture-2026-05-03` and `frontend-reducer-implementation-plan-2026-05-03`, by `frontend-reducer-conventions` and the shipped slices.
- `SPEC_LAUNCH_AUTH_STATE_MACHINE_2026_05_14` C-2..C-6, by the direct-account login flow.

**Mark implemented (historical):**
- `SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27`, built as the launcher reducer.
- The Phase E specs and plans: `SPEC_PHASE_E_SRV_REDUCER`, `SPEC_PHASE_E_SAGAS`, `SPEC_PHASE_E4_LAYOUT_REDUCER`, `PHASE_E_SAGAS_EXECUTION_PLAN`.
- `SPEC_SAGA_ARCHITECTURE_TARGET`, `SAGA_ARCHITECTURE_EXECUTION_PLAN` and `SPEC_SAGA_DURABILITY`.
- `PLAN_SRV_REDUCER_MODULARIZATION`.
- `agent-pane-document-reducer-2026-05-03` (#681).
- `browser-pane-reducer-roadmap`, Phases 0–6.
- `SPEC_AGENT_PANE_STATE_MACHINE_2026_05_07`.
- `docs/plans/PHASE3_LAYOUT_REDUCER_IMPL_PLAN`.
- `SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION`.
- `docs/analysis/AGENT_PANE_REDUCER_AUDIT_2026_05_12`, moot after A6.
- `SPEC_PILLAR1_STEP6_SAGA_COLLAPSE`, done in 50533c3e7.

**Fix the status line:**
- `SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02`: done, via #3906.
- `SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06`: implemented in #1987. The retry countdown stays in the hook by decision.
- `SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18`: add #3441 (`LayoutStackMove`).
- `SPEC_REDUCER_SSOT_CONSOLIDATION_2026_06_22`: L1, L2, L4, L7 and L8 are done; L3, R5, R6 and srv-events gap handling are open.
- `SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30`: Phases 2–3 are done.
- `SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19`: the modal now has 5 local signals, not 2.
- `docs/retro/multi-reducer-status-2026-04-29`: still says "Active reference"; mark it superseded.

**Refresh:**
- `MASTER_REDUCER_STACK_STATUS_2026-05-05` §0:
  - Items 1, 2 and 4 are closed (2 except the seed and the `object.rs` fallback); item 3 (`PipeTarget::Srv`/`LauncherSelf`) is still dormant.
  - It cites `app_api.rs`, which is gone.
  - It uses `is_quitting` (now `quit_state`).
  - It says `UpdateWindowMeta` bypasses the reducer; it no longer does.
  - Its "sole writer" claim is false (§2).
- `docs/architecture/DECISION_SAGA_REDUCER_TWO_FRAMEWORKS_2026_09_07`:
  - It doesn't mention the cef reducer, which is a third framework and its own re-open trigger.
  - Its "no I/O / deterministic" claim is false (§2).
  - Its line counts are stale.
- In-code headers that still describe unfinished work:
  - `crates/srv/src/state.rs:18-21`, `:94-100`;
  - `crates/srv/src/reducer.rs:85-88`;
  - `persist_subscriber.rs:298-303`;
  - `crates/cef/src/reducer/mod.rs:11-20`, `:73-75`.

**Tracking issues opened 2026-10-14:** #4860 (pane tabs Phases 1–3), #4861 (strong layout authority Phases 4–6).
