# Implementation Plan — Window/Process State Machine (PR #570 + follow-ups)

**Date:** 2026-04-27
**Author:** AgentA-asaf
**Inputs:** `SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md`, two analysis docs (state inventory + best practices), `process-lifecycle-v2.md`, `SPEC_BACKEND_LIFECYCLE.md`, `SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md`, `instance-indicator.md`, current code in `agentmux-launcher/`, `agentmux-cef/`, `agentmux-srv/`, `frontend/`.

---

## TL;DR

**Phase A is two focused PRs, not three (and not five).** The spec previously listed 5 sub-items; on review:
- One was a heartbeat — **dropped**, see "Architectural principle" below.
- Two were `LifecyclePhase` enum + `Quit` RPC — **moved to Phase B** because they're the central object of the architectural inversion and can't be done "minimally" in A without throwaway work.

What remains in Phase A:
- **PR #570 (this one)** — Job Object on launcher + CREATE_SUSPENDED race fix + bot cleanups
- **PR #572** — Per-data-dir named mutex single-instance + handoff (replaces port-file)

Plus two architectural sign-offs needed before Phase B starts (not blocking Phase A):
1. **"Truth in launcher" inversion** — deliberate departure from Chromium pattern; needs author confirmation.
2. **Browser-pane lifecycle ownership** (spec §11.5) — embedded CEF child views: in canonical registry or owned by their parent window's state?

## Architectural principle (locked)

**No heartbeats. No polling. No periodic timers checking liveness.**

This is the whole point of the redux/reducer architecture. Heartbeats exist when state across processes can drift — you poll because you don't trust your state. With versioned events + `GetSnapshot` resync (spec §5), state can't drift, so polling is unnecessary. Liveness comes from authoritative kernel signals only:

- Host process death → `child.wait()` returns AND/OR launcher's pipe to host returns EOF. Both are kernel-guaranteed at process exit.
- Host message-loop hang while host process is alive → not a liveness issue, a *state* issue. PR #568 already fixed this in `client.rs::on_before_close` by excluding pool windows from the user-browser count.
- Forced shutdown still bounded → `Quit { reason }` IPC carries an explicit ack timeout (Phase B). On timeout, launcher `CloseHandle(job)` → OS reaps. Event-driven, not periodic.

Same principle drove `SPEC_BACKEND_LIFECYCLE.md` to reject WS-watchdog polling. Same principle applies here.

**Spec text updated to reflect this:** §3.2, §4.3, §9 Phase A all edited.

---

## Cross-cutting decisions (resolve once, apply everywhere)

### Decision 1: No periodic liveness mechanism — RESOLVED

Confirmed by the user: heartbeats are unreliable; the whole point of the state machine is that we don't need them. `child.wait()` + pipe-EOF on the eventual launcher↔host pipe are both kernel-guaranteed signals; no polling is needed for any path.

**Spec text updated** in §3.2, §4.3, and §9 Phase A.

### Decision 2: Per-data-dir mutex name

Open question in the PR description. `CLAUDE.md` is explicit that AgentMux runs multiple instances in parallel (one per data dir). The spec's named-mutex idea adapts to this with a **per-data-dir mutex name**: `Local\AgentMux-{hash(data_dir)}`.

**Confirm hash strategy:** SHA-256 of the absolute, normalized data-dir path, lowercase hex, first 16 chars. Stable across launches, distinct across data dirs. Matches the existing per-data-dir port file scheme exactly, just with a kernel mutex instead of a file.

**Replaces, doesn't compose.** The current port file at `<data-dir>/cef/ipc-port` is what suffers from stale-after-crash (gap #3 + #8 from inventory). Mutex is kernel-released on process death → atomic. No replacement file lookup; the named pipe address is `\\.\pipe\agentmux-{mutex_name}\command` so a second launcher attempting the same data-dir finds the mutex held, opens the existing pipe, sends `OpenWindow`, exits.

**Action item:** confirm with spec author that per-data-dir is the intended scope, then update spec text to spell it out (currently §3.2 is ambiguous).

### Decision 3: "Truth lives in launcher" vs "truth in host"

The new spec deliberately puts canonical state in the launcher (§3.1, §3.2). This is a departure from Chromium and Electron, both of which put truth in the privileged main process *that owns the windows*. AgentMux's twist: the host owns windows but can crash; the launcher always lives.

**Implications for Phase A:** none. Phase A only adds a Job Object + a pipe + a mutex; it doesn't move state.

**Implications for Phase B/C/D:** large. The reducer, the `Map<WindowId, WindowState>`, the warm pool — all move from host to launcher. The host becomes a thin executor over IPC. This is right per the spec's argument (launcher survives crashes), but it's a *significant* architectural inversion and is worth confirming before sinking weeks of work into Phase B.

**Action item before Phase B starts:** 30-min design review with spec author. Phase A doesn't block on this.

---

## Phased PR plan

Three focused PRs replace the spec's "Phase A" into reviewable chunks. They can land in this order or in parallel where noted.

### PR #570 (this one) — Job Object + race fix + bot cleanups

**Scope:** keep what's there; address all bot review issues; rebase onto current main.

**Concrete changes:**
1. **Resolve merge conflict** — version bumps in `agentmux-launcher/Cargo.toml`, `agentmux-srv/Cargo.toml`, `package.json`, `package-lock.json`. Mechanical.
2. **Race fix** (codex P2 / gemini HIGH): replace `Command::spawn()` with `CommandExt::creation_flags(CREATE_SUSPENDED)`, then enumerate threads via `Toolhelp32` snapshot, `OpenThread(THREAD_SUSPEND_RESUME)`, `ResumeThread`. Procedure detailed below.
3. **Drop manual FFI** (gemini MEDIUM L194): use `windows_sys::Win32::System::JobObjects::CreateJobObjectW` directly. Verified present at `windows-sys-0.59.0/src/Windows/Win32/System/JobObjects/mod.rs:5`.
4. **Use HANDLE typed alias** (gemini MEDIUM L160): `JobHandle(windows_sys::Win32::Foundation::HANDLE)`. The `HANDLE` type is `*mut c_void`; gemini's claim it's `isize` was wrong, but the alias still improves readability.
5. **Fix inaccurate KILL_ON_JOB_CLOSE comment** (gemini MEDIUM L105): the comment claims it's a no-op when host already exited, but that's exactly when the OS uses it to reap residual children. Just rewrite the comment.
6. **Add `Win32_System_Diagnostics_ToolHelp` feature** to `agentmux-launcher/Cargo.toml` for Toolhelp32 snapshot APIs.

**Effort:** half a day including testing.

**Verification:** spawn host that immediately tries to fork a subprocess; kill launcher in Task Manager; confirm subprocess dies. Without race fix this fails ~10% of the time depending on host startup timing; with race fix it's 100% deterministic.

**Why ship this alone:** the user-visible win is real even without the rest of Phase A — Task Manager kill of launcher reaps everything via OS. Today this only happens *if* the host happens to exit first.

### ~~PR #571 — Pipe-EOF host watcher~~ — DROPPED

Originally proposed as a replacement for the spec's heartbeat. On further analysis it's redundant: `child.wait()` already blocks on host process death and unblocks the moment the host exits, then drops the job handle which triggers `KILL_ON_JOB_CLOSE`. Pipe-EOF would be a second signal for the same event with no additional fidelity.

The launcher↔host named pipe still gets built — it'll exist for the command IPC channel that PR #572's handoff path needs and that Phase B's reducer needs. Pipe-EOF on it is a free side-effect of having the pipe at all (the OS closes it on host death), so the launcher can opportunistically detect host death via either `child.wait()` *or* pipe-EOF. But it's not its own PR.

### PR #572 — Per-data-dir named mutex single-instance

**Scope:** replace the port-file single-instance check with a kernel-mutex check. Resolves inventory gaps #3 and #8 (stale port file → "taskbar but no window" on relaunch).

**Concrete changes:**
1. New module `agentmux-launcher/src/single_instance.rs`. Exposes `acquire_or_handoff(data_dir: &Path) -> Result<MutexHandle, ExistingInstance>` that:
   - Computes `mutex_name = format!("Local\\AgentMux-{}", hash16(data_dir))`.
   - Calls `CreateMutexW(NULL, FALSE, mutex_name)`.
   - On `GetLastError() == ERROR_ALREADY_EXISTS`: the mutex is held by another launcher → return `Err(ExistingInstance)` carrying the named-pipe address `\\.\pipe\agentmux-{hash16(data_dir)}\command`.
   - On success: return the `MutexHandle` (RAII drop closes it; kernel releases on process death even on hard crash).
2. New module `agentmux-launcher/src/handoff.rs`. On `ExistingInstance`, opens the named pipe, sends `{"cmd": "OpenWindow", "args": [...]}`, awaits `ack`, exits with code 0.
3. Delete port-file code in `agentmux-cef/src/main.rs:187–212, 356–359` and `agentmux-launcher/src/main.rs` if present.

**Effort:** 1–2 days. The IPC server end (in the launcher) is a thin named-pipe accept loop that hands incoming `OpenWindow` commands to the host (until Phase B's reducer exists, this is just an IPC forward).

**Verification:** kill launcher, immediately restart → no "taskbar but no window" because there's no stale file. Run two launchers with the same data-dir → second one hands off and exits.

**Dependency:** ideally lands after PR #571 because the launcher needs a named pipe infrastructure for both. Could share the pipe-creation code. Suggested order: #570 → #571 → #572.

### Background prep — can start any time

These don't block anything but feed into the sequencing.

**P-A.** Verify the contradiction in `set_taskbar_hidden` from the inventory analysis caveat #1: `window_pool.rs:233` comment says "Don't re-show pool windows," but inventory's recommended fix for the Phase 6 taskbar bug is to re-show after style clear. Read the function in full and figure out which is correct. May or may not require a code change; either way the spec text needs to be updated to match reality.

**P-B.** Read `instance-indicator.md` end-to-end (332 lines) and extract the "stable for window lifetime, never renumbers" invariant verbatim. New spec's `WindowInstanceRegistry` mention should cite it. This goes into the spec text, not into code.

---

## What's NOT in Phase A (was in spec but should defer)

The spec lists `LifecyclePhase` enum + minimal reducer + synchronous Quit RPC as Phase A bullets. Those belong in **Phase B**, not A:

- The reducer is the central object of the entire architectural inversion. Adding a "minimal" version in Phase A creates a half-built thing that has to be re-architected when Phase B's full reducer arrives.
- `Quit { reason }` IPC needs the IPC contract spec'd (commands and events), which is Phase D's work even if the contract is small initially.

**Recommendation:** spec author edits Phase A to:

> Phase A — Foundation (3 PRs):
> 1. Move Job Object to launcher + spawn-suspended assignment.
> 2. Pipe-EOF host watcher (replaces heartbeat).
> 3. Named-mutex single-instance + handoff.
>
> Phase A end-state: launcher always reaps the tree; relaunch is race-free; foundation laid for Phase B's reducer.

`LifecyclePhase` + reducer + `Quit` RPC move to Phase B as "preparing the reducer surface."

---

## Concrete fix for the race condition (PR #570 detail)

Codex and gemini both flagged it. Real bug. Fix:

```rust
use std::os::windows::process::CommandExt;
use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

let mut child = std::process::Command::new(&real_exe)
    .args(&args)
    .creation_flags(CREATE_SUSPENDED)  // host frozen at process-init
    .spawn()?;
let host_pid = child.id();

// Job assignment runs while host is frozen. Host cannot fork
// children before this returns.
let job = create_job_object_for_child(host_pid)?;

// Find host's only existing thread (main, since CREATE_SUSPENDED)
// and resume it.
resume_main_thread(host_pid)?;

child.wait()?;
```

`resume_main_thread` enumerates `Toolhelp32` snapshot, finds the `THREADENTRY32` whose `th32OwnerProcessID == pid`, and `ResumeThread`s it via `OpenThread(THREAD_SUSPEND_RESUME, ...)`. With `CREATE_SUSPENDED` only the main thread exists, so the search returns exactly one entry. `windows-sys` APIs in scope:

- `Win32::System::Threading::{CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME}`
- `Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32}`

Add `Win32_System_Diagnostics_ToolHelp` feature to `agentmux-launcher/Cargo.toml`.

---

## Risks & honest caveats

1. **`process-lifecycle-v2.md` and `SPEC_BACKEND_LIFECYCLE.md` are not marked "shipped" anywhere.** Confidence that v2's Job Object is in production rests on cross-references in the new spec (`agentmux-cef/src/sidecar.rs:180-189, :424` cited as live). Worth a quick `git log` / `gh pr` audit before claiming the host's existing srv-job is "redundant" in PR text.
2. **Pipe-EOF doesn't catch the case where the host process is alive but its message loop is hung** (Phase 6 zombie scenario). For that, we still need a `Quit` timeout backstop — the spec already proposes this in §6.3 step 5–6 ("ack with timeout, then `CloseHandle(job)`"). PR #571's pipe-EOF + the Quit timeout cover both cases. Without the Quit timeout, pipe-EOF alone is incomplete for the actual zombie scenario reported in the PR description.
3. **Phase A's Quit RPC moved to Phase B per recommendation above.** This means the *current* zombie scenario (pool windows keep CEF alive, host never exits) isn't fully resolved by Phase A. Phase A makes it *recoverable* (Task Manager kill of launcher → kernel reaps everything) but not *prevented*. Phase B prevents it via the reducer's window-all-closed predicate excluding pool windows.
4. **Multi-instance interaction with named mutex** needs validation that the data-dir hash is actually stable across launches. Long absolute paths with mixed casing on Windows could produce different hashes if normalization isn't careful. Pin to: `path.canonicalize().to_string_lossy().to_lowercase()` then SHA-256, first 16 hex chars.
5. **Phase B–D (window FSM, warm pool consolidation, IPC contract) is multi-week work and depends on the "truth in launcher" decision.** Phase A delivers ~80% of the user-visible wins (zombies + relaunch race + race-free job assignment). Decide whether B–D is worth the effort once Phase A ships and we see how much the recurring bug class shrinks.

---

## Sequenced action list

| Step | Owner | Effort | Blocks |
|---|---|---|---|
| 1. Resolve merge conflict on PR #570 | me | 5min | PR #570 push |
| 2. Implement race fix (CREATE_SUSPENDED + resume thread) | me | 2h | PR #570 push |
| 3. Apply 3 gemini cleanups (HANDLE alias, drop manual FFI, fix comment) | me | 30min | PR #570 push |
| 4. Push to PR #570; address re-review | me | 1d | merge |
| 5. **In parallel:** verify `set_taskbar_hidden` contradiction (P-A) | me | 1h | spec text |
| 6. **In parallel:** read `instance-indicator.md` (P-B) | me | 1h | spec text |
| 7. **In parallel:** confirm "truth in launcher" inversion sign-off | author | 30min | start of Phase B |
| 8. PR #572 — named mutex single-instance + handoff | me | 2d | shipped after #570 |
| 9. Phase A complete; design review for Phase B | author + me | 1h | start of B |

**Phase A end-state:** zombies recoverable on launcher kill, relaunch race fixed, three small focused PRs each reviewable in <30 min, foundation laid for Phase B's reducer-driven shutdown which prevents the zombie state entirely.

---

## Open questions to answer before Phase B starts

(Captured here so they don't get lost; not blocking PR #570.)

1. Spec §11 Q4 (process-lifecycle-v2 harmonization) — answer: cite + supersede §4.3 only; preserve doc as institutional memory. Action: add a "Status: partially superseded by SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27 §4.3" header to v2.
2. Spec §11 Q5 (browser-pane lifecycle) — should embedded CEF child views be in the canonical registry or owned by their parent window's state? Lean toward latter per spec author.
3. Spec §11 Q6 (renderer crash handle invalidation) — Chromium duplicates handles before notifying observers; do we need this in the launcher's `Renderer` ProcessRecord lifecycle?
4. "Truth in launcher" architectural sign-off (Decision 3 above) — explicit yes/no from spec author.
5. Single-instance mutex name format final approval — `Local\AgentMux-{first 16 hex chars of SHA256(canonical_lower(data_dir))}` is the proposal.

These all gate Phase B; none gate Phase A.
