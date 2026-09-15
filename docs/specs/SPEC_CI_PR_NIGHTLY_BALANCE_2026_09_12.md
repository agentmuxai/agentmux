# SPEC — Balanced PR and nightly test lanes

**Status:** proposed
**Date:** 2026-09-12
**Scope:** GitHub Actions validation for pull requests and `main`
**Related:** [`SPEC_CI_TEST_RUNNER_2026_06_22.md`](SPEC_CI_TEST_RUNNER_2026_06_22.md), [`SPEC_NIGHTLY_CROSS_PLATFORM_BUILDS_2026_06_23.md`](SPEC_NIGHTLY_CROSS_PLATFORM_BUILDS_2026_06_23.md), [`INCIDENT_2026_09_10_CI_PR_WINDOWS_RUNNER_HANG_BACKLOG.md`](../incident/INCIDENT_2026_09_10_CI_PR_WINDOWS_RUNNER_HANG_BACKLOG.md)

## 1. Decision

AgentMux uses two deliberately different validation lanes:

1. **PR lane:** fast, deterministic, high-signal checks for regressions that commonly break during development. It must not run the full CEF-dependent workspace suite.
2. **Nightly lane:** the complete, slower, cross-platform validation suite, including CEF-dependent crates, serial full-workspace tests, and broader integration coverage.

The full nightly suite may be run manually for a high-risk change, but it is not duplicated on every ordinary PR.

## 2. Why this is changing

The original June 2026 design separated CEF-free PR checks from the CEF-heavy nightly suite. On 2026-06-23, the fast workflow was intentionally changed to nightly-only (`99dec9ef6`, PR #1717). On 2026-07-01, PR #1885 added `ci-pr.yml` with `cargo check --workspace --tests` and `cargo test --workspace` on every PR after test-build regressions #1823 and #1876 reached `main`.

That addition duplicated the overnight workload instead of replacing it. The Windows PR job spent most of its budget compiling/testing CEF-dependent code and frequently reached the 20-minute timeout, while the nightly suite continued to run the same work. The incident report recommended profiling and moving the slowest tests back to nightly — since done: PR #3201 raised the PR-lane timeout while this was diagnosed, and PR #3221 scoped `ci-pr.yml` to the CEF-free crates below, so the PR lane no longer builds `agentmux-cef` at all. This section is kept as the historical rationale for that decision, not a description of a still-open problem.

The goal is not “fewer tests.” The goal is to put the right tests at the right feedback boundary.

## 3. PR lane requirements

### 3.1 Required checks

The PR lane MUST include:

- Rust compile/test coverage for the CEF-free crates that contain most application logic:
  `agentmux-common`, `agentmux-srv`, `agentmux-launcher`, `agentmux-bashwrap`, and `agentmux-mcp`.
  The last two are independent workspace members not pulled in transitively by the other
  three, so they need their own explicit listing or they get no PR-lane compile/test gate
  at all.
- `cargo check --tests` for those same CEF-free packages, so `#[cfg(test)]` constructors and platform-gated test code compile before merge.
- Serial Rust execution (`--test-threads=1`) until the documented process-global test isolation issues are removed.
- Frontend typechecking and Vitest.
- Fast deterministic repository gates that are directly relevant to changed files, including generated RPC/schema freshness and documentation/spec gates where applicable.

The PR lane SHOULD run on the primary Windows runner plus a non-blocking Linux visibility leg. Platform-specific failures should become required only after they are green and understood.

### 3.2 Explicit exclusions

The PR lane MUST NOT run:

- `cargo check --workspace --tests` when it causes the CEF crate to build merely to validate CEF-free code.
- `cargo test --workspace`.
- CEF download/build/link work, full CEF reducer/lifecycle coverage, or broad overnight integration/soak tests.

Those checks belong to the nightly lane unless a PR is explicitly marked for full validation.

### 3.3 Budget

- Target: **under 10 minutes** on the required Windows PR leg.
- Soft ceiling: **15 minutes**.
- A PR-lane change that exceeds the soft ceiling for three consecutive representative runs requires either optimization or moving coverage to nightly.
- A timeout is a CI design failure, not evidence that the code is correct or incorrect; it must be visible as such in the job summary.

## 4. Nightly lane requirements

The nightly lane MUST retain the broad safety net:

- `cargo build --release --workspace` where the cross-platform build matrix requires it.
- `cargo test --workspace -- --test-threads=1`, including `agentmux-cef` and Windows-specific lifecycle tests.
- The complete frontend suite (one platform is sufficient for platform-agnostic Vitest coverage).
- Manual dispatch for maintainers who need full validation before merge.

Nightly runs SHOULD use a more generous timeout than PR validation, preserve artifacts/logs for failures, and run on Windows, Linux, and macOS according to the existing staged required/non-blocking policy.

Heavy or unstable tests SHOULD be explicitly categorized (for example, `#[ignore]` plus nightly `--include-ignored`, or a separate named test job) rather than silently making the PR lane slower.

## 5. High-risk change escape hatch

Changes touching CEF, platform lifecycle, process supervision, release packaging, or shared persistence MAY request the nightly/full suite before merge through a manual workflow dispatch or an explicit PR label/command. This is an intentional escalation and must be recorded in the PR checks; it does not change the default lane for all PRs.

## 6. Coverage mapping and anti-drift rules

Every test moved between lanes MUST have a short entry in the workflow or this spec explaining:

- the regression class it catches;
- why it needs PR or nightly placement;
- its expected runtime and platform requirements.

The PR and nightly workflows MUST not silently duplicate the same expensive full-workspace command. A workflow change that broadens PR scope to the nightly suite requires updating this spec and documenting the measured feedback-time impact.

CI status names SHOULD identify the lane (`PR fast` versus `nightly full`) so a cancelled/timeout run is distinguishable from a test failure.

## 7. Acceptance criteria

This spec is implemented when:

1. Ordinary PRs no longer invoke the full CEF-dependent workspace test command.
2. Required PR checks complete under the budget on representative Windows runs.
3. The nightly workflow still exercises the full workspace and platform-specific lifecycle tests.
4. A maintainer can trigger the nightly/full suite for a high-risk PR without changing repository-wide defaults.
5. CI documentation and the specs index describe the same lane split as the workflows.

