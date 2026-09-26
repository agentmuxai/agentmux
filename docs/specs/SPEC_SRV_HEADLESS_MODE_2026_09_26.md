# SPEC: srv headless mode — run agentmux-srv without the launcher or a desktop host

**Date:** 2026-09-26
**Status:** active — slice 1 (§4: `--headless`, env preparation, lock, auth-key file, fixed loopback ports) ships in PR #3893. Slices 2–4 (container image, headless secret backend, frontend serving + allowed origins) remain.
**Author:** Maricon

---

## 1. Problem

`agentmux-srv` only runs as a child of the launcher (or, in dev, the CEF host). The parent prepares everything srv relies on:

| What srv relies on | Who provides it today | Without it |
|---|---|---|
| `AGENTMUX_AUTH_KEY` | launcher (`srv_spawner.rs`) | srv exits at startup (`config.rs`) |
| The data-path env (`AGENTMUX_INSTANCE_DIR`, `…_DATA_DIR`, `…_RUNTIME_DIR`, … via `DataPaths::to_env_vars`) | launcher | startup succeeds, but `DataPaths::from_env()` is `None` and about 20 call sites error or degrade (agent open, CLI resolution, installs, sessions, auth dirs) |
| A stdin pipe | launcher | EOF on stdin shuts srv down, and `/dev/null` (a container's stdin) is EOF immediately |
| A parent process | launcher / host | srv exits when its parent does (`install_process_watchers`) |
| Knowing the ports | launcher parses the `AGENTMUXSRV-ESTART` line | ports are random on every start |
| Single instance | launcher's IPC socket + lock | nothing stops two srvs sharing a data dir |

So srv can't be a container entrypoint, a system service, or a CI fixture without re-implementing the launcher's setup around it.

## 2. Goals

1. `agentmux-srv --headless` (or `AGENTMUX_HEADLESS=1`) starts with **no parent**, **no stdin**, and **no pre-set env**, and keeps running until SIGINT/SIGTERM.
2. **The auth key is never printed.** It comes from `AGENTMUX_AUTH_KEY`, from `--auth-key-file <path>`, or is generated and written to a file only the owner can read. Only the file's path is logged.
3. **Predictable addresses**: `--web-port` / `--ws-port` fix the ports.
4. **One headless srv per data dir.**
5. **Nothing else changes** for the launcher- and host-spawned srv.

**Non-goals (this slice):**
- Listening on anything but loopback. Remote access is a reverse proxy's job, with TLS and its own authentication, in front of a loopback srv (§5).
- Serving the frontend.
- A keychain replacement (§5).

## 3. Design

### 3.1 Startup

`main` checks `headless::requested()` before anything else. It is true for `--headless` or `AGENTMUX_HEADLESS=1`, **but never when a subcommand such as `migrate` runs**. A subcommand needs none of this setup, and the lock would refuse a `migrate --verify` run beside a running headless server.
- **Headless:** `headless::prepare_env()` runs, and the parent watcher is **not** installed.
- **Otherwise:** `install_process_watchers()` runs as before.

`install_shutdown_handlers(watch_stdin)` skips the stdin thread in headless mode. SIGINT/SIGTERM handling is unchanged.

### 3.2 `prepare_env` (`agentmux-srv/src/headless.rs`)

Runs before logging is initialized, so the log dir comes from the paths it resolves. Errors are printed and exit 1. In order:

1. **Data paths:** if `DataPaths::from_env()` is `None`:
   - resolve `DataPaths` for this version, with `AGENTMUX_RUNTIME_MODE` or `Installed`;
   - `ensure_dirs()`;
   - export `to_env_vars()`, exactly what the launcher exports.

   The root follows the usual rules: `AGENTMUX_HOME_OVERRIDE`, then `AGENTMUX_DATA_HOME`, then `~/.agentmux`. If the env is already set, it is used as is.

   **Then `AGENTMUX_HOME_OVERRIDE` is removed.** Every path is explicit in the env now, as the launcher leaves it, and srv resolves its stores from the exported data dir. Left set, the override would win in `agentmux_root()`, and every version's databases would open at the override root.
2. **Lock:** an exclusive lock on `srv-headless.lock` **in the directory holding the databases** (`MuxLock::acquire_at`), held for the process lifetime. That directory is `--wavedata` when given (the precedence `Config` applies), otherwise the resolved data dir. A second headless srv on the same data dir exits 1 and names the lock.
3. **Auth key**, unless `AGENTMUX_AUTH_KEY` is already set: `--auth-key-file` / `AGENTMUX_AUTH_KEY_FILE` (trimmed, must not be empty); otherwise a fresh key (two v4 UUIDs, 244 random bits, 64 hex characters) written to `<instance runtime>/srv-auth-key` with mode `0600`, **replaced on every start**, as a launcher-spawned srv gets a new key each launch. The path is printed; the key never is. It then becomes `AGENTMUX_AUTH_KEY`, and srv's normal config reads it and removes it from the environment.
4. **Ports:** `--web-port` / `--ws-port` are kept **in-process**, not in the env, so neither a launcher-spawned srv nor an agent can inherit them. The startup listeners bind `STARTUP_BIND_ADDR`'s loopback host with that port (`headless::startup_bind_addr`). A srv that isn't headless, or a headless srv without the flag, binds `STARTUP_BIND_ADDR` itself (OS-chosen port). **Still loopback only.**
5. **Cloud subscriber:** `AGENTMUX_DISABLE_CLOUD_SUBSCRIBER=1` unless the caller set it. The subscriber reads the OS keychain at startup, which a container usually lacks.
6. **No LAN listeners:** bootstrap calls `LanListenerSupervisor::forbid_lan()`, so a saved `network:lan_discovery: true` (or a later settings change) can't bind srv's ports on other interfaces or advertise them. mDNS advertising follows the supervisor's bound listeners, so it stays off too.

### 3.3 Readiness

- Unchanged: the `AGENTMUXSRV-ESTART ws:… web:…` line on stderr.
- `GET /` (no auth) answers `{"status":"ok","version":…}` once srv is serving, so it works as a container health check.

## 4. Slices

| Slice | Content |
|---|---|
| **1** | §3: `--headless`, `prepare_env`, per-channel lock, auth-key file, fixed loopback ports, no stdin/parent watchers, cloud subscriber off by default |
| 2 | `docker/Dockerfile.srv`: the existing agent image's build stage plus `-p agentmux-srv`; runtime needs node/npm, git and bash; `tini` entrypoint; data volume; `HEALTHCHECK` on `/` |
| 3 | A secret backend for machines without an OS keychain (the file-backed fallback `secret_store.rs` already lists as a follow-up), so Armory keys, MuxBus credentials and OAuth accounts work headless |
| 4 | Serving the built frontend, and a configurable allowed-origins list for CORS and the `/ws` origin check, for a same-origin client behind a reverse proxy |

## 5. Security notes

- **The auth key grants everything srv can do,** shell creation included. Every agent srv spawns also holds it (`pane_env.rs`). Headless mode doesn't widen that; it only changes who creates the key.
- **srv stays on loopback.** To use it from another machine, put it behind a reverse proxy that terminates TLS and authenticates users, and have the proxy send the key. Don't publish srv's ports directly.
- **Windows:** `MuxLock` does not exclude on non-Unix platforms (it only checks the file can be created), so goal 4 holds on Linux and macOS only.
- **Keychain-backed features fail** until slice 3 lands. Reads fail fast with an error; writes can hang if a D-Bus session exists without a secret agent.

## 6. Testing (slice 1)

- **Unit tests (`headless.rs`):**
  - flag and env detection;
  - `--flag value` and `--flag=value` parsing;
  - the generated key is 64 hex characters, mode `0600`, and new each time;
  - key files are trimmed and must not be empty;
  - the bind address defaults to `STARTUP_BIND_ADDR`;
  - the lock admits one holder and is released on drop;
  - a subcommand is never headless;
  - a forbidden LAN supervisor ignores `apply(true)`. Mutation check: without the guard, this test fails.
- **Live run** (clean environment via `env -i`, `AGENTMUX_HOME_OVERRIDE=<tmp>`, stdin from `/dev/null`, `--web-port 18190 --ws-port 18191`):
  - still running after 12 s;
  - `GET /` gives 200;
  - the key file is mode `600`, 64 bytes;
  - `/agentmux/discovery` gives 200 with the key and 401 without;
  - a second `--headless` on the same home exits 1, naming the lock;
  - SIGTERM gives "received SIGTERM, shutting down" and the process exits;
  - the stores open at `channels/stable/versions/<v>/data/db/objects.db`, not the override root;
  - with `network:lan_discovery: true` saved, srv listens only on `127.0.0.1`. The control run, without `forbid_lan`, also binds the host's LAN address.
- **The full `cargo test -p agentmux-srv` suite** covers the non-headless startup paths, which are unchanged.
