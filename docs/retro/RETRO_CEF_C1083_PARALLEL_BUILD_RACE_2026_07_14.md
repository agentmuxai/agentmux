# Retro: MSVC C1083 in `libcef_dll_wrapper` — Actual Root Cause: Windows MAX_PATH (2026-07-14)

**TL;DR (added after the fact — read this first):** the title and most of
this document describe a build-parallelism theory that was thoroughly
tested and **disproven**. The real, confirmed root cause is Windows'
260-character `MAX_PATH` limit: this dev machine's agent workspace path
(`C:\Users\...\agenty-0629j\Usersasafe.agentmuxagentsagenty-0629jagentmux\...`
— note the unusual duplicated-looking final segment) is long enough that,
combined with CEF's own deeply-nested `libcef_dll/{cpptoc,ctocpp}/test/`
source tree and its longest auto-generated filenames, several `.obj` output
paths exceed 260 characters even with `CMAKE_OBJECT_PATH_MAX=500` already
set. `LongPathsEnabled` is `0` (disabled) in this machine's registry.
**Fix that was verified end-to-end: `subst K: "<repo path>"` and build from
`K:\` instead — a plain drive-letter alias needs no admin rights and
immediately shortens every path Windows sees for the workspace.** A full
`agentmux-cef` release build that failed identically at full parallelism,
reduced parallelism, with a fresh build directory, and with corrected
`TMP`/`TEMP` all *succeeded cleanly* the moment it ran from `K:\` — see the
final section below for the actual test sequence and results. Everything
under "Root cause" and "Mitigation options" below is preserved as-written
for the record, but should be read as a documented false start, not
current guidance.

## What happened

Three consecutive Windows native builds on this dev machine — two `task dev`
attempts and one `task package` attempt, all on `main` around v0.53.5 — failed
identically while `cef-dll-sys`'s build script shells out to CMake/Ninja to
compile the vendored `libcef_dll_wrapper` C++ target:

```
[24/37] Building CXX object libcef_dll_wrapper\CMakeFiles\libcef_dll_wrapper.dir\ctocpp\test\translator_test_scoped_library_child_ctocpp.cc.obj
...\libcef_dll\ctocpp\test\translator_test_scoped_library_child_ctocpp.cc : fatal error C1083: Cannot open compiler generated file: '': Invalid argument
...
ninja: build stopped: subcommand failed.
thread 'main' (55888) panicked at .../cmake-0.1.58/src/lib.rs:1132:5:
command did not execute successfully, got: exit code: 1
build script failed, must exit now
task: Failed to run task "build:host:windows": exit status 101
```

`task dev`'s existing `||` fallback (`build:host:windows`,
`bash scripts/repair-cef-extract.sh && cargo build ...`, added for the
unrelated Defender-extraction race — see
`docs/retro/RETRO_CEF_BUILD_RACE_2026_04_24.md`) fired automatically on one
of the three runs and reproduced the exact same failure on retry. That repair
script targets a missing-directory extraction problem, not this one, so it
correctly no-ops and doesn't help here.

## Root cause (diagnosed, not directly instrumented)

Every failure lands inside the same batch of auto-generated CEF API test
wrapper `.cc` files (`libcef_dll/{cpptoc,ctocpp}/test/*_cpptoc.cc` /
`*_ctocpp.cc` — dozens of small, near-identical translation units), all
compiled by `cmake --build ... --parallel 32` into a **shared PDB**
(`/Fdlibcef_dll_wrapper\...\libcef_dll_wrapper.pdb`, `/FS` forcing
synchronous access). In each failed run, **dozens of these files fail with
the identical error simultaneously** (not just one stray file) — see Evidence
below. That fan-out pattern, concentrated in the single densest/most
tightly-batched part of the wrapper build, points at MSVC's PDB-write path
(`mspdbsrv.exe`) becoming saturated under very high concurrent `cl.exe`
invocation, not a one-off filesystem hiccup.

The `--parallel 32` figure is not hardcoded in our Taskfile — the `cmake`
crate derives it from Cargo's `NUM_JOBS`, which defaults to the full logical
CPU count (32 on this machine, confirmed via `wmic cpu get
NumberOfLogicalProcessors`) when no `-j`/`--jobs` is passed to `cargo build`.

**Correction (post-write):** this retro originally claimed "12 other
AgentMux instances" were running, based on `tasklist | grep -c
"agentmux-0.53"` → 12. That count was wrong — it conflated OS process count
with app-instance count. `Get-CimInstance Win32_Process` with parent PIDs
shows there is exactly **one** real AgentMux instance running: one
`agentmux.exe` launcher, which spawned one `agentmux-srv-0.53.2-...exe`
sidecar and one `agentmux-0.53.2.exe` host — and that single host process
had **9 child processes**, all also named `agentmux-0.53.2.exe`. That 9-way
fan-out from a single parent is normal CEF/Chromium multi-process
architecture (GPU process, renderer(s), utility, network service, crashpad
handler, etc.) for **one window** — the same reason Task Manager shows a
dozen `chrome.exe` entries for one browser window. 1 launcher + 1 srv + 1
host + 9 CEF children = 12 processes, matching the earlier grep count
exactly, but from a single instance, not twelve.

Separately, the same process listing did show a genuinely large number of
`agentmux-bashwrap.exe` (12) and `agentmux-mcp.exe` (4) processes, none of
which are children of the AgentMux app's own process tree — these are
shell-tool and MCP-server helper processes, almost certainly spawned by
several concurrent agent sessions (this one included — `agentmux-bashwrap`
is literally the shell wrapper this session's own Bash tool calls run
through) doing unrelated work on the same physical machine at build time.
That's a real source of background CPU load, but it's categorically
different from "12 AgentMux app instances" and its actual contribution to
the C1083 race is unverified — noted here as a corrected, weaker version of
the original contention theory, not a replacement root cause.

This is a **race**, not a deterministic break — the same code, same
toolchain, same CEF revision failed 3/3 times in this session but has
presumably succeeded on prior builds (v0.53.4 and earlier all shipped
normally). Machine load at build time is the variable.

## Evidence

`task dev` attempt #2 (full log captured), first custom-build-script failure:

```
[32/51] Building CXX object libcef_dll_wrapper\...\panel_ctocpp.cc.obj
...api_version_test_scoped_client_cpptoc.cc : fatal error C1083: Cannot open compiler generated file: '': Invalid argument
ninja: build stopped: subcommand failed.
thread 'main' (46840) panicked at .../cmake-0.1.58/src/lib.rs:1132:5
```

Same log, `grep -c "fatal error C1083"` → **50 occurrences** across the
single failed build attempt (i.e. ~50 distinct `.cc` files hit it in that one
`ninja --parallel 32` invocation, not just the one whose error happened to
surface first). The Taskfile's `||` fallback then re-ran
`repair-cef-extract.sh && cargo build ...` automatically — the retry hit the
**identical** failure signature a second time (`grep -n "panicked at"` shows
two separate panic blocks in the one log).

`task package` attempt (separately captured), same signature, this time
visibly interleaved with the `[N/37] Building CXX object ...` progress lines
— errors follow immediately after several different files' own "Building"
announcement, confirming multiple in-flight `cl.exe` invocations failing
concurrently rather than one bad file blocking the rest:

```
[20/37] Building CXX object ...api_version_test_scoped_library_child_child_v1_ctocpp.cc.obj
...api_version_test_scoped_library_child_child_v1_ctocpp.cc : fatal error C1083: ...
[21/37] Building CXX object ...translator_test_scoped_client_child_cpptoc.cc.obj
...translator_test_scoped_client_child_cpptoc.cc : fatal error C1083: ...
[22/37] Building CXX object ...translator_test_scoped_library_child_child_ctocpp.cc.obj
...translator_test_scoped_library_child_child_ctocpp.cc : fatal error C1083: ...
```

`tasklist | grep -c "agentmux-0.53"` → **12** at the time of both failures —
**see the correction above**: this is one AgentMux instance's own process
tree (launcher + srv + host + 9 CEF sub-processes), not 12 separate
instances. `wmic cpu get NumberOfLogicalProcessors` → **32**, matching the
`--parallel 32` cmake invocation exactly — i.e. the build alone was already
requesting full-machine parallelism, before accounting for whatever else
(the one running AgentMux instance, this session's own tooling, other
agents' bashwrap/mcp processes) was also active.

## Why this wasn't caught earlier

- This exact `libcef_dll_wrapper` C++ compile has presumably succeeded on
  this machine for every prior release (v0.53.4 and earlier built fine per
  `VERSION_HISTORY.md`). Nothing in this session's code changes touches CEF,
  CMake, or the build scripts — the frontend/backend PRs merged today
  (tab-drag fix, MCP seed catalog, swarm labels, Linux splash timing, CLI pin
  consolidation) are unrelated.
- Multiple agents on this repo normally work on disjoint code in parallel
  (separate branches/PRs), which is safe by design. Nothing about that model
  previously surfaced the risk that several agents' `task dev`/`task package`
  invocations landing on the *same physical machine* at the *same time* would
  contend for CPU/PDB-write bandwidth during the native build itself — that
  gap is specific to this failure, not a general flaw in the parallel-agent
  workflow.
- The one AgentMux instance running at the time isn't itself unusual (a
  single window with CEF's normal multi-process fan-out). What's less well
  understood is how much background CPU load this session's own tooling
  (and any other concurrent agent sessions' `bashwrap`/`mcp` processes on
  this shared machine) contributes at native-build time — the isolation
  model documented in `CLAUDE.md` ("Multiple Instances Run in Parallel")
  guarantees data-dir/port/binary isolation between *running app instances*;
  it says nothing about, and doesn't protect against, concurrent *build* CPU
  contention from unrelated processes on the same box.

## Mitigation options (not yet applied)

**Option A (cheap, this-session-actionable) — cap build parallelism**

Pass `-j <n>` to the outer `cargo build` (or set `NUM_JOBS=<n>` in the
environment before invoking `task dev`/`task package`), e.g. `NUM_JOBS=8`.
The `cmake` crate reads `NUM_JOBS` and forwards it as `cmake --build
--parallel <n>`, so this directly caps the contended C++ compile without
touching vendored code. Cost: slower host build (more wall-clock, less CPU
racing). Worth trying regardless of the corrected instance count above —
`--parallel 32` against a shared PDB is aggressive on its own, even before
factoring in anything else running on the machine.

**Option B (upstream-ish) — patch `cef-dll-sys`'s build script**

The fork at `https://github.com/AgentU-asaf/cef-rs` (pinned in `Cargo.lock`)
could cap its own `--parallel` request to something like
`min(NUM_JOBS, 8)` regardless of what Cargo passes down, so this doesn't
require every developer/agent to remember to set `NUM_JOBS`. Cost: fork
maintenance, needs review against upstream `cef-rs` drift.

**Option C (process-level) — stagger concurrent native builds**

If multiple agents on this machine are expected to run `task dev`/`task
package` around the same time, some out-of-band coordination (a lockfile,
a shared "build slot" convention, or just checking `tasklist | grep -c
agentmux` before kicking off a build) would avoid the contention window
entirely. Cost: process discipline, no code change.

**Recommendation:** Option A immediately (zero code change, just an env var
on this machine when other instances are running), Option B as a proper fix
if this keeps recurring across machines/agents.

## Action items

- [ ] Retry `task dev` / `task package` with `NUM_JOBS=8` (or similar) set,
  to confirm Option A actually avoids the race (not yet verified — this
  retro was written instead of exhausting further retries at full
  parallelism, per explicit direction this session).
- [ ] If Option A is confirmed effective, consider documenting `NUM_JOBS`
  guidance in `CLAUDE.md`'s Build Prerequisites section for machines running
  multiple concurrent AgentMux instances.
- [ ] If this recurs on a lightly-loaded machine (few/no sibling instances
  running), that would falsify the contention theory and point back at
  something CEF/toolchain-version-specific instead — worth re-opening this
  retro's root-cause section if so.
  **This item fired: see "Actual root cause" below.**

## Actual root cause, found later (Windows `MAX_PATH`)

Prompted by a direct question ("is there a solution to the CEF build race
issue?") after this retro had sat unresolved, Option A (parallelism cap)
was finally tested properly — and every variant of it failed identically,
which is what led to actually finding the real cause instead of continuing
to tune around it.

**Testing Option A properly, and disproving it:**

1. `NUM_JOBS=8 <task invocation>` — no effect; log still showed `--parallel
   32`. Traced to the `cmake` crate's own logic: `NUM_JOBS` in this context
   is an *output* Cargo sets for build scripts (derived from Cargo's own
   `-j`/parallelism), not a free-form input — a shell-level `NUM_JOBS=8`
   gets clobbered by Cargo's own injected value before the build script
   ever reads it.
2. The correct lever is `-j 8` passed directly to `cargo build`, or the
   `CARGO_BUILD_JOBS=8` input env var (which cargo itself reads to decide
   its own `-j`). Verified both correctly produced `--parallel 8` in the
   cmake invocation.
3. Ran a full `agentmux-cef` release build with `--parallel 8` confirmed
   in the log — **identical C1083 failure**, same file batch. Parallelism
   was not the cause; the retro's whole root-cause section was wrong.
4. Deleted the `cef-dll-sys-*` build directory entirely and rebuilt from
   scratch (ruling out corrupted/stale incremental ninja state from the
   many earlier interrupted attempts) — **identical failure** on the fresh
   directory too.
5. Noticed the error text itself — `Cannot open compiler generated file:
   ''` (an empty filename) — and checked `TMP`/`TEMP`: this Bash/MSYS
   shell sets `TMP=/tmp`, `TEMP=/tmp` (POSIX-style), while `cmd.exe`'s
   native environment correctly shows `C:\Users\asafe\AppData\Local\Temp`.
   Explicitly set correct Windows-style `TMP`/`TEMP` before the build —
   **still identical failure.** (This may still be worth fixing separately
   for hygiene, but it isn't the cause here.)
6. Measured the actual path length of one of the specific failing object
   files: the `.cc` source path was 236 characters; the corresponding
   `.obj` output path was **269 characters** — over Windows' classic
   260-character `MAX_PATH` limit, *despite* `CMAKE_OBJECT_PATH_MAX=500`
   already being set in the cmake invocation (evidently insufficient to
   keep every path component under the limit on this particular directory
   depth). This pattern fits perfectly: every failing file across every
   attempt in this whole retro was from the small set of *longest-named*
   files in `libcef_dll/{cpptoc,ctocpp}/test/`
   (`api_version_test_scoped_client_child_v2_cpptoc.cc`,
   `translator_test_ref_ptr_client_child_cpptoc.cc`, etc.) — never the
   shorter-named files in the same directory.
7. Checked the registry: `HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem
   \LongPathsEnabled` → **`0`** (disabled) on this machine. Enabling it
   requires admin/elevated access — not attempted without explicit
   authorization, per this session's operating guidelines around
   system-level changes.
8. Instead tried a *non-invasive*, non-admin workaround: `subst K:
   "<repo path>"` — mapping a plain drive letter to the deeply-nested
   workspace path, which Windows then treats as a short `K:\...` path for
   everything under it, with zero admin rights needed and instantly
   reversible (`subst K: /D`).
9. Rebuilt `agentmux-cef` release from `K:\` (same source, same
   `--parallel 8`, same corrected `TMP`/`TEMP`, fresh build directory) —
   **exit 0, zero `C1083` occurrences, finished cleanly in ~2 minutes.**
   Confirmed the fix.

**Why this matches every earlier observation in this retro:** the
"contention" theory's evidence (dozens of files failing per run, always
the same file set, seemingly worse under heavier load) is equally
consistent with a fixed, deterministic path-length threshold — the same
handful of longest-named files fail *every single time*, at any
parallelism level, on any machine load, simply because their absolute
path is always over 260 characters on this specific workspace layout.
"Machine load" was never actually the variable; it was coincidental
correlation from testing during a busy session.

## Solution (verified, actionable today)

**Immediate, no-admin-required:** before running `task dev` / `task
package` on a machine where the repo lives at a long/deeply-nested path,
map a short drive-letter alias and build from there:

```powershell
subst K: "C:\path\to\this\repo"
```

then `cd K:\` and run the normal `task dev` / `task package` commands from
that drive. Undo any time with `subst K: /D`.

**Durable fix, requires admin:** enable Windows long-path support once,
system-wide, and this stops being necessary:

```powershell
# Run as Administrator
New-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem" `
  -Name "LongPathsEnabled" -Value 1 -PropertyType DWord -Force
```

(A reboot is not required for most tools, but some processes may need to
restart to pick up the new registry value.) Not applied during this
session — flagged for the user/machine owner to decide, since it's a
system-wide change outside a single build's scope.

**Root fix for the repo itself (not attempted here):** the workspace path
that triggered this is specific to how this particular agent's working
directory was provisioned (a duplicated-looking path segment,
`agenty-0629j\Usersasafe.agentmuxagentsagenty-0629jagentmux`) — shortening
that provisioning convention, or having `cef-dll-sys`'s fork request even
shorter object-file names/hashes (Option B from the now-superseded
mitigation section above would coincidentally have helped here too, by
shrinking path length rather than parallelism), would reduce how easily
any deeply-nested checkout hits this limit in the first place.

## Open questions

1. Does capping parallelism (Option A) fully eliminate the race, or just
   reduce its frequency? Not yet tested.
2. Is `mspdbsrv.exe`'s synchronous PDB-write path actually the bottleneck, or
   is this a Windows handle-table/temp-file exhaustion issue unrelated to
   PDB writing specifically? The error text ("compiler generated file: ''")
   is generic enough that either is plausible; no deeper MSVC-internals
   instrumentation was done here.
3. Would splitting `libcef_dll_wrapper`'s `test/` subdirectory compile into
   a smaller-batch/non-`/FS` shared-PDB config reduce contention
   independent of `-j`? Not investigated — out of scope for an in-session
   diagnosis.

## Timeline (2026-07-14, this dev machine, exact clock times not captured)

| Order | Event |
|-------|-------|
| 1 | `task dev` (via `scripts/dev-agent.cmd`) run in background for manual smoke-testing the tab-reorder fix (PR #2148). Failed: `ninja: build stopped: subcommand failed`, C1083, panic in `cmake-0.1.58`. |
| 2 | Retried `task dev` a second time, output redirected to a file for full visibility. Failed identically; the Taskfile's own `\|\| repair-cef-extract.sh && cargo build ...` fallback fired automatically and reproduced the same failure on its retry too (50 distinct C1083 occurrences logged across the pair of attempts). |
| 3 | User chose to hold off on further `task dev` retries; PR #2148 merged and released as v0.53.5 without a live in-app smoke test. |
| 4 | Later, asked to pull latest `main` and produce a fresh portable. `task package` hit the identical C1083 signature a third time, same `libcef_dll/*/test/*.cc` batch. |
| 5 | Retro written instead of a fourth blind retry, per explicit direction ("lets get a retro written to file..we should be clear for this stuff"). Initial draft incorrectly attributed the contention to "12 other AgentMux instances," based on an `agentmux-0.53*` process-name grep that actually counted one instance's own CEF multi-process fan-out (launcher + srv + host + 9 sub-processes). Corrected after being challenged — confirmed via `Win32_Process` parent-PID inspection that only one real instance was running. |
| 6 | Unrelated bashwrap pager-hang investigation and fix (PR #2156) completed and merged in between — see `RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md`. |
| 7 | Asked directly "is there a solution to the CEF build race issue?" — prompted actually testing the retro's own long-deferred Option A (parallelism cap) instead of continuing to describe it as untested. |
| 8 | `NUM_JOBS=8` had no effect (still `--parallel 32` in the log) — traced to `NUM_JOBS` being a Cargo-owned output env var for build scripts, not a usable input; a shell-set value gets overwritten. `-j 8` / `CARGO_BUILD_JOBS=8` correctly produced `--parallel 8` instead. |
| 9 | Full build with confirmed `--parallel 8` — identical C1083 failure. Fresh `cef-dll-sys` build directory (ruling out stale ninja state) — identical failure. Corrected `TMP`/`TEMP` (Bash's `/tmp` vs. Windows' real temp path) — identical failure. Parallelism theory fully disproven. |
| 10 | Measured the actual failing `.obj` path length: 269 characters, over Windows' 260-char `MAX_PATH`, despite `CMAKE_OBJECT_PATH_MAX=500` already set. Checked the registry: `LongPathsEnabled` = `0`. This matched every prior observation in the retro (same fixed set of longest-named files failing, every time, regardless of load). |
| 11 | Verified with a non-admin workaround: `subst K: "<repo path>"`, then rebuilt `agentmux-cef` release from `K:\` — exit 0, zero C1083 occurrences, ~2 minutes. Root cause confirmed. Retro corrected in place; a full `task package` from `K:\` kicked off to actually produce the originally-requested portable. |
| 12 | `task package` completed successfully from `K:\` — zero C1083 occurrences across the entire build (frontend + all Rust binaries including the full CEF host). Produced `agentmux-0.53.5+g346ace1c.20260714T234617.453541-x64-portable` (402 MB directory, 179 MB ZIP) on the Desktop. This was the original ask from the start of this session ("lets get a fresh portable"), finally delivered after this whole investigation. |
