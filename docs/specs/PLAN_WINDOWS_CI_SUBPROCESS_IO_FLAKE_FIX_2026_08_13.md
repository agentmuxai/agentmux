# Plan — fix the recurring `create_no_window_flag_set` flake on Windows nightly CI

**Date:** 2026-08-13
**Status:** proposed, not yet implemented
**Context:** follow-up to
[retro-nightly-ci-red-windows-not-macos-2026-08-13.md](../retro/retro-nightly-ci-red-windows-not-macos-2026-08-13.md),
written after that retro concluded "one-off flake, no fix needed unless it
recurs." Checking history turned up a prior identical failure, which changes
that conclusion — see §2.

## 1. Symptom

`agentmux-srv/tests/subprocess_io.rs::create_no_window_flag_set` (Windows-only,
`#[cfg(windows)]`) fails intermittently on the `windows-latest` leg of
`ci-nightly-build.yml`, which is the one hard-gated (non-`continue-on-error`)
platform in that workflow:

```
thread 'create_no_window_flag_set' (3400) panicked at agentmux-srv\tests\subprocess_io.rs:198:5:
CREATE_NO_WINDOW: stdout pipe produced no data — node.exe may be writing to a console instead of the pipe
```

The test spawns a real `node.exe` child with `CREATE_NO_WINDOW` (0x0800_0000)
set, writes to its stdin, and asserts a line arrives on stdout within a fixed
timeout. It exists to catch a real regression class: without the flag, a
child's stdio can end up attached to a console instead of the pipe, and this
is the one integration test that exercises the real Windows IOCP pipe path
end-to-end (unit tests / `cargo check` can't).

## 2. This is not the first time — timeline

| Date | Event |
|---|---|
| 2026-06-28 | First known failure. Timed out at the **original 5s** timeout. Fixed in `410875c55` by widening to **15s**, reasoning: "node.exe cold-start under CREATE_NO_WINDOW exceeding the 5s read timeout on a loaded CI runner." |
| 2026-08-13 | Second known failure, at **15s** — the same test, the same assertion, the same stated cause ("cold start on a loaded runner"), just a higher number. |

Two occurrences of the identical symptom, ~6.5 weeks apart, both "fixed" by
raising the same timeout, is a pattern: **the timeout bump is not the actual
fix, it's a fix for the last measured worst case.** Nothing about the June
fix addressed *why* node.exe cold-start is sometimes slow — it just moved the
threshold past the one data point that had been observed. There's no reason
to expect a third bump (e.g. to 30s) behaves any differently the next time a
slow runner shows up; it will pass more often by construction, but the
underlying variance is still uninvestigated. Per this repo's own standard for
this kind of bug (see the mDNS retro, §6: "didn't stop at the first
plausible-sounding explanation") — a second recurrence of the same guess is
the signal to actually check the guess instead of restating it.

## 3. What's already ruled out

- **Not sibling-test contention within the same run.** `ci-nightly-build.yml`
  already runs `cargo test --workspace -- --test-threads=1` (serial, for an
  unrelated reason — see that file's comment on process-shared FileStore /
  focus-flag test isolation). Only one test executes at a time, workspace-wide;
  `subprocess_io.rs`'s other node-spawning tests aren't running concurrently
  with this one.
- **Not a recent code regression.** No commit merged the day before either
  failure (2026-06-27 or 2026-08-12) touches `CREATE_NO_WINDOW` call sites
  (`agentmux-srv/src/util.rs`, `agentmux-srv/src/agents/runner.rs`) or
  `subprocess_io.rs` itself. The test file's last substantive change before
  today is the June timeout bump.
- **Not chronic / every-night.** The Windows leg passed clean in every nightly
  run checked between the two incidents (2026-08-07 through 2026-08-12, 6
  consecutive runs). This is intermittent, not a steady-state flake — that's
  consistent with "GH-hosted Windows runner had a slow moment," but that's
  still a guess, not a verified cause.

## 4. Root-cause hypotheses (unverified — this is what §5 investigates)

1. **Windows Defender real-time scanning.** GitHub-hosted Windows runners run
   Defender with real-time protection on by default; a freshly-invoked or
   infrequently-invoked executable (`node.exe`) can incur a scan delay on
   first exec that dwarfs normal process-creation time. This is a
   well-documented GH Actions Windows perf issue (multiple upstream reports of
   2-10x build/exec slowdowns from Defender scanning `%TEMP%` / build output /
   newly-written binaries). Nothing in this workflow currently excludes
   `node.exe`, the cargo target dir, or the checkout path from scanning.
2. **General GH-hosted Windows runner variance.** Shared infrastructure,
   outside this repo's control, has occasional slow nights (noisy neighbor,
   host contention). Consistent with "intermittent, not chronic," but not
   independently confirmed here — the June and August incidents both just
   asserted this without checking runner metrics.
3. **A real, rare bug in the CREATE_NO_WINDOW + Tokio piped-stdio path** (e.g.
   a narrow race in when the child's pipe handle becomes readable under that
   creation flag specifically) that manifests identically to "slow spawn."
   Low prior probability — the assertion has never failed with data that
   *doesn't* eventually show up if you wait longer via a manual bump — but
   not yet ruled out because no failure has been root-caused beyond "raise the
   number."

## 5. Investigation to run before picking a fix

These are cheap and should happen before committing to §6's fix, since the
right fix depends on which hypothesis holds:

1. **Add one-time diagnostics to the test on next failure**, gated so they
   don't run on the hot path: on timeout, before panicking, shell out to
   `Get-MpComputerStatus` (Defender status) and log wall-clock time since
   process spawn vs. since test start. Cheap, in-repo, no new dependency.
2. **Check whether this is really the first `node.exe` invocation of the
   test run.** `subprocess_io.rs` has 6 other tests that also call
   `spawn_node`; if `create_no_window_flag_set` isn't reliably first, the
   "cold Defender scan" theory needs the *whole test binary's* first node
   spawn timed, not just this test's.
3. **Pull the actual runner metrics for both failed runs** (2026-06-28 and
   2026-08-13) via the Actions run's usage/timing data if available, to see
   whether the whole job was measurably slower that night (supports
   hypothesis 2) versus this one step being an outlier in an otherwise-normal
   run (supports hypothesis 1 or 3).

## 6. Candidate fixes, ranked

**Recommended: do 6a now (cheap, safe, addresses the most-likely cause) +
6b as a stopgap in the same change. Hold 6c/6d unless §5 points elsewhere.**

**6a. Exclude the repo/build paths and `node.exe` from Windows Defender
real-time scanning in `ci-nightly-build.yml`'s Windows leg**, via
`Set-MpPreference -DisableRealtimeMonitoring $true` (or narrower
`-ExclusionPath`/`-ExclusionProcess`) as a setup step before `cargo build`.
This is the standard, widely-used mitigation for this exact class of GH
Windows-runner slowness and hasn't been tried here yet (confirmed: no
workflow in this repo touches Defender). If hypothesis 1 (§4.1) is right,
this fixes the root cause instead of widening a timeout again. Low risk —
runner is ephemeral and torn down after the job either way.

**6b. Replace the single fixed-timeout assertion with a bounded retry**
(spawn a fresh child, wait up to ~10s, retry once or twice before failing)
instead of one 15s (or 30s) shot. This doesn't fix the root cause, but it's a
strictly better stopgap than another blind timeout bump: it tolerates one
slow cold-start without giving up 2x the total wall-clock budget on *every*
run the way a flat 30s bump would, and a test that only passes on retry #2
is a visible signal (log it) that something is still off, instead of silently
looking identical to a fast pass.

**6c. Widen the timeout again (15s → 30s) with no other change.** The
default fallback if 6a doesn't reduce the recurrence and §5 doesn't
implicate anything actionable. Explicitly listed as lowest-preference
because it's the exact fix that already failed to hold for 6.5 weeks — only
worth doing again once 6a/6b are in place too, as a belt-and-suspenders
measure, not as the standalone fix.

**6d. Move `windows-latest` off the hard gate (add `continue-on-error` like
macOS/Linux already have).** Rejected: Windows is explicitly documented as
"the REQUIRED leg (primary platform)" in the workflow's own header comment —
softening the one real gate to solve a test-infra flake would hide genuine
Windows regressions, not just this flake. Not proposing this.

## 7. Suggested implementation order

1. Land 6a (Defender exclusion step) + 6b (bounded retry in the test) together
   — both are small, low-risk, and don't require waiting on a real failure to
   validate the mechanism (6b's retry logic can be unit-tested by forcing a
   short first-attempt timeout in a debug/test-only branch, or just reviewed
   carefully — it's a small loop).
2. Add the diagnostics from §5.1 into the retry-failure path (only logs on
   the *final* failed attempt) so if this recurs a third time, the next
   investigation starts with real data (Defender status, actual elapsed
   time) instead of another guess.
3. Watch the next 2-3 weeks of nightly runs. If clean, close this out. If it
   recurs, the diagnostics from step 2 should say which of §4's hypotheses
   was right, and 6c becomes a targeted (not blind) next step.

## 8. Non-goals

- Not touching `windows-latest`'s `continue-on-error` status (§6d).
- Not changing `--test-threads=1` — that's serving a documented, unrelated
  purpose (FileStore/focus-flag test isolation) and isn't implicated here
  (§3).
- Not adding `serial_test` or similar — no evidence sibling-test parallelism
  is a factor given tests already run serially workspace-wide.
