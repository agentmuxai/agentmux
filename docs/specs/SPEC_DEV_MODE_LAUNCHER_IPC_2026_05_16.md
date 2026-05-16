# SPEC: Restore Launcher IPC in `task dev` Mode

**Status:** Implemented (PR #882)
**Date:** 2026-05-16
**Author:** AgentA
**Related:** `SPEC_LAUNCHER_DEV_INTEGRATION_2026-05-13.md`, `SPEC_STATUS_BAR_WINDOW_COUNT_2026_05_16.md` (PR #880)

---

## 1. Problem

Two user-visible symptoms in `task dev` builds, both with the same root cause:

### 1.1 Status-bar window count out of sync

PR #880 swapped `StatusBar.tsx:91` from rendering `windowInstanceNumAtom` (per-window ordinal) to `windowCountAtom` (intended: total active windows, identical in every window). Tested in dev with 3 windows open:

- Window 1: no parenthesis (`windowCount = 1`, gated hidden)
- Window 2: `(2)`
- Window 3: `(3)`

All three should read `(3)`. Each window's `windowCountAtom` is stuck at its boot-time snapshot value. `frontend/app/store/launcher-event-reducer.ts:101-105` `project()` is the only writer to `windowCountAtom`, and it's only called when launcher events arrive through the reducer's `dispatch` path. In dev, no launcher events arrive after boot.

### 1.2 Opacity slider missing in dev

The per-window opacity slider in `frontend/app/statusbar/InstancePanel.tsx:414-450` is gated on `entry.windowId` being truthy. `entry.windowId` is populated only by the `BackendWindowIdRegistered` launcher event hitting the frontend reducer at `frontend/app/store/launcher-event/reducer.ts:186-211`. In dev, that event never arrives, so the gate is permanently false and the slider hides for every window.

Both symptoms trace to the same gap: **launcher events stop arriving at the frontend after boot in `task dev`**.

## 2. Investigation

The full pipeline `launcher process → host → renderer JS` (verified against current source):

```
agentmux-launcher (separate exe, spawned by task dev)
  ├─ Owns named-pipe IPC server at `ipc::pipe_name(&dir_hash)`
  │  (agentmux-launcher/src/main.rs:244, 526)
  ├─ Sets `AGENTMUX_LAUNCHER_PIPE=<pipe_path>` on host spawn
  │  (agentmux-launcher/src/main.rs:526) ← confirmed wired
  └─ Reducer emits Event::WindowOpened / Event::BackendWindowIdRegistered
     on Command::ReportXxx (agentmux-launcher/src/reducer/window.rs)

agentmux-cef (host)
  ├─ main.rs:295-299 → CALLS connect_to_launcher() ONLY when
  │  is_dev_build_exe(host_exe_dir) is FALSE
  │  ← THE BUG: skips IPC connection in every dev build
  ├─ launcher_ipc::connect_to_launcher checks AGENTMUX_LAUNCHER_PIPE,
  │  opens pipe, registers reader loop, calls
  │  apply_event_to_shadow → dispatch_to_renderers
  └─ launcher_event_bridge::dispatch_to_renderers fans out
     window.__agentmux_launcher_event(json) to every browser frame
     (agentmux-cef/src/launcher_event_bridge.rs:155-177)

Renderer
  ├─ frontend/util/launcher-events.ts installs window.__agentmux_launcher_event
  └─ Reducer at frontend/app/store/launcher-event/reducer.ts processes
     events, calls project() → updates windowCountAtom, entry.windowId, etc.
```

The break is at `agentmux-cef/src/main.rs:295-299`:

```rust
let _launcher_ipc = if agentmux_common::is_dev_build_exe(&host_exe_dir) {
    None
} else {
    runtime.block_on(launcher_ipc::connect_to_launcher(app_state.clone()))
};
```

`is_dev_build_exe` (definition at `agentmux-common/src/runtime_mode.rs:159`) returns true when the host exe is in `dist/cef-dev/`, `target/debug/`, or `target/release/`. In `task dev`, the host runs from `dist/cef-dev/runtime/agentmux-cef.exe` → `is_dev_build_exe` returns true → connection skipped → events never delivered.

The same stale guard appears at lines 312-316 for the srv IPC pipe (`AGENTMUX_SRV_PIPE_PATH`). Same skip, same consequence for srv events.

### 2.1 Why the guard exists

The comment at lines 289-294 explains:

> Dev-build env-isolation guard: a dev build inheriting `AGENTMUX_LAUNCHER_PIPE` from a parent AgentMux pane it was launched from would connect to the PARENT's launcher pipe and route its host events into the parent's launcher state. Skip the connection in dev mode — `task dev` has no launcher process anyway.

The comment was *correct* before `SPEC_LAUNCHER_DEV_INTEGRATION_2026-05-13.md` (the spec that made `task dev` invoke the launcher on Windows for production-parallel layout). After that spec landed, the second sentence is false: `task dev` *does* run the launcher, and `AGENTMUX_LAUNCHER_PIPE` is set legitimately. The guard now over-fires.

### 2.2 Why the original concern is still valid (sometimes)

The scenario the guard protects against:

1. Developer launches `task dev`. Dev launcher runs, dev host connects.
2. Inside the dev session, they open an agent pane that runs `cargo build && ./target/release/agentmux-cef.exe` directly (debugging, perf comparison, etc.).
3. That second host inherits `AGENTMUX_LAUNCHER_PIPE` from its parent shell, which inherited it from the dev launcher.
4. The standalone host connects to the *dev launcher's* pipe and pollutes its state.

This is rare but real. The fix needs to allow case (1) while still preventing case (4).

## 3. Goals

- **G1** `task dev` on Windows receives the full launcher event stream — `WindowOpened`, `WindowClosed`, `BackendWindowIdRegistered`, and anything else the launcher reducer emits.
- **G2** Window-count atom synchronizes across windows: opening / closing any window updates every other window's count within reactive frame.
- **G3** Opacity slider renders in every window's InstancePanel in dev.
- **G4** The original isolation guard's intent is preserved: a *standalone* host that happened to inherit `AGENTMUX_LAUNCHER_PIPE` from a parent pane's environment doesn't accidentally connect.
- **G5** Symmetric fix for the srv IPC pipe at line 312-316 (same bug, same shape).

## 4. Non-goals

- Frontend polling as a workaround (would mask the underlying flow break and add IPC chatter).
- Changing the launcher's pipe naming or handshake protocol.
- Cross-platform parity beyond Windows for v1 — `task dev` on Linux/macOS still invokes the host directly (per CLAUDE.md), so launcher IPC isn't applicable there yet.

## 5. Proposed design

Replace the path-based `is_dev_build_exe` guard with a parent-process identity check. The host connects to the launcher pipe only when its **parent process is the AgentMux launcher** (under any of its on-disk names — see §5.1).

### 5.1 New helper

`agentmux-cef/src/parent_process.rs` — host-only helper, doesn't pollute `agentmux-common`:

```rust
const ACCEPTED_PARENT_STEMS: &[&str] = &["agentmux-launcher", "agentmux"];

#[cfg(target_os = "windows")]
pub fn parent_is_agentmux_launcher() -> Option<bool> { ... }
```

Implementation: Windows `CreateToolhelp32Snapshot` + `Process32FirstW/NextW` in two passes — first finds the current PID's `th32ParentProcessID`, second finds the entry where `th32ProcessID == parent_pid` and reads its `PROCESSENTRY32W.szExeFile`. The exe filename is then lower-cased, `.exe` stripped, and compared case-insensitively against either accepted stem.

**Why `szExeFile` instead of `QueryFullProcessImageNameW`** (codex P2 on round 2): the latter requires a caller-allocated buffer; using a fixed `MAX_PATH` (260 wide chars) buffer would fail when the staged launcher path on a developer's Windows checkout exceeds that length, returning `None` and falling through to the dev guard — re-introducing the very regression this PR fixes. `szExeFile` is the filename only (no path), bounded by `MAX_PATH` wide chars by Win32 design, never overflows for typical exe names.

**Why two accepted stems** (codex P1 on PR #882 round 1 caught this):
- `agentmux-launcher` — Cargo bin name. Used in dev (`task dev` copies `target/release/agentmux-launcher.exe` to `dist/cef-dev/agentmux-launcher.exe`).
- `agentmux` — user-facing name in portable / installed builds. `scripts/package-portable.sh:38` copies the launcher binary to `agentmux.exe` so the user-facing icon reads as "AgentMux", not "AgentMux Launcher". `QueryFullProcessImageNameW` returns the on-disk path, so the parent's stem from a production launch is `agentmux`, not `agentmux-launcher`. Accepting only the Cargo bin name would regress every portable build's IPC connection.

### 5.2 New guard

```rust
// agentmux-cef/src/main.rs ~295
let parent_is_launcher = parent_process::parent_is_agentmux_launcher();
let should_connect_launcher = match parent_is_launcher {
    Some(true) => true,
    Some(false) => false,
    // Parent detection failed — fall back to the path-based guard
    // so production builds still connect (env var is set) and dev
    // builds still skip.
    None => !agentmux_common::is_dev_build_exe(&host_exe_dir),
};
let _launcher_ipc = if should_connect_launcher {
    runtime.block_on(launcher_ipc::connect_to_launcher(app_state.clone()))
} else {
    None
};
```

`launcher_ipc::connect_to_launcher` already checks `std::env::var("AGENTMUX_LAUNCHER_PIPE")` at its own entry (`launcher_ipc.rs:78`) and returns `None` if unset, so the env-var check doesn't need to be duplicated at the guard site. The guard's job is only to decide *whether to attempt the connection at all* based on parent identity / build mode.

Equivalent semantics to: "connect if our parent is the launcher (deterministic case) OR — when parent detection fails — when we're a production build (path-based fallback)."

Same `should_connect_launcher` reused at line 312-316 for srv IPC (both pipes are launcher-owned, so the guard is the same).

### 5.3 Why parent-process is the right discriminator

| Scenario | Env var set? | `is_dev_build_exe`? | Parent is launcher? | Connect? |
|---|---|---|---|---|
| Production portable / installed | yes | no | yes | ✓ |
| `task dev` (post #SPEC_LAUNCHER_DEV_INTEGRATION) | yes | yes | yes | ✓ (was ✗ — this is the fix) |
| Dev host launched standalone, no parent launcher | no | yes | no | ✗ |
| **Dev host inheriting env from sibling agent pane** | yes | yes | no | ✗ (guard preserved) |

The fourth row is the case the original guard protected. The new guard catches it identically — the inheriting host's parent is `cmd.exe` / a shell, not `agentmux-launcher`.

## 6. Edge cases

| Case | Handling |
|---|---|
| Parent process exits during host startup | `parent_is_agentmux_launcher` returns None → fall through to `is_dev_build_exe` check. Production build still connects (env var set, not a dev build). Dev build skips. Safe. |
| Symlinks or renamed launcher exe | Exe filename comparison is filename-only (after extension strip), so `agentmux-launcher` matches `agentmux-launcher.exe`, `agentmux-launcher`, but not `my-renamed-launcher.exe`. Document the rename constraint. |
| Multi-host single launcher | Each host's parent IS the launcher; both connect to the same pipe. Pipe server already handles N clients (`agentmux-launcher/src/ipc/server.rs`). |
| Tests that mock the host | Tests run from `target/debug` typically without env var; new guard skips connection. Same behavior as today. |
| Future cross-platform parity | Linux/macOS add their own parent-process helpers when those platforms get launcher integration. Same gate shape. |

## 7. Phased rollout

Single PR. Files touched:

| File | Change | LOC |
|---|---|---|
| `agentmux-cef/src/parent_process.rs` (new) | `parent_is_agentmux_launcher() -> Option<bool>` helper, Windows-only for v1 | ~140 |
| `agentmux-cef/src/main.rs:295-316` | Replace both guards (launcher IPC + srv IPC) | ~10 |
| Tests | Unit test for the helper (mockable via env or test harness) + integration smoke that asserts the bridge fires in dev | ~50 |

Phase split unnecessary — both fixes ride together because they share the helper.

## 8. Risk

- **Windows API correctness**: `parent_is_agentmux_launcher` walks the process tree via Win32. Bugs in the snapshot loop (stale PID, race with parent exit) would return None and degrade to today's behavior. Mitigation: comprehensive unit test + structured fallback.
- **Re-introducing the parent-inheritance bug**: only if parent-process detection fails AND `is_dev_build_exe` returns false. By construction (the new gate's `||`), failure of detection means production builds still connect (correct) and dev builds skip (correct). The guard is strictly tighter than today's, not looser, in dev cases.
- **Performance**: `parent_is_agentmux_launcher` is called once at host startup. ~1ms cost.

## 9. Test plan

- [ ] Unit: `parent_is_agentmux_launcher` returns `Some(true)` when invoked from a process spawned by `agentmux-launcher.exe` (or `agentmux.exe` in portable builds).
- [ ] Unit: `parent_is_agentmux_launcher` returns `Some(false)` when launched directly from a shell (`cmd`, `bash`).
- [ ] Unit: `parent_is_agentmux_launcher` returns `None` on non-Windows targets.
- [ ] Integration: in a `task dev` session, open 3 windows. All status bars show `v<X.Y.Z> (3)`.
- [ ] Integration: in a `task dev` session, open 1 window, then a 2nd. Both status bars update from `()` and `(1)` to `(2)`.
- [ ] Integration: in a `task dev` session, click the version chip in any window. Each entry in the InstancePanel shows an opacity slider (gated on `entry.windowId`, which is now populated).
- [ ] Regression: portable build still connects (production path unchanged).
- [ ] Regression: a manually-spawned dev host (no parent launcher) does NOT connect even if `AGENTMUX_LAUNCHER_PIPE` is set.

## 10. Open questions

- **Q1** Should we replace the path-only `is_dev_build_exe` discriminator entirely, since parent-process is strictly more informative? Lean **no** for this PR — keep both checks composed via `||` for defense-in-depth. Revisit when cross-platform parity ships.
- **Q2** The launcher's pipe naming uses `dir_hash` of the data dir. Should the host *also* check that the pipe path embedded in `AGENTMUX_LAUNCHER_PIPE` corresponds to its own data dir? That would be an even stronger guard, surfacing config drift. Lean **defer** — parent-process check is sufficient and simpler to validate.
- **Q3** Symmetric fix for srv IPC at line 312-316 — same guard pattern, identical shape. Include in this PR or split? Lean **include** — same bug, same fix, no extra surface.

---

🤖 Authored by AgentA, 2026-05-16. Implementation ships in the same PR per `feedback_no_doc_only_prs.md`. Likely PR #881 or #882 depending on what merges first.
