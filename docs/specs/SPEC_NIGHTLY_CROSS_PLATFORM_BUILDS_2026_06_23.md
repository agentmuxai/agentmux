# SPEC — Nightly cross-platform CI builds

- **Status:** Phase A implementing; Phase B deferred
- **Date:** 2026-06-23
- **Author:** AgentA
- **Tracking:** GitHub issue **#1718**
- **Related:** `SPEC_CI_TEST_RUNNER_2026_06_22.md` (the nightly *test* runner — same billing model,
  same non-blocking-until-green approach; this is its *build* sibling).
- **Motivation:** AgentMux ships on Windows / macOS / Linux, but **nothing builds it cross-platform
  in CI** — a Linux or macOS build break (a `#[cfg]` slip, a missing dep, a platform API change)
  ships unnoticed until someone builds locally on that OS. This is the exact blind spot the test
  runner just exposed for *tests* (smithay/Wayland, the `..`-path test); this catches it for *builds*.

---

## 1. Billing basis (the same hard rule as the test runner)

Standard GitHub-hosted runners (`ubuntu-latest`, `windows-latest`, `macos-latest`) are **free on
public repos for ANY workload — tests or builds**. Only *larger* runners are billed (CI-1 in the
test-runner spec). So a nightly 3-platform build costs **$0** on standard runners — cost is not a
constraint; wall-clock + disk are.

## 2. Phase A — compile-build matrix (this PR) — `ci-nightly-build.yml`

- **Trigger:** nightly `schedule` (07:00 UTC — ahead of the test nightlies at 08:00/09:00) +
  `workflow_dispatch`.
- **Matrix:** `windows-latest` (REQUIRED — primary platform), `ubuntu-latest` + `macos-latest`
  (**non-blocking**, `continue-on-error`, until greened — same staged approach the tests used).
- **Build:** `cargo build --release --workspace` — compiles every crate **including `agentmux-cef`**,
  so it pays the CEF build (`cef-dll-sys`: ~200 MB download + CMake/Ninja C-wrapper) and surfaces any
  cross-platform compile/link break. NOT packaging (that's Phase B) — just "does it build."
- **Per-platform build deps:**
  - Windows — CMake ships with VS; install **Ninja** (`choco install ninja`).
  - Ubuntu — `ninja-build cmake libwayland-dev libxkbcommon-dev libgtk-3-dev` (CEF-linux + the
    launcher's smithay/Wayland). Expect to extend this on the first run (CEF-linux link deps).
  - macOS — `brew install ninja cmake`.
- **Cache:** `Swatinem/rust-cache` (`~/.cargo` + `target/` + the CEF download).
- **Wall-clock:** ~15-30 min/platform cold (CEF build); faster warm. Free regardless.

## 3. Phase B — full artifacts (deferred — issue #1718)

`task package` on each platform → produce + **upload portable artifacts** (Windows ZIP, macOS
`.app`/`.dmg`, Linux AppImage) as nightly dogfood builds (`actions/upload-artifact`). Heavier:
per-platform packaging tooling, artifact retention policy, and **macOS codesigning/notarization**
(unsigned `.app`s gatekeeper-block). Land after Phase A's per-platform builds are green.

## 4. Caveats / open questions (tracked on #1718)

1. **Disk.** A standard runner has ~14 GB free SSD; the CEF download + build + `target/` may be
   tight. If a leg ENOSPCs, options: free disk in-job (delete unused toolchains), or accept a
   *larger* (billed) runner — a **maintainer cost decision, not a silent opt-in**. Verify on the
   first nightly run.
2. **Non-Windows greening.** ubuntu/macOS will likely fail the first run (untested platform builds,
   like the tests). They're non-blocking; green them incrementally → flip each to required.
3. **Cadence/stagger.** 07:00 UTC keeps it ahead of the two test nightlies; revisit if runner
   contention shows up.
4. **macOS codesigning** (Phase B only).

## 5. Success criteria
- Phase A: nightly Windows build is green (the primary build can't silently break); ubuntu/macOS
  build *and report* (non-blocking) so cross-platform breaks are visible.
- Eventually: all three green + required, and Phase B publishes nightly artifacts.

## 6. Implementation note
Ships as `.github/workflows/ci-nightly-build.yml` (Phase A) in a normal PR with this spec. Phase B is
a follow-up PR tracked on #1718.
