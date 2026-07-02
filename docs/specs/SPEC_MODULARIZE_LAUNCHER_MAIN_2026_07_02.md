# Spec: Modularize `agentmux-launcher/src/main.rs`

**Date:** 2026-07-02
**File:** `agentmux-launcher/src/main.rs` (2,264 lines)
**Type:** Pure reorganization — zero logic changes, zero behavior changes
**Tier:** Large — **HIGHEST RISK of the four** (touches isolation invariants I1–I6)

---

## Current state

- **2,264 lines, 0 inline tests, 21 top-level functions + 1 enum**
- ~37 platform `#[cfg]` blocks (windows / macos / linux / not(windows) / not(macos) / any(...))
- 15 sibling modules already exist (`config`, `data_dir`, `diag`, `event_log`, `hash`, `host_pipe`, `ipc/`, `mem_supervisor`, `reducer`, `saga`, `splash*`, `srv_spawner`, `state`, `wrr`) — this split ADDS to that pattern, moving logic out of `main.rs`.

## Isolation invariants — MANDATORY reviewer gate (per CLAUDE.md I1–I6)

Any moved function touching these must be reviewed to confirm the move changes nothing:

| Invariant | Functions | APIs |
|-----------|-----------|------|
| I1 pipe uniqueness | `hash::data_dir_hash16`, `ipc::pipe_name`/`srv_pipe_name` | hash → namespace key |
| I2 no global lifecycle handles | `create_job_object` (unnamed job), `srv_spawner::assign_pid_to_job` | `CreateJobObjectW`, `AssignProcessToJobObject` |
| I3 bounded blast radius | `spawn_host_supervised`, job drop/cleanup | `KILL_ON_JOB_CLOSE` |
| I4 forward-only contact | `forward_open_new_window` | HTTP POST to existing host (side-effect-free) |
| I5 keyed shared OS objects | `pipe_name(&dir_hash)`, `splash::spawn_splash(&dir_hash)` | pipe/event names embed `dir_hash` |
| I6 data isolation | `paths.data_dir` (channel+version keyed) | env forwarding |

**Because this is a PURE MOVE, the invariants are preserved by construction — no naming, no handle lifetime, no forwarding logic changes. The spec requires the PR description to explicitly state "no `CreateJobObjectW`/`AssignProcessToJobObject`/`OpenProcess`/`TerminateProcess`/pipe-naming logic was modified, only relocated" so reagent's I1–I6 gate can confirm at a glance.**

## Must stay in `main.rs`

- `fn main()` — entry point + runtime construction
- `suppress_os_crash_dialogs()` — must run before runtime
- `async fn launcher_main()` — top-level orchestrator (may shrink, but stays at crate root as the flow spine)

## Proposed new sibling modules

```
src/
├── main.rs              (~150 lines: main() + launcher_main() orchestration + mod decls)
├── job_object.rs        (JobHandle, create_job_object — Windows; I2/I3)
├── host_spawn.rs        (spawn_host_supervised + resume_main_thread [win], spawn_host_unix + terminate_child_gracefully [unix]; I2/I3)
├── second_instance.rs   (ForwardError, forward_open_new_window[_or_log], bind_socket_with_recovery [unix]; I1/I4/I5)
├── supervisor/
│   ├── mod.rs
│   ├── windows.rs       (run_windows loop — ~600 lines)
│   └── unix.rs          (run_unix loop + next_signal — ~550 lines)
├── binary_resolution.rs (find_cef_binary)
└── logging.rs           (log, dirs_fallback_home)
```

## Execution notes

- Do this in **one PR** but keep the diff a clean move: each extracted function keeps its exact body + `#[cfg]` guards; `main.rs` gains `mod` decls and the call sites become `job_object::create_job_object(...)`, `supervisor::run_windows(...)`, etc.
- Extracted functions that `main.rs`/`launcher_main` call must be `pub(crate)`.
- Preserve every platform guard exactly. Windows and Unix supervisor loops go to separate files under `supervisor/`, each fully `#[cfg]`-gated.
- No `#![allow(unused_imports)]`; trim imports per file via `cargo check`.
- **Suggest splitting into two PRs if the diff is unreviewable:** PR-A = the low-invariant extractions (`binary_resolution`, `logging`, `host_spawn` non-job parts, `supervisor`); PR-B = the I1–I6-touching bits (`job_object`, `second_instance`). This lets reagent's invariant gate focus on a small surface. Author's call based on diff size.

## Verification gate

- `cargo check -p agentmux-launcher` clean on Windows, zero new warnings
- Manual re-read of every I1–I6 touchpoint confirming byte-identical logic
- CI ubuntu run covers the Unix supervisor/socket paths
- reagent review with explicit I1–I6 confirmation in the PR body

## Risk: **High** (invariant-sensitive). Mitigation: pure move, explicit invariant statement in PR body, optional two-PR split isolating the I1–I6 surface. No logic edits under any circumstances — if a move seems to *require* a logic tweak, STOP and leave that function in `main.rs`.
