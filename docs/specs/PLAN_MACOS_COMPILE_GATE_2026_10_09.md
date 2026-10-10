# Plan — a macOS compile check on PRs, and a nightly macOS leg that can fail

**Date:** 2026-10-09
**Status:** active — phase 1 shipped in PR #4571 (the non-blocking macOS check); phases 2 and 3 wait on measurements (§4).
**Author:** AgentA@Area54
**Builds on:**
- [SPEC_CI_CHECK_PLACEMENT_PROTOCOL_2026_09_21.md](SPEC_CI_CHECK_PLACEMENT_PROTOCOL_2026_09_21.md) R3 (macOS is nightly-only) and R4 (a platform that matters gets a compile-only PR gate)
- [PLAN_CI_TEST_SPEED_AND_DRY_FOLLOWUPS_2026_10_09.md](PLAN_CI_TEST_SPEED_AND_DRY_FOLLOWUPS_2026_10_09.md) §4 (left macOS-only work out of scope)
- [SPEC_CI_PATH_CONDITIONAL_CHECKS_2026_09_21.md](SPEC_CI_PATH_CONDITIONAL_CHECKS_2026_09_21.md) (the `changes` job and the `CI required` aggregate)

## 1. The gap

The PR lane compiles the Rust crates on Windows and Ubuntu and tests them on Ubuntu. macOS is nightly-only (placement protocol R3), and the nightly macOS leg is `continue-on-error`. So a change that breaks the macOS build, for example in `#[cfg(target_os = "macos")]` code, Keychain or signing, or process tracking, can merge, and the nightly still reports green.

macOS code is being changed often enough for this to matter: in the two days before this plan, four merged PRs changed macOS behaviour (#4449 process tracking, #4482 code signing, #4488 Keychain reads, #4486 a CLI-install test).

## 2. What the nightly shows

The nightly macOS leg since 2026-10-06:

| Run | Result |
|---|---|
| 10-06 | success |
| 10-07, four runs | failure: `trailing_writer::tests::spaced_jobs_each_run`, a timing-flaky test in `agentmux-cef` (fixed by #4480) |
| 10-08, three runs | cancelled at the 60-minute limit |
| 10-09 | success, release 24.7 min and test 36.0 min as parallel legs (#4535) |

None of these was a compile break, and the leg has finished cleanly only once since the split. So the nightly leg can't become blocking yet (phase 3), and the PR check has to stay compile-only: a timing test run on every PR is the wrong place to discover macOS flakiness.

## 3. Phases

**Phase 1 (this PR): a non-blocking macOS compile check on PRs.**
- A `rust-macos` job in `ci-pr.yml`: `cargo check --tests` on the same CEF-free crates as the Windows and Ubuntu legs, on `macos-latest`, with `Swatinem/rust-cache`.
- It runs on the same condition as the `rust` job (`needs.changes.outputs.rust != 'false'`), so docs-only and version-only PRs skip it.
- `continue-on-error: true`, and it is **not** in `CI required`'s `needs`: it reports on every Rust PR but can't block one.
- `timeout-minutes: 35`, the same bound as the `rust` job.

**Phase 2 (after measuring): make it required.**
- Measure for about a week, at least 15 runs: time from job start to end, time in the queue, and every failure (a real macOS break, a flake, or infrastructure).
- If the median is at most 8 minutes, the 90th percentile at most 12, and no failure was a flake: move `macos-latest` into the `rust` job's matrix (compile-only, like Windows), which puts it under `CI required`. Update placement protocol R4 to name macOS.
- If it is slower, keep it informational and record the numbers here.

**Phase 3: the nightly macOS leg can fail.**
- Remove `continue-on-error: ${{ matrix.os == 'macos-latest' }}` from `ci-nightly-build.yml` after five consecutive nightly runs in which both macOS legs (release and test) succeed. There is one such run so far (10-09).

## 4. Measuring

```bash
gh api "repos/agentmuxai/agentmux/actions/workflows/ci-pr.yml/runs?per_page=50" --jq '.workflow_runs[].id' |
  while read id; do
    gh api "repos/agentmuxai/agentmux/actions/runs/$id/jobs" \
      --jq '.jobs[] | select(.name == "cargo check --tests (macos-latest)") | [.created_at, .started_at, .completed_at, .conclusion] | @tsv'
  done
```

`started_at - created_at` is the queue wait; `completed_at - started_at` is the job.

## 5. Not in this plan

- Running the tests on macOS per PR. The nightly test leg covers them; phase 3 makes it count.
- `agentmux-cef` on macOS. It needs the CEF download and build, which is why every PR leg leaves it out; the nightly release leg covers it.
