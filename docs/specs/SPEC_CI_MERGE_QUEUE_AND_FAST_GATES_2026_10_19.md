# Spec: CI that keeps main green: a merge queue with fast gates, quicker PR builds, and a red-main alert

**Status:** active. Phase 0 shipped in #5118; Phase 1 (every required check reports in a merge queue) in #5119. Left: Phase 2 (turn the queue on), 3 (agents' merging), 4 (quicker PR builds), 5 (red-main alert), 6 (the Codex sweep).
**Date:** 2026-10-19
**Author:** Lazo
**Verified against:** `agentmux` `main` @ `4c16363db` (2026-10-19), by reading the workflows and measuring recent runs through the Actions API.
**Builds on:** [SPEC_CI_PATH_CONDITIONAL_CHECKS_2026_09_21.md](SPEC_CI_PATH_CONDITIONAL_CHECKS_2026_09_21.md) (the classifier and `CI required`), [SPEC_CI_PR_NIGHTLY_BALANCE_2026_09_12.md](SPEC_CI_PR_NIGHTLY_BALANCE_2026_09_12.md) (what runs per PR and what runs nightly), [SPEC_CI_CHECK_PLACEMENT_PROTOCOL_2026_09_21.md](SPEC_CI_CHECK_PLACEMENT_PROTOCOL_2026_09_21.md).

## 1. Ask

The owner, 2026-10-19, after main went red: "the file size check should be brought in before merge", then, given a merge queue that re-runs only the fast checks: "ok, do the cheaper variant. fix the issue, and improve the CI flow, we want it more robust, but quicker if possible. write an entire fix plan to file".

## 2. What happened

Every push to main from `7d8ce7111` (#5116) on failed `doc status + grep gates`, including the v0.59.33 release commit `4c16363db`:

```
crates/srv/src/backend/history/mod.rs: 1900 lines, allowed 1899 (+1)
```

- #5115 added a line to `history/mod.rs` and raised its baseline from 1898 to 1899.
- #5116's CI ran against a main from before #5115. It added `mod agent_sessions;` (+1) and, against its base, needed no baseline change.
- Each was green. Merged one after the other, the file is 1900. #5118 fixes the baseline.

**Why the gate didn't stop it:** branch protection on main requires `check`, `CI required`, `changeset`, `PR title names the agent` and `Codex review`, but not that a branch be up to date (`strict: false`), and there is no merge queue. So a PR's green CI result can be several merges old when it lands. Any check whose result depends on the rest of main can be beaten by a merge race. That covers the size ratchet, the specs index, a rename in one PR and a new call in another, and two changes to one generated file.

Nobody was told main was red. It was found by the owner, after three failing pushes.

## 3. Today, measured

Two weeks of runs, read from the Actions API on 2026-10-19:

- **Merge rate:** about 50 PRs merged into main in the last 24 hours, about 2 an hour, in bursts.
- **`CI (PR)` wall time on a PR:** 8–10 minutes for a full run (20 recent green runs: 7–10 min).
- **The critical path is the ubuntu Rust leg** (6–9 min): `cargo check --tests` ~1.5 min, then `cargo nextest` ~4.5–7 min. The latter is **~4.5 min compiling test binaries** (`Finished test profile … in 4m 34s`) and **~2 min running** 8,991 tests. The Windows leg (check only) and the macOS check take 3–5 min; vitest shards 1–3 min. `doc status + grep gates` takes under a minute.
- **Required checks, by source:**

| Required context | Where it comes from | Runs on |
|---|---|---|
| `check` | `no-coauthor-trailers.yml` | `pull_request` |
| `CI required` | `ci-pr.yml`, aggregating `changes`, `docs`, `docs-index`, `rust`, `frontend` | `pull_request`, `push` to main |
| `changeset` | `changeset.yml` | `pull_request` |
| `PR title names the agent` | `agent-pr-title.yml` | `pull_request` |
| `Codex review` | a commit status set by `codex-review-gate.yml` | `pull_request_target`, comments, reviews, dispatch, schedule |

- **`Codex review gate`'s scheduled sweep has never started.** Every `schedule` run since it was added (#4900, 2026-10-14) ends in `startup_failure` with no jobs and no error text. `actionlint` finds nothing, and the repository's token defaults (`read`) allow a job to ask for more. So PRs whose Codex status was lost to an outage are not being re-evaluated, as #4900 intended.

## 4. Principles

- **Main stays green by construction:** nothing lands that wasn't checked together with what lands before it.
- **The queue adds as little time as possible:** it re-runs only what a merge race can break and what takes about a minute. The full suite still runs on every PR and on every push to main.
- **Merging must not get harder for agents:** the same `gh-agent pr merge <n>` keeps working.
- **When main goes red anyway, someone hears about it within minutes,** not after three pushes.
- **Every step can be undone by one settings change or one revert.**

## 5. The plan

### Phase 0: unblock main (done, #5118)

Set `history/mod.rs`'s baseline to 1900, which main already is. Moving a line out of the file is left to #5115's and #5116's authors.

### Phase 1: make the required checks report in a merge queue

A merge queue only lands a group when every required check passes on the group's commit, which GitHub reports as a `merge_group` event. Today four of the five required checks run only on PR events, so turning the queue on first would stall every merge. This phase makes each one report on `merge_group`, and decides how much each runs there.

- **`ci-pr.yml`** gets `merge_group: types: [checks_requested]`. In a merge group, `changes` reports `queue=true`:
  - **`docs` (doc status + grep gates) runs in full.** Under a minute; it holds the size ratchet, the specs index check, the doc lifecycle, comment hygiene and the other grep gates. These are exactly the checks a merge race breaks.
  - **`docs-index` runs on ubuntu only.**
  - **`rust` runs on ubuntu only, and only `cargo check --tests`** (CEF-free crates, about 1.5 min on a warm cache). That catches what one PR's rename does to another's call. No nextest, doc tests or binding checks. The Rust cache is restored from main's but not saved by the queue, so queue runs don't evict PR caches.
  - **`frontend` runs one leg, with `tsc --noEmit` and the generated-stub checks (`gen-rpc-stubs`, `gen-mux-events`), no vitest.**
  - `rust-macos` doesn't run (it isn't required).
  - `CI required` is unchanged: a skipped leg counts as passed, as it already does for path-conditional legs.
- **`check`, `changeset`, `PR title names the agent`** describe the PR, not the code, and each PR's own run already passed before it could enter the queue. On `merge_group` each job runs and passes at once, saying so in its log.
- **`Codex review`** is a commit status on the PR's head. On `merge_group`, a new workflow, `merge-queue-codex-status.yml`, sets `Codex review` to success on the group's commit. A PR can only enter the queue with that status green on its own head, so the queue commit inherits a result that was already required. It's a file of its own, not a job in `codex-review-gate.yml`, because that workflow's scheduled runs fail to start (Phase 6), and a merge group must not depend on whatever causes that.
- **Expected queue time:** about 2–3 minutes: the docs gates and `tsc` run in parallel with a warm-cache `cargo check`.
- **Where the queue mode lives:** in `ci-pr.yml`'s `changes` step, which writes `queue=true` and the narrowed matrices (`rust_os`, `frontend_shards`, `docs_index_os`). Every other run leaves them unset, and each matrix falls back to its full list, so PR and push runs are unchanged.
- **Tests:** the workflows are linted with `actionlint`. A PR that only adds the triggers is safe on its own: `merge_group` never fires until the queue exists. The first queue run (Phase 2's canary) is the end-to-end test.

### Phase 2: turn on the merge queue (repository admin)

Once Phase 1 is on main, add a ruleset for `main` with a merge queue rule:

- merge method **squash** (as now);
- **build concurrency 5**, merge groups **of 1 to 5 PRs**, minimum wait 1 minute, so a burst lands together;
- **"only merge non-failing pull requests"**: a failing group is split and the failing PR removed, the rest retried;
- status check timeout 30 minutes.

The existing branch protection stays, so the five required checks apply to the queue as well.

- **Canary:** after enabling, one small PR goes through the queue first. If its group stalls, the rule is removed (one settings change) and Phase 1's triggers stay harmless.
- **Announce** on the repository's coordination channel and in Global Memory before enabling (Phase 3).
- **Needs an admin**, via `gh-agent sudo --reason … -- api -X POST repos/agentmuxai/agentmux/rulesets …`. It is never a bypass: `--admin` merges stay forbidden (CLAUDE.md).

### Phase 3: agents' merging

`gh pr merge` already handles a queue: on a branch that requires one, it adds the PR to the queue, or enables auto-merge if checks are still running, and any `--squash` is set by the queue instead (`gh pr merge --help`, gh 2.63). What changes for agents:
- **The PR stays OPEN after the merge command** until the queue lands it, usually 2–4 minutes later. A watcher that expects `MERGED` straight away must wait for it, and treat "removed from the merge queue" (the PR's timeline event) as a failure to report.
- **Never `--admin`:** it skips the queue.
- These two lines go into the Workspace Rules Global Memory entry and the coordination channel when Phase 2 is enabled.

### Phase 4: quicker PR CI

The ubuntu Rust leg is the critical path, and two thirds of its nextest step is compiling and linking test binaries. Two standard changes, each measured on its own PR against the numbers in §3:
- **`CARGO_PROFILE_TEST_DEBUG=line-tables-only` and `CARGO_PROFILE_DEV_DEBUG=line-tables-only`** for the CI Rust legs. Backtraces keep file and line; full debug info is the largest part of test-binary codegen and link time. Expected: a third less compile time.
- **The mold linker on the ubuntu leg** (`rui314/setup-mold`): linking 18 large test binaries is a sizeable share of the 4.5 minutes.
- Both change the Rust cache key once, so the first run after each is cold. The measurement is the second run and later.
- **Target:** ubuntu Rust leg from 6–9 min to about 4–6, so a full PR run lands in 6–7 minutes instead of 8–10.

### Phase 5: a red main is announced

A job at the end of `ci-pr.yml` that runs on `push` to main:
- **When `CI required` fails:** it opens, or comments on, one issue titled "main is red", with the commit, the failing jobs and links. The issue is assigned no one; github-consumer's issue notifications reach the agents watching the repository.
- **When a later push to main is green:** the same job closes that issue, naming the commit that fixed it.
- One issue at a time, so a red streak is one thread, not one per push. With the queue, this should be rare: it catches what the queue's fast gates don't check (a test that only fails on main).

### Phase 6: the Codex sweep that never starts

The scheduled sweep has never started (§3), so PRs whose Codex status was lost are not re-evaluated. Two cheap steps, in order:
1. Move the `sweep` job into its own workflow file with the permissions it needs at workflow level (`actions: write`), so its startup can't depend on `evaluate`'s job-level `if` or `concurrency` expressions on a `schedule` event.
2. If it still fails to start, the run page's annotation says why: it is shown only in the web UI, not the API, so the owner reads it, or a manual `workflow_dispatch` of the new file shows it.

## 6. Order and size

1. Phase 0: #5118.
2. Phase 1: one PR (workflows, classifier and its tests, this spec). Safe on its own.
3. Phase 2: settings, after Phase 1 merges, with the canary and the announcement (Phase 3).
4. Phase 4: one PR, measured over a few runs before and after.
5. Phase 5: one PR.
6. Phase 6: one PR.

## 7. Risks and rollback

- **A required check never reports on `merge_group` and the queue stalls.** Phase 1 adds every required check before Phase 2 enables the queue; the canary catches anything missed; rollback is removing the ruleset.
- **The queue checks less than a PR run.** By design: no tests in the queue. A test broken only by two PRs together is still caught by the push run on main, which Phase 5 now announces. Before this spec nothing caught any race at merge time.
- **Queue run time grows** if the Rust cache misses. The queue restores main's cache, which every push to main saves.
- **Agents' watchers misread a queued PR as not merged.** Phase 3's note, and the PR stays open, so nothing is lost; at worst a watcher reports "not merged yet".
- **A release PR waits in the queue** like any other. The auto-tagger finds the release commit by its subject on main, which the queue's squash commit keeps (`chore: release vX (#N)`).

## 8. Decisions for the owner

- **D1. Merge queue settings (§5 Phase 2):** squash, concurrency 5, groups of up to 5, 1 minute wait, only non-failing PRs. *Recommended as written.*
- **D2. No tests in the queue:** the queue re-runs the docs and size gates, `cargo check` and `tsc`, about 2–3 minutes, and leaves tests to the PR run and the push to main. *Recommended*, as decided on 2026-10-19 ("do the cheaper variant").
- **D3. Red main announced as one GitHub issue (§5 Phase 5).** *Recommended.* The alternative is a comment on the PR that broke it, which is harder to tell apart from review noise.
