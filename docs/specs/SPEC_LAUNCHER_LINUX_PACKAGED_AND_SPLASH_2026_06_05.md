# SPEC: Launcher + reducer/saga parity on Linux + Linux splash

**Date:** 2026-06-05
**Repo state:** branch `agentu/spec-linux-launcher-splash`, base `main` post-#1272 (`[patch.crates-io]` for CEF 148 patched-libcef) and post-#1275 (CEF 148 Vulkan SwiftShader bundle).
**Author:** AgentU-asaf (driven by Claude)
**Status:** Spec — ready to implement (phased, with two distinct workstreams of very different size)
**Motivated by:** Linux AppImage launches the CEF host directly; no launcher in the launch path; the launcher's window/pool/instance reducer + durable saga coordinator (already written, already used on Windows) is dormant on Linux because the non-Windows IPC server is a no-op stub and `spawn_host_unix` doesn't export `AGENTMUX_LAUNCHER_PIPE`. We want the **full reducer + saga system** on Linux, not just "process supervision."
**Builds on:** [`SPEC_LAUNCHER_MACOS_PACKAGED_AND_SPLASH_2026_05_31.md`](./SPEC_LAUNCHER_MACOS_PACKAGED_AND_SPLASH_2026_05_31.md) (macOS precedent — same architectural gap, different OS), [`SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`](./SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md) (saga schema + replay semantics — platform-neutral), [`SPEC_CEF_148_LINUX_FORWARD_PORT_2026_06_04.md`](./SPEC_CEF_148_LINUX_FORWARD_PORT_2026_06_04.md) §0 (X11 ozone default on Linux).

---

## 0. TL;DR

1. **Linux launches the CEF host directly today** — `scripts/linux-apprun.sh:39` does `exec "$this_dir/usr/bin/agentmux"`, and `scripts/build-appimage-linux.sh` never even copies the launcher binary into the AppImage. The launcher's entire reducer + saga + window-pool + single-instance subsystem is sitting in `dist/cef/agentmux-launcher` unused.
2. **The reducer and saga code is already platform-neutral.** `agentmux-launcher/src/state.rs:168-226` (State struct), `src/reducer/mod.rs:84-540` (60+ Command handlers), `src/saga/mod.rs` (coordinator), `src/saga/log/schema.rs:32-61` (SQLite schema) have **no `#[cfg(target_os = …)]` gates**. They've never been driven on Linux not because they're Windows-coupled, but because the only **transport** that delivers Commands to them is Windows-only.
3. **Three concrete code gaps prevent driving the reducer on non-Windows** (each verifiable in 30 seconds): `ipc::run_ipc_server` on non-Windows is `tokio::spawn(async {})` (`src/ipc/server.rs:175-185` — comment: "Phase 7 of the broader tear-off cross-platform work will add Unix domain socket support"); `host_pipe/` has no Unix-socket impl and the host's `launcher_ipc::connect_to_launcher` is `cfg(not(target_os = "windows"))` → `None` (`agentmux-cef/src/launcher_ipc.rs:298-303`); `spawn_host_unix` (`main.rs:389-432`) exports `AGENTMUX_LAUNCHER_PID` only, not `AGENTMUX_LAUNCHER_PIPE` like Windows' `spawn_host_supervised` does at `main.rs:328`.
4. **Two workstreams, very different sizes:**
   - **A0 — AppRun + packaging + process tree** (~80 LOC of script). The launcher becomes the AppImage entry point; it supervises srv + host as a process group. **Delivers only srv supervision parity** — not single-instance, not window pool, not sagas, not instance numbering. Worth shipping early because it sets up the process tree the rest of the work assumes.
   - **A1 — non-Windows IPC + host_pipe + spawn_host_unix supervision** (~800–1,500 LOC across launcher + host + tests). This is the real work: implement the Unix-domain-socket IPC server, the host-side socket client, the AGENTMUX_LAUNCHER_PIPE env handshake, the equivalent of CREATE_SUSPENDED + Job Object via fork/exec + process groups + cgroup-v2. Once landed, **all 17 `report_*` calls in `agentmux-cef/src/launcher_ipc.rs` start hitting the (already-implemented) reducer**, and the saga coordinator starts writing to its (already-implemented) SQLite log. `--diag sagas` is already platform-neutral (`diag.rs:707-750`) so it lights up for free.
5. **B — Linux splash** (unchanged from prior revision; ~400 LOC of x11rb + cfg gate). Independent of A0/A1 in scope, but practically depends on A0 so the launcher is the entry point in the first place.

---

## 1. Current state — evidence

### 1.1 AppRun bypasses the launcher

`scripts/linux-apprun.sh:39` (inside `run_normally()`):

```bash
exec "$this_dir/usr/bin/agentmux" "$@"
```

The launcher binary is **built but never copied** into the AppImage. `scripts/build-appimage-linux.sh` stages only the host at `:84` (`cp dist/cef/agentmux-cef "$APPDIR/usr/bin/agentmux"`) and the srv at `:88`. The launcher in `dist/cef/agentmux-launcher` is dead weight, sitting in the dev tree.

### 1.2 No Linux splash module

```
agentmux-launcher/src/
├── splash.rs       ← #![cfg(target_os = "windows")]   312 lines
├── splash_mac.rs   ← #![cfg(target_os = "macos")]     372 lines
└── (no splash_linux.rs)
```

`main.rs:108-113` falls through `#[cfg(not(target_os = "macos"))]` straight to `launcher_main()` — no splash spawn on Linux.

### 1.3 X11 ozone is the Linux default (PR #1261)

`agentmux-cef/src/app.rs:418-440` defaults `--ozone-platform=x11` (XWayland under any Wayland compositor). Opt-out: `AGENTMUX_OZONE_PLATFORM=wayland`. Relevant only for the splash design (§B.1).

### 1.4 Host has IPC wiring with 17 `report_*` calls — all no-op on Linux

`agentmux-cef/src/launcher_ipc.rs:478-773` exposes the public API the rest of the host calls into:

```
pub fn report_window_opened(label, kind, parent_label) { … }
pub fn report_window_closed(label) { … }
pub fn report_pool_window_added(label) { … }
pub fn report_pool_window_removed(label) { … }
pub fn report_pool_window_promoted(label) { … }
pub fn report_panes_reaped(label) { … }
pub fn report_pool_drain_decision(label, was_last) { … }
pub fn report_host_counts(windows, pool) { … }
pub fn report_backend_window_id_registered(label, window_id) { … }
pub fn report_backend_window_id_unregistered(label) { … }
pub fn report_host_pool_count(count) { … }
pub fn report_hwnd_opened(hwnd, class_name, title, label_hint) { … }
pub fn report_hwnd_destroyed(hwnd) { … }
pub fn report_hwnd_visibility_changed(hwnd, visible) { … }
pub fn report_hwnd_foreground_changed(hwnd) { … }
pub fn report_hwnd_iconic_changed(hwnd, iconic) { … }
pub fn report_hwnd_position_changed(hwnd, rect) { … }
pub fn report_monitor_topology_changed(rects) { … }
```

On Linux today they're all no-ops at the transport layer: `agentmux-cef/src/launcher_ipc.rs:298-303`:

```rust
#[cfg(not(target_os = "windows"))]
pub async fn connect_to_launcher(
    _state: std::sync::Arc<crate::state::AppState>,
) -> Option<LauncherIpcHandle> {
    None
}
```

That `None` propagates: every send path checks the handle and short-circuits. The reducer in the launcher process (when it eventually runs) never sees a `Command`.

### 1.5 The non-Windows IPC server is an explicit stub

`agentmux-launcher/src/ipc/server.rs:175-185`:

```rust
#[cfg(not(target_os = "windows"))]
pub fn run_ipc_server(
    _pipe_name: String,
    _ctx: ServerCtx,
) -> tokio::task::JoinHandle<()> {
    // Non-Windows: pipe IPC isn't built yet. Phase 7 of the broader
    // tear-off cross-platform work (separate spec) will add Unix
    // domain socket support. For now return an immediately-finished
    // task so the caller can hold a handle uniformly.
    tokio::spawn(async {})
}
```

This spec is the "separate spec" the comment references.

### 1.6 `spawn_host_unix` doesn't export the IPC env var

Compare side by side. Windows (`agentmux-launcher/src/main.rs:293-374`):

```rust
#[cfg(target_os = "windows")]
fn spawn_host_supervised(...) -> Option<tokio::process::Child> {
    …
    host_cmd
        .args(args)
        .env("AGENTMUX_BACKEND_WS", &srv.ws_endpoint)
        .env("AGENTMUX_BACKEND_WEB", &srv.web_endpoint)
        .env("AGENTMUX_BACKEND_PID", srv.pid.to_string())
        .env("AGENTMUX_AUTH_KEY", &srv.auth_key)
        .env("AGENTMUX_INSTANCE_ID", &srv.instance_id)
        .envs(host_env.iter().cloned())
        .env("AGENTMUX_LAUNCHER_PIPE", pipe_path)       // ← present
        .creation_flags(CREATE_SUSPENDED)                // ← Job-Object handshake
        .kill_on_drop(false);
    if let Some(dir) = host_runtime_dir {
        host_cmd.env("AGENTMUX_HOME", dir);              // ← present
    }
```

Unix (`main.rs:389-432`):

```rust
#[cfg(not(target_os = "windows"))]
fn spawn_host_unix(...) -> Option<tokio::process::Child> {
    let mut host_cmd = tokio::process::Command::new(real_exe);
    host_cmd
        .args(args)
        .env("AGENTMUX_BACKEND_WS", &srv.ws_endpoint)
        .env("AGENTMUX_BACKEND_WEB", &srv.web_endpoint)
        .env("AGENTMUX_BACKEND_PID", srv.pid.to_string())
        .env("AGENTMUX_AUTH_KEY", &srv.auth_key)
        .env("AGENTMUX_INSTANCE_ID", &srv.instance_id)
        .env("AGENTMUX_LAUNCHER_PID", std::process::id().to_string())
        .envs(host_env.iter().cloned())
        .kill_on_drop(false);
        // ← no AGENTMUX_LAUNCHER_PIPE
        // ← no AGENTMUX_HOME
        // ← no CREATE_SUSPENDED / Job Object equivalent
```

### 1.7 The reducer + saga code is platform-neutral and already written

`agentmux-launcher/src/state.rs:168-226` — `State` struct fields:

```rust
pub struct State {
    pub lifecycle: LifecyclePhase,
    pub processes: HashMap<u32, ProcessRecord>,
    pub windows: HashMap<String, WindowMirror>,
    pub pool: HashSet<String>,
    pub instance_registry: HashMap<String, u32>,
    pub next_instance_num: u32,
    pub backend_window_ids: HashMap<String, String>,
    pub event_version: u64,
    pub next_client_id: u64,
    pub monitors: Vec<Rect>,
    pub pending_hwnds: HashMap<u64, PendingHwnd>,
    …
}
```

`agentmux-launcher/src/reducer/mod.rs:84-540` — dispatch on the same `Command` enum the IPC server parses:

```rust
pub fn update(state: &mut State, cmd: Command, ctx: &Ctx) -> Vec<Event> {
    let mut cmd_events = match cmd {
        Command::Register { kind, pid, version } => …,
        Command::Ping { nonce } => …,
        Command::Goodbye => …,
        Command::ReportWindowOpened { … } => window::handle_report_window_opened(…),
        Command::ReportWindowClosed { label } => …,
        Command::ReportPoolWindowAdded { … } => pool::handle_report_pool_window_added(…),
        Command::ReportPoolWindowRemoved { … } => pool::handle_report_pool_window_removed(…),
        Command::ReportHostCounts { windows, pool } => …,
        // … 60+ variants total
    };
}
```

`agentmux-launcher/src/saga/log/schema.rs:32-61` — saga storage is a portable SQLite file:

```rust
const DDL: &str = "
CREATE TABLE IF NOT EXISTS launcher_saga (
    saga_id        INTEGER PRIMARY KEY,
    name           TEXT NOT NULL,
    state          TEXT NOT NULL CHECK (state IN ('running', 'completed', 'failed', 'compensating', 'failed_compensation')),
    started_at     TEXT NOT NULL,
    ended_at       TEXT,
    input_json     TEXT NOT NULL,
    failure_reason TEXT
);
CREATE TABLE IF NOT EXISTS launcher_saga_step (
    saga_id        INTEGER NOT NULL REFERENCES launcher_saga(saga_id) ON DELETE CASCADE,
    step_index     INTEGER NOT NULL,
    name           TEXT NOT NULL,
    state          TEXT NOT NULL CHECK (state IN ('pending', 'succeeded', 'failed', 'compensated')),
    cmd_json       TEXT,
    target         TEXT,
    started_at     TEXT NOT NULL,
    ended_at       TEXT,
    output_json    TEXT,
    failure_reason TEXT,
    PRIMARY KEY (saga_id, step_index)
);
```

Sagas defined: `pool_respawn` (`saga/pool_respawn.rs`) and `window_cleanup` (`saga/window_cleanup.rs`). Both `#[cfg]`-free.

### 1.8 `--diag sagas` is already cross-platform

`agentmux-launcher/src/diag.rs:707-720`:

```rust
#[cfg(target_os = "windows")]
pub async fn run_sagas_diag(launcher_exe_dir: &std::path::Path) -> Result<(), String> {
    run_sagas_diag_impl(launcher_exe_dir).await
}

#[cfg(not(target_os = "windows"))]
pub async fn run_sagas_diag(launcher_exe_dir: &std::path::Path) -> Result<(), String> {
    // The saga log is a SQLite file with no platform-specific bits;
    // the cross-platform parity goal for `--diag sagas` is "works
    // wherever the launcher writes the log." …
    run_sagas_diag_impl(launcher_exe_dir).await
}
```

Once A1 makes the launcher write the saga log on Linux, LSD-3 offline forensics lights up for free.

### 1.9 host-side IPC client speaks newline-delimited JSON over the named pipe

`agentmux-cef/src/launcher_ipc.rs:159-186`:

```rust
let (tx, mut rx) = mpsc::unbounded_channel::<Command>();
if COMMAND_TX.set(tx).is_err() { … }
let writer_for_drain = Arc::clone(&writer);
tokio::spawn(async move {
    while let Some(cmd) = rx.recv().await {
        let mut buf = match serde_json::to_vec(&cmd) {
            Ok(b) => b,
            Err(e) => { … continue; }
        };
        buf.push(b'\n');
        let mut w = writer_for_drain.lock().await;
        if let Err(e) = w.write_all(&buf).await { … }
    }
});
```

Same protocol as the server (`ipc/server.rs:330-352`): one JSON `Command` per line. A `tokio::net::UnixStream` replacement preserves this byte-for-byte.

### 1.10 `find_cef_binary` candidate order (relevant for §3 A0)

`agentmux-launcher/src/main.rs:1458-1497`:

```rust
fn find_cef_binary(runtime_dir: &std::path::Path) -> std::path::PathBuf {
    let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };

    let versioned = format!("agentmux-{}{}", env!("CARGO_PKG_VERSION"), ext);
    let versioned_path = runtime_dir.join(&versioned);
    if versioned_path.exists() { return versioned_path; }

    if let Ok(entries) = std::fs::read_dir(runtime_dir) {
        let prefix = "agentmux-";
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(prefix)
                && !name.starts_with("agentmux-cef")
                && !name.starts_with("agentmux-srv")
                && !name.starts_with("agentmux-launcher")
                && name.ends_with(ext)
            { return entry.path(); }
        }
    }

    let versioned_old = format!("agentmux-cef-{}{}", env!("CARGO_PKG_VERSION"), ext);
    if versioned_old_path.exists() { return versioned_old_path; }

    runtime_dir.join(format!("agentmux-cef{}", ext))   // ← final fallback
}
```

A bare `agentmux` (no dash, no version) matches none of (1) versioned, (2) `agentmux-*` dir scan, (3) `agentmux-cef-{version}`. Falls through to `agentmux-cef`. Implication for A0: keep the AppImage host's filename as `agentmux-cef` so the existing fallback resolves it without a launcher code change.

---

## 2. What Linux loses today, and what each workstream actually delivers

| Capability | Code that owns it | Today on Linux | After A0 only | After A1 |
|---|---|---|---|---|
| Process tree + srv supervision | `spawn_host_unix` + process group | ❌ srv is host's child; host crash leaves srv zombie | ✅ launcher owns both; process-group reap on launcher exit | ✅ (same) |
| Single-instance enforcement | IPC server + reducer `Command::Register` | ❌ second AppImage races on extract dir, sqlite, log file | ❌ still — needs IPC server | ✅ |
| Window pool (pooled tear-off) | reducer `pool::*` + saga `pool_respawn` | ❌ host falls back to cold `CreateWindowTask` | ❌ still | ✅ |
| Instance numbering ("AgentMux 2") | reducer `instance_registry` | ❌ every instance is "AgentMux" | ❌ still | ✅ |
| Durable saga coordination | `saga/log/` + `pool_respawn` + `window_cleanup` | ❌ no log written | ❌ still | ✅ |
| `--diag sagas` offline forensics | `diag::run_sagas_diag` | ❌ vacuous (no log to read) | ❌ still vacuous | ✅ reads saga log written by A1 |
| Cold-start splash | `splash_linux.rs` (new) | ❌ blank 200–600 ms | ❌ still | requires A0 + B |
| 17 `report_*` host-side hooks | `agentmux-cef/src/launcher_ipc.rs` | ❌ all no-op (connect → None) | ❌ all still no-op | ✅ all deliver |

**A0's effective delivery is one row.** That row is worth shipping — it's the foundation A1 supervises on top of — but **calling A0 "parity" was wrong**, and the prior revision of this spec did that. A1 is where parity lives.

---

## 3. Workstream A0 — launcher in the AppImage launch path

### Code changes (in implementation order)

1. **`scripts/build-appimage-linux.sh`** — two edits:

   (a) Keep the host binary at its cargo name `agentmux-cef` instead of renaming to `agentmux`, so the launcher's existing `find_cef_binary` final fallback (§1.10) resolves it without a launcher code change. Today:
   ```bash
   cp dist/cef/agentmux-cef "$APPDIR/usr/bin/agentmux"        # line 84
   ```
   After:
   ```bash
   cp dist/cef/agentmux-cef "$APPDIR/usr/bin/agentmux-cef"
   ```

   (b) Copy the launcher binary in:
   ```bash
   cp dist/cef/agentmux-launcher "$APPDIR/usr/bin/agentmux-launcher"
   ```
   Add a `require dist/cef/agentmux-launcher` to the existing artifact-existence check at line 64.

2. **`scripts/linux-apprun.sh`** — change the `exec` target. Today (line 39):
   ```bash
   exec "$this_dir/usr/bin/agentmux" "$@"
   ```
   After:
   ```bash
   exec "$this_dir/usr/bin/agentmux-launcher" "$@"
   ```
   No env-var exports needed — the launcher resolves siblings from its own `exe_dir` (the flat-layout case in `launcher_main` after the macOS-dev integration).

3. **Process-group reaping on Unix.** Today `spawn_host_unix` does `.kill_on_drop(false)`. The macOS launcher's analogue (per `SPEC_LAUNCHER_MACOS_DEV_INTEGRATION_2026_05_30`) puts the launcher + children into a single process group via `setsid()` so a `kill(-pgid, …)` reaps both children atomically. Mirror that here. ~10 LOC change in `agentmux-launcher/src/srv_spawner.rs` + `main.rs`'s `spawn_host_unix`.

4. **No other Rust changes** for A0.

### A0 explicitly does NOT deliver

- Anything that requires the IPC server or saga coordinator to actually run — that's A1.
- Splash UI — that's B.

### A0 verification

```
task build:host && task build:backend && task build:frontend && task bundle
bash scripts/build-appimage-linux.sh ~/Desktop
~/Desktop/AgentMux_<VERSION>_amd64.AppImage &
sleep 5
pstree -p $(pgrep -f AgentMux_.*.AppImage)
# Expected:
# AgentMux_.AppImage(N)─┬─agentmux-launcher(N+1)─┬─agentmux-srv-..(N+2)
#                       │                        └─agentmux-cef(N+3)─┬─zygote
#                       │                                            └─renderers
```

- [ ] `pstree` shows `launcher → { srv, host }` (not `AppRun → host`)
- [ ] `kill -TERM <launcher-pid>` reaps srv + host + descendants within 200 ms (process-group reap)
- [ ] The host log shows: `AGENTMUX_LAUNCHER_PIPE unset — running without launcher IPC (dev mode)` — this is **expected** until A1 lands

### A0 risks

| Risk | Severity | Mitigation |
|---|---|---|
| AppImage extract-once-cache re-execs `AppRun` from the extract dir — the new exec target must exist in both paths | Low | Update both fast and FUSE-fallback `run_normally` paths consistently |
| `host_env` plumbing in `spawn_host_unix` needs the AppImage's `LD_LIBRARY_PATH` to reach the child (libcef.so resolution) | Low | `AppRun` already exports `LD_LIBRARY_PATH` before exec; tokio inherits parent env by default |
| `find_cef_binary` matching `agentmux-launcher` itself if probe-order assumptions change | Low | Existing dir scan already excludes `agentmux-launcher` (`main.rs:1481`) |

---

## 4. Workstream A1 — non-Windows IPC + host_pipe Unix + supervision parity

**This is where the reducer + saga + single-instance + pool machinery actually starts running on Linux.** Each piece below maps to a specific file and a specific cfg gate that needs to be relaxed or extended.

### A1.1 — Implement `run_ipc_server` for Unix

**File:** `agentmux-launcher/src/ipc/server.rs:175-185` (the existing stub).

**Change:** replace the stub with a `tokio::net::UnixListener`-based accept loop that mirrors the Windows impl at lines 119-173. The handler at lines 187-217 (`handle_connection`) is already generic over a `Read + Write` half pair — pass it the Unix-stream halves directly. The newline-delimited JSON `Command` protocol stays identical.

**New helper:** `pipe_name` (`ipc/mod.rs:28-39`) currently returns a Windows path. Add a `cfg(unix)` arm returning `$XDG_RUNTIME_DIR/agentmux/{data_dir_hash16}.sock` (fallback `/tmp/agentmux-{uid}-{data_dir_hash16}.sock` if XDG_RUNTIME_DIR is unset). Bind path includes the user's UID to avoid cross-user squatting in the `/tmp` fallback.

**Stale-socket handling:** `connect → if ECONNREFUSED → unlink → bind` pattern. The launcher's existing single-instance saga logic (which currently runs on Windows only) decides whether to claim ownership vs forward the launch to the existing instance.

**Estimate:** ~200 LOC + ~100 LOC of tests.

### A1.2 — Implement the host-side socket client

**File:** `agentmux-cef/src/launcher_ipc.rs:298-303` (the `None`-returning stub).

**Change:** replace with the Unix analogue of the Windows client at lines 49-102:

```rust
#[cfg(not(target_os = "windows"))]
pub async fn connect_to_launcher(state: Arc<AppState>) -> Option<LauncherIpcHandle> {
    let sock_path = match std::env::var("AGENTMUX_LAUNCHER_PIPE") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            tracing::info!("AGENTMUX_LAUNCHER_PIPE unset — running without launcher IPC");
            return None;
        }
    };
    let stream = match tokio::net::UnixStream::connect(&sock_path).await { … };
    let (read, write) = tokio::io::split(stream);
    // … same writer task + reader task as Windows variant at lines 159-186
}
```

The 17 `report_*` callers (lines 478-773) are unchanged — they push `Command` into the `COMMAND_TX` channel which the writer task drains.

**Note:** `AGENTMUX_LAUNCHER_PIPE` is reused as the env var name on Unix (even though the resource is a socket path, not a named pipe). This avoids touching any of the 17 `report_*` call sites or the host's connect-on-startup code in `agentmux-cef/src/app.rs`.

**Estimate:** ~120 LOC.

### A1.3 — Export `AGENTMUX_LAUNCHER_PIPE` from `spawn_host_unix`

**File:** `agentmux-launcher/src/main.rs:389-432`.

**Change:** add `.env("AGENTMUX_LAUNCHER_PIPE", sock_path)` next to the existing `AGENTMUX_LAUNCHER_PID` export. Also add `.env("AGENTMUX_HOME", host_runtime_dir)` to match Windows' behavior for the host's data_dir resolution path.

**Estimate:** ~5 LOC.

### A1.4 — Process supervision parity (CREATE_SUSPENDED equivalent)

The Windows path uses `CREATE_SUSPENDED + AssignProcessToJobObject + ResumeThread` to guarantee the child is in the launcher's Job Object **before its first instruction executes**. Without this, a child can crash before the Job Object catches it, leaking processes.

The Unix analogue: `pre_exec` callback that calls `setsid()` (new process group) + `setpgid(0, launcher_pgid)` (join launcher's group) + (optionally) `prctl(PR_SET_PDEATHSIG, SIGKILL)` so the kernel auto-reaps the child if the launcher dies abnormally. The cgroup-v2 path (write child PID to `cgroup.procs` before exec) is more robust but requires the user-level systemd-managed cgroup; defer to A1.5.

**File:** `agentmux-launcher/src/main.rs:389-432`. Add a `pre_exec` closure via `std::os::unix::process::CommandExt::pre_exec`.

**Estimate:** ~50 LOC + Linux-only #[cfg] gate (pre_exec is Unix-portable; the prctl call is Linux-only).

### A1.5 — cgroup-v2 reaping (optional hardening)

If the user has a systemd-managed user slice (`/sys/fs/cgroup/user.slice/user-$UID.slice/`), create a per-launcher cgroup and write children's PIDs to it. The Windows Job Object semantics (kill all descendants when the Job Object closes) are then matched by `cgroup.kill` writes on launcher exit, even across `setsid()` boundaries the descendants might create.

**Defer to A1.x follow-up** — `prctl(PR_SET_PDEATHSIG, SIGKILL)` from A1.4 catches the common case; cgroup-v2 is the belt for the suspenders.

### A1.6 — Single-instance handshake

Once A1.1 + A1.2 + A1.3 land, the **reducer's `Command::Register` handler** (already in `reducer/mod.rs:103-227`) starts firing on Linux. Single-instance behavior is then governed by the existing reducer logic + saga `pool_respawn`. No new code in the reducer.

The launcher's startup path needs one new branch: before binding the listener at A1.1's socket path, try `UnixStream::connect`. If it succeeds, this is a **second-instance launch** — forward args via a Command to the existing launcher and exit 0. If it returns ECONNREFUSED with a stale socket file present, `unlink` and proceed to bind.

**File:** new branch in `main.rs::launcher_main()`, ~30 LOC.

### A1 verification

```
~/Desktop/AgentMux_<VERSION>_amd64.AppImage &
sleep 5
# Existing checks from A0 still pass.

# A1-specific:
ls $XDG_RUNTIME_DIR/agentmux/*.sock                              # socket bound
sqlite3 ~/.local/share/agentmux/.../db/launcher-sagas.db .schema  # saga log written
~/Desktop/AgentMux_<VERSION>_amd64.AppImage                       # second launch
# Expected: second launcher exits within 100 ms; existing launcher's host gets focus.

agentmux-launcher --diag sagas                                    # offline forensics
# Expected: prints saga log contents (single-instance, pool_respawn events).
```

- [ ] Socket file exists at `$XDG_RUNTIME_DIR/agentmux/<hash>.sock` while running, gone after launcher exit
- [ ] Host log shows `[ipc] connected to launcher` instead of `AGENTMUX_LAUNCHER_PIPE unset`
- [ ] Reducer state visible via the host's debug endpoint (existing on Windows)
- [ ] `--diag sagas` reads non-empty log
- [ ] Second AppImage instance exits cleanly without spawning a second host
- [ ] kill-9 on host: launcher restarts host within 500 ms (existing saga, just newly running)
- [ ] kill-9 on launcher: host + srv are reaped via pdeathsig within ~200 ms

### A1 risks

| Risk | Severity | Mitigation |
|---|---|---|
| `XDG_RUNTIME_DIR` not set on some headless distros / Wayland-less environments | Medium | Fallback to `/tmp/agentmux-{uid}-{hash}.sock` with explicit 0600 perms |
| Stale socket from a crashed launcher prevents new launches | Low | `connect → ECONNREFUSED → unlink → bind` pattern; documented |
| `pre_exec` closure restrictions (no allocation, no `Mutex`, etc. — runs between fork and exec) | Medium | Mirror the macOS launcher's known-safe pre_exec from prior work |
| Saga log written to extracted-dir on first run vs. real data dir confusion | Low | `data_dir::resolve_paths` already handles this on Windows; reuse |
| Reducer's window-pool sagas reference HWND on Windows; need X11/Wayland window-id equivalents | Medium | Reducer accepts opaque `String` window_id already (see `backend_window_ids: HashMap<String, String>`); host sends X11 window IDs as strings |

### A1 implementation order (suggested PRs)

1. **A1.1+A1.2+A1.3** — IPC server + host client + env export. Smallest possible "the wire is up" PR. Verification: host log shows connection; reducer increments `event_version` on first Command.
2. **A1.4** — `pre_exec` supervision parity. Independent of IPC; can land in parallel.
3. **A1.6** — single-instance handshake. Depends on A1.1.
4. **A1.5** — cgroup-v2 hardening. Optional follow-up.

---

## 5. Workstream B — Linux native splash

(Unchanged structurally from the previous revision; included here for completeness.)

### B.1 — Design rationale: X11 via `x11rb`

PR #1261 made `--ozone-platform=x11` the Linux default. The CEF host runs as an X11 client under every compositor via XWayland. A splash sharing the same X11 abstraction is the one-display-server-abstraction choice and covers ~100% of installs. `AGENTMUX_OZONE_PLATFORM=wayland` opts out of the splash in v1.

### B.2 — `agentmux-launcher/src/splash_linux.rs` shape

```rust
#![cfg(target_os = "linux")]

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

include!(concat!(env!("OUT_DIR"), "/brain_dims.rs"));
static BRAIN_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/brain_rgba.bin"));

const SPLASH_PADDING: i32 = 12;
const SPLASH_SIZE: u16 = (BRAIN_W + SPLASH_PADDING * 2) as u16;
const BG_R: u8 = 0x1A;
const BG_G: u8 = 0x1A;
const BG_B: u8 = 0x1F;

pub struct Splash { inner: Arc<SplashInner> }

impl Splash {
    pub fn show() -> Option<Self> { /* … */ }
    pub fn run_until_dismissed(self) { /* … */ }
}
```

### B.2.1 Window setup

- `create_window` with `override_redirect=1`; center on primary monitor via RANDR
- EWMH hints: `_NET_WM_WINDOW_TYPE_SPLASH`, `_NET_WM_STATE_ABOVE`, `_NET_WM_STATE_SKIP_TASKBAR`, `_NET_WM_STATE_SKIP_PAGER`

### B.2.2 Painting

- One `Pixmap` the size of the window; software alpha-blend the brain bitmap over the solid backdrop per tick (~60 Hz)
- Pulse curve copy-pasted from `splash.rs` (Windows)

### B.2.3 Dismiss protocol — reuse macOS's

`agentmux-cef/src/client/mod.rs:1265` already writes `AGENTMUX_SPLASH_READY_FILE` for macOS:
```rust
#[cfg(target_os = "macos")]
{ if let Ok(path) = std::env::var("AGENTMUX_SPLASH_READY_FILE") {
      if !path.is_empty() { let _ = std::fs::write(&path, b"ready"); }
}}
```
Add `target_os = "linux"` to the cfg gate — single-character change.

### B.2.4 Wayland-native opt-out

`AGENTMUX_OZONE_PLATFORM=wayland` → launcher skips `Splash::show()`. Documented as v1 scope cut; Wayland-native is a follow-up.

### B.3 `build.rs` — brain bitmap embedding

`agentmux-launcher/build.rs` is currently `#[cfg(target_os = "windows")]`-gated end-to-end. macOS bypasses build.rs entirely by doing `include_bytes!("../resources/brain.png")` in `splash_mac.rs:37` and decoding via NSImage at runtime.

Two options for Linux:

**Option 1** — extend `build.rs` with a `#[cfg(target_os = "linux")]` arm that decodes PNG → raw RGBA8 and emits `brain_rgba.bin` + `brain_dims.rs`. ~30 LOC, no runtime PNG decoder. *Recommended.*

**Option 2** — runtime decode via the `png` crate. Adds ~50 KB to the launcher binary; no build.rs change.

### B.4 Cargo deps (new)

```toml
[target.'cfg(target_os = "linux")'.dependencies]
x11rb = { version = "0.13", default-features = false, features = ["xinerama", "randr", "shape"] }
```

x11rb adds ~150 KB stripped. No C deps beyond `libxcb` (every distro ships it).

### B.5 Verification

- [ ] Brain logo appears within 50 ms of `AppRun` start
- [ ] Pulse animation smooth under Mutter (X11) and Mutter (Wayland → XWayland)
- [ ] Splash disappears within 200 ms of host first frame
- [ ] `AGENTMUX_OZONE_PLATFORM=wayland` → no splash, no errors
- [ ] Multi-monitor: centered on primary
- [ ] Host crash before first paint: splash 5 s timeout → clean exit

---

## 6. Sequencing & dependencies

```
A0 ──► A1.1+A1.2+A1.3 ──► A1.6
 │           │              │
 │           ├──► A1.4 ─────┤
 │           │              │
 │           └──► A1.5 (defer)
 │
 └──► B (independent of A1 but practically depends on A0 for the launcher entry point)
```

Recommended PR order:

| PR | Workstream | Approx LOC | Risk |
|---|---|---|---|
| 1 | A0 (AppRun + packaging + process group) | ~80 (scripts) + ~10 (Rust) | Low |
| 2 | A1.1+A1.2+A1.3 (the wire is up) | ~325 + ~150 tests | Medium |
| 3 | A1.4 (pre_exec supervision parity) | ~50 | Low–Medium |
| 4 | A1.6 (single-instance handshake) | ~30 | Low |
| 5 | B (splash_linux.rs) | ~400 | Low |
| 6 | A1.5 (cgroup-v2 hardening) | ~80 | Defer until real pain |

A0 is shippable alone; A1's value compounds (one row at a time after A0).

---

## 7. Out of scope

- **Wayland-native splash** (`wlr_layer_shell` / smithay). Tracked as a B follow-up.
- **Single-instance arg-forwarding** beyond "second instance returns." A separate spec.
- **`launcher_ipc` error surfacing** — replacing silent no-ops with `Result<()>`. Cross-platform concern; not Linux-specific.
- **Refactoring the reducer to make existing platform-neutral comments true on macOS too.** macOS gets parity from the same A1 work conceptually but is owned by `SPEC_LAUNCHER_MACOS_PACKAGED_AND_SPLASH_2026_05_31` already.

---

## 8. Open questions

1. **`/tmp` fallback security**: when `XDG_RUNTIME_DIR` is unset, `/tmp/agentmux-{uid}-{hash}.sock` with 0600 perms is the proposed fallback. Acceptable, or do we want a `~/.cache/agentmux/sock/<hash>.sock` fallback (avoids the `/tmp` race entirely)?
2. **`AGENTMUX_LAUNCHER_PIPE` env var name on Unix.** Reusing the Windows name avoids touching the 17 host call sites and the host's connect path. Alternative: introduce `AGENTMUX_LAUNCHER_SOCKET` and have the host probe both. Recommend: reuse the existing name.
3. **A1 vs Phase-2-of-the-tear-off-spec**: the IPC server stub's comment ("Phase 7 of the broader tear-off cross-platform work") implies this work was deferred there. Is there an existing tear-off Phase-7 spec we should merge with this one, or should this spec own A1 outright?
4. **macOS overlap with A1.** macOS hits the same `cfg(not(target_os = "windows"))` stubs. Should A1's implementation be platform-shared (`cfg(unix)`) or Linux-only with macOS as a follow-up? Recommend: platform-shared. The macOS launcher PR (#1263) would then opt into A1's IPC by changing nothing — the env-export side already happens in `spawn_host_macos` or its equivalent.

---

## 9. Decision

**Approve A0 as the next concrete PR** (small, mechanical, ship now). **Approve A1.1+A1.2+A1.3 as the immediate follow-up** — that PR is what lights up the reducer + saga on Linux. **B (splash) is parallel to A1** and can land any time after A0.

A0 alone is **not** parity. A1 delivers parity. This spec is the source of truth for what each step actually delivers; the impact table in §2 is the contract.
