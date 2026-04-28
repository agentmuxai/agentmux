# B.1 Concrete Design — Move srv spawn from host to launcher

**Status:** to execute
**Branch:** `agenta/phase-b1-launcher-srv-spawner` (off `c5f60944` — main with PR #570 merged)

---

## Goal

After B.1 ships, the process tree is:

```
launcher (Job Object J0, KILL_ON_JOB_CLOSE)
├── srv (assigned to J0 directly; survives host crash)
└── host (assigned to J0; renderers inherit J0)
```

vs today (post PR #570):

```
launcher (Job Object J0)
└── host (assigned to J0)
    ├── srv (assigned to host-owned J1 — DIES when host dies)
    └── renderers (inherit J0)
```

The key change: srv becomes a sibling of host under the launcher, so srv outlives a host crash. This is the foundation for Phase B's launcher-driven state machine, which needs to be able to restart the host while keeping srv (and its DB state) alive.

## Non-goals for B.1

- The reducer / state machine itself (Phase B sub-PRs B.3+).
- Named-pipe IPC for commands/events (Phase B sub-PR B.2).
- Removing host's `spawn_backend` entirely — kept as fallback for `task dev` where the launcher isn't in the loop.
- Touching frontend.

## Env-var contract launcher → host

Launcher passes srv coordinates to host via env vars; host honors env if present, falls back to existing spawn-srv path otherwise (dev mode).

| Env var | Set by | Consumed by | Meaning |
|---|---|---|---|
| `AGENTMUX_BACKEND_WS` | launcher | host | srv's WebSocket endpoint, e.g. `ws://127.0.0.1:8123/ws` |
| `AGENTMUX_BACKEND_WEB` | launcher | host | srv's HTTP endpoint, e.g. `http://127.0.0.1:8123` |
| `AGENTMUX_BACKEND_PID` | launcher | host | srv's PID, for diagnostics |
| `AGENTMUX_DATA_DIR` | launcher | host | data dir already chosen by launcher; host uses for CEF cache + lockfile |
| `AGENTMUX_CONFIG_DIR` | launcher | host | config dir |
| `AGENTMUX_USER_HOME_DIR` | launcher | host | per-agent user home |
| `AGENTMUX_AUTH_KEY` | launcher | host | shared auth key (also given to srv) |
| `AGENTMUX_INSTANCE_ID` | launcher | host | `v{cargo_pkg_version}` |

If `AGENTMUX_BACKEND_WS` is unset on host startup, host runs the existing `spawn_backend` path (`task dev` mode where the host runs without the launcher).

## File-by-file changes

### `agentmux-launcher/Cargo.toml`
- Add `tokio = { version = "1", features = ["rt-multi-thread", "macros", "process", "io-util", "time", "sync"] }`
- Add `dirs = "5"` (path resolution matches host)
- Add `chrono = { version = "0.4", default-features = false, features = ["clock"] }` (started_at logs)

### `agentmux-launcher/src/data_dir.rs` (NEW, ~80 LoC)
Mirror the path computation from `agentmux-cef/src/sidecar.rs:38–105`. Public API:
```rust
pub struct DataPaths {
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub user_home_dir: PathBuf,
    pub portable_root: Option<PathBuf>,
}
pub fn resolve_paths(launcher_exe_dir: &Path, version: &str, is_dev: bool) -> Result<DataPaths, String>;
```

Portable detection: launcher's `exe_dir` IS the portable root if `runtime/` exists alongside. (Different from host's check, which has the host inside `runtime/` and the parent as the portable root — the launcher and host should arrive at the SAME data_dir paths.)

### `agentmux-launcher/src/srv_spawner.rs` (NEW, ~250 LoC)
Adapted from `agentmux-cef/src/sidecar.rs::spawn_backend`. Public API:
```rust
pub struct SrvSpawnResult {
    pub pid: u32,
    pub ws_endpoint: String,
    pub web_endpoint: String,
    pub instance_id: String,
    pub auth_key: String,
    pub started_at: String, // RFC3339
    // Child handle held inside the launcher main loop; dropped on shutdown.
    pub child: tokio::process::Child,
}

pub async fn spawn_srv(
    paths: &DataPaths,
    job_handle: HANDLE,
) -> Result<SrvSpawnResult, String>;
```

Key adaptation vs host's version:
- Uses `tokio::process::Command` for async stdio.
- Spawns srv with **`CREATE_SUSPENDED`**, **assigns to launcher's J0**, then **`ResumeThread`** (same race-fix pattern as host spawn in PR #570).
- Does NOT create a separate job for srv (launcher's J0 covers it).
- Generates the auth key locally so launcher knows it (today the host generates it).

### `agentmux-launcher/src/main.rs` (MODIFIED, ~+100 LoC)
New `#[tokio::main]` entry point. Sequence:
1. Resolve launcher's `exe_dir`.
2. `SetDllDirectoryW("runtime")` (existing).
3. Resolve real CEF host binary (existing).
4. Acquire (placeholder for B.6 mutex; for now just compute paths).
5. Resolve `DataPaths` via `data_dir::resolve_paths`.
6. Create launcher's Job Object J0 (existing).
7. Spawn srv suspended → assign to J0 → resume → wait for `WAVESRV-ESTART` (with 30s timeout). Get `SrvSpawnResult`.
8. Spawn host suspended (existing) with all env vars from the table above set.
9. Assign host to J0 → resume.
10. `child.wait()` on host. (Srv runs independently; we'll need to wait for it too — see "concurrent waits" below.)
11. On host exit: send `Quit` signal to srv (or just drop its handle, which closes its stdin → srv's existing PPID death detection takes over).
12. Drop J0 → kernel reaps anything still alive.

Concurrent waits: `tokio::select!` between `host.wait()` and `srv.wait()`. If host exits first → close srv gracefully (eventually, RPC-style; for now drop child handle). If srv exits first → that's a srv crash; log it, propagate exit code to host exit eventually (Phase B's reducer handles this; for B.1 just log).

### `agentmux-cef/src/main.rs` (MODIFIED, ~+30/-10 LoC)
Replace the unconditional `sidecar::spawn_backend(&app_state).await` call with:
```rust
let backend_ready = if env::var("AGENTMUX_BACKEND_WS").is_ok() {
    // Launcher already spawned srv; ingest its endpoints from env.
    use_env_endpoints(&app_state)
} else {
    // Dev mode: launcher not in loop, spawn srv ourselves.
    runtime.block_on(sidecar::spawn_backend(&app_state)).is_ok()
};
```

`use_env_endpoints` is a new helper that reads the env vars and stores endpoints + PID + auth_key into `app_state` exactly as `spawn_backend` would. ~20 LoC.

### `agentmux-cef/src/sidecar.rs` (MODIFIED, ~-30 LoC)
- **Delete** `create_job_object_for_child` and the call in `spawn_backend` (lines 180–199). Launcher's J0 covers srv via inheritance now (or directly if launcher spawns srv as in B.1). Host's J1 is redundant defense-in-depth that, after B.1, would actively HARM the goal of srv-survives-host-crash — so it goes.
- Keep everything else for the `task dev` fallback path.

## Verification (manual)

After B.1:
1. Build a portable, run it. Confirm srv spawns from the launcher (check `~/.agentmux/logs/agentmux-launcher.log` for the new srv-spawn entry).
2. Open the InstancePanel; confirm UI works as before (no behavior change visible).
3. Kill `agentmux-cef.exe` via Task Manager (NOT the launcher). Confirm srv stays alive (it's in launcher's J0, not host's job). Confirm launcher's `child.wait()` returns; launcher does its cleanup; srv dies via dropped handle / launcher's J0 close.
4. Kill `agentmux.exe` (launcher) via Task Manager. Confirm both host and srv die immediately via OS-enforced J0 reap.
5. `task dev`: confirm host still spawns srv via the fallback path; UI works as before.

## Risks

1. **Auth key ownership** — today host generates the auth key. B.1 moves this to launcher (so launcher can pass it to both srv and host). Need to verify nothing else in the host startup path depends on host being the auth-key author.
2. **Backend-event forwarding** — host currently parses `WAVESRV-EVENT:` lines from srv's stderr and forwards to frontends as `agentmuxsrv-event`. If launcher captures srv's stderr, it can't forward to frontend (no IPC channel yet). For B.1: log them in launcher; events not forwarded. Phase B sub-PR B.2 (IPC) will properly forward.
3. **AGENTMUX_DEV** env propagation — currently host sets it on the srv command. Launcher needs to do the same.
4. **portable_root detection** — launcher's `exe_dir` IS the portable root; host's grandparent IS the portable root. Both must arrive at the same `data_dir` for CEF cache to work across both processes. Easy to get wrong; add a sanity log.
5. **Tokio runtime in launcher** — adds ~3 MB to launcher binary. Acceptable per Phase B Decision 1.

## Out of scope (queued for later sub-PRs)

- B.2: Named-pipe IPC server in launcher (commands + events).
- B.3: `Command` + `Event` types + pure reducer skeleton.
- B.4: Launcher state mirror (read-only diff vs host state).
- B.5: Migrate state stores to launcher-authoritative (one HashMap per sub-PR).
- B.6: Per-data-dir mutex single-instance.
- B.7: Frontend deletes polling loop; subscribes via host bridge.
- B.8: Phase B exit; delete host-side state stores.

---

Going to execute this now. Starting with Cargo.toml + data_dir.rs.
