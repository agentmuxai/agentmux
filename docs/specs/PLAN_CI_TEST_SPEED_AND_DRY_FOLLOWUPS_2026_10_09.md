# Plan — faster CI tests, and the open architecture / DRY follow-ups

**Date:** 2026-10-09
**Status:** active — most steps merged, a few deferred with reasons (§5). Merged: step 1 (#4534), 2 (#4535), 3 (#4536), 4 (#4541), 5 (#4546), 8 (#4542), 9 (#4545), 10 (#4547 names, #4551 payloads), 12 (#4548), 13 (#4549), 14 (#4537), 15 (#4550), 17 in part (#4552, #4553), 20 (#4539). In review: 7 (#4543), 16 (#4540).
**Author:** Agent5@narko
**Builds on:**
- [SPEC_CI_TEST_RUNNER_2026_06_22.md](SPEC_CI_TEST_RUNNER_2026_06_22.md) §6.4 (the serial-for-now decision this plan retires)
- [SPEC_CI_PR_NIGHTLY_BALANCE_2026_09_12.md](SPEC_CI_PR_NIGHTLY_BALANCE_2026_09_12.md) (the PR lane's time target)
- [SPEC_CI_CHECK_PLACEMENT_PROTOCOL_2026_09_21.md](SPEC_CI_CHECK_PLACEMENT_PROTOCOL_2026_09_21.md)
- [SPEC_CODE_COMMENT_DENSITY_AND_CONDENSING_2026_09_30.md](SPEC_CODE_COMMENT_DENSITY_AND_CONDENSING_2026_09_30.md) rule C7
- [SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md](SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md)
- [../reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md](../reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md)
- [SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md](SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md)

## 1. Why this order

Four rules decide the order:

1. **Speed up every later PR first.** The PR lane's critical path is the ubuntu Rust job at 10–12.6 min. Every step after the CI work waits on CI several times, so the cheap CI wins go first.
2. **Fix drift that can already show the user something wrong before refactoring structure.**
3. **Do a small pilot of a pattern before rolling it out across the codebase.** The turn-model clean-up (step 8) is the pilot for generating event types (step 10).
4. **Add the guardrail before or with a split, so the result stays split.** The size ratchet (step 14) comes before the file splits (step 17).

Steps within a phase are independent and can run in parallel. A later phase does not have to wait for every step of an earlier one, except where §3 says so.

## 2. Where we start (2026-10-09)

| Lane | Today | Where the time goes |
|---|---|---|
| PR, ubuntu Rust | 10.0–12.6 min | `cargo check --tests` 1.2 min. `cargo test` 6.8 min, of which compiling takes 3.3 min and the one srv binary (6,709 tests on `--test-threads=1`) runs 174 s. `check-rpc-bindings.sh` runs its own `cargo test -p agentmux-srv export_bindings`: 3.4 min. |
| PR, vitest | 4.2–6.4 min | 10,710 tests in 671 files. 64% of the time is setting up jsdom, which is the default for every file. |
| PR, Windows | about 3.3 min | Compile only. |
| Nightly, Windows | 59.4 min of 60 | Release build 25.7 + workspace test 22.1 + cef 6.3. |

**Target:** the PR critical path at 6 min or less; vitest at 3 min or less; nightly Windows with at least 15 min of headroom.

Re-measure after every phase-1–3 step, write the result in the PR description, and update this table when a phase closes.

## 3. The steps

### Phase 1 — quick wins (each a small PR)

**1. Run the bindings check on the main `cargo test`.**
- The ubuntu job's `cargo test` already runs the `export_bindings_*` tests, so `frontend/types/rpc` has already been regenerated when it ends.
- Split `check-rpc-bindings.sh` into "regenerate" (what it does today, kept for local use) and `--check-only` (only the `git status --porcelain` test on `frontend/types/rpc`). Have `ci-pr.yml` call `--check-only` after `cargo test`.
- Saves about 3.4 min.
- *Done when:* CI still fails on a deliberately stale binding (verify once on a throwaway branch), and the job is 3 min or more shorter.

**2. Give nightly Windows room.**
- Raise `timeout-minutes` in `ci-nightly-build.yml`.
- Stop compiling the workspace twice (the release build, then the debug build for `cargo test`): either test the release profile, or move the release build to its own job.
- *Done when:* a nightly run has at least 15 min of headroom.

**3. Fix the 11 broken doc links and report link breakage nightly.**
- Fix the 11 relative links in 6 `docs/` files that broke in the `crates/` move (`node scripts/check-doc-links.mjs --all` lists them).
- Have the nightly also run `check-doc-links.mjs --all` and `check-comment-hygiene.mjs --dead-refs` and post the result to the standing docs-stale issue that `docs-stale-sweep.yml` keeps. It reports and does not fail the build, which keeps the earlier decision against repo-wide gates (`ci-nightly-build.yml`). A break merged today is then visible tomorrow.
- *Done when:* `--all` reports zero, and the nightly posts its findings.

### Phase 2 — the Rust test runner (the critical path)

**4. Switch to cargo-nextest in the ubuntu job.** (Depends on step 1, so the bindings check doesn't need its own `cargo test` again.)
- Replace `cargo test … -- --test-threads=1` with `cargo nextest run`. nextest runs each test in its own process, so tests that share in-process globals (the in-memory FileStore, `set_var`, static locks) no longer collide, and the suite can use every core.
- Add `.config/nextest.toml` with:
  - `retries = 1`, so a test that passes on retry is reported as FLAKY instead of hidden;
  - JUnit output;
  - a `[test-groups]` serial group for any test that still shares *outside* state (fixed ports, fixed paths).
- Keep the doctests on `cargo test --doc`, because nextest doesn't run them.
- Update §6.4 of the CI test runner spec to retire the "serial for now" note.
- Every FLAKY result in CI gets an issue; the retry is a report, not a fix.
- *Done when:* the srv run takes about 45 s or less, and three consecutive main runs are green.

**5. Remove wall-clock sleeps from Rust tests.**
- There are 90 real `sleep(` calls in test-only files. `crates/srv/src/server/tests.rs` alone sleeps about 30 s, including a 15 s sleep meant for Windows that also runs on Linux.
- Switch to `#[tokio::test(start_paused = true)]` / `tokio::time::advance` where the code under test uses tokio time, and to waiting for the state otherwise.
- Start with the largest sleeps.
- *Done when:* no test sleeps 1 s or more, except documented OS waits limited to their OS.

**6. (Only if still above target) split across runners.**
- `cargo nextest archive` once, then `--partition count:k/2` on two runners.
- The extra runner pays its own setup, so this is worth it only if steps 4–5 leave the job above 6 min.

### Phase 3 — the vitest runner

**7. Use the node environment by default, and jsdom only where the DOM is needed.**
- In `vitest.config.ts`, use `environment: "node"` with a `projects` entry (or a per-file `// @vitest-environment jsdom`) for component and DOM tests.
- Reducer, store, parser and util tests move to node unchanged.
- Expected: most of the 64% spent on setup goes away.
- *Done when:* vitest is under 3 min, with an unchanged test count.

If vitest is still above 3 min after step 7, add a two-way `--shard` matrix in the same PR. As with step 6, only when the measurement says so.

### Phase 4 — drift and fresh duplicates

**8. Turn-model clean-up.** These are duplicates added by #4492–#4511, so the cost is lowest now. This step is also the pilot for step 10.
- `TriggerKind::is_external()` in Rust, serialized onto the trigger. It replaces the allow-lists in `notify/router.rs` and `turn-awareness.ts` and the deny-list in `turn-trigger-text.ts`.
- One noun table for trigger kinds, in `turn-trigger-text.ts`. Today two tables disagree ("task" vs "finished task").
- One Rust "when did the turn end" function on `TurnLedger`, instead of three copies. The pane trusts `ended_at_ms` and drops its own lapse arithmetic.
- srv publishes `join_until_ms`, so `HELD_FLUSH_JOIN_MS` is no longer copied into `turn-ledger.ts`.
- `fmtWorkedDuration` goes, in favour of `formatElapsedCompact`.
- The notification router reads the typed ledger instead of re-parsing its JSON.
- `#[derive(TS)]` on `TurnLedger` / `TurnTrigger`, replacing the hand-written TS mirror and parser.

**9. One frame-intercept table for live and replay.**
- `useAgentStream.ts` and `parseHistoryLines.ts` repeat about 220 lines of frame intercepts, and have drifted twice: `clearHiddenReinjectionState` runs only on replay, and two intercepts run in opposite orders.
- Replace both with one ordered table, `interceptFrame(raw, sink)`, where only the sink differs (dispatch vs node put). `task-wake.ts` is the existing example of this shape.
- *Done when:* a test feeds the same lines through live and replay and gets the same nodes; both drifts are fixed.

### Phase 5 — the Rust ↔ TypeScript boundary

**10. Generate event names and published event payloads.** (Depends on step 8 as the pilot.)
- An `EventName` enum and `#[derive(TS)]` on every published payload, starting with `BlockControllerRuntimeStatus`. This generates what `mps-events.ts` and `srv-types.d.ts` now mirror by hand.
- An eslint rule against raw event-name strings in `muxEventSubscribe` (49 today).

**11. A `useMuxEvent(event, scope, parse, { replay })` hook.**
- One way to build scopes, one subscribe-then-replay path.
- Move the five hand-written replay consumers onto it, then the rest as files are touched.

**12. Take back the `register_handler` regression.**
- Untyped handlers went from 49 to 65 since 2026-09-30. Move them to `register_typed`, so `check-rpc-bindings.sh` covers them.
- Add a ratchet so the count can only fall.

### Phase 6 — controllers

**13. `ControllerStatusCore`.**
- One embedded struct for status, version, exit code, publish and heartbeat, replacing the set/snapshot/publish triple copied into the ACP, App Server, subprocess, persistent and shell controllers.
- A `PassStats` hook each protocol fills from its own completion message, so ACP, App Server and subprocess turn ledgers carry tokens and cost too.
- After this, the pane's `turnCarry` is a fallback, not the only source.

### Phase 7 — guardrails, then structure

**14. A generic file-size ratchet.**
- One script with the current sizes of every non-test source file above about 1,500 lines. A file may only shrink; a new file may not enter the list.
- It replaces the one per-file cap (`agent-view.tsx` is exactly at it today).

**15. Stores stop importing views.**
- Move the pure modules the stores use (`tool-labels`, `plan`, `session-outcome`, `live-feed`, `task-outcomes`, `auth-state`) out of `app/view/agent/` to a shared layer.
- Add an eslint rule: no `@/app/view` imports under `app/store`.
- In srv, move `update_object_meta` into a backend service the server layer re-exports, so backend code stops reaching into `server`.

**16. A settings consistency test.**
- One test reads `schema/settings.json` and checks each key's default against `settings-template.jsonc`, the settings section's control default, and srv's read.
- Today a new setting touches 6–7 places, with its default in four.

**17. Split the files the ratchet holds.**
- Rust: `storage/agents.rs`, `storage/migrations.rs`, `server/reactive.rs` (open since the 09-30 analysis), `inject.rs`, `shell/lifecycle.rs`.
- Frontend: `swarm-model.ts`, `AgentFooter.tsx`.
- Test files, split along their modules' own lines: `agent-pane-state/reducer.test.ts`, `useAgentCommands.test.ts`, `server/tests.rs`, `storage/store/tests.rs`. For vitest this also adds parallelism; for Rust it is readability only.

### Phase 8 — polish

**18. One status ranking.** The Swarm line, the working row, the placeholder and the subagent label each have their own first-match chain. Route them through one ranking, so an agent reads the same everywhere. Local time formatters move to `util/format-time.ts`.

**19. Small leftovers from the earlier audits:**
- a constant for the `"X-AuthKey"` literal (101 sites);
- one "not found" classification (400 in `http_app_api.rs`, 404 in `http_naming.rs`);
- the remaining hand-written `kill_tree`s (bashwrap, launcher);
- tests that tie `tab-presets.ts` and `agent-color.ts` to their Rust sources.

**20. A weekly web-link report.**
- lychee over `docs/` and code comments (about 562 web URLs, 466 on github.com), restricted to github.com and our own domains, posting to the docs-stale issue.
- Non-blocking on purpose: a gate on outside URLs makes CI depend on other people's servers.

## 5. Where it landed (2026-10-09)

**Measured:**

| | Before | After |
|---|---|---|
| PR lane, ubuntu Rust job | 10.0–12.6 min | 5.8–7.6 min (most of what's left is compiling) |
| RPC bindings step | 3.4 min | 1 s (#4534) |
| Rust tests, PR lane | srv binary alone 174 s, serial | 92 s for all 7,534, parallel; flakes reported FLAKY (#4541) |
| Nightly, Windows | 55–59 min of 60 | release 27.7 min, test 30.5 min, in parallel (#4535) |
| Nightly, macOS | cancelled at 60 min | release 24.7 min, test 36.0 min |

**Re-scoped or deferred, and why:**
- **Step 5:** the long sleeps the plan named were all in `#[ignore]` tests. Only one slow test that runs waits purely on tokio time; it now uses a paused clock (10 s → 0.3 s). The other slow tests wait on real processes.
- **Step 6** (split the Rust job across runners): not needed. Under nextest the test run is 92 s, and the remaining job time is compiling, which another runner would repeat.
- **Step 11** (`useMuxEvent`): of 39 subscribe/cleanup sites, only 7 follow the plain pattern a hook would replace; the rest resubscribe on id changes or live at module level. Deferred.
- **Step 12:** shipped as a ratchet (60 untyped handlers, may only fall). Most are App API commands whose caller is the Rust MCP server, so generated TS buys them nothing.
- **Step 13:** Codex turns now carry their figures (#4549), but the pane shows them only once the App Server transcript renders there (its translator gap). ACP's prompt response has no usage to read. The `ControllerStatusCore` consolidation is not needed for either and is deferred.
- **Step 17:**
  - Done: the largest frontend test (#4552) and `storage/agents.rs` (#4553).
  - Left: `migrations.rs`, `server/reactive.rs`, `inject.rs`, `shell/lifecycle.rs` and `swarm-model.ts`; the file-size ratchet keeps them from growing meanwhile.
  - `useAgentCommands.test.ts` is better left whole: its module-level `vi.mock` setup would be copied into every piece.

## 4. Not in this plan

- Condensing comments (Phase 1+ of the comment density spec). That is its own program.
- macOS-only work (the ObjC externs, nightly macOS), which needs a macOS runner to verify.
