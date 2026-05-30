# SPEC: Integrating `agentmux-launcher` into macOS / Linux `task dev`

**Date:** 2026-05-30
**Repo state:** `main` @ `51c3ba56` (v0.40.0)
**Author:** AgentO-asaf
**Status:** Spec ready to implement (phased)
**Continues:** [`SPEC_LAUNCHER_DEV_INTEGRATION_2026-05-13.md`](./SPEC_LAUNCHER_DEV_INTEGRATION_2026-05-13.md) — that spec integrated the launcher into **Windows** `task dev` and explicitly deferred Unix to "Phase 2 / Phase 7 cross-platform parity" (§6.2). **This is that Phase.**
**Related:** `SPEC_DEV_MODE_LAUNCHER_IPC_2026_05_16.md`, `SPEC_TASK_DEV_LAUNCHER_GAPS_2026_05_06.md`, `SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`

---

## 1. Problem

On Windows, `task dev` runs the production-parallel layout: `agentmux-launcher.exe` at the root spawns the host + srv from `runtime/`, so the launcher's Job Object, single-instance pipe, saga coordinator, splash, supervision loop, and launcher-events-to-renderer bridge are all exercised in dev exactly as in a packaged build (`Taskfile.yml::dev:serve`, Windows branch).

On **macOS / Linux**, `task dev` bypasses the launcher entirely and invokes the host directly (`cd "$DEV_DIR" && ./agentmux-cef --url=...`). The launcher's Unix `main()` branch is a stub that just `exec`s into the host (`agentmux-launcher/src/main.rs`, `#[cfg(not(target_os = "windows"))]` → `Command::new(real_exe).exec()`).

Consequences on macOS dev (and any future macOS package build that wants launcher parity):
- **No single-instance enforcement** owned by a supervisor — the host self-manages via Chromium's process-singleton on the cache dir (works, but it's a different mechanism than production-on-Windows).
- **No external supervision** — if the host crashes, nothing restarts it (the crash-budget retry ladder + `--disable-gpu` fallback are launcher-only).
- **No launcher saga coordinator** in the loop — pool-respawn / window-cleanup-cascade sagas (and their durable SQLite log + startup recovery) don't run on macOS dev.
- **No launcher event bus → renderer bridge** path is exercised — the `Event::*` → CEF-JS-bridge flow that drives InstancePanel atoms is Windows-launcher-fed; on macOS it relies on a different/absent path.
- **srv lifecycle differs** — on macOS the *host* spawns srv (`agentmux-cef/src/sidecar.rs::spawn_backend`); under the launcher, the *launcher* would spawn srv as a sibling so srv survives a host crash (Phase B.1 model).

The goal: run `agentmux-launcher` in the loop on macOS (and Linux) `task dev`, so the Phase B supervisor paths are exercised on the dev platform a developer actually uses, and so a future `task package:macos` has a launcher to ship.

---

## 2. The launcher is Windows-biased in implementation, portable in architecture

From a full audit of `agentmux-launcher/`, the responsibilities split cleanly:

### Already portable (no work) ✅
| Piece | Where | Notes |
|---|---|---|
| **Saga coordinator** | `saga/mod.rs` + submodules | Pure Tokio state machine; no OS calls |
| **Saga durability (SQLite)** | `saga/log/`, `saga/recovery.rs` | rusqlite; portable. Startup recovery + 7-day vacuum |
| **Event bus + disk log** | `event_log.rs` | `tokio::sync::broadcast` + JSONL append |
| **Path resolution** | `data_dir.rs` → `agentmux_common::DataPaths` | Already cross-platform |
| **Env-var handoff to host/srv** | `main.rs` (spawn env blocks) | `AGENTMUX_BACKEND_WS/_WEB/_PID/_AUTH_KEY/_INSTANCE_ID`, `to_env_vars()` — identical on all platforms |
| **srv binary resolution** | `srv_spawner.rs::resolve_srv_binary` | `agentmux-srv-{ver}-{os}.{arch}` search order — portable |
| **ESTART handshake parse** | `srv_spawner.rs::parse_estart` | `AGENTMUXSRV-ESTART ws:… web:… instance:…` — portable |
| **Host supervision loop** (wait→classify→retry-budget) | `main.rs::spawn_host_supervised` | Tokio wait; the algorithm is portable, only the `--disable-gpu` rung + crash-budget constants are generic |

### Windows-locked → needs a Unix port ⚠️
| Piece | Where (Win32) | Unix equivalent |
|---|---|---|
| **IPC transport** (launcher ↔ host command pipe) | named pipe `\\.\pipe\agentmux-{hash}\command` — `ipc/server.rs`, `ipc/mod.rs` | `tokio::net::UnixListener` on `<data-dir>/launcher.sock` |
| **Single-instance enforcement** | `first_pipe_instance(true)` bind; `ERROR_ACCESS_DENIED` → forward | `flock`/`fcntl` exclusive lock on `<data-dir>/launcher.lock`; `EWOULDBLOCK` → read `ipc-port` → forward |
| **Process-tree containment (Job Object J0, KILL_ON_JOB_CLOSE)** | `main.rs::create_job_object` + `assign_pid_to_job` | POSIX process group (`setpgid` + `killpg`) on launcher exit; **and/or** lean on existing parent-death watchers (below) |
| **Suspend/resume spawn** (`CREATE_SUSPENDED` + `ResumeThread`, used to assign to the job before the child runs) | `srv_spawner.rs`, `main.rs` host spawn | Not needed on Unix — spawn normally; the process-group join is immediate |
| **Splash screen** | `splash.rs` (`#[cfg(windows)]` layered popup) | Skip on Unix (Chromium window appears after init anyway); native splash is out of scope |
| **Collision dialog** (`MessageBoxW`) | `main.rs` | stderr line; no native dialog needed in dev |
| **DLL search path** (`SetDllDirectoryW`) | `main.rs` | N/A — Unix uses rpath / `DYLD`/`LD_LIBRARY_PATH` |

### Already solved on Unix (delegate, don't re-implement) 🟢
- **Parent-death detection.** `agentmux-srv` already watches its parent via **kqueue `EVFILT_PROC`** (macOS) / **pidfd** (Linux) and exits when the parent dies (visible in dev logs: `kqueue EVFILT_PROC registered for parent pid …`). So even without a Job Object, srv self-terminates when the launcher (its parent, under the launcher model) dies. The host needs the same or an stdin-EOF watcher when launcher-managed (the standalone path already handles its own srv).

**Bottom line:** the macOS/Linux launcher port is **~5 Win32 modules** (IPC transport, single-instance lock, process-group containment, drop suspend/resume, skip splash). Everything else compiles and runs as-is. The crate already *compiles* on macOS (the Win32 bits are all `#[cfg(windows)]`), producing today a no-op launcher that execs the host — we replace that stub with a real Unix supervisor.

---

## 3. Design

### 3.1 Layout — mirror the Windows production-parallel `dist/cef-dev/`

`dev:serve` (Windows) already builds:
```
dist/cef-dev/
  agentmux-launcher.exe          # root: the supervisor the user launches
  runtime/
    agentmux-cef.exe             # host
    libcef.dll + paks + …        # CEF runtime
    agentmux-srv-<ver>-…​.exe     # sidecar
```
and runs `cd dist/cef-dev && AGENTMUX_DEV=1 ./agentmux-launcher.exe`.

For macOS/Linux, build the analogous tree:
```
dist/cef-dev/
  agentmux-launcher              # root
  runtime/
    agentmux-cef                 # host
    Frameworks/…                 # macOS CEF framework (or libcef.so on Linux)
    libGLESv2.dylib … (macOS GL libs next to the host — see bundle:darwin)
    agentmux-srv-<ver>-darwin.arm64
```
and run `cd dist/cef-dev && AGENTMUX_DEV=1 ./agentmux-launcher`.

The launcher's Unix branch resolves host + srv from `runtime/` (mirroring the Windows `runtime/` resolution). The macOS CEF framework path resolution (`../Frameworks`) is relative to the host exe, so the host must sit at `runtime/agentmux-cef` with `runtime/Frameworks/` alongside — consistent with the `bundle:darwin` layout already added (the GL-lib copy + framework). **Verify** the `../Frameworks` lookup still resolves from `runtime/` (it should: `runtime/agentmux-cef` → `runtime/../Frameworks`? No — that resolves to `dist/cef-dev/Frameworks`. So either place `Frameworks/` at `dist/cef-dev/Frameworks` (one level up from the host) OR keep the host's framework lookup pointed at `runtime/Frameworks`. This is the one layout subtlety to pin down in implementation — see Open Question 1.)

### 3.2 Single-instance — `flock` on `<data-dir>/launcher.lock`

New `agentmux-launcher/src/lock.rs` (Unix):
- `open(O_CREAT|O_RDWR)` `<data-dir>/launcher.lock`, then `flock(fd, LOCK_EX|LOCK_NB)`.
- Success → we own the instance; hold the fd for the process lifetime.
- `EWOULDBLOCK` → another instance owns it → read `<data-dir>/.../ipc-port` (the host already writes `port:token` there post-CEF-init), POST `open_new_window` over loopback, exit. Reuses the existing portable `forward_open_new_window` path (the forward itself is already cross-platform — only the *detection* is Windows-pipe-specific).

This mirrors the named-pipe `first_pipe_instance` semantics on a primitive that's reliable on macOS + Linux.

### 3.3 IPC transport — Unix socket

New `agentmux-launcher/src/ipc/unix_socket.rs`:
- `tokio::net::UnixListener::bind(<data-dir>/launcher.sock)` (unlink stale first).
- Per-connection handler reusing the existing command-routing layer (the framing/dispatch in `ipc/server.rs` is portable; only the transport accept-loop is Win32).
- Host connects via `tokio::net::UnixStream` using a `AGENTMUX_LAUNCHER_SOCK` env path (the macOS analogue of `AGENTMUX_LAUNCHER_PIPE`).
- The launcher's saga `IssueCmd::Host` dispatch (`host_pipe::send_command`) gets a Unix-socket sibling.

### 3.4 Process-tree containment — process group + delegate to parent-death watchers

Two complementary mechanisms (belt + suspenders):
1. **Process group**: launcher calls `setpgid(0,0)` at startup (new group), spawns srv + host with the same pgid; on launcher exit (normal or signal), `killpg(pgid, SIGTERM)` then `SIGKILL` after a grace period. New `agentmux-launcher/src/process_group.rs` (Unix).
2. **Parent-death**: srv already self-exits via kqueue/pidfd when its parent (the launcher) dies. The host needs the same — add a kqueue/pidfd parent-death watcher to the host when it detects launcher-managed mode (`AGENTMUX_LAUNCHER_SOCK` present), OR rely on the process-group kill. Recommend the process-group kill as primary, parent-death as backstop.

### 3.5 srv + host spawn — drop the suspend/resume + job dance on Unix

`srv_spawner.rs` / host spawn: gate `CREATE_SUSPENDED` + `assign_pid_to_job` + `ResumeThread` to `#[cfg(windows)]`; on Unix, spawn normally (after `setpgid`) — there's no window where the child runs before joining the group, so no suspend needed. The portable parts (binary resolution, env, ESTART parse, supervision) are unchanged.

### 3.6 Splash — skip on Unix

`main.rs` already gates splash spawn behind `#[cfg(windows)]`. On Unix, pass no `AGENTMUX_SPLASH_EVENT`; the host's splash-dismiss signal becomes a no-op (already conditional).

### 3.7 Host launcher-mode detection — unchanged

The host already detects launcher-managed mode via `use_launcher_endpoints()` (`sidecar.rs`): if `AGENTMUX_BACKEND_WS` is set, adopt the launcher's srv endpoints instead of spawning its own. This is **already portable** — once the Unix launcher spawns srv and passes `AGENTMUX_BACKEND_WS/_WEB/_AUTH_KEY/_INSTANCE_ID`, the host on macOS adopts them with zero host changes. The standalone path (`task dev:standalone`, env absent → host spawns srv) remains the escape hatch.

---

## 4. Phasing

### Phase 1 — Compile + run a real Unix launcher that spawns srv + host (no supervision parity yet)
- `dev:serve` Unix branch builds the `dist/cef-dev/` + `runtime/` layout and invokes the launcher.
- Launcher Unix branch: resolve host+srv from `runtime/`, spawn srv (portable path, no suspend/job), parse ESTART, spawn host with the launcher env (`AGENTMUX_BACKEND_*`, `AGENTMUX_LAUNCHER_SOCK`), wait on host exit.
- `setpgid` + `killpg`-on-exit for containment.
- **Outcome:** `task dev` on macOS launches via the launcher; srv is launcher-spawned and survives a host crash; killing the launcher reaps the tree. Saga coordinator + event log run (they're portable). Single-instance + IPC socket may be stubbed initially.
- Build: add `agentmux-launcher` to `build:host:darwin`/`:linux` (currently `-p agentmux-cef` only).

### Phase 2 — Single-instance + IPC transport
- `lock.rs` (flock) single-instance + forward-to-existing.
- `ipc/unix_socket.rs` launcher↔host command channel; wire saga `IssueCmd::Host`.

### Phase 3 — Supervision parity + cleanup
- Crash-budget retry ladder verified on Unix (portable already; just exercise it).
- Host parent-death watcher as backstop.
- Decide splash (skip) and collision UX (stderr).

Phase 1 is the high-value slice: it puts the launcher in the loop on macOS dev and unblocks `task package:macos` later. Phases 2–3 bring full parity.

---

## 5. Build / Taskfile changes

- `build:host:darwin` / `build:host:linux`: add `-p agentmux-launcher` to the cargo build, and copy the launcher binary to `dist/cef-dev/` + the host/srv/runtime into `dist/cef-dev/runtime/`.
- `dev:serve` Unix branch: reshape `dist/cef-dev/` to the `runtime/` layout (mirror the Windows branch) and invoke `./agentmux-launcher` instead of `./agentmux-cef`.
- Keep `task dev:standalone` (host invoked directly) as the no-launcher escape hatch on all platforms — it already exists and is the debugging fallback.
- `bundle:darwin`: ensure the CEF framework + GL libs land where the host (now at `runtime/agentmux-cef`) resolves them (Open Question 1).

---

## 6. Risks

| Risk | Mitigation |
|---|---|
| `../Frameworks` host lookup breaks under the `runtime/` layout on macOS | Pin the framework dir placement (OQ 1) before reshaping the layout; smoke-test `task dev` launches without the `icudtl.dat`/LibraryLoader errors fixed earlier |
| Process-group kill reaps too much / too little | Belt-and-suspenders with srv's existing parent-death watcher; grace period before SIGKILL; test crash + clean-exit paths |
| Single-instance flock stale-lock after a crash | flock is released by the kernel on process death (unlike a lockfile with a written PID), so a crashed launcher's lock auto-frees — no stale-PID problem (contrast the Chromium SingletonLock issue, PR #1171) |
| Dev `AGENTMUX_DEV=1` env not propagated through the launcher to the host | The Windows branch already inlines `AGENTMUX_DEV=1` on the launcher invocation and the host inherits it via spawn; mirror that on Unix |
| Launcher-managed srv vs host-managed srv double-spawn | `use_launcher_endpoints()` already refuses to spawn a duplicate when `AGENTMUX_BACKEND_WS` is set (even if empty → hard error); no change needed |

---

## 7. Open questions

1. **macOS CEF framework placement under `runtime/`.** The host resolves `Chromium Embedded Framework.framework` via `../Frameworks` relative to the host exe. With the host at `dist/cef-dev/runtime/agentmux-cef`, `../Frameworks` = `dist/cef-dev/Frameworks`. Either (a) place `Frameworks/` at `dist/cef-dev/Frameworks` (one level above the host) and the GL libs in `runtime/`, or (b) point the host's `framework_dir_path` at `runtime/Frameworks`. Pin this before Phase 1 layout work. (The `framework_dir_path`/`resources_dir_path` are set in `agentmux-cef/src/main.rs` from `host_exe_dir/../Frameworks/...`, added in the macOS bundle PR — so option (a) matches the existing code with zero host change.)
2. **Do we need the IPC socket for Phase 1?** The saga coordinator's `IssueCmd::Host` is the only consumer; if no Phase-1 saga dispatches to the host on macOS dev, the socket can land in Phase 2 without blocking Phase 1.
3. **Linux parity in the same PR or a follow-up?** The Unix code is largely shared (flock, Unix socket, process group all work on both), but the CEF runtime layout differs (`libcef.so` vs `.framework`) and I can't test Linux here. Recommend implementing the shared Unix code cross-platform but gating the `dev:serve`/`build:host` layout reshape to darwin first, linux as a fast follow.

---

## 8. Acceptance criteria

**Phase 1 (macOS):**
- [ ] `task dev` on macOS launches `agentmux-launcher`, which spawns srv (launcher-managed) then the host; the window comes up identically to today.
- [ ] `muxlog`/logs show the launcher supervising: launcher startup, srv ESTART adopted by the host (`use_launcher_endpoints`), host spawned with `AGENTMUX_BACKEND_*`.
- [ ] Killing the launcher PID reaps srv + host + renderers (process group), no orphans.
- [ ] Killing the host alone → launcher restarts it within the crash budget (supervision parity).
- [ ] `task dev:standalone` still works (no-launcher escape hatch).
- [ ] Windows `task dev` unchanged.

**Phase 2/3:**
- [ ] Second `task dev` on the same data dir → single-instance forwards `open_new_window` to the running instance and exits (flock + ipc-port forward).
- [ ] Launcher saga coordinator runs on macOS dev (pool-respawn / window-cleanup sagas dispatch); durable log + startup recovery work.

---

## 9. Decision

Implement the Unix launcher as the **Phase 2/7 continuation** the earlier spec deferred. Start with **Phase 1** (launcher in the loop on macOS dev: real srv+host spawn, process-group containment, portable saga/event-log running) as a standalone PR — it delivers "the launcher operating on desktop dev" and unblocks a future `task package:macos`. Single-instance (flock) + IPC socket (Phase 2) and full supervision/cleanup parity (Phase 3) follow as independent PRs. Keep `task dev:standalone` as the escape hatch throughout.
