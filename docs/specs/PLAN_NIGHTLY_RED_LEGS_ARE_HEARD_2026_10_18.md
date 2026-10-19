# Plan: a failing nightly leg is heard, and macOS tests count

**Status:** active. Shipped: the macOS test fix (#5125, Loap-3). This PR: Phase 1. Left: Phase 2 (fix what Phase 0 finds), Phase 3 (macOS blocks after five green nightlies), and the macOS timeout if Phase 0 shows it is needed.
**Date:** 2026-10-18
**Author:** Camper
**Verified against:** `agentmux` `main` @ `71a3e2300` (2026-10-19 UTC), by reading `ci-nightly-build.yml` and the job logs of the last 30 nightly runs through the Actions API.
**Builds on:** [SPEC_CI_MERGE_QUEUE_AND_FAST_GATES_2026_10_19.md](SPEC_CI_MERGE_QUEUE_AND_FAST_GATES_2026_10_19.md) (Lazo: the merge queue, and Phase 5's `main-red` issue), [SPEC_CI_CHECK_PLACEMENT_PROTOCOL_2026_09_21.md](SPEC_CI_CHECK_PLACEMENT_PROTOCOL_2026_09_21.md) (R3: macOS tests are nightly-only; §3.1: a check counts only if it can fail the run), [SPEC_NIGHTLY_CROSS_PLATFORM_BUILDS_2026_06_23.md](SPEC_NIGHTLY_CROSS_PLATFORM_BUILDS_2026_06_23.md).

## 1. Ask

The owner, 2026-10-18, after a macOS test was found failing on every nightly while the runs showed green: "yes, lets fix the macos test fail issue. we just had a bunch of CI stuff from Lazo regarding ci and building, read that first, write a plan to file". Then, on this plan: "yes fix the nightly now, use best recommendations", so §7's recommended answers are the decisions.

## 2. What happened

`serve_measures_a_sessions_shell_through_the_daemon` (`crates/remote/tests/durable.rs`, added in #4784 on 2026-10-13) expected the session's shell to be named `sh`. On macOS `/bin/sh` is bash, so it is reported as `bash`. The test failed on **every** macOS nightly from 2026-10-13 to 2026-10-18 (six scheduled runs and one manual one). #5125 (Loap-3) fixed it on 2026-10-19.

Nobody heard about it, for three reasons:

1. **The macOS leg can't fail the run.** `build-and-test` has `continue-on-error: ${{ matrix.os == 'macos-latest' }}`. Four of the six runs concluded `success` with `cargo test (macos-latest)` red. The job's own conclusion was `failure` every time; only the run's was masked.
2. **Nothing reports a failed nightly to anyone.** Lazo's `main status` job (#5121) reports pushes to main; the nightly has no equivalent. The red scheduled runs of 10-15 and 10-18 (Windows, ubuntu and macOS legs) weren't announced either.
3. **One failing test hid the rest of the suite.** `cargo test --workspace` stops at the first test binary that fails. On macOS it stopped after 15 binaries, at `agentmux-remote`'s `durable` test. **`agentmux-srv`'s tests (the largest suite) and the `agentmux-cef --features test-authfile` step haven't run on macOS since 2026-10-12.** So other macOS failures may be waiting behind this one, and the same is true of any leg on any night.

What ran on macOS, per night (job conclusion, first failure):

| Night | macOS test job | Stopped at |
|---|---|---|
| 10-12 | success | (full suite, 43 min) |
| 10-13 to 10-17 | failure | `durable.rs`, `serve_measures_a_sessions_shell_through_the_daemon` |
| 10-18 (scheduled) | failure | `agentmux-cef` didn't compile (#5092; fixed in #5106) |
| 10-18 (manual) | failure | `durable.rs`, as above |

## 3. Constraints

- **Time.** The macOS test job took 43 minutes for the full suite on 10-12. On 10-18 it took 44 minutes just to reach `durable.rs`, and the job's limit is 60 (`timeout-minutes: 60`). With srv's tests running again it may time out, which would look like a failure and hide everything after it. Phase 0 measures this.
- **macOS runners are scarce.** The 10-18 manual run's macOS test job queued for an hour before starting. Nothing in this plan adds a macOS job.
- **Lazo is changing `ci-pr.yml` now** (merge queue, Phases 2–6). This plan touches `ci-nightly-build.yml` and adds one script; it doesn't touch `ci-pr.yml`. The issue job copies the shape of `main status` (one issue per red streak, closed by a green run) so the two read alike.

## 4. The plan

### Phase 0: the real macOS state (in progress, no code)

A nightly dispatched on `71a3e2300` (after #5125), run `39186668162`. It answers:
- which macOS tests fail now that the suite gets past `durable.rs`;
- how long the full macOS test job takes against the 60-minute limit.

Each failing test goes to the agent whose PR added it (or the area's owner), as #5125 did. If the job times out, Phase 1 raises the limit.

### Phase 1: a failing leg is heard, and a failure doesn't hide the rest (one PR)

In `ci-nightly-build.yml`:

1. **`cargo test --workspace --no-fail-fast`** on every leg. Every test binary runs; the step still fails if any test failed, and the log lists every failure, not the first. The same for the `agentmux-cef` step.
2. **The `agentmux-cef --features test-authfile` step runs even if the workspace step failed** (`if: ${{ !cancelled() && matrix.mode == 'test' }}`). It is a separate suite; today a failure anywhere in the workspace skips it.
3. **A `nightly status` job** at the end, on `schedule` and `workflow_dispatch` runs of `main`. It reads every job's own conclusion (not the run's, which `continue-on-error` masks):
   - **any job failed or timed out:** open one issue labelled `nightly-red` (or comment on the open one), listing the failing jobs, the first failing tests from each job's log (up to 10, or the crate that didn't compile), and the run link;
   - **every job green:** close the open `nightly-red` issue, naming the run.
   It uses the same rules as `main status`: one issue per red streak, and a cancelled run says nothing. Two nightlies never run at once (the workflow's concurrency group queues them), so unlike pushes to main they can't race to create the issue. The doc link report is left out: it is a report by design and posts to the docs-stale issue. Issues notify the agents watching the repository, as `main-red` does.
4. **macOS timeout** to 90 minutes if Phase 0 shows the full suite near 60 (a timeout is a hang bound, not a budget).

`continue-on-error` on macOS stays in this phase. With the issue, it no longer hides a failure; it only keeps the run's own colour from changing, which Phase 3 decides.

Tests: `actionlint` on the workflow. The logic is `scripts/ci-nightly-status.mjs`, with `gh` passed in, so `scripts/ci-nightly-status.test.mjs` runs the whole flow against a fake: a red macOS leg in a run that shows green, a timed-out job, a cancelled run, an open issue already present, a green night closing it, a run on a branch, and the log parser on the 2026-10-18 log's coloured compile error. `DRY_RUN=1` prints the writes instead of making them. Dry-run against two real red nightlies (through the API, nothing written): run `39150321986` produces "cargo test (macos-latest): `serve_measures_a_sessions_shell_through_the_daemon`", and run `39125279795` lists its four legs, each with "`agentmux-cef` (did not compile)".

### Phase 2: fix what Phase 0 found

One PR per failing test, by its owner. The `nightly-red` issue tracks them; it closes itself when a nightly is green.

### Phase 3: macOS blocks (one line)

After **five consecutive nightlies with every macOS job green** (the standard the placement protocol applied to ubuntu), drop `continue-on-error` from `build-and-test`. The run's own colour then matches the issue. This is the owner's call (D2).

### Later, not in this plan

- `main status` (Lazo's job) could use the same script, so the two issue jobs share one implementation. That is Lazo's file and is offered to Lazo, not done here.
- A test that fails only on macOS can't be caught before merge: PR CI runs only `cargo check` on macOS (R4). If macOS failures keep arriving, a PR-time macOS test of the crates that touch the OS (`remote`, `procstats`, `bashwrap`) is the next step, at the cost of a scarce runner per PR.

## 5. Order and size

1. Phase 0: running now.
2. Phase 1: one PR, `ci-nightly-build.yml` and `scripts/ci-nightly-status.mjs` with its test, this plan.
3. Phase 2: as found.
4. Phase 3: a one-line PR after five green macOS nightlies.

## 6. Risks and rollback

- **The issue is noisy** if a leg flakes. One issue per streak and a comment per run bound it; a flaky test is then visible, which is the point.
- **`--no-fail-fast` makes a red leg run longer** (every binary instead of stopping at the first). A red leg was already going to be red; the time goes to the information we lacked.
- **A timeout reads as a failure** and the issue says "timed out", not which test; the log shows where it stopped.
- **Rollback:** revert the PR; nothing else depends on it.

## 7. Decisions (the owner took the recommendations, 2026-10-18)

- **D1. Report a red nightly as one GitHub issue (`nightly-red`)**, like `main-red`. *Recommended.*
- **D2. macOS blocks after five green nightlies** (Phase 3). *Recommended.* The alternative is to keep it advisory and rely on the issue alone.
- **D3. macOS test timeout to 90 minutes** if Phase 0 shows it is needed. *Recommended.*
