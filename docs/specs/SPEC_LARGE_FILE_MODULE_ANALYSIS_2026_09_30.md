# SPEC: Large-file module analysis and DRY opportunities

**Date:** 2026-09-30
**Author:** Maricon (charlie)
**Status:** active — items 1, 2, 4 and 12 shipped (#4029, #4030, #4032, #4048); items 5, 10 and 11 partly shipped (#4052, #4050, #4057); `agent-view.tsx` split complete (#4039–#4079 and step 6b); the four largest Rust files split (#4061–#4070); inline-test moves (#4054, #4056). What is left of each is in §0.1.
**Baseline:** `main` @ `4b5814f47` (v0.58.3). Every `path:line` below was read on that commit.
**Related:**
[`SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13`](SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md) (the first agent-view split plan; 7 of 12 steps landed, the file grew anyway),
[`REPORT_AGENT_PANE_ARCHITECTURE_MODULARIZATION_ANALYSIS_2026_07_31`](REPORT_AGENT_PANE_ARCHITECTURE_MODULARIZATION_ANALYSIS_2026_07_31.md) (agent-view at 1,959 lines),
[`SPEC_MONOLITH_MODULE_SPLITS_2026_09_22`](SPEC_MONOLITH_MODULE_SPLITS_2026_09_22.md) (the `persistent/` and `bundle/` move-only splits — the Rust precedent this doc reuses),
[`REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06`](../reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md) (the previous DRY audit; §6 below re-measures its open items),
[`REPORT_TOOL_PREVIEW_DRY_AND_ARCHITECTURE_2026_09_26`](../reports/REPORT_TOOL_PREVIEW_DRY_AND_ARCHITECTURE_2026_09_26.md) (tool-preview DRY; not repeated here).

---

## 0. TL;DR — best opportunities

Ordered by benefit ÷ risk. Size: S ≈ under a day, M ≈ 1–2 days, L ≈ multi-PR.

| # | Opportunity | Benefit | Risk | Size | Depends on |
|---|---|---|---|---|---|
| 1 | **Claude launch args have drifted between Rust and TS** — `--exclude-dynamic-system-prompt-sections` is in `providers.rs:265,274` but not in `catalog.ts:136,150`, so the argv a pane gets depends on which path created it (§5.1 #4). Add the flag to TS and an args-equality test next to `pin-consistency.test.ts` | Fixes a live divergence (since #1964, 2026-07-04) and stops the next one | Very low | S | none |
| 2 | **`agentmux-mcp`'s private root resolver** (`window_capture.rs:33-43`) ignores `AGENTMUX_HOME_OVERRIDE` and falls back to `/` — the exact behaviour #3372 removed from srv. Call `agentmux_common::data_paths::agentmux_root()` (§5.1 #5) | Fixes a live divergence; mcp already depends on common | Very low | S | none |
| 3 | **Split `agent-view.tsx`** (3,001 lines, 110 commits in 60 days) along the seams in §3, starting with the zero-coupling moves | Highest-churn file in the repo; turns the "inline logic is unassertable" problem the file itself names (`agent-view.tsx:1954-1957`, `:2107-2111`) into testable hooks | Low for steps 1–6, medium for 7–10 (forward references, Codex-P1-dense turn logic) | L (11 PRs, each S/M) | Step 0: widen two grep-shaped guard tests first (§3.3) |
| 4 | **One `busyInputFromState()`** feeding `paneBusyForInput` from both `agent-view.tsx:1727-1737` and `useAgentCommands.ts:1227-1237` | Removes the last hand-synced copy of the busy rule; it has drifted twice (#3143, ReAgent P1 on #3340) | Low | S | none (ships alone or as §3.5 step 3) |
| 5 | **Name → slug/id derivations: one named function per rule, one table test** — 10 implementations of 5 rules across Rust and TS (§5.1 #2) | Stops path/key mismatches that have already cost P1s on #2901 and #3633 | Low (pure functions) | S | none |
| 6 | **Split `agentmux-srv/src/server/mod.rs`** (3,327 lines, 81 commits): `AppState`, the 133-route table, auth middleware and ~50 inline HTTP handlers in one file | Second-highest churn; security fixes (5 of its last 12 commits) land in the file every new route touches | Low (move-only, `SPEC_MONOLITH_MODULE_SPLITS` recipe) | M | none |
| 7 | **Make `agentmux-mcp` table-driven, then split it** (4,809 lines, 75 tools each hand-wired in three places) | 09-06 audit step 17, still open; ~3,000 lines of per-tool HTTP plumbing (§4.2) | Low (move), medium (fold) | M | none |
| 8 | **Split `bootstrap.rs`** (2,440 lines, 61 commits) — two functions are 560 and 490 lines | Every new subsystem edits one of two giant functions | Low | S–M | none |
| 9 | **Move the `*_impl` bodies out of `app_api/mod.rs`** into the submodules that already register them | One command is currently split across two files | Low (move-only) | S | none |
| 10 | **Finish the `agentmux-common` lifts and gate them** — `common::time` exists since #3033, yet 9 local copies still compute time themselves, 5 of them added after the lift (§6.1) | A lift without a gate regrows; a grep gate makes the next copy a CI error | Very low | S | none |
| 11 | **Small frontend helpers:** `isAuthFailure`, `isStopping`, `readZoom`/`clampZoom`, `setBlockMeta` — 9, 5, 15 and ~26 hand-written copies (§5.2) | Removes the commonest re-derivations | Very low | S each | none |
| 12 | **Delete the dead `@keyframes pulse`** in `_control-bar.scss:37`; it collides globally with `_status-dot.scss:28` | Last-loaded-wins visual bug | Very low | S | none |

**Should `agent-view.tsx` be split?** Yes — but for testability and review load, not for the line count, and only with a regrowth guard. §3.1 explains; the 04-13 attempt took out seven hooks and the file still went from 1,959 lines (2026-07-31) to 3,001.

### 0.1 Progress (2026-09-30)

| Item | Status | PRs |
|---|---|---|
| 1. Claude launch args drifted | Shipped; the test now compares every provider's args, found from `providers.rs` | #4029 |
| 2. `agentmux-mcp`'s root resolver | Shipped; its tests are isolated from the ambient environment | #4030 |
| 3. Split `agent-view.tsx` | All steps (0–10) shipped: 3,001 → 1,508 lines, 16 modules, a size ratchet, the A6/A9 guards widened. Step 10 made `turn-confirmation.ts` the one owner of the backend-confirmed turn state and moved the focus re-poll and held-message delivery to `useTurnReconciliation.ts`, each with tests pinning the codex/reagent P1 orderings. Step 6b moved the bottom panels into `components/AgentBottomPanels.tsx`, a fragment so each panel stays a direct flex child. **Left:** nothing from §3.5; the estimate of 1,000–1,200 lines assumed `handleSendMessage` and more of the JSX skeleton would move | #4039, #4042, #4043, #4046, #4047, #4049, #4051, #4053, #4055, #4071, #4075, #4078, #4079, this PR |
| 4. One builder for the busy predicate's input | Shipped | #4032 |
| 5. Name → slug rules | Shipped: the Rust side (`agentmux_common::slug`, four named rules, table + oracle tests), and the two TS mirrors (`deriveSlug`, and `jsAsciiSlug`, named out of `buildStartupPayload.ts`) run over the Rust test table by `slug-rules.test.ts` | #4052, this PR |
| 10. Time helpers | **Partly shipped:** the CI ratchet stops new copies; 91 grandfathered files (more than §6.1 counted: it covered helper definitions, not inline uses). **Left:** migrating them to `agentmux_common::time` | #4050 |
| 11. Small frontend helpers | **Partly shipped:** `isAuthFailure`. **Left:** `isStopping`, `readZoom`/`clampZoom`, `setBlockMeta` | #4057 |
| 12. Duplicate `@keyframes pulse` | Shipped | #4048 |
| §4 inline tests | Moved to their own files: `app_api/mod.rs` 4,985 → 2,292, `reactive.rs` 3,839 → 2,598, `native_memory_handlers.rs` 3,339 → 1,674 lines; test counts identical before and after | #4054, #4056 |
| §4 Rust splits | `bootstrap.rs` 2,440 → `bootstrap/` (9 files, `mod.rs` 58); `server/mod.rs` 3,327 → 110 (state, routes, auth, `http_*` families); `app_api/mod.rs` 2,292 → 439 (each `*_impl` in the submodule that registers it); `agentmux-mcp/src/main.rs` 4,809 → 346 (helpers and tests out, then `call_tool`'s 71 arms into `tools/<family>.rs`, 14 families, with a test that every advertised tool has exactly one handler). Test counts identical before and after each move. **Left:** §4.2 (b), one `ToolDef` table per family, which changes some error wording | #4061, #4068, #4066, #4070, this PR |

Found along the way and fixed: `sanitizeLogTextForTerminal` let the text of an OSC sequence with a space (or an ST terminator) through into the shell drawer (#4045); the progress ring's flicker and orphaned background-task rows (retro in `docs/retro/`, #4035, #4040).

---

## 1. Method

- **Inventory.** `git ls-files` filtered to `*.rs *.ts *.tsx *.scss *.mjs *.js *.cjs *.sh *.ps1`, then `wc -l`.
- **Excluded as generated:** every file whose first five lines carry a generator marker (`This file was generated`, `DO NOT EDIT`, `@generated`, `auto-generated`). That matched the 374 ts-rs bindings in `frontend/types/rpc/` and `agentmux-launcher/src/splash_font.rs`, and nothing else. `*.d.ts` were excluded too.
- **Excluded as not hand-written source:** `node_modules/`, `target/`, `dist/`, `build/`, `test/`, fixtures, snapshots, lockfiles.
- **Excluded as tests:** `*.test.*`, `*.spec.*`, `tests.rs`, `*_tests.rs`, any `tests/` directory. **Inline Rust test modules are subtracted**, not excluded: many large `.rs` files are a third to two-thirds `#[cfg(test)] mod … { }` (e.g. `app_api/mod.rs` is 4,985 lines of which 2,726 are tests). The "prod" column counts lines outside those modules, measured by matching each `#[cfg(test)]`-attributed top-level `mod x {` to its closing `}` at column 0.
- **Churn:** commits touching the file since 2026-07-31 (60 days), `git log --since=2026-07-31 --name-only`. `main` merged 1,492 commits in that window. Files created by a rename in the window (e.g. `persistent/spawn.rs`, 2026-09-22) are undercounted, because `--follow` was not used.
- **Priority signal:** prod lines × churn.
- **Duplication:** grep/`rg` sweeps for repeated expressions, cross-checked by reading each site; `git log --grep` over commit bodies for `drift`, `duplicat`, `second copy`, `two copies`, `same rule`, `re-deriv`, `keep in sync`. Nothing below is from a clone-detection tool; where a count is approximate it says so.

---

## 2. Ranked inventory (top 30 by prod lines × churn)

| # | Path | Total | Prod | Churn (60d) | Lang | Responsible for |
|---|---|---:|---:|---:|---|---|
| 1 | `frontend/app/view/agent/agent-view.tsx` | 3,001 | 3,001 | **110** | TSX | Agent pane content + chrome model: picker cross-fade, pane registration, turn reconciliation, auth/bind, live feed, working indicator, the whole JSX tree |
| 2 | `agentmux-srv/src/server/mod.rs` | 3,327 | 3,327 | **81** | Rust | `AppState`, the HTTP router (133 `.route(` calls), ~50 inline HTTP handlers, auth middleware |
| 3 | `agentmux-mcp/src/main.rs` | 4,809 | 3,916 | 47 | Rust | MCP stdio server: `tools/list` and `call_tool` dispatch + HTTP call for all 75 tools (schemas live in `tool_schemas.rs`) |
| 4 | `agentmux-srv/src/bootstrap.rs` | 2,440 | 2,402 | 61 | Rust | srv startup: logging, stores + migrations, background subsystems, listeners, delivery hooks |
| 5 | `agentmux-srv/src/server/app_api/mod.rs` | 4,985 | 2,259 | 54 | Rust | App API: `open_pane`, `agent_define_core`, memory / global-memory / bundle / identity `*_impl` bodies |
| 6 | `agentmux-srv/src/backend/storage/agents.rs` | 3,829 | 3,158 | 38 | Rust | `AgentDefinition` / `AgentInstance` storage, slugs, name keys, purge |
| 7 | `agentmux-srv/src/server/reactive.rs` | 3,839 | 2,565 | 38 | Rust | Jekt delivery: four signature verifiers, tiers, `deliver`, reactive HTTP handlers |
| 8 | `agentmux-srv/src/backend/storage/migrations.rs` | 3,022 | 2,381 | 40 | Rust | SQL schema DDL for five databases (`run_object_schema` alone is 558–1396) |
| 9 | `frontend/app/view/agent/styles/_document-nodes.scss` | 2,635 | 2,635 | 32 | SCSS | Every transcript node's styling |
| 10 | `agentmux-srv/src/server/websocket.rs` | 2,271 | 1,875 | 41 | Rust | WS envelope, event forwarding, handler registration |
| 11 | `agentmux-srv/src/backend/reactive/handler.rs` | 2,368 | 2,324 | 31 | Rust | `ReactiveHandler` — agent registry, injection, supervisor nudges |
| 12 | `agentmux-srv/src/backend/blockcontroller/persistent/spawn.rs` | 2,143 | 2,143 | 30 | Rust | `spawn_process` (Phase 3 of `SPEC_MONOLITH_MODULE_SPLITS`, still open) |
| 13 | `agentmux-srv/src/server/native_memory_handlers.rs` | 3,339 | 1,668 | 30 | Rust | Native memory RPCs |
| 14 | `agentmux-srv/src/backend/blockcontroller/shell/lifecycle.rs` | 2,831 | 2,257 | 22 | Rust | `impl Controller for ShellController` (kept whole deliberately) |
| 15 | `agentmux-srv/src/server/app_api/agent_open.rs` | 1,750 | 1,210 | 41 | Rust | `agent.open` |
| 16 | `frontend/app/view/agent/agent-model.ts` | 960 | 960 | 46 | TS | `AgentViewModel` |
| 17 | `agentmux-srv/src/server/agent_handlers/input.rs` | 2,129 | 1,461 | 30 | Rust | Agent input RPCs |
| 18 | `frontend/app/view/agent/components/AgentComposerStrip.tsx` | 1,525 | 1,525 | 26 | TSX | Composer status strip |
| 19 | `frontend/app/block/blockframe.tsx` | 1,400 | 1,400 | 28 | TSX | Block frame / header |
| 20 | `frontend/app/view/swarm/swarm-model.ts` | 2,085 | 2,085 | 17 | TS | Swarm view model |
| 21 | `agentmux-srv/src/muxbus/cloud_subscriber.rs` | 2,012 | 1,511 | 23 | Rust | Cloud push subscription |
| 22 | `agentmux-srv/src/server/app_api/session.rs` | 2,302 | 1,699 | 20 | Rust | Session App API (resume preflight etc.) |
| 23 | `agentmux-srv/src/backend/blockcontroller/persistent/mod.rs` | 1,674 | 1,674 | 19 | Rust | Persistent controller core |
| 24 | `frontend/app-init.ts` | 1,295 | 1,295 | 24 | TS | Frontend bootstrap |
| 25 | `agentmux-srv/src/backend/blockcontroller/mod.rs` | 1,545 | 1,039 | 29 | Rust | Block controller registry |
| 26 | `frontend/app/view/agent/components/AgentPicker.tsx` | 1,127 | 1,127 | 25 | TSX | My Agents picker |
| 27 | `agentmux-cef/src/state/mod.rs` | 1,624 | 1,624 | 17 | Rust | CEF host shared state |
| 28 | `frontend/app/view/agent/components/AgentFooter.tsx` | 1,309 | 1,309 | 21 | TSX | Composer |
| 29 | `frontend/app/view/swarm/swarm-view.tsx` | 1,210 | 1,210 | 22 | TSX | Swarm view |
| 30 | `agentmux-srv/src/backend/rpc_types/block.rs` | 1,207 | 899 | 29 | Rust | block/pane RPC types |

Observations:

- **Large and cold is fine.** `agentmux-bashwrap/src/bash_wrap.rs` (3,633 total, 2,300 prod, 9 commits), `srv/reducer/layout.rs` (3,284 / 1,076 / 4), `blockcontroller/app_server_protocol.rs` (2,633 / 1,221 / 1) and `persist_subscriber.rs` (2,426 / 1,251 / 1) are big but nobody is editing them. Not targets.
- **Rust size is inflated by inline tests.** Of the ten largest `.rs` files, six are more than a third tests. Moving tests to `tests/` (the `persistent/` recipe) is the cheap first move for `app_api/mod.rs` (2,726 test lines), `server/reactive.rs` (1,274), `native_memory_handlers.rs` (1,671).
- **The agent pane is four of the top 30** (`agent-view.tsx`, `agent-model.ts`, `AgentComposerStrip.tsx`, `AgentFooter.tsx`) plus `_document-nodes.scss`.

---

## 3. `frontend/app/view/agent/agent-view.tsx`

### 3.1 Verdict — split it, for testability, with a regrowth guard

Size history, from `git show <rev>:agent-view.tsx | wc -l` at month boundaries: 1,215 (07-15) → 1,959 (07-31) → 2,301 (08-15) → 2,485 (09-01) → 2,984 (09-15) → 3,001 (today). In the 60-day window: 110 commits, +3,341 / −2,299 lines. Commit subjects are overwhelmingly `fix(agent-pane)` / `fix(agent)` / `feat(agent-pane)` (51 of 110).

Reasons to split, in order of weight:

1. **The file says its own logic is untestable where it sits.** Three separate comments: `decideSyntheticRow` was extracted "after this effect produced several P1s across PR #2951 — it sits inline in the pane component and no existing harness can reach it" (`agent-view.tsx:1954-1957`); the bind retry ladder was moved out "for the same reason … inline logic in this component is unassertable by any existing harness" (`:2107-2111`); the shell-exit body lives in `shell-exit-collapse.ts` "so it can be tested — this file has no render harness" (`:791-793`). Every extraction so far was reactive, after bugs. §3.4 lists the remaining inline logic in the same position and where each piece goes.
2. **Review load.** One file takes 1.8 commits a day. Any agent-pane PR touches it, so any two agent-pane PRs conflict in it.
3. **One closure scope makes coupling invisible.** `AgentPresentationView` (`:514-2999`) is a single ~2,490-line function. Values are used before they are defined and bridged by mutable `let`s (§3.3), which is how the ordering P1s on PR #2338 happened (`:1212-1221` records one).

What splitting will *not* do: shrink the prose. 1,279 of 3,001 lines (43 %) are comment lines, much of it the rationale for past P1s. That text is valuable and should move *with* the code it explains, not be trimmed.

What stops it regrowing: the 04-13 spec's own status line — seven extractions landed "and more than that went back in". Add a size ratchet test next to the existing grep-shaped guards (§3.5 step 0): fail if `agent-view.tsx` grows past its current line count, and lower the number with each extraction. The docs ratchet (`scripts/check-docs-lifecycle.mjs`) is the local precedent for a ratchet that only tightens.

### 3.2 Section map

`AgentPresentationView` is the body from `:514` to `:2999`. "Reads" lists what each section depends on besides `model`, `paneModel` and `block()`.

| Lines | Section | Reads | Writes / exposes |
|---|---|---|---|
| 157-197 | Constants, `ANSI_SEQUENCE_RE`, `sanitizeLogTextForTerminal` (untested; the comment at 157-164 describes the regex but sits above `PICKER_FADE_OUT_MS`) | — | pure |
| 218-340 | `AgentBlockContent`: history-tab / picker / presentation switch, picker cross-fade (243-284) | `agentId`, `prefersReducedMotionAtom` | — |
| 379-508 | `buildAgentPaneChromeModel`: fork pills, tab switch/close | `nodeModel`, `useForkSet`, `agentModels` | `PaneChromeModel` |
| 537-559 | Basic derivations: `hidden`, `providerKey`, `provider`, `outputFormat`, `agentName`, `currentAgent` | block meta, `agentDefinitions` | accessors used everywhere |
| 569-596 | Stash drawer callbacks onto the model | `paneModel.state.stashOpen` | `model._openAgentStashModal` etc. |
| 598-709 | Pane registration + dispose diagnostics; shell sub-block id mirror (636-640); ctx-token meta mirror (702-708) | `turnPhase`, `lastContextTokens` | registration, `shellSubBlockIdRef` |
| 711-728 | Layout slice lifecycle | — | `layoutView` |
| 730-815 | Activity log → shell terminal bridge: `log`, `formatLogLine`, `handleShellTermReady/Dispose`, `handleShellExited` | `termWrite` signal, `shellSubBlockIdRef` | `log` (passed to ~10 hooks) |
| 817-955 | Reveal readiness part 1 + `useHistoryPagination` (open-trace, readiness gates, `historyPainted`, double-rAF settle) | `liveFeedOn` (declared later), `scheduleRollOff` (hoisted fn) | `history`, `historyPainted` |
| 957-961 | `useResumePreflight` | — | `resumePreflight` |
| 963-1114 | **Live feed roll-off**: `runRollOff`, `whenIdle`, `scheduleRollOff`, `feedOverBudget`, triggers, `earlierHistoryAvailable`, `earlierTurnsKnown` | settings, `history.*`, `hidden`, `followingBottom` (assigned in JSX) | `scheduleRollOff`, `gapsBefore`, … |
| 1116-1132 | `displayDocument` (history link, gap rows, resume preflight injection) | roll-off outputs, `resumePreflight` | read-only doc |
| 1134-1169 | Snapshot persistence, decisions, questions hooks | `getDocument`, `log`, `handleSendMessage` (declared later, called via thunk) | queues |
| 1171-1263 | **Turn-end tracking**: `wasTurnActive`, `turnJustEndedAtom`, `reconcileTurnActive`, `trackTurnJustEnded`, ghost-tool scrub | `commands` (declared at 1803) | `turnJustEndedAtom` |
| 1265-1282 | `postSystemNotification` | — | doc dispatch |
| 1284-1359 | `useAgentControllerStatus` + `onRecovered` | `retryLastTurn` (declared later), `rootRef` | `status` |
| 1361-1414 | Reveal readiness part 2: auth-phase gate, 3 s safety timeout, `revealing → live` | `status.launchPhase` | readiness releases |
| 1416-1436 | `showingLaunchActivity`, `loginStatus` | `status`, `failure` | accessors |
| 1438-1445 | Launch on mount | `status.startLaunchFlow` | — |
| 1447-1508 | `useControllerStatusEvents` wiring | `declareAuthHealthy` (declared at 2089) | — |
| 1510-1587 | Focus re-poll of `GetControllerStatus` | `trackTurnJustEnded`, `commands` | — |
| 1589-1633 | Block activity, activity summary, next-prompt suggestion, ambient narration | `turnPhase`, `turnJustEndedAtom`, `composerIsEmptyFn` (assigned in JSX) | — |
| 1634-1680 | `useAgentStream`; `scrollToBottomFn` ref | `scrollToBottomFn` (assigned in JSX) | `backgroundTasksAtom` |
| 1682-1746 | **Working indicator**: promotion clock (1696-1707), `paneBusy` (1727-1737), `workingRowVisible` (1740-1746) | `turnPhase`, `compacting`, `reconnecting`, `attachedTask`, `registryAttachedTaskSince`, document | `paneBusy` — the progress bar (2541-2556), working row (2823-2848) and composer strip (2866) all read it |
| 1748-1799 | **Attached-task axis dispatch** (second promotion clock at 1795-1798) | document, `allSubagentsAtom`, `registryAttachedTaskSince` | `AttachedTaskObserved/Cleared` |
| 1801-1862 | `useAgentCommands` (24 options) | `status.*`, `wasTurnActive`, `scrollToBottomFn` | `commands` |
| 1864-1948 | `handleSendMessage`, Esc-on-empty, `retryLastTurn` | `commands`, snapshot | — |
| 1949-2001 | Synthetic pre-launch auth failure row (decision is in `failure/synthetic-row.ts`) | `status.canRetry`, `failure` | failure dispatch |
| 2003-2143 | **Account binding + auth health**: account cache, linked account, `authEmail`, `bindCandidates`, `runBind`, `declareAuthHealthy`, bind recheck, `agentidentities:changed` subscription | `provider`, `status`, snapshot | `authEmail`, `onBindAccount`, `onSwitchAccount` |
| 2145-2202 | `useAgentFailure` wiring | many callbacks | `failureUI` |
| 2204-2242 | Held-message flush + deferred controller refresh effect | `currentTool`, `turnPhase`, `commands` | — |
| 2244-2312 | Startup sequence (`onReadyFn`): four RPCs + `buildStartupPayload` | `currentAgent`, `provider`, `agentDefinitions` | sends first turn |
| 2314-2398 | Search, keyboard, zoom memo, drop-attach, PTY width, context menu | `rootRef` | — |
| 2400-2999 | JSX: loading cover, shutdown overlay, stash drawer (2451-2518), zoomed wrapper (2519-2940: progress-bar Portal 2541-2556, `/btw` overlay, search bar, document scroll region 2619-2657, auth panel, decision/question/pending panels, chips and banners, failure row 2755-2773, ActivityDock, working row 2823-2848, session notices, composer strip 2864-2892, composer region 2894-2939), shell drawer (2954-2995) | everything above | — |

### 3.3 Coupling hazards a split has to respect

- **Forward references bridged by mutable `let`.** `onReadyFn` (declared 821, assigned 2247), `historyReadyFn` (836, assigned in JSX 2650), `scrollToBottomFn` (1680, JSX 2646), `followingBottom` (977, JSX 2642), `composerIsEmptyFn` (1611, JSX 2935). `commands` is used inside `trackTurnJustEnded` (1233) and the focus poll (1577) before its `const` at 1803 — safe only because those run from async callbacks. `retryLastTurn` and `declareAuthHealthy` are likewise called before their declarations. Any extracted hook must take these as accessors or callbacks, never as values captured at call time.
- **`wasTurnActive` is shared mutable state read and written from two directions.** `trackTurnJustEnded` writes it (1222); `useAgentCommands` reads it through `isBackendTurnActive` / `isBackendTurnConfirmedIdle` (1832, 1846). The ordering comment at 1212-1221 exists because getting that read/write order wrong was a Codex P1 on #2338. It needs one owner object that both sides receive.
- **Two grep-shaped guard tests read this file by name.** `agent-view-dispatch-via-pane-model.test.ts:27` (no raw `dispatch*` calls; A9) and `frontend/app/store/agent-pane-view.test.ts:113` (no writes to reducer-owned signals; A6). Code moved into a new hook is no longer scanned. **Widen both to scan the new modules before the first extraction**, or each move quietly weakens the guard.
- **DOM placement is load-bearing.** The stash drawer must be a direct flex child of `.agent-view` (2471-2487); the loading cover and shell drawer must stay outside `.agent-view-zoomed` (2439-2447, 2948-2953). Extracted components must render their root element directly, with no wrapper `div`.
- **Hook order inside the body is semantic.** Registration (648) must precede any hook whose `onMount` dispatches (600-605). `beginAgentOpenOnMount` must precede `useHistoryPagination` (866-868).

### 3.4 Proposed modules

Dependency direction: `agent-view.tsx` imports the new modules; the new modules import stores, `activity/`, `failure/`, `working-indicator.ts`, never `agent-view.tsx`. Existing homes are used wherever one fits.

| Target | Moves from | ~Lines out | Notes |
|---|---|---:|---|
| `agent-block-content.tsx` | `AgentBlockContent` 199-342 + `PICKER_FADE_OUT_MS` | 150 | Only importer is `agent-manifest.tsx:7`; re-export or repoint |
| `agent-pane-chrome-model.ts` | `buildAgentPaneChromeModel` 344-508 | 165 | Independent of the presentation view |
| `hooks/useShellLogBridge.ts` | 157-197, 730-815 | 130 | Returns `log`, `onTermReady`, `onTermDispose`, `clearTermWrite`. `sanitizeLogTextForTerminal` gets its first tests |
| `working-indicator.ts` (extend) | inputs of 1727-1737 | 15 | `busyInputFromState(state, showingLaunch, nodes)`; `useAgentCommands.ts:1227-1237` calls it too (§5.1 #1) |
| `activity/tool-adapter.ts` (extend) | 1696-1707, 1795-1798, `ActivityDock.tsx:81-94` | 25 | `createPromotionClock(nodes)` — one tick accessor instead of three nonce+timer copies |
| `hooks/useWorkingIndicator.ts` + `components/AgentProgressBar.tsx` | 1682-1746, Portal 2541-2556 | 90 | Returns `paneBusy`, `workingRowVisible`, `hasPromotedTool` |
| `activity/useAttachedTaskAxis.ts` | 1748-1799 | 55 | The start-time merge (1784-1787) becomes a pure function in `activity/attached-task.ts` with tests |
| `hooks/useLiveFeedRollOff.ts` | 963-1114 | 155 | Takes `history` as accessors; exposes `scheduleRollOff`, `liveFeedOn`, `gapsBefore`, `earlierHistoryAvailable`, `earlierTurnsKnown`, `setFollowingBottom` |
| `hooks/usePaneReveal.ts` | 868-894, reveal half of 913-943, 1361-1414 | 120 | Owns readiness, gates, `historyPainted`, the settle rAFs, the auth-phase timeout |
| `failure/useAccountBinding.ts` | 2003-2080 | 80 | `authEmail`, `bindCandidates`, `onBindAccount`, `onSwitchAccount` |
| `failure/useAuthHealth.ts` | 1949-2001, 2082-2143 | 120 | Synthetic row, `declareAuthHealthy`, bind recheck, identity-change subscription; uses the new `isAuthFailure` |
| `startup/sendStartupSequence.ts` | 2244-2312 | 70 | Plain async function with injected RPCs, next to `buildStartupPayload.ts`; testable without a render harness |
| `hooks/useTurnReconciliation.ts` | 1171-1263, 1510-1587, 2204-2242 | 230 | Owns a `TurnConfirmation` object (`wasTurnActive`, `track`, `isActive`, `isConfirmedIdle`) created before `useAgentCommands` and passed to it. Separate from `hooks/useTurnLifecycle.ts` (422 lines, stream-side) |
| `components/AgentStashDrawer.tsx`, `components/AgentShellDrawer.tsx`, `components/AgentBottomPanels.tsx` | 2451-2518, 2941-2995, 2659-2859 | 330 | JSX only. Root element rendered directly (§3.3) |

Estimated result: `agent-view.tsx` at roughly 1,000–1,200 lines — hook calls, `handleSendMessage`, and the JSX skeleton. This is an estimate from the ranges above, not a measurement.

### 3.5 Order

Each step is its own PR, behaviour-preserving, with `npx vitest run frontend/app/view/agent` and `tsc --noEmit` green. Steps 1–4 have no forward references at all.

0. **Guards first.** Widen the two grep-shaped tests (§3.3) to scan `agent-view.tsx` plus the new module list; add the line-count ratchet at 3,001. Also move the misplaced ANSI comment (157-164) onto the regex.
1. Move `AgentBlockContent` and `buildAgentPaneChromeModel` out (pure moves).
2. `useShellLogBridge` + tests for `sanitizeLogTextForTerminal`.
3. `busyInputFromState` + `createPromotionClock` + `useWorkingIndicator` + `AgentProgressBar`. Removes two DRY copies (§5.1 #1, §5.2 #5).
4. `sendStartupSequence` (async function, unit-tested).
5. `useAttachedTaskAxis`.
6. JSX components: stash drawer, shell drawer, bottom panels.
7. `useLiveFeedRollOff` — first step with cross-hook ordering (history ↔ roll-off).
8. `usePaneReveal`.
9. `useAccountBinding` + `useAuthHealth`.
10. `useTurnReconciliation` — last: densest Codex-P1 history (PR #2338 is cited 13 times in this file's comments), and the only step that changes how state is shared rather than only where code lives.

---

## 4. The next offenders

### 4.1 `agentmux-srv/src/server/mod.rs` — 3,327 prod lines, 81 commits

What is in it: module declarations (1-45); `AppState` (98-318); `build_routers_with` (336-857), one function with 133 `.route(` calls; health/diag/discovery (866-1043); shell and PTY-shell HTTP handlers (1044-2146, including `handle_pty_shell_create` at 1509-1899); pane/agent open (2147-2210); App-API HTTP adapters for memory and global memory (2211-2686), which are thin wrappers over `app_api::*_impl`; presets, identity, self (2687-2824); naming (2825-3019); layout/tab/window (3020-3134); origin checks and auth middleware (3135-3327).

Why it churns: every new route, every new `AppState` field, and every security fix lands here. Five of the last twelve subjects are `fix(security)` / `feat(security)`, reviewed in a diff that also carries route plumbing.

Proposed split (move-only, `SPEC_MONOLITH_MODULE_SPLITS` §3 recipe): `server/state.rs` (`AppState`, `HostIpc`), `server/routes.rs` (`build_routers_with`), `server/auth.rs` (3135-3327 — so security review has one file), `server/http/{shell.rs, pty_shell.rs, memory.rs, naming.rs, layout.rs, diag.rs}`. Low risk; `pub(crate)` re-exports keep paths stable.

DRY found here: two error-string → HTTP-status classifiers that disagree. `app_api_error_status` (`:2211-2224`) maps `"not found"` to 400; `name_call_error_status` (`:3009-3017`) maps it to 404. Both classify by substring. See §5.3 #8.

### 4.2 `agentmux-mcp/src/main.rs` — 3,916 prod lines, 47 commits

`agentmux-mcp/src/` is already three files — `main.rs` (4,809), `tool_schemas.rs` (1,042: 75 schema string constants), `window_capture.rs` (823) — but every tool is still wired by hand in three places: its schema constant, a parse line in `tools/list` (`main.rs:163-272`), and an arm of `call_tool`'s single `match` (`main.rs:764`; arms from 783 to the `_ =>` at 3727). The arms repeat one shape. Counts by grep over `main.rs`: `"missing required parameter"` 66×, `X-AuthKey` header 70×, `local_url.trim_end_matches('/')` 69×. The helper that already does this properly, `srv_get_json` (`main.rs:330`, separates transport from HTTP errors, four tests), has two callers. `require_agent_env` (`:306`) has 42 callers, yet the same check is written inline elsewhere (e.g. `:789`, `:853`, `:895` per the sweep), and `AGENTMUX_AGENT_ID` is read inline 9 times despite `agent_slug()` at `:408`.

Proposed, in two PRs: (a) move-only — `tools/<family>.rs` per tool family (shell, pty shell, memory, global memory, panes/tabs/windows, fleet, browser, UI, work queue, cron, …), `call_tool` dispatching by family; (b) the fold — one `ToolDef { name, schema, handler }` table per family so `tools/list` and dispatch read the same table, and a POST sibling of `srv_get_json` as the single HTTP path. (b) changes error wording on some tools; review it as a behaviour change, not a move.

### 4.2a Why these three before the rest

`server/mod.rs`, `agentmux-mcp/src/main.rs` and `app_api/` change together: of the 47 commits touching mcp `main.rs` in the window, 29 also touched `server/mod.rs` or `server/app_api/` (e.g. `818ba0ffc` #3237, Global Memory tools: +136 in mcp, +453 in `app_api/mod.rs`, +97 in `server/mod.rs`). Splitting all three along the same family lines (memory, global memory, panes, fleet, …) means a new tool lands in three small family files instead of three monoliths.

### 4.3 `agentmux-srv/src/bootstrap.rs` — 2,402 prod lines, 61 commits

Two functions carry most of it: `open_stores_and_migrate` (480-1041, 560 lines) and `spawn_background_subsystems` (1062-1550, 490 lines). `install_agent_turn_delivery` (2204-2366) is a third. The recent subjects are all "feat: add subsystem X" — each one appends to one of those functions.

Proposed split: `bootstrap/{logging.rs (276-417), watchers.rs (47-275), stores.rs (457-1041), background.rs (1042-1550), network.rs (1551-1775), plumbing.rs (1776-2056), shutdown.rs (2057-2164), delivery.rs (2165-2398)}`, and within `background.rs` one `fn spawn_<subsystem>` per subsystem so a new one is a new function, not a longer one. Low risk.

### 4.4 `agentmux-srv/src/server/app_api/mod.rs` — 2,259 prod + 2,726 test lines, 54 commits

The submodules already exist (`memory.rs`, `identity.rs`, `bundle/`), but the bodies they call live in the parent: `memory.rs:21,57` call `memory_list_impl` / `memory_write_impl` defined at `mod.rs:1160,1247`; `bundle/mod.rs:125,324` call `bundle_list_impl` / `bundle_self_get_impl` at `mod.rs:892,979`; global-memory impls (1386-1781) are called from `server/mod.rs`'s HTTP adapters. So one command is split across two files in the wrong direction.

Proposed: move each `*_impl` into the submodule that registers it (`memory.rs` gains 1160-1385; a new `global_memory.rs` gets 1386-1781; `bundle/` gains 892-1052; `identity.rs` gains 769-891), keep `open_pane` (96-400) as `pane_open.rs` beside `pane.rs`, and move the ten inline test modules to `tests/`. Leaves `mod.rs` as registration plus `SelfOwner`. Move-only.

### 4.5 `agentmux-srv/src/backend/storage/agents.rs` — 3,158 prod lines, 38 commits

Two `impl Store` blocks: definitions (369-1479) and instances (1480-2607), then name-key and purge helpers (2608-3106). Proposed: `storage/agents/{definition.rs, instance.rs, name_keys.rs, purge.rs, row.rs}` — `row.rs` holding `AGENT_DEFINITION_SELECT`, `INSTANCE_COLUMNS` and the two row mappers. The slug derivations here (218-254, 2857-2882) are the subject of §5.1 #2.

### 4.6 `agentmux-srv/src/server/reactive.rs` — 2,565 prod lines, 38 commits

Shape: forwarding (26-313); four signature verifiers — `verify_jekt_signature` (355), `verify_reagent_signature` (412), `verify_lan_signature` (462), `verify_cross_channel_signature` (574) — each with its own max-age constant (314, 382, 441, 539); tier resolution (677-762); `deliver` and hold/relay (1024-1687); HTTP handlers (1688-2771); WS registration (2772). Eleven inline test modules, interleaved with the code.

Proposed: `server/reactive/{forward.rs, signatures.rs, tier.rs, deliver.rs, handlers.rs, ws.rs}` plus `tests/`. Note the naming overlap with `backend/reactive/` (`handler.rs`, 2,324 prod lines): `server::reactive` is transport + delivery, `backend::reactive` is the registry. A one-line cross-reference in each module header would save readers the confusion (the same fix `999efdd59` applied to saga/reducer).

### 4.7 Also worth a line

| File | Suggested move | Risk |
|---|---|---|
| `backend/storage/migrations.rs` (2,381 prod) | One file per database: `schema/{objects.rs, shared_store.rs, identity_store.rs, filestore.rs, saga_log.rs}` — `run_object_schema` alone is 558-1396 | Low |
| `frontend/.../styles/_document-nodes.scss` (2,635) | Split by node family into `_nodes-tool.scss`, `_nodes-markdown.scss`, `_nodes-user-message.scss`, `_nodes-results.scss`, `_nodes-dividers.scss`, `_nodes-peek.scss`. Two tests read it by name (`styles/peek-panel.test.ts:16`, `styles/tool-panel-height.test.ts:36,42,62`) and must be repointed. Also dedupe `.agent-tool-write`, defined twice (1472 inside `.agent-view`, 1979 at top level) | Low |
| `persistent/spawn.rs` | Already scoped as Phase 3 of `SPEC_MONOLITH_MODULE_SPLITS` §6; not repeated here | Medium |
| `useAgentCommands.ts` (1,940), `AgentComposerStrip.tsx` (1,525) | Revisit after agent-view step 10 — `useAgentCommands` takes 24 options from agent-view, and several go away with `TurnConfirmation` | — |

---

## 5. DRY findings

### 5.1 Where drift has already caused bugs

These come first because the drift is not hypothetical.

**#1 — The busy predicate's inputs, assembled twice.** `agent-view.tsx:1727-1737` builds `paneBusyForInput`'s input from `paneModel.state`; `useAgentCommands.ts:1227-1237` builds the same input from `paneSnapshot()`, with its own `?? { kind: "Idle" }` / `?? null` defaults. Both spell out `attachedTask != null || registryAttachedTaskSince != null` and `hasBlockingForegroundToolCall(document)`. History: `4468ae908` (#3143) — "Not a race — three hand-written copies of one idea that had drifted", and its review round added `compacting`/`reconnecting` after the first revision missed them; `35bb36481` (#3340) — ReAgent P1: the send path "called `turnHeldOnlyByBackgroundWork` alone and thereby dropped the `compacting`/`reconnecting` terms" (recorded at `working-indicator.ts:37-42`, which ends "THIS FUNCTION MUST CHANGE WITH IT"). **Home:** `busyInputFromState(state, showingLaunchActivity, nodes)` in `working-indicator.ts`, called at both sites.

**#2 — Name → slug / id / path component.** Ten implementations of five different rules:

| Location | Rule | Used for |
|---|---|---|
| `agentmux-srv/src/backend/storage/agents.rs:231-254` `derive_slug` | lowercase, ASCII alnum/`-`/`_`, else `-`, collapse dashes, cap 64, `"agent"` fallback | definition slug |
| `frontend/app/view/agent/agent-config-builder.ts:173-180` `deriveSlug` | TS mirror of the above ("keep the two in sync", `:168-170`) | SKILL.md preview paths |
| `agents.rs:218-225` `default_agent_working_dir` | lowercase, **Unicode** alnum/`-`/`_`, else `-`, no collapse | default work dir |
| `agentmux-srv/src/server/app_api/agent_open.rs:534-536` | inline copy of the Unicode rule (`agent_slug`) | `agent.open` |
| `agents.rs:2858-2863` `agent_open_fallback_id` | Unicode rule again | signing-key names |
| `frontend/app/view/agent/startup/buildStartupPayload.ts:193` | lowercase, `/[^a-z0-9-_]/g` → `-`, **no collapse** | `AGENT_SLUG` template var when the agent has no slug |
| `agents.rs:2872-2882` `frontend_fallback_id` | Rust re-implementation of that uncollapsed regex, including JS's UTF-16 code-unit behaviour. Its comment (`:2866`) cites `agent-config-builder.ts`, whose `deriveSlug` *does* collapse; the only uncollapsed copy in the frontend today is `buildStartupPayload.ts:193`. Which one it means is unclear | signing-key names |
| `frontend/app/view/agent/defaults/instance-slug.ts:24-30` `slugifyInstanceName` | lowercase, whitespace → `-`, **strips** (not replaces) the rest | instance slug |
| `agentmux-srv/src/backend/reactive/registry.rs:177` and `:335` | Unicode rule, else **`_`** (two copies in one file) | registry file names |

History: PR #2901 — work dir derived two ways, "Personal Bundle broke for the common case" (`agent_open.rs:537-541`); #3633 — two Codex P1s on `frontend_fallback_id` / `key_names_of` (`agents.rs:2845-2847`, `:2869-2871`) because the key purge has to reproduce every rule ever used to name a key. A guard test already pins one pair (`native_memory_handlers.rs:3166-3176`). **Proposal:** not one rule — the rules differ on purpose (a path component and a display slug are different things) — but one named function per rule in one place (`agentmux-common` for the Rust ones), each existing site calling it, and one table-driven test that runs every function over the same inputs (`"Zed Bot"`, `"Ä"`, `"Agent 🚀"`, `"--x--"`) so the differences are stated rather than discovered. `registry.rs:177` and `:335` should become one call.

**#3 — Persistent-vs-container controller rule.** `1bee61f6b` (#2867): "the same rule lives in two places and only srv's copy was right" — container agents had never launched. That fix extracted `frontend/app/view/agent/launch-args.ts`, used by `launchAgentDefinition` (`agent-model.ts:516`). Copies remain: the bare-provider `launchAgent` still inlines the pre-fix check (`agent-model.ts:313-316`, `:335`), and srv has it twice (`agent_open.rs:428-431` and `:523-526`). The `launchAgent` copy is harmless only if that path can never create a container agent — not verified here. **Home:** `launch-args.ts` on the TS side; one `fn controller_type_for(provider, agent_type)` in `agent_open.rs`.

**#4 — Claude launch args (live divergence).** `providers.rs:246-281` carries `--exclude-dynamic-system-prompt-sections` in both `launch_args` (`:265`) and `persistent_launch_args` (`:274`), added by `7f2f591f1` (#1964, 2026-07-04). The TS catalog's `launchArgs` (`catalog.ts:136`) and `persistentLaunchArgs` (`:150`) never got it, although `catalog.ts:145-146` says "Keep in sync with `static CLAUDE`". The persistent controller reads its argv from block meta `cmd:args` (e.g. `persistent/eager_resume.rs:87`); a pane launched from the UI writes that from the TS catalog (`agent-model.ts:346`, `:815` via `launch-args.ts:55-57`), while `agent.open` writes it from `providers.rs` (`agent_open.rs:528`). So the flag depends on how the pane was created. Not observed at runtime; inferred from those four sites. The only cross-language guard, `pin-consistency.test.ts`, compares `pinnedVersion` only. **Fix:** add the flag to the TS catalog and extend that test to compare `launchArgs`/`persistentLaunchArgs` against the Rust statics, the way it already reads Rust source for pins. Longer term the 09-06 audit's Phase 2 item 9 (backend owns the catalog, frontend receives it) removes the copy.

**#5 — AgentMux root path in `agentmux-mcp` (live divergence).** `agentmux-mcp/src/window_capture.rs:25-43` "replicates [srv's] exact logic" — `AGENTMUX_DATA_HOME`, else `~/.agentmux`, else `/.agentmux`. The canonical resolver, `agentmux_common::data_paths::agentmux_root()` (`data_paths.rs:1019-1030`), also honours `AGENTMUX_HOME_OVERRIDE` first and errors instead of falling back to `/`. `402a1cc34` (#3372, "one resolver for the AgentMux root") removed exactly this behaviour from srv; the mcp copy was missed. `agentmux-mcp` already depends on `agentmux-common`. Other hand-built `~/.agentmux` paths that ignore both override variables, per the Rust sweep (not each re-read): `agentmux-launcher/src/logging.rs:16`, `agentmux-bashwrap/src/main.rs:112`, `agentmux-bashwrap/src/bash_wrap.rs:193`, `agentmux-cef/src/app/window_settings.rs:195-196`. **Fix:** call `agentmux_root()` at each.

**#6 — Claude settings/skill builder, Rust and TS.** `agentmux-srv/src/backend/agent_config.rs` and `frontend/app/view/agent/agent-config-builder.ts` implement the same eight functions: `build_settings_with_hooks` :439 / `buildSettingsWithHooks` :326, `prepend_user_hook_array` :387 / `prependUserHookArray` :299, `expand_template` :648 / `expandTemplate` :158, `render_skill_md` :727 / `renderSkillMd` :204, `sanitize_trigger` :753 / `sanitizeTrigger` :221, `skill_name_slug` :769 / `skillNameSlug` :240, `unique_skill_slug` :796 / `uniqueSkillSlug` :261, `build_mcp_config` :212 / `buildMcpConfig` :462 (pairs from the Rust/TS sweep; spot-checked `deriveSlug`). History: `07846c989` (#2378) — Codex P1, the TS builder "still only injected" PreToolUse after Rust gained PreCompact; a UTF-16 vs byte truncation mismatch from #2322 is recorded at `agent_config.rs:691`. Guard: only `SESSION_START_HOOK_PARTS` is pinned across languages (`agent-config-builder.test.ts:326-330`). **Proposal:** decide which side owns config materialisation for UI launches (srv already does it for `agent.open`); until then, a shared JSON fixture of inputs → expected `settings.json`/`SKILL.md` read by both test suites, the pattern `docs/specs/fixtures/redaction-vectors.json` already uses for redaction.

**#7 — Two scans of every `objects.db`.** `agentmux-srv/src/registry/migrate.rs:385` `enumerate_sources` and `registry/def_migrate.rs:212` `collect_scan_dbs`, whose own doc comment says it must scan the same places. They differ: only `migrate.rs` tracks unreadable directories (`read_dir_tracking`, `incomplete`), and `def_migrate.rs:339` `is_missing_table` misses the `SqlInputError` shape that `migrate.rs:551` `is_missing_column_or_table` handles. `migrate.rs` already exports `enumerate_objects_dbs` (`:369`). **Home:** `migrate.rs`.

**#8 — Duplicated wire types.** `BlockDef`/`FileDef` exist twice (`backend/obj.rs:227,235` and `backend/wconfig/types.rs:480,490`); `b9d8dd27d` (#3381) pinned their JSON equivalence with a test rather than merging, and says so. Listed for completeness; the test is doing its job.

### 5.2 Frontend

| # | What | Locations | Count | Home |
|---|---|---|---:|---|
| 1 | `term:zoom` read-and-clamp memo | `swarm-view.tsx:42-46`, `editor-model.ts:406-410`, `armory-model.ts:84-88`, `warden-model.ts:50-54`, `termViewModel.ts:233-238`, `AgentHistoryTabView.tsx:40-44`, `agent-view.tsx:2344-2349`, `AgentPicker.tsx:983-987`, `AgentShellSubblock.tsx:165-169` | 9 | export `readZoom(meta)` / `clampZoom` from `app/store/zoom.ts` (it already has `MIN_ZOOM`/`MAX_ZOOM` at :42-43 and a private `clampZoom` at :57) |
| 2 | Ctrl+Wheel zoom handler with its own step 0.1 (the universal one is `WHEEL_STEP = 0.05`, `zoom.ts:46`) | `editor-view.tsx:118-137`, `armory-view.tsx:56-71`, `warden-view.tsx:36-51`, `term.tsx:242-257`, `swarm-view.tsx:47-69`, `AgentShellSubblock.tsx:431-449` | 6 | Route through `zoomBlockIn/Out`. `agent-view.tsx:2381-2385` records that its own copy "fought the universal handlers" and was deleted |
| 3 | "Live auth failure" `failure?.data.code === "auth"` | `agent-view.tsx:1434, 1903, 2091, 2115, 2137`; `useAgentCommands.ts:768, 861, 1441, 1728` | 9 | `isAuthFailure()` in `store/agent-pane-state/types.ts` next to `isDisconnected` (:509). The "clear only if auth" sub-pattern exists 3× (`useAgentCommands.ts:768-770`, `:861-863` `clearAuthFailure`, `agent-view.tsx:2091-2093`); "auth blocked" 2× (`agent-view.tsx:2115`, `:2137`) |
| 4 | Hand-rolled phase checks: `kind === "Interrupting"` | `useTurnLifecycle.ts:143, 239, 247`; `agent-view.tsx:2548, 2827` | 5 | `isStopping(phase)`. Also `agent-view.tsx:2216` uses `Idle \|\| Done`, which is not `!workingFromPhase` (it excludes `Disconnected`) — possibly intended, not documented |
| 5 | Nonce + `setTimeout` until `nextToolPromotionAt` | `agent-view.tsx:1696-1707`, `:1757-1799`; `ActivityDock.tsx:81-94` | 3 | `createPromotionClock()` in `activity/tool-adapter.ts` |
| 6 | `RpcApi.SetMetaCommand(TabRpcClient, { oref: MOS.makeORef("block", id), meta: … as any })` | 26 call sites in 22 files (e.g. `agent-view.tsx:704, 2984`, `useAgentCommands.ts:1531`, `zoom.ts:114`, `ResizableDetailsDrawer.tsx:102`); `editor-view.tsx:131` hand-builds the oref string | ~26 | `setBlockMeta(blockId, meta)` next to `MOS` |
| 7 | Floating-popover positioning block (`registerFloating` → rAF → `computeMenuPosition` + `autoUpdate`) | verbatim in `statusbar/BackendStatus.tsx:82-117`, `CpuCoresPopover.tsx:132-163`, `DiskVolumesPopover.tsx:57-85`, `HostPopover.tsx:153-188`, `TokenBreakdownPopover.tsx:113-148` | 5 (+9 same shape) | `useAnchoredFloating(anchorRect, placement)` in `app/util/menu-position.ts` |
| 8 | Armory and Warden scaffold | `warden-model.ts:12` says it "Mirrors armory-model.ts"; views `armory-view.tsx:84-120` vs `warden-view.tsx:64-95` | 2 | `createSectionedPaneModel` + `SectionedRailPane` |
| 9 | Double-rAF "after paint" | `agent-view.tsx:936-937`, `tab-actions.ts:142-143`, `droppable-tab.tsx:246-247` | 3 | `afterPaint(cb)` beside `util/settle-detector.ts` |
| 10 | `mcp` ↔ `skill` frontend half (09-06 audit Phase 4, storage half done in #3045) | `mcp-manager.tsx` 250 / `skill-manager.tsx` 225; `AgentMcpModal.tsx` 138 / `AgentSkillsModal.tsx` 161 | 2 pairs | still open, as the audit scoped it |

### 5.3 Rust

Counts are from grep over non-test code; the first four were re-counted for this doc.

| # | What | Locations | Count | Home |
|---|---|---|---:|---|
| 1 | Untyped RPC registration: `register_handler` + hand-written `serde_json::from_value(data).map_err(…)` + a local `struct Req` | e.g. `app_api/memory.rs:11,29,47`, `app_api/agent_io.rs` (9), `websocket.rs` (6), `agent_handlers/identity.rs` (6), `app_api/identity.rs` (4) | 49 `.register_handler(` call sites vs 216 `.register_typed` | `WshRpcEngine::register_typed` (`backend/rpc/engine.rs:274`) — finishes the #3291–#3344 series, which found real drift while migrating (`290976a0c` "fix install.cancel error drift", `1fda36620` "two real drifts in AgentDefinition") |
| 2 | Same table DDL written per database | `db_accounts` at `storage/migrations.rs:610`, `:1469`, `:1863` (index prefix differs: `idx_`, `idx_ss_`, `idx_ids_`); per the sweep also `db_bundles`, `db_drone_definitions`, `db_agent_native_memory` ×3 and `db_muxbus_credentials` ×4, plus repeated `ADD COLUMN`s | 3–4 per table | shared DDL `const`s parameterised by index prefix, in the per-database split of §4.7 |
| 3 | MCP per-tool plumbing | §4.2 | 75 tools | a tool table + `srv_get_json` |
| 4 | Launcher IPC connect loop, Windows and Unix copies | `agentmux-cef/src/launcher_ipc/mod.rs:130-361` vs `:391-566` (non-comment lines differ in 4 places: `split(client)`/`split(stream)`, pipe/socket in a log line); same shape in `cef/src/srv_ipc.rs:59` vs `:190` | 2 pairs | one generic `run<S: AsyncRead + AsyncWrite>(stream)` |
| 5 | Newline-delimited JSON frame write (`to_vec`, push `\n`, `write_all`, `flush`) | `srv/src/srv_ipc/server.rs:425`, `cef/src/srv_ipc.rs:101`, `cef/src/launcher_ipc/mod.rs:157,207,412,451`, `launcher/src/host_pipe/mod.rs:435,665`, `launcher/src/ipc/server.rs:1145`, `launcher/src/diag.rs` (6) | 16 | `write_ndjson` in `agentmux-common/src/ipc.rs` (which has the message types but no framing) |
| 6 | Epoch time | 9 local `fn now_ms/now_secs/now_unix_secs` that compute instead of calling `common::time` (§6.1), `now_unix_secs` identical ×3 (`muxbus/cloud_subscriber.rs:82`, `server/ui_handlers.rs:56`, `server/reactive.rs:316`), `unix_secs` ×3 in the messaging pollers, and ~90 inline `SystemTime::now().duration_since(UNIX_EPOCH)` (the sweep's non-test count; 122 raw `SystemTime::now()` hits outside common including inline tests) | ~100 | `agentmux_common::time` + the gate in §6.1 |
| 7 | Small helpers defined 2–3× | `random_seed_bytes` (`storage/wan_identity.rs:237`, `agent_lan_keys.rs:33`, `agent_wan_keys.rs:54`); `sha256_hex` (`continuity_state.rs:203`, `project_instructions.rs:79`, `memory_record.rs:46`); `shell_quote` (`backend/shellintegration.rs:322` = `cef/commands/cli_login.rs:1260`); `cleanup_old_logs` (`cef/src/logging.rs:109` = `srv/src/bootstrap.rs:379`); `is_zero_i64` ×3 | 2–3 each | `agentmux-common` for the cross-crate ones; the existing `pub(crate)` copy for the in-crate ones |
| 8 | Error string → HTTP status by substring, two classifiers that disagree | `server/mod.rs:2211` (`"not found"` → 400) vs `:3009` (→ 404) | 2 | one classifier, or typed errors from the `*_impl`s |
| 9 | ANSI stripping, six implementations with different edge behaviour | TS: `agent-view.tsx:176`, `element/install/ansi.ts:19`; Rust: `bashwrap/src/bash_wrap.rs:1929`, `srv/backend/reactive/sanitize.rs:17`, `cef/commands/cli_login.rs` (~1073) (plus `ansiline.tsx:121`, which parses rather than strips) | 5–6 | one per language; they serve different inputs, so the gain is modest |
| 10 | Secret keyword lists outside the shared redactor | `bundle_export.rs:117` `SECRET_NAMES` (already drifted once inside its own file, `02a9d7389` #2333), `cef/commands/platform.rs:103`, `reactive/sanitize.rs:166,177`, `cef/commands/cli_login.rs:987` (own `redact_secrets`) | 5 | `agentmux-common/src/redact.rs`, which already has a cross-language fixture |

Also noted by the sweep and not re-verified: `X-AuthKey` as a string literal in ~140 places with no constant; `LOCAL_URL`/`AUTH_KEY` env reads duplicated between `agentmux-mcp/src/main.rs:72-88` and `agentmux-bashwrap/src/mps_client.rs:79-80`; layout-tree ops implemented in both `backend/layout/mod.rs` and `frontend/layout/lib/` with no shared fixtures; AskUserQuestion error strings matched in `useAgentQuestions.ts:64-70` against literals in `persistent/input.rs`.

### 5.4 SCSS

- **`@keyframes pulse` is defined twice** and keyframe names are global: `view/agent/styles/_control-bar.scss:37` (opacity 0.6↔1, nested but still global; no `animation: pulse` uses it) and `view/identity/styles/_status-dot.scss:28` (1↔0.4, used at `:18`). Whichever loads last wins for the status dot. Delete the dead one.
- **Mixins exist and are bypassed.** `app/mixins.scss` defines `ellipsis` (308), `flex-center` (380), `abs-fill` (395), `focus-ring` (411). By grep: the overflow/ellipsis/nowrap trio is hand-written in ~98 places (15 in `_document-nodes.scss`) and `mixins.ellipsis` is used twice; `flex-center` and `focus-ring` are used zero times against ~58 and 16 hand-written equivalents. The focus rings mix 1 px and 2 px. Low value per site; worth a sweep only alongside the `_document-nodes.scss` split.
- **Same-file repeats in `_document-nodes.scss`:** `.agent-tool-write` (1472 and 1979), `.agent-search-card-favicon` (1735 and 1827), and the centered-rule divider shared by `.agent-context-compacted` (2124) and `.agent-session-outcome` (2304).
- **Raw colour literals that bypass tokens:** `#ef4444` in `drone-view.scss:118,119,454` and three warden managers; `#60a5fa` and `#22c55e` likewise. `stylelint`'s `color-no-hex` already flags these (550 of 592 current `lint:scss` errors); `eslint.theme-colors.config.js` does not cover SCSS.

---

## 6. Status of the 2026-09-06 DRY audit, re-measured

| Audit item | Then | Now | State |
|---|---|---|---|
| `CREATE_NO_WINDOW` (step 1) | 21 files | 1 `pub` in `agentmux-common/src/win32.rs:27`, 2 private (`cef/commands/autostart.rs:41`, `srv/backend/attachments/extract.rs:120`) | done; 2 stragglers |
| `now_ms` / `now_secs` (step 2) | 26 copies | `common::time` exists (#3033). 37 local definitions: 28 are thin wrappers over it, **9 still compute the time themselves**, and 5 of those 9 were added after the lift (§6.1) | regrowing |
| `event_log.rs` (step 3) | 415 × 2 | 462 in common; srv 28, launcher 29 | done |
| ObjC externs (step 5) | ~55 | not re-measured | open (needed a macOS build) |
| Codegen (Phase 2) | none | ts-rs bindings, 374 files, with `check-rpc-codegen-hygiene.mjs` | done |
| Platform forks (Phase 3) | 3 `zoom.*` | one `zoom.ts` | done |
| `mcp`/`skill` (Phase 4) | 4 layers | storage done (#3045); UI half open (§5.2 #10) | half |
| Split `agentmux-mcp/src/main.rs` (step 17) | 4,633 | 4,809 | open (§4.2) |

### 6.1 Why `now_ms` regrew, and the fix

The lift created a home but nothing pointed new code at it. The nine self-computing copies: `agentmux-launcher/src/tray/linux.rs:33`, `tray/windows.rs:87`, `srv/backend/continuity_state.rs:207`, `backend/notify/router.rs:430`, `blockcontroller/persistent/segments.rs:28` (all five added 2026-09-24, after the lift), plus `muxbus/cloud_subscriber.rs:82`, `server/ui_handlers.rs:56`, `server/reactive.rs:316`, and `migrations/m0030_backfill_bundle_component_refs.rs:75` (added 2026-09-10, also after the lift, but deliberate — migrations freeze copies of live logic, per the policy in `agentmux-srv/src/migrations/mod.rs`). Dates are from `git log -S` on each definition.

A gate in the style of `scripts/check-name-resolver-callers.sh` — fail on a new non-test `fn now_ms|now_secs|now_unix_secs` or `SystemTime::now().duration_since(UNIX_EPOCH)` outside `agentmux-common` and `src/migrations/` — turns the next copy into a CI error. Then migrate the existing eight, and let the 28 wrappers go whenever their files are next touched.

---

## 7. Non-goals and cautions

- **No behaviour changes inside a move.** The Rust splits follow `SPEC_MONOLITH_MODULE_SPLITS` §1: no body changes, no type changes, no public path changes.
- **Do not merge the slug rules.** §5.1 #2 is about naming and pinning them, not making them equal — equalising would move existing agents' directories and signing keys.
- **Do not split `shell/lifecycle.rs`.** Its header records why a trait impl stays whole.
- **Do not unify the saga/reducer frameworks.** Decided in `DECISION_SAGA_REDUCER_TWO_FRAMEWORKS_2026_09_07.md`.
- **Keep the prose.** agent-view's comments are its design record; move them with their code.

## 8. Verification per change

- Frontend: `npx tsc --noEmit`; `npx vitest run frontend/app/view/agent frontend/app/store`; the two widened guard tests must still find the moved dispatch sites (their third assertion checks the count is non-zero).
- Rust: `cargo check -p <crate>` with no new warnings; `cargo test -p <crate>`; for moves, item-count parity (`fn`, `#[test]`, `#[tokio::test]`, `tokio::spawn`) between old and new files, as `SPEC_MONOLITH_MODULE_SPLITS` §7 did.
- SCSS: `npm run lint:scss` error count must not rise; screenshot the agent pane before and after the `_document-nodes.scss` split.
