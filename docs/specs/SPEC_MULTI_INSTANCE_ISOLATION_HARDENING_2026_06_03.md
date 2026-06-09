# SPEC: Multi-Instance Isolation Hardening & Crash-Safety Verification

**Date:** 2026-06-03
**Status:** Draft
**Author:** AgentX
**Related:** [[SPEC_VERSION_ISOLATION_2026_06_01]], `docs/specs/single-instance-new-window.md`, `agentmux/CLAUDE.md` § "Multiple Instances Run in Parallel"

---

## 1. Motivation

On 2026-06-03 a freshly-built `v0.42.0` portable was launched on a host that was
already running `v0.41.0` (the instance hosting an interactive agent pane). Shortly
after, the running instance disappeared / the session died. The human's reasonable
read was *"launching the new build killed the instance it was running in."*

AgentMux's **stated design** (`CLAUDE.md`) is the opposite:

> "AgentMux is designed to run multiple instances simultaneously — different
> versions, dev + portable, or multiple portable copies. Each instance is fully
> isolated… You can test v0.33.14 while v0.33.13 is still running."

So either (a) a real isolation regression exists, or (b) the crash was unrelated and
the design held. A code audit (§3) shows the isolation design is **sound** and the
launch almost certainly did **not** kill the host. **But we cannot prove it** — there
is no automated test asserting "launch instance B ⇒ instance A survives," and no
runtime telemetry that would detect a real violation. This spec closes that gap:
**ratify the isolation invariants, enforce them with tests, and add observability so
a future violation is caught immediately instead of inferred after a crash.**

This is a *confidence / safety-net* spec, not a bug fix for a located defect. The
separate CEF packaging bug (the actual reason `v0.42.0` showed splash-then-nothing)
is covered in [[SPEC_WINDOWS_CEF_BUNDLE_VERSION_INTEGRITY_2026_06_03]].

---

## 2. Goals / Non-Goals

**Goals**
- G1. Document the isolation invariants as a single normative list (the contract).
- G2. Add automated tests that fail if any invariant is violated — especially a
  *survival* test: spawning/killing instance B never affects instance A.
- G3. Add lightweight startup telemetry that records the exact resources an instance
  claims (pipe name, data dir, channel, version, job handle id) so a cross-instance
  collision is diagnosable from logs alone.
- G4. Audit and eliminate the one *shared* (non-per-instance-keyed) resource found
  (splash window class) or prove it is harmless under contention.

**Non-Goals**
- Changing the data-dir / channel layout (owned by [[SPEC_VERSION_ISOLATION_2026_06_01]]).
- macOS/Linux launcher parity (tracked elsewhere; invariants below are Windows-first
  because that's where the launcher owns lifecycle today).
- Fixing the CEF version mismatch (separate spec).

---

## 3. Current behavior (audit, with evidence)

All paths in `agentmux-launcher/` unless noted. Verified 2026-06-03 at HEAD
`3012bfc5`.

### 3.1 Single-instance arbitration — **SAFE**
- Pipe name: `\\.\pipe\agentmux-{hash16}\command` — `src/ipc/mod.rs:37-46`.
- `hash16 = fnv1a_64( lower(canonical(data_dir)) + "\x00" + version )` —
  `src/hash.rs:54-60`. **Version is part of the key**, so two versions sharing a
  channel data dir get distinct pipes (unit test `hash.rs:92-97`).
- First instance wins via `ServerOptions::first_pipe_instance(true)` —
  `src/ipc/server.rs:83-101`. A second launcher gets `ERROR_ACCESS_DENIED (5)`,
  then **forwards** an `open_new_window` command to the running host over its
  authenticated localhost IPC and `exit(0)` — `src/main.rs:752-793`,
  `forward_open_new_window` `src/main.rs:1348-1404`. It never touches the running
  instance's processes, pipes, or job. **No kill path.**

### 3.2 Windows Job Object — **SAFE**
- Created **unnamed**: `CreateJobObjectW(null, null)` with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` — `src/main.rs:1230-1256`. An unnamed job
  has no global name, so **no other process can open a handle to it**.
- Children assigned by PID+HANDLE the launcher itself owns —
  `srv_spawner.rs:356-380`, host at `src/main.rs:353-365`. The kernel refuses to
  reassign an already-jobbed process to a different job, so launcher B can never
  capture launcher A's host/srv.
- On host-spawn failure the launcher `drop(job)`s, which `KILL_ON_JOB_CLOSE` reaps
  **only its own** srv/host, then `exit(1)` — `src/main.rs:1047-1071`. A failed
  startup of B cannot reap A's processes.

### 3.3 Splash — mostly safe, one shared resource
- Splash dismiss **event** is per-instance: `AgentMuxSplash-{dir_hash}` —
  `src/splash.rs:80-102`. SAFE.
- Splash **window class** is a hard-coded global string `"AgentMuxSplash"` —
  `src/splash.rs:104-123`. `RegisterClassExW` collisions are *tolerated*
  (`ERROR_CLASS_ALREADY_EXISTS` ignored, line ~122). This is the only non-keyed
  shared resource in the launcher. Risk is low (class registration is per-HINSTANCE
  and tolerated) but it is **unaudited under true concurrent registration** → G4.

### 3.4 Data dir / channel — **ISOLATED** (see [[SPEC_VERSION_ISOLATION_2026_06_01]])
- Root: `AGENTMUX_HOME_OVERRIDE` else `~/.agentmux` —
  `agentmux-common/src/data_paths.rs:380-388`.
- Layout is **version-scoped**: `channels/<channel>/versions/<version>/{data,logs,
  cef-cache,runtime}` — `data_paths.rs:200-210`.
- Channel baked at build time via `option_env!("AGENTMUX_BUILD_CHANNEL_DEFAULT")`,
  default `"stable"` — `data_paths.rs:54-58`, `agentmux-common/build.rs:23`. Portable
  builds bake `local-<branch>-<hash>` — `scripts/package.sh:85-87`.
- Portable detection is the explicit marker `agentmux-portable.marker` next to the
  exe — `agentmux-common/src/runtime_mode.rs:328-341`; absence falls back to
  `Installed`.

### 3.5 The specific 06-03 case
| | v0.42.0 portable | v0.41.0 running |
|---|---|---|
| channel | `local-main-<hash>` | `stable` |
| data dir | `…/channels/local-main-*/versions/0.42.0/data` | `…/channels/stable/versions/0.41.0/data` |
| pipe hash | `hash(portable-data, "0.42.0")` | `hash(stable-data, "0.41.0")` |

Different channel **and** different version **and** different pipe ⇒ **no shared
resource**. Conclusion: **the launch did not, by code, kill the host.** The crash
cause is **unproven** — most likely the host exited for an unrelated reason near the
same time. (The `v0.42.0` host never even initialized CEF — see the companion spec —
so it never registered any host-side window class / AppUserModelID that could
collide.)

---

## 4. The invariants (normative contract)

Every running AgentMux instance MUST own only resources keyed by its
`(channel, version, data_dir)` triple. Specifically:

- **I1 — Pipe uniqueness.** No two distinct `(data_dir, version)` pairs map to the
  same single-instance pipe. (Already tested at the hash level; extend to the full
  name.)
- **I2 — No global lifecycle handles.** The launcher creates only *unnamed* job
  objects; it never opens a job/process/handle it did not create.
- **I3 — Bounded blast radius.** Any launcher failure path may terminate **only**
  processes in its own job. No code path may `TerminateProcess`/`taskkill`/job-kill a
  PID outside its own job.
- **I4 — Forward-only cross-instance contact.** The *only* permitted interaction with
  another instance is the authenticated `open_new_window` forward; it is
  side-effect-free w.r.t. that instance's lifecycle.
- **I5 — Keyed shared OS objects.** Every named OS object (event, mutex, pipe,
  semaphore) embeds the `dir_hash`. Exceptions must be explicitly justified and
  proven collision-tolerant (today: the splash window class — G4).
- **I6 — Data isolation.** No two instances of different `(channel, version)` write to
  the same data/logs/cef-cache directory.

---

## 5. Proposed work

### 5.1 Tests (G2) — the core deliverable
1. **Survival integration test** (`agentmux-launcher/tests/multi_instance_survival.rs`,
   `#[cfg(windows)]`): start instance A (real launcher, throwaway
   `AGENTMUX_HOME_OVERRIDE` + distinct baked channel via env), wait until its pipe +
   `ipc-port` exist; start instance B with a *different* override/channel; assert
   B comes up (or fails on its own) and **A's pipe + host PID are still alive**; then
   kill B and re-assert A is alive. This is the test that would have directly
   refuted/confirmed the 06-03 theory.
2. **Pipe-name uniqueness property test**: extend `hash.rs` tests to assert the full
   `pipe_name()` differs across a matrix of `{same dir × diff version}`,
   `{diff dir × same version}`, `{portable vs stable channel dirs}`.
3. **No-foreign-handle audit test**: a `grep`-style unit/CI check (or `#[test]` that
   scans source) asserting `OpenProcess`/`OpenJobObjectW`/`AssignProcessToJobObject`
   call sites only ever use PIDs the launcher spawned (allow-list by function). Cheap
   guard against I2/I3 regressions.
4. **Degraded-mode multi-instance test**: job-absent path (`job.is_none()`) with two
   instances — assert the explicit `start_kill` only targets own srv.

### 5.2 Telemetry (G3)
- At launcher startup, emit one structured log line to `agentmux-launcher.log`:
  `instance_claim{ pid, version, channel, data_dir, dir_hash, pipe_name, job=unnamed }`.
- On the second-instance forward path, log
  `forward_open_new_window{ target_pipe, target_port, result }`.
- Rationale: a real cross-instance collision becomes greppable
  (two PIDs claiming the same `dir_hash`) instead of inferred from a vanished window.
  Directly addresses "we couldn't prove what killed it."

### 5.3 Splash window class (G4)
- Either key the class name per `dir_hash` (`AgentMuxSplash-{hash}`) for full
  consistency with I5, **or** add a code comment + a concurrency test proving
  tolerated re-registration is harmless, and record the exception in §4/I5.
- Preference: **key it** (cheap, removes the lone exception).

### 5.4 Documentation
- Add a short "Isolation Invariants (I1–I6)" section to `agentmux/CLAUDE.md` linking
  here, so future launcher changes are reviewed against the contract (reagent gate
  candidate: any diff touching `CreateJobObjectW`, `AssignProcessToJobObject`,
  `OpenProcess`, `TerminateProcess`, or pipe/event naming must reference this spec).

---

## 6. Testing & acceptance

- All new tests pass on Windows CI.
- The survival test demonstrably fails if I3 is intentionally broken (e.g. temporarily
  make launcher B kill by image name) — proving the test has teeth.
- A manual run of two real portables (different channels) confirms the telemetry lines
  show distinct `dir_hash`es.

## 7. Rollout

1. Land telemetry (5.2) first — zero-risk, immediately useful for any recurrence.
2. Land tests (5.1) + splash keying (5.3).
3. Land docs + reagent-gate note (5.4).
Single PR per item, changesets per `CLAUDE.md` (no version bump in feature PRs).

## 8. Open questions

- O1. Was there any *host-side* (agentmux-cef) global object (AppUserModelID, a named
  mutex, a fixed taskbar/jumplist registration) that two instances could contend on?
  The launcher audit is clean; a follow-up host-side audit (out of scope here) should
  confirm. Because the 06-03 `v0.42.0` host died pre-CEF-init, it's not implicated in
  *this* incident, but the audit closes the loop.
- O2. Should `task package` refuse to *launch-test* a portable whose channel/version
  could ever coincide with a running instance? Likely unnecessary given I1–I6, but
  worth a one-line guard if cheap.
- O3. Confirm the unrelated cause of the 06-03 host exit if any artifact survives
  (host log under the stable channel's `versions/0.41.0/logs/` around 06:48 PT).
