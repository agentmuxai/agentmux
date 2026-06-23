# SPEC — CI test runner on public GitHub-hosted runners

- **Status:** Draft (spec-first; no workflow yet)
- **Date:** 2026-06-22
- **Author:** AgentA
- **Motivation:** the repo has **1,672 Rust `#[test]`/`#[tokio::test]` functions** (148 files) and
  **101 vitest frontend test files**, but **no CI workflow runs any of them**. Regressions ship
  silently — the orphaned-process-tree lifecycle bug (#1676 / Discussion #1680) is the cautionary
  tale: its quit gate had *zero* automated coverage, so it broke unnoticed. This spec adds a runner.
- **Related:** Discussion #1680 (§9 "stand up CI tests"); `SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21.md`.

---

## 1. Billing constraint (the hard rule)

The repo is **public** and org-owned. Per GitHub's billing docs:

> "GitHub Actions usage is **free** … for **public repositories that use standard GitHub-hosted
> runners**." — and — "**Larger runners are always charged for, even when used by public
> repositories.**"

**Invariant CI-1: all jobs MUST run on STANDARD GitHub-hosted runners** (`ubuntu-latest`,
`windows-latest`, `macos-latest`) — **never** larger runners (extra cores/RAM/disk) and never a
custom/self-hosted label (the existing `[self-hosted, input-bench]` runner doesn't exist). On
standard runners, public-repo minutes are **free and uncapped**, so cost is **not** a constraint —
only **wall-clock** is. Any reviewer seeing a `runs-on:` that isn't a bare `*-latest` standard label
must reject it.

> We could **not** verify actual minute consumption — the `gh` token lacks `admin:org` billing scope
> (the billing API 404s). The "free" basis is the policy above, which is sufficient given CI-1.

---

## 2. The real cost: the CEF build (wall-clock, not money)

- `agentmux-cef` (where the host reducer + lifecycle tests live) depends on `cef-dll-sys`, which
  **downloads ~200 MB of CEF** and **builds a C wrapper with CMake + Ninja** — **~10-20 min cold**.
- `agentmux-launcher`, `agentmux-srv`, `agentmux-common` are **CEF-free** → fast (a few min).
- `vitest` (frontend) is fast.
- CMake ships on GitHub runners; **Ninja must be installed** (a setup step); the CEF download must be
  **cached** or it re-downloads every run.

So the design splits **fast/CEF-free (cheap wall-clock)** from **CEF-dependent (slow)** and runs the
slow part on a schedule, not on every push.

---

## 3. Design — two lanes

### Lane A — PR/push fast lane (CEF-free) — `ci-fast.yml`
- **Trigger:** `pull_request` + `push`.
- **Runner:** `ubuntu-latest` (standard).
- **Runs:**
  - `cargo test -p agentmux-launcher -p agentmux-srv -p agentmux-common` (no CEF link → no CEF build).
  - `npm ci && npx vitest run` (frontend).
- **Wall-clock target:** < 5 min. Gives fast PR feedback on the bulk of the logic (launcher saga,
  srv, shared utils, frontend) without paying the CEF build.
- **Cache:** `actions/cache` (or `Swatinem/rust-cache`) on `~/.cargo` + `target/`; npm cache.

### Lane B — nightly full suite (incl. CEF) — `ci-nightly.yml`
- **Trigger:** `schedule:` (cron, ~daily off-peak, e.g. `0 9 * * *` UTC) + `workflow_dispatch`.
- **Runner:** `windows-latest` (standard) — **required** so the Windows-specific lifecycle code
  (`wrr/win_event.rs`, `lib.rs` `TerminateProcess`, the `#[cfg(windows)]` close paths from #1676)
  actually compiles + runs; the cross-platform reducer/launcher/srv tests run here too.
- **Setup:** install Ninja (`ninja --version` must work for `cef-dll-sys`); CMake is preinstalled.
- **Runs:** `cargo test --workspace` (incl. `agentmux-cef` — pays the CEF build once/night) +
  `npx vitest run`.
- **Cache:** `~/.cargo`, `target/`, **and the CEF download dir** (the cef-rs crate's download cache)
  keyed on the pinned `cef-dll-sys` rev — turns the cold ~20 min into a warm few-min build.
- **Wall-clock:** cold ~20 min, warm ~5-10 min. Free on the standard runner regardless.

> Optional later: add `ubuntu-latest` to Lane B as a matrix leg for non-Windows parity, once the
> Windows leg is proven.

---

## 4. Scope — what it does NOT cover (deliberately)

- **No live-CEF e2e** ("launch host → close window → assert process tree exits"). That needs a
  display + the full packaged bundle and **cannot run on a standard GitHub-hosted runner** (no GPU /
  driving a live AgentMux — see the unused `input-bench` self-hosted job). The lifecycle reducer
  *logic* is covered by `agentmux-cef` unit tests (Lane B); the end-to-end "tree exits" check stays a
  **local-only smoke** (`tools/tests/`), as recorded in Discussion #1680. This spec does not try to
  make e2e a CI gate.

---

## 5. Success criteria

1. A failing unit test (any crate) or vitest fails the relevant lane — red on the PR (Lane A) or the
   nightly run (Lane B).
2. The lifecycle reducer tests (`counts_as_live_user_window`, `reconcile_quit`, the quit-gate tests)
   run in Lane B — i.e. the #1676-class regression would now be caught automatically.
3. Both lanes stay on **standard runners** (CI-1) → $0.
4. Lane A < 5 min; Lane B warm < ~10 min.

---

## 6. Open questions

1. **Does a standard `windows-latest` runner have enough disk/RAM for the CEF build?** (Standard =
   ~4 vCPU / 16 GB RAM / ~14 GB free SSD.) The CEF download + build + `target/` may be tight. If it
   OOMs/ENOSPC, options are: free up disk in the job (delete unused toolchains), or accept that the
   full CEF suite needs a larger (billed) runner — at which point this becomes a **cost decision for
   the maintainer**, NOT a silent opt-in. Verify on the first nightly run.
2. **Nightly cadence + timezone** — daily, or weekdays only? Cron in UTC.
3. **Matrix** — Windows-only first, or ubuntu+windows from the start?

## 6.4 Pre-existing test debt surfaced standing up CI (deferred, tracked)

Standing up the runner flushed out four accumulated failures/flakes (none from the CI change
itself — exactly the regressions the runner exists to catch). To ship a **green** runner now, each
is handled minimally + tracked; the follow-up is a "test-health" pass that removes the workarounds:

1. **Rust runs SERIALLY (`--test-threads=1`) for now.** `agentmux-cef`'s `allow_pane_focus_once_*`
   tests share a process-global `AtomicBool` and race under parallel runs. Serial is deterministic.
   *Follow-up: give those tests a `Mutex`/non-global → drop `--test-threads=1` (faster).*
2. **`agentmux-srv::backend::agent_session::write_then_read_roundtrip` is `#[ignore]`d.** A
   process-global read cache in `read_session_state` is keyed by definition-id, not by `FileStore`,
   so a sibling test pollutes it — fails even serially (ordering, not parallelism). *Follow-up: key
   the cache by store (or disable it in `open_in_memory`) → un-ignore.*
3. **`tools/**` excluded from the frontend vitest** (`vitest.config.ts`). `tools/tests/lib/
   bench-stats.test.mjs` is standalone Node tooling, not a frontend jsdom test. *Follow-up: a
   separate tools-test job if those are worth gating.*
4. **`AgentLaunchModal` "auth state on memory change" is `it.skip`'d — a REAL pre-existing failure**
   (fails in isolation): a Memory selection change resets auth-ready state. This is a genuine
   regression that shipped because there was no CI. *Follow-up (highest priority of the four):
   triage product-bug-vs-stale-test and fix → un-skip.*
5. **Rust fast lane is an OS matrix; only Windows is REQUIRED.** AgentMux ships on
   Windows/macOS/Linux, so the rust fast lane runs all three — but only the `windows-latest` leg
   blocks PRs (the primary dev platform; green). `ubuntu-latest` + `macos-latest` run **non-blocking**
   (`continue-on-error: ${{ matrix.os != 'windows-latest' }}`) for cross-platform visibility, because
   the CEF-free crates have untested-platform failures: Linux needs `smithay-client-toolkit`'s Wayland
   build deps (installed in-job) AND fails `registry::schema::dotdot_workdir_is_rejected` (OS-sensitive
   `..`-path handling); macOS is TBD on first run. *Follow-up: triage + green each platform, then flip
   its leg to required.* (vitest stays a single ubuntu job — JS is platform-agnostic and passes.)
6. **`agentmux-srv::identity::auth_session::timeout_transitions_pending_to_failed_on_poll` `#[ignore]`d.**
   Its `force_age` test helper does `Instant::now() - Duration::from_secs(SESSION_TIMEOUT_SECS + 1)`,
   which underflows (panics) on a CI runner whose monotonic uptime is below the timeout — passes
   locally (high uptime), fails on a freshly-booted runner. Production is unaffected (never subtracts
   from `Instant`). *Follow-up: make the timeout mockable (deadline model / injectable clock) →
   un-ignore.*

These workarounds keep the gate meaningful (it catches NEW regressions today) without blocking the
runner on a multi-test cleanup. Track the cleanup as its own effort.

---

## 7. Implementation note

Ships as `.github/workflows/ci-fast.yml` + `.github/workflows/ci-nightly.yml` in a normal PR (not a
doc-only PR — the spec rides with the workflows). Start minimal (Lane A + a Windows-only Lane B) and
iterate on caching/matrix from the first real runs.
