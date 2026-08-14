# Retro: last night's nightly CI was red, but not because of macOS

**Date:** 2026-08-13
**Area:** `.github/workflows/ci-nightly-build.yml`, `agentmux-srv/tests/subprocess_io.rs`

---

## 1. Symptom (as reported)

"macOS is failing in last night's CI, and we did work yesterday on this" —
the assumption being that yesterday's macOS CI fix (PR #2550, see
[retro-macos-ci-mdns-multicast-unsupported-2026-08-12.md](retro-macos-ci-mdns-multicast-unsupported-2026-08-12.md))
didn't actually work.

## 2. What actually happened

It didn't. **macOS is green.** The confusion came from two different things
both being true in the same run:

- `CI — nightly cross-platform build + test`, run
  [31681639637](https://github.com/agentmuxai/agentmux/actions/runs/31681639637)
  (2026-08-13 08:20 UTC), **finished `failure` overall.**
- But its four jobs were: `macos-latest` ✅, `ubuntu-latest` ✅, `vitest` ✅,
  **`windows-latest` ❌.**

The overall run reads "failure" because `windows-latest` is the only leg
with a hard gate — `ci-nightly-build.yml:34` sets
`continue-on-error: ${{ matrix.os != 'windows-latest' }}`, i.e. macOS and
Linux are still soft-gated (staged rollout, per §7 of yesterday's retro) and
can't turn the run red on their own. macOS didn't fail silently either: its
job conclusion is a genuine `success`, not a masked failure.

Yesterday's fix (marking the mDNS wire-discovery test `#[ignore]`, PR #2550)
worked exactly as intended — macOS has been clean in both nightly runs
since it merged (07:44 UTC nightly-artifacts run and 08:20 UTC
nightly-build run, 2026-08-13).

## 3. What actually failed: a Windows test, unrelated to yesterday's work

The `windows-latest` leg failed one test:

```
test create_no_window_flag_set ... FAILED
thread 'create_no_window_flag_set' (3400) panicked at agentmux-srv\tests\subprocess_io.rs:198:5:
CREATE_NO_WINDOW: stdout pipe produced no data — node.exe may be writing to a console instead of the pipe
```

This test spawns a real `node.exe` child with `CREATE_NO_WINDOW` set and
asserts its stdout pipe produces a line within a 15s timeout — it exists to
catch a real regression class (child writes to a console instead of the
pipe). Checked whether this connects to yesterday's work at all:

- No commit merged to `main` on 2026-08-12 touches subprocess-spawn code
  (`util.rs`, `runner.rs`, `identity_auth_spawn.rs`, `shell_node.rs`,
  `srv_spawner.rs`, or any other `CREATE_NO_WINDOW` call site). Yesterday's
  merges were the Warden feature set, the mDNS test fix, an mDNS-firewall-bind
  fix (`b0421ab7c`, a different subsystem), a WinGet/MS-Store CI job
  disable, and a clipboard fix — none touch this path.
- The test itself hasn't changed since `e78d117d0` (RAII/Job-Object guards,
  predates yesterday by a wide margin).
- Checked the prior 6 nightly runs (2026-08-07 through 2026-08-12): the
  `windows-latest` job passed clean in every one. This is the first failure
  of this test in at least a week.

That combination — no relevant code change, no prior history of failing —
points to a one-off environment flake (a loaded/cold Windows runner where
`node.exe` didn't get scheduled fast enough to write to its pipe inside 15s)
rather than a regression. The test's own comment already documents that its
timeout was widened from 5s to 15s once before specifically to absorb
"node.exe cold-start on a loaded Windows CI runner" — this looks like the
same class of noise, just still not fully eliminated.

## 4. Why the mix-up

Both "CI is red" and "we did CI work yesterday" were true, and the
yesterday-work (mDNS/macOS) was assumed to be the same failure as
last night's redness without checking which leg actually failed. The
`continue-on-error` staging on macOS/Linux means the run's top-level
conclusion doesn't tell you *which* platform failed — you have to open the
per-job breakdown, which is where this stopped being ambiguous.

## 5. Follow-up

- No code fix needed for `create_no_window_flag_set` unless it recurs. If it
  fails again on `windows-latest` with the same signature, that upgrades it
  from "one-off flake" to "recurring" and warrants either a longer timeout
  or a runner-capacity investigation, per the discipline in
  [retro-macos-ci-mdns-multicast-unsupported-2026-08-12.md §6](retro-macos-ci-mdns-multicast-unsupported-2026-08-12.md) —
  don't stop at the first plausible explanation without checking history first,
  which is what this retro did before concluding "flake."
- Yesterday's retro (§7) already flagged deciding whether to graduate
  macOS/Linux off `continue-on-error` now that both the FSEvents watcher bug
  and the mDNS test are resolved. Still open, still a separate decision from
  this one.
