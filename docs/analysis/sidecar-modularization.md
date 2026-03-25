# Sidecar Modularization Analysis

**Date:** 2026-03-24
**File:** `src-tauri/src/sidecar.rs` (639 lines)
**Question:** Is modularization worth doing, and if so, how?

---

## Verdict

**Modularize, but surgically — not wholesale.**

The file is not too large to navigate, and four of its seven functions are already self-contained and well-named. The real problem is a single function: `spawn_backend` (lines 155–438, ~283 lines) doing ten sequential concerns in one body. That's the one to break up. Everything else can stay in `sidecar.rs` and work fine.

The second motivator for modularization is the restart loop from [`BACKEND_RESILIENCE_SPEC`](../specs/backend-resilience.md) Phase 2. The event loop inside `spawn_backend` (`tokio::spawn`, lines 341–414) is where the restart trigger must live — and it's currently tangled inside the startup orchestrator. Extracting that loop is the prerequisite for implementing restart without turning `spawn_backend` into spaghetti.

---

## Current Structure

| Lines | Function | Size | Platform | Concern |
|-------|----------|------|----------|---------|
| 23–90 | `ensure_versioned_sidecar` | ~68 | all | copies sidecar binary to version-isolated dir |
| 92–100 | `BackendSpawnResult` | 9 | all | result struct |
| 107–146 | `create_job_object_for_child` | ~40 | `#[cfg(windows)]` | Windows Job Object / KILL_ON_JOB_CLOSE |
| 155–438 | `spawn_backend` | ~283 | all | orchestrator (10 concerns, see below) |
| 446–611 | `cleanup_stale_backends` | ~165 | `#[cfg(unix)]` | kill stale sidecar processes |
| 616–630 | `cleanup_stale_endpoints` | ~16 | all | delete stale wave-endpoints.json files |
| 632–639 | `handle_backend_event` | ~8 | all | relay backend events to frontend |

### What `spawn_backend` does sequentially (10 concerns, 283 lines)

1. **Dir resolution** (155–196): resolve `data_dir`, `config_dir`, `version_instance_id`, create dir tree
2. **Stale process cleanup** (176–181): call `cleanup_stale_backends` + `cleanup_stale_endpoints`
3. **Auth key** (198–201): pull auth key from `AppState`
4. **Binary location** (203–240): probe portable path → dev path → `ensure_versioned_sidecar`
5. **wsh deploy** (242–280): copy `wsh` binary to `bin/wsh-{version}-{os}-{arch}`
6. **Process spawn** (289–305): build env vars + args, call `.spawn()`
7. **PID + state storage** (307–317): store child handle, PID, started_at in `AppState`
8. **Job Object** (319–335): `#[cfg(windows)]` create and store Job Object
9. **Event loop** (337–414): `tokio::spawn`, parse `WAVESRV-ESTART`, relay logs, handle `Terminated`
10. **Endpoint wait + result** (416–437): 30s timeout, construct `BackendSpawnResult`

---

## Proposed Module Structure

### Option A — Minimal (Recommended)

Extract exactly two things. Keep everything else in `sidecar.rs`.

```
src-tauri/src/
  sidecar.rs          ← orchestrator; spawn_backend shrinks to ~80 lines
  sidecar/
    binary.rs         ← concerns 3–5: binary location, auth key, wsh deploy
    event_loop.rs     ← concern 9: tokio::spawn + CommandEvent handler
```

**`sidecar/binary.rs`** (~120 lines)

```rust
pub struct SidecarCommand { /* CommandChild + rx */ }

pub fn resolve_binary_path(app: &tauri::AppHandle, name: &str) -> Result<PathBuf, String>
pub fn deploy_wsh(app_path: &Path) -> ()
pub fn build_sidecar_command(app: &tauri::AppHandle, auth_key: &str, version_data_home: &Path, ...) -> Result<(Receiver, CommandChild), String>
```

Groups the three binary-related concerns that are today interleaved with dir setup in `spawn_backend`.

**`sidecar/event_loop.rs`** (~100 lines)

```rust
pub async fn run_event_loop(
    mut rx: Receiver<CommandEvent>,
    app_handle: tauri::AppHandle,
    endpoint_tx: Sender<(String, String, String, String)>,
)
```

This is the key extraction for Phase 2 restart. Currently the event loop is a `tokio::spawn` that breaks on `Terminated`. After extraction, the restart loop becomes:

```rust
// In spawn_backend, after process restart:
tokio::spawn(event_loop::run_event_loop(rx, app_handle.clone(), endpoint_tx.clone()));
```

And in `run_event_loop`, the `Terminated` arm can emit `backend-terminated` and then signal the orchestrator to re-spawn instead of just breaking. That signal path (a `RestartSignal` channel or an `AppState` flag) is cleanly encapsulated in the event loop module rather than being embedded inside the orchestrator.

---

### Option B — Full Decomposition (Overkill for now)

```
src-tauri/src/
  sidecar.rs          ← orchestrator only (~80 lines)
  sidecar/
    binary.rs         ← binary path resolution + wsh deploy
    job_object.rs     ← Windows Job Object (already only used once)
    event_loop.rs     ← CommandEvent handler + ESTART parsing
    cleanup.rs        ← cleanup_stale_backends + cleanup_stale_endpoints
```

**Not recommended yet.** `cleanup.rs` extracts two small functions that are already well-named and have no restart-related complexity. `job_object.rs` is 40 lines only called once — moving it to its own file doesn't reduce cognitive load. The savings don't justify the extra files.

Revisit Option B if:
- `cleanup_stale_backends` gains a Windows counterpart (currently Unix-only)
- Job Object logic grows (e.g., memory limits, I/O limits)

---

## Where Phase 2 Restart Hooks In

The [`BACKEND_RESILIENCE_SPEC`](../specs/backend-resilience.md) Phase 2 requires:

1. `restart_backend` Tauri command → calls a new `respawn_backend(app)` function
2. `respawn_backend` reuses the binary resolution + spawn logic from `spawn_backend`
3. After restart, `backend-ready` event emitted with new endpoints
4. Frontend `reconnectRpcClient(wsEndpoint, authKey)` re-initializes the RPC client

With **Option A** in place:

- `respawn_backend` calls `binary::build_sidecar_command` directly — no duplication
- `tokio::spawn(event_loop::run_event_loop(...))` is called identically for both initial spawn and restart
- The restart counter lives in `AppState` and is checked inside `event_loop::run_event_loop` before emitting the restart signal
- The 30s `WAVESRV-ESTART` timeout becomes a shared helper callable from both `spawn_backend` and `respawn_backend`

Without Option A, `respawn_backend` would have to either duplicate the `spawn_backend` body or call `spawn_backend` itself — which means it also re-runs the stale cleanup, dir creation, and auth key lookup on every restart. That's wrong behavior for a hot restart.

---

## What NOT to Extract

| Item | Reason to keep in `sidecar.rs` |
|------|---------------------------------|
| `ensure_versioned_sidecar` | Only called from `spawn_backend`; 68 lines, single responsibility, no restart relevance |
| `BackendSpawnResult` | Tiny struct used by `lib.rs` — moving it fragments the API surface |
| `cleanup_stale_backends` | Unix-only, called once, self-contained; extracting saves nothing |
| `cleanup_stale_endpoints` | 16 lines. Not worth a module. |
| `handle_backend_event` | 8 lines. Stays. |

---

## Implementation Order

If implementing alongside Phase 2 restart (recommended — don't modularize for its own sake):

1. **Extract `event_loop.rs`** first — this unblocks the restart feature and is the highest-value extraction.
2. **Add restart logic in `event_loop.rs`**: on `Terminated`, check restart counter in `AppState`, emit `backend-terminated` as before, then send a restart signal on a new channel rather than just `break`.
3. **Extract `binary.rs`** so `respawn_backend` can call `build_sidecar_command` without duplicating the env var / arg construction.
4. **Add `restart_backend` Tauri command** in `commands/backend.rs` that calls the new `respawn_backend(app)` function.

If doing standalone (not tied to Phase 2):

Only extract `event_loop.rs`. The rest of `spawn_backend` is sequential setup code — messy in a single function, but it's not shared, not tested in isolation, and not blocking anything. Clean it up when the restart feature gives you a reason to touch it.

---

## Summary

| Extract? | What | Why |
|----------|------|-----|
| **Yes — now** | `event_loop.rs` (lines 341–414) | Prerequisite for restart feature; current location embeds the terminated handler inside the spawner |
| **Yes — with restart PR** | `binary.rs` (concerns 3–5) | Enables `respawn_backend` without duplication |
| **No** | Everything else | Already well-scoped, not shared, not blocking |

The file is not the problem. The function is. Extract the event loop, and `spawn_backend` becomes readable. Add `binary.rs` when you write `respawn_backend`. Done.
