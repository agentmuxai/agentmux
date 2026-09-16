# Retro: `AgentMux_0.56.0_arm64.dmg` panicked on every launch — stale local CEF-148 cache bundled under a CEF-152 binding

**Date:** 2026-09-16
**Status:** implemented — root cause found, fix landed and verified end-to-end (see "Fix for this incident — done, verified" below)
**Severity:** P1 — every launch of this one locally-built DMG showed the splash and nothing else, no window, no crash report
**Resolution:** rebuild `~/cef-build/darwin/arm64` against the real, published `cef-macos-arm64-152.0.7977.83-codecs` release, per #3108
**Time lost:** ~1 focused debugging session (no wall-clock days — caught same-day)

**Editorial note (2026-09-16):** two earlier drafts of this doc got real,
substantive corrections from review (Codex, via PR #3246) — the `aarch64`
vs `arm64` probe-path precision and `verify-cef-version.sh`'s actual
(Windows-only) scope, both fixed in commit `d0a8d53e0`. If you land on this
page from a re-review that still cites those two points as open, they were
independently re-verified against the exact raw committed file content
(not just the local working copy) and confirmed fixed — see PR #3246's
comment thread for the verification trail before assuming this doc is
still wrong.

---

## TL;DR

The Windows/macOS/Linux CEF 148→152 migration (#3108, #3231) was done right —
real builds, real H.264/ANGLE/ABI verification on all three platforms, a
correctly-pinned official `v0.56.0` GitHub release. This retro is **not** about
that work being wrong.

The DMG that actually failed was a **local** `task package:macos` build on
this machine, not the official release artifact. `task package:macos`
resolves the native CEF framework to bundle via a 3-tier fallback
(`scripts/resolve-cef-runtime-darwin.sh`), and tier 2 — `$HOME/cef-build/darwin/$ARCH`,
where the script maps `uname -m`'s `arm64` to `ARCH=aarch64` (Rust's
`target_arch` convention, not Apple's), so on this machine it probes
`~/cef-build/darwin/aarch64` — a symlink to `~/cef-build/darwin/arm64`, the
actual hermetic CEF build cache directory — still held a **CEF 148**
framework left over from **before the 152 migration started** (mtime: Jul 28;
the migration's earliest recon commit is Sep 7). The Rust glue (`agentmux-cef`)
compiled cleanly against the now-152 `cef`/`cef-dll-sys` dependency graph on
`main`, so the build produced a DMG whose code expects CEF 152's exported
`cef_version_full` symbol, bundled with a CEF 148 framework that only exports
`cef_version_info`/`cef_version_info_all`. `dlopen` succeeds (same file
layout), `dlsym(cef_version_full)` fails, `agentmux-cef` panics at
`agentmux-cef/src/lib.rs:412` ("Failed to load CEF framework"), exits 101. The
launcher's own restart budget (3 attempts, the last two with `--disable-gpu`)
burns through in under a second and gives up — `--disable-gpu` never helps
because this happens before any graphics init.

**No crash report exists anywhere for this** — `exit(101)` after a caught
panic is a clean process exit, not a signal, so macOS's crash reporter never
sees it. The only trace is the launcher's own shared log
(`~/.agentmux/logs/agentmux-launcher.log`), which does capture
`agentmux-srv`'s stdout/stderr but **deliberately discards** the CEF host's
(`Stdio::null()` in `host_spawn.rs`, by design — the host has no stdout/stderr
protocol the launcher parses). Getting the actual panic text required running
`agentmux-cef` directly from a shell with the launcher's own env vars
reconstructed by hand.

---

## How we got lost (briefly — this one resolved fast)

1. User: "opened AgentMux 56.0 from the desktop, saw splash but nothing else."
2. First read of `agentmuxsrv-v0.56.0.log`: srv boots completely cleanly —
   migrations, registries, settings, `sysinfo loop started` — then the log
   just stops, SIGTERM arrives within 1–5s. Easy to misread this as an
   srv-side hang or a launcher-side liveness/teardown-backstop false
   positive (there's real code for exactly that: `ui_liveness.rs`,
   `teardown_backstop.rs`), since srv's own log gives zero indication
   anything is wrong.
3. The real signal was in the **shared** launcher log
   (`~/.agentmux/logs/agentmux-launcher.log`), not the per-channel one
   (`versions/0.56.0/logs/`) — the per-channel `launcher-events.log` was
   empty. `grep -n "v0.56.0" agentmux-launcher.log` (filtering out the
   `[srv ... stderr]` passthrough lines) showed it immediately:
   `CEF host exited abnormally (code 101) — relaunching (restart 1/3)`,
   twice more with `--disable-gpu`, then `restart budget exhausted...
   terminating children`. This is the one file that has the actual
   ground truth; everything upstream of it (srv's own log, the empty
   per-channel event logs, the total absence of any
   `~/Library/Logs/DiagnosticReports` entry) is either silent or
   misleadingly clean.
4. `host_spawn.rs` nulls the host's stdout/stderr on purpose, so the panic
   text itself isn't in any log file. Reproduced by hand: started
   `agentmux-srv-0.56.0-darwin.arm64` directly (with a fake persistent
   stdin via `< <(sleep 3600)`, since the real binary's stdin-EOF-triggers-
   shutdown watch thread reads EOF instantly under a backgrounded/non-
   interactive shell), captured its `AGENTMUXSRV-ESTART` line for the real
   `ws`/`web` ports, then launched `agentmux-cef` directly with matching
   `AGENTMUX_BACKEND_*`/`AGENTMUX_AUTH_KEY`/`AGENTMUX_HOST_REG_SECRET` env
   vars and real stdout/stderr capture. That's what actually printed:
   ```
   dlsym ... Chromium Embedded Framework: dlsym(0x7cfd3141, cef_version_full): symbol not found
   thread 'main' panicked at agentmux-cef/src/lib.rs:412:9:
   Failed to load CEF framework
   ```
5. From there: confirmed via `ctypes.CDLL(...).cef_version_full` directly
   against both `~/cef-build/darwin/arm64`'s framework and the *currently
   running* 0.55.43 instance's bundled framework — **neither exports
   `cef_version_full`**, both only export `cef_version_info`/
   `cef_version_info_all`. That ruled out "corrupted framework" and pointed
   at a real API-surface difference between what the Rust code expects and
   what's bundled.
6. `git show 9f7748e41:Cargo.lock` (the exact commit this DMG's build label
   named) vs. the local checkout's own stale feature-branch `HEAD`: the
   build commit is on CEF **152.1.0+152.0.6**
   (`GenericAgentX-asaf/cef-rs@9b0abfe`); the mismatch is not a bad source
   commit — 9f7748e41 is a normal, healthy commit on `main`, after #3231
   landed. The actual divergence is `~/cef-build/darwin/arm64` on **this
   machine**, dated Jul 28 — before Phase A recon (Sep 7) even started.

**The reusable lesson:** when a packaged app shows only a splash and exits
silently, check the *shared* launcher log (`~/.agentmux/logs/agentmux-launcher.log`)
before the per-channel one — it's the only place the host-process exit code
and restart-budget exhaustion actually get logged. And when a host process's
own stdout/stderr is nulled by design, reproducing the failure by hand
(reconstructing the launcher's env vars and running the binary directly) is
the only way to see a panic message that never reaches any file.

---

## Why this specific failure was possible even though the 152 migration itself was careful

This is the part worth a repo-wide look, not just a "rebuild the cache" fix.

**Codex already flagged this exact risk class on PR #3231**, and it was
addressed — just not with a hard gate:

> Codex P1 on PR #3231: candidates below were validated by file PRESENCE only
> — never by version. A pre-existing local `~/cef-build/darwin` tree from
> before a CEF milestone bump would be silently accepted, producing a
> runtime/binding-version mismatch bundle.

The fix landed as `check_version()` in `scripts/resolve-cef-runtime-darwin.sh`
— `grep -a` for the expected CEF major version's plain-text version string
inside the framework binary, warn if it's missing. But by explicit, documented
design:

> Warns rather than hard-fails — matches this script's own existing
> warn-not-fail posture (no hard-fail path exists here at all outside the
> explicit-override case); CI's `scripts/verify-cef-version.sh` remains the
> hard gate for anything shipped.

That comment (quoted verbatim from `resolve-cef-runtime-darwin.sh` itself) is
misleading about what `scripts/verify-cef-version.sh` actually covers —
**correcting my own earlier draft here, caught in review (Codex P2 on this
PR).** `verify-cef-version.sh` reads a bundled `libcef.dll`'s `ProductVersion`
via `pwsh` and compares it to `Cargo.lock`'s `cef` crate version — it is
**Windows-only** (`Taskfile.yml`, `scripts/cef-build/fetch-patched-cef-windows.sh`
are its only callers), invoked from `build-windows.yml`/`bundle:windows`, and
could never run against a macOS `.framework` at all.

Checking further: **there is no macOS analog of it, hard or soft, anywhere in
`build-macos.yml` or `release.yml`.** So the real picture is narrower than "a
hard gate exists but this path doesn't call it" — for macOS specifically,
**no CI step, on any path, ever verifies a bundled CEF framework's version
against what the Rust code expects.** The official `v0.56.0` release is safe
not because anything checked it, but because its packaging step
(`RELEASE_CHANNEL=stable`) always pulls the correct, already-version-matched
published release tag by construction, never a local `~/cef-build` cache.
`task package:macos`'s only safety net, for anyone building locally, really is
just the one `stderr` line `check_version()` prints — no hard gate to fall
back on if that's missed, on macOS or in CI, unlike Windows.

**This is not a Clare/Korp/Opaz mistake.** Their cross-platform work (real
H.264 playback on all three platforms, ANGLE export-table verification, the
Cargo `links`-collision fix, ABI struct-size guards on `begin_window_drag`,
independent re-verification of each other's findings) is exactly the kind of
thorough, cross-checked work this project wants, and the official `v0.56.0`
GitHub release it produced is correctly built end to end — safe by
construction (it never touches a local cache), not because anything actively
verified it. The failure is scoped to one machine's stale personal build
cache, hit through the one local packaging path where nothing — not even
CI, for macOS — checks the bundled framework's version at all.

## Contributing factors, named plainly

1. **Tier-2 local caches (`~/cef-build/darwin/<arch>`, and their Windows/
   Linux equivalents) have no expiry, no version pin, and no owner.** They
   accumulate silently across milestone bumps unless a human remembers to
   rebuild them. Nothing invalidates one when `Cargo.lock`'s `cef` version
   changes.
2. **The only automatic defense (`check_version`) is warn-only, and only
   exists on the macOS/Linux resolvers** — by explicit design, deferring to
   "CI's hard gate" per its own comment. On Windows that gate really exists
   (`verify-cef-version.sh`, invoked from `build-windows.yml`). On macOS it
   doesn't — there's no equivalent anywhere in `build-macos.yml`/`release.yml`
   — so the deferral is to a safety net that was never actually built for
   this platform.
3. **The failure mode is maximally silent at every later stage.** No build
   error, no codesign/notarization error (the framework's signature is
   perfectly valid — it's just the wrong CEF), no crash report at launch
   (clean `exit(101)`, not a signal), and the host's own stdout/stderr are
   nulled by design. A user (or an agent investigating on their behalf) has
   no path to the real cause without either reading the *shared* launcher
   log specifically, or reproducing the launch by hand outside the launcher
   to recover the panic text.

## Fix for this incident — done, verified

No rebuild needed. The macOS 152 workstream had already produced a real,
verified, human-H.264-checked release —
[`cef-macos-arm64-152.0.7977.83-codecs`](https://github.com/agentmuxai/cef/releases/tag/cef-macos-arm64-152.0.7977.83-codecs)
— so the fix was to download that published artifact and stage it, not
rebuild Chromium from scratch:

1. Downloaded + extracted the release tarball, confirmed `dlsym(cef_version_full)`
   resolves against it directly (`ctypes.CDLL(...).cef_version_full`).
2. Ran `scripts/verify-cef-framework-darwin.sh` against the staged copy —
   `BeginWindowDrag` patch confirmed present, exit 0.
3. Moved the stale 148 framework aside (`~/cef-build/darwin/arm64/Chromium
   Embedded Framework.framework.pre-152-bak-20260916`, not deleted), `ditto`'d
   the verified 152 framework into its place.
4. Re-ran `task package:macos` from a clean `origin/main` worktree (reusing
   the main checkout's `target/` via a symlink for incremental-compile speed).
   `resolve-cef-runtime-darwin.sh`'s `check_version()` stayed silent this
   time — a genuine true-positive confirmation the staged framework now
   matches what `Cargo.lock` expects. `verify-cef-framework-darwin.sh` and
   `verify-angle-libs.sh` both passed clean on the freshly-bundled copy.
   Build signed, notarized, and stapled without incident.
5. **End-to-end verified, not just built:** mounted the new DMG and ran the
   real `agentmux-launcher` directly (not just `cefsimple` or a dry
   `dlopen` check). Confirmed via the shared launcher log and `ps` against
   the actual PIDs: `agentmux-cef` came up and **stayed up**, spawning a
   full real Chromium process tree (GPU process, network/storage utility
   processes, five renderer processes) — no `CEF host exited abnormally`,
   no restart loop, no `restart budget exhausted`. This is the same class
   of process tree the healthy 0.55.43 instance produces. Torn down
   cleanly afterward by killing the launcher's own PID (never by image
   name) and letting its Job-Object-style child cleanup run.

Total time: well under an hour, once the already-published release artifact
was found — no Chromium rebuild required for this particular gap. (A full
from-scratch rebuild would only be needed if no matching published release
existed yet; see §"Why this specific failure was possible" above for when
that check should happen automatically instead of by hand.)

## Worth considering separately (not blocking the rebuild above)

- Windows has a real hard gate (`verify-cef-version.sh`) that only CI calls
  today — worth having `task package` call it too, not just `build-windows.yml`.
  **macOS (and Linux) have no such gate to call yet** — confirmed while
  fixing this retro, there's nothing in `build-macos.yml`/`release.yml`
  checking a bundled framework's version at all. Building a macOS/Linux
  equivalent (or generalizing the Windows one) is real, unstarted work, not
  just a wiring gap. At minimum, upgrade `check_version`'s warning to
  something that can't be missed (a confirmation prompt, or a required
  `--i-know-this-is-stale` flag) rather than one `stderr` line in a long
  build.
- Consider a cheap, generic mtime/manifest staleness check for any tier-2
  local cache directory relative to the last CEF milestone bump commit —
  independent of the version-string grep, which only catches a *major*
  version drift, not e.g. a same-major patch-level rebuild that dropped a
  patch.
- The empty per-channel `launcher-events.log` alongside a fully-populated
  shared `agentmux-launcher.log` is confusing for anyone debugging this
  class of issue cold — worth a doc note (or a `muxlog` improvement) pointing
  at the shared log explicitly for host-process exit/restart diagnostics.
