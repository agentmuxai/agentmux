# GitHub Actions Workflow Audit — 2026-06-28

11 workflow files in `.github/workflows/`.

---

## Inventory

### On-demand / release-triggered

| File | Trigger | Purpose |
|------|---------|---------|
| `build-linux.yml` | `workflow_dispatch`, `repository_dispatch[build-linux]` | Linux AppImage release pipeline. Downloads patched libcef.so from agentmuxai/cef, runs `scripts/package-linux.sh`, uploads AppImage to GitHub Release (when `release-tag` input set). Requires `A5AF_PACKAGES_TOKEN`, `CEF_RUNTIME_TOKEN`. |
| `build-macos.yml` | `workflow_dispatch`, `repository_dispatch[build-macos]` | macOS signed + notarized DMG (arm64) release pipeline. Creates temp keychain, imports Developer ID cert, runs `task package:macos`, uploads DMG. Requires 5 Apple secrets + `A5AF_PACKAGES_TOKEN`. |
| `container-image.yml` | `push` (tags `v*`), `workflow_dispatch` | Builds + pushes Docker agent image (`ghcr.io/agentmuxai/agent-claude`) for linux/amd64 + linux/arm64. Pins a specific Claude Code version (defaults to latest). Triggered automatically on every version tag. |
| `input-bench-report.yml` | `workflow_dispatch` only | Input latency bench (agent or terminal). Requires a self-hosted `[input-bench]` runner — **none registered yet**, so any accidental schedule would queue indefinitely. Report-only mode (never a blocking gate). |
| `release-consistency.yml` | `pull_request`, `push` (main) — both filtered to `VERSION_HISTORY.md` changes | Verifies all 5 version locations agree. Fast (no build/install). Only fires on release-intent commits. |

### Nightly scheduled

| File | UTC time | Purpose |
|------|----------|---------|
| `ci-nightly-artifacts.yml` | **06:00** | Phase B (issue #1718): full packaging on all 3 platforms. Windows: portable ZIP + Inno Setup EXE + MSIX. macOS: signed + notarized DMG. Linux: AppImage. Uploads as 7-day artifacts. Heavy — pays CEF build + packaging toolchain on every platform. |
| `ci-nightly-build.yml` | **07:00** | Phase A (issue #1718): `cargo build --release --workspace` on 3-platform matrix. Windows required; ubuntu/macOS non-blocking. Surfaces cross-platform compile/link breaks. **Build only — no tests run.** |
| `ci-fast.yml` | **08:00** | CEF-free `cargo test` on 3-platform matrix (`-p agentmux-launcher -p agentmux-srv -p agentmux-common`, `--test-threads=1`) + `npx vitest run` on ubuntu. Windows required; others non-blocking. Named "fast" because it skips the CEF build cost. |
| `ci-nightly.yml` | **09:00** | Full workspace `cargo test --workspace -- --test-threads=1` on Windows (with CEF — requires Ninja, pays the full CEF compile). Also runs `npx vitest run` on the same Windows runner. The only nightly that tests CEF-dependent code. |
| `input-handler-layout-reads.yml` | **10:00** | Lint: enforces the invariant "no layout reads after style mutations on the keystroke path" (SPEC_INPUT_RESPONSIVENESS_... §3). Runs `tools/lint/check-input-handler-layout-reads.sh`. Fast (ubuntu, no build). |
| `input-handler-sync-ipc.yml` | **10:00** | Lint: enforces "no synchronous IPC on any input path" (invariant I2). Runs `tools/lint/check-input-handler-sync-ipc.sh`. Fast (ubuntu, no build). Fires at same time as layout-reads lint above. |

---

## Nightly schedule at a glance

```
06:00  ci-nightly-artifacts  ── packaging (Windows + macOS + Linux)
07:00  ci-nightly-build      ── cargo build --release --workspace (3 platforms)
08:00  ci-fast               ── cargo test CEF-free (3 platforms) + vitest
09:00  ci-nightly            ── cargo test --workspace + CEF (Windows) + vitest
10:00  input-handler-layout-reads  ─┐ fast lints, run concurrently
10:00  input-handler-sync-ipc      ─┘
```

Six workflow runs fire every night. Wall-clock for the Rust legs: ~15–30 min cold per platform (CEF build); vitest is fast.

---

## Issues found

### 1. vitest runs twice nightly (redundant)
`ci-fast.yml` runs `npx vitest run` on ubuntu-latest at 08:00.
`ci-nightly.yml` also runs `npx vitest run` on windows-latest at 09:00.
The frontend test suite is platform-agnostic. One of these is redundant; keeping the Windows run (in ci-nightly.yml) gives slightly more signal but the ubuntu run in ci-fast is earlier and cheaper.

### 2. Nightly build has no tests (ci-nightly-build.yml)
`ci-nightly-build.yml` compiles `--workspace` including the CEF crate on all 3 platforms, paying the full CEF build cost (~15–30 min/platform), but exits without running any tests. The tests that cover the same crates run separately an hour later in ci-fast and ci-nightly. Since the CEF build is already paid, adding a `cargo test` step costs only marginal runner time.

### 3. Four separate nightly Rust workflows where two would suffice
The four nightly Rust workflows (build, fast-test, CEF-test, artifacts) exist for historical reasons (phased rollout per specs). They are now independent jobs at staggered times, which means:
- A failure in build (07:00) does not block fast-test (08:00) — you can get passing tests on broken build artifacts.
- The CEF build is paid **twice**: once in ci-nightly-build (--release, all platforms) and again in ci-nightly (debug/test, Windows only).
- Two separate vitest runs nightly (see issue #1 above).

Proposed consolidation: merge ci-fast.yml into ci-nightly-build.yml (add test + vitest steps after the build), then delete ci-fast.yml and ci-nightly.yml. Result: 2 nightly Rust workflows instead of 4.

### 4. input-bench-report.yml — unregistered runner, dispatch-only (low risk)
Requires `[self-hosted, input-bench]` runners not yet registered. Since it is `workflow_dispatch`-only there is no runaway queue risk. Risk is that a future refactor adds a `schedule:` trigger without noticing the runner requirement. Worth a comment noting the runner is not yet provisioned.

### 5. ci-fast.yml naming is misleading
Named "CI — cross-platform CEF-free crates (nightly)" internally but the filename `ci-fast.yml` implies per-PR speed. It is a nightly job. If ci-fast is merged into ci-nightly-build (issue #3), this goes away. If kept, rename to `ci-nightly-test-fast.yml`.

---

## Secrets inventory

| Secret | Used by |
|--------|---------|
| `A5AF_PACKAGES_TOKEN` | build-linux, build-macos, ci-nightly-artifacts (macOS + Linux legs), container-image |
| `CEF_RUNTIME_TOKEN` | build-linux, ci-nightly-artifacts (Linux leg) |
| `APPLE_CERTIFICATE` | build-macos, ci-nightly-artifacts (macOS leg) |
| `APPLE_CERTIFICATE_PASSWORD` | build-macos, ci-nightly-artifacts (macOS leg) |
| `APPLE_ID` | build-macos, ci-nightly-artifacts (macOS leg) |
| `APPLE_PASSWORD` | build-macos, ci-nightly-artifacts (macOS leg) |
| `APPLE_TEAM_ID` | build-macos, ci-nightly-artifacts (macOS leg) |

`GITHUB_TOKEN` (auto-provisioned) is used with `contents: write` in build-linux, build-macos, and ci-nightly-artifacts for release asset uploads.

---

## Recommended actions

| Priority | Action |
|----------|--------|
| High | Add `cargo test --workspace -- --test-threads=1` + vitest steps to `ci-nightly-build.yml` after the build step. |
| High | Delete `ci-fast.yml` — fully superseded by the expanded ci-nightly-build. |
| High | Delete `ci-nightly.yml` — Windows CEF test is covered by the Windows leg of the expanded ci-nightly-build. |
| Low | Add a comment to `input-bench-report.yml` that the self-hosted runner is not yet provisioned, to prevent accidental schedule trigger. |
| Low | Update `SPEC_NIGHTLY_CROSS_PLATFORM_BUILDS_2026_06_23.md` status section after the consolidation lands. |
