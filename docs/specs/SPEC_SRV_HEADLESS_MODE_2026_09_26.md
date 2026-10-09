# SPEC: srv headless mode — run agentmux-srv without the launcher or a desktop host

**Date:** 2026-09-26
**Status:** implemented — slice 1 (§4: `--headless`, env preparation, lock, auth-key file, fixed loopback ports) in PR #3893; slice 2 (§3.4: container image) in PR #3898; slice 3 (§3.6: secrets without a keychain) in PR #3915; slice 4 (§3.5: frontend serving + allowed origins) in PR #3900.
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
5. **Nothing else changes** for the launcher- and host-spawned srv, except that it now also takes the data-dir lock (§3.2).

**Non-goals (this slice):**
- Listening on anything but loopback. Remote access is a reverse proxy's job, with TLS and its own authentication, in front of a loopback srv (§5).
- Serving the frontend.
- A keychain replacement (slice 3, §3.6).

## 3. Design

### 3.1 Startup

`main` checks `headless::requested()` before anything else. It is true for `--headless` or `AGENTMUX_HEADLESS=1`, **but never for a subcommand such as `migrate`, or for `--help` / `--version`**. A subcommand needs none of this setup, and the lock would refuse a `migrate --verify` run beside a running headless server.
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

   **Root-relative paths match the desktop app.** `open_stores_and_migrate` sets `AGENTMUX_DATA_HOME` to the data dir for every srv, and a launcher-started srv never has `AGENTMUX_HOME_OVERRIDE`. So `DataPaths::from_env().home_dir` is the data dir in a normal desktop run, and root-relative paths such as CLI installs (`<home_dir>/instances/…`) resolve under it.
   - **Observed on a desktop install:** `~/.agentmux/channels/<ch>/versions/<v>/data/instances/…`, with no root-level `~/.agentmux/instances`.
   - Removing the override makes headless srv resolve exactly the same way. Keeping it would put a headless srv's CLIs somewhere a desktop srv on the same data would not look.
   - Whether `home_dir` should be the true root for every srv is a separate, pre-existing question, **out of scope here**.
2. **Lock:** `base::acquire_data_dir_lock` takes an exclusive lock on `srv.lock` **in the directory holding the databases**, held for the process lifetime. That directory is `--wavedata` when given (the precedence `Config` applies), otherwise the resolved data dir.
   - **Every srv takes this same lock**, launcher-, host- or headless-started: bootstrap takes it before opening stores. So no two servers share a set of databases, however each was started.
   - Headless takes it early, before writing anything. The call is idempotent within a process, so bootstrap's later call is a no-op.
   - A second server on the same data dir exits 1 and names the lock.
3. **Auth key**, unless `AGENTMUX_AUTH_KEY` is already set: `--auth-key-file` / `AGENTMUX_AUTH_KEY_FILE` (trimmed, must not be empty); otherwise a fresh key (two v4 UUIDs, 244 random bits, 64 hex characters) written **beside the databases** (`<data dir>/srv-auth-key`) with mode `0600`, so two servers on different data dirs can't overwrite each other's key, **replaced on every start**, as a launcher-spawned srv gets a new key each launch. The path is printed; the key never is. It then becomes `AGENTMUX_AUTH_KEY`, and srv's normal config reads it and removes it from the environment.
4. **Ports:** `--web-port` / `--ws-port` are kept **in-process**, not in the env, so neither a launcher-spawned srv nor an agent can inherit them. The startup listeners bind `STARTUP_BIND_ADDR`'s loopback host with that port (`headless::startup_bind_addr`). A srv that isn't headless, or a headless srv without the flag, binds `STARTUP_BIND_ADDR` itself (OS-chosen port). **Still loopback only.**
5. **Cloud subscriber:** `AGENTMUX_DISABLE_CLOUD_SUBSCRIBER=1` unless the caller set it. The subscriber reads the OS keychain at startup, which a container usually lacks.
6. **No LAN listeners:** bootstrap calls `LanListenerSupervisor::forbid_lan()`, so a saved `network:lan_discovery: true` (or a later settings change) can't bind srv's ports on other interfaces or advertise them. mDNS advertising follows the supervisor's bound listeners, so it stays off too.

### 3.3 Readiness

- Unchanged: the `AGENTMUXSRV-ESTART ws:… web:…` line on stderr.
- `GET /health` (no auth) answers `{"status":"ok","version":…}` once srv is serving, so it works as a container health check. `GET /` answers the same unless `--frontend-dir` serves the UI there (§3.5).

### 3.4 Container image (slice 2): `docker/Dockerfile.srv`

Build from the repo root: `docker build -f docker/Dockerfile.srv -t agentmux-srv .`

- **Build stage** (`rust:1-bookworm`): `cargo build --release -p agentmux-srv -p agentmux-mcp -p agentmux-bashwrap`.
  - Agents started by srv launch `agentmux-mcp` and `agentmux-bashwrap` by bare name from `PATH`, as in the container agent image, so both ship in `/usr/local/bin`.
  - srv itself links only glibc; `agentmux-mcp` needs `libdbus-1-3` and `libxcb1` at runtime.
- **Runtime** (`node:24-slim`, Debian and glibc):
  - node/npm, because agent CLIs are npm packages srv installs and runs; plus git, bash, procps, curl and ca-certificates;
  - the unprivileged user `agentmux` (UID 1000), `HOME=/home/agentmux`, working dir `/workspace`.
- **Entrypoint:** `tini -- agentmux-srv --headless`, with `CMD --web-port 8190 --ws-port 8191`. tini is PID 1, forwards SIGTERM, and reaps the shells and agents srv spawns. srv exits 0 on SIGTERM.
- **State:** everything lives under `~/.agentmux`, declared a `VOLUME`. Databases and the generated key sit in `channels/stable/versions/<v>/data/`.
- **Auth key:** `AGENTMUX_AUTH_KEY` in the environment, a mounted file via `--auth-key-file`, or generated. A generated key's path is printed at start; the key never is.
- **Health:** `HEALTHCHECK` runs `curl http://127.0.0.1:8190/health`, which answers whether or not `--frontend-dir` is given (§3.5).
- **Loopback only, in the container too.** srv is reachable only from its own network namespace: a reverse proxy sharing it (a sidecar in the same pod or task, or `docker run --network container:<proxy>`) terminates TLS, authenticates users and sends the key. `docker run -p` can't reach it, on purpose.
- **CI:** `.github/workflows/srv-image.yml` builds the image (linux/amd64, no push) when its inputs change, or on demand. It then smoke-tests it:
  - it goes healthy;
  - the key file is mode `600`;
  - `/agentmux/discovery` gives 200 with the key and 401 without;
  - every listening socket is loopback;
  - it runs as `agentmux`, with the helpers, node and git on `PATH`;
  - `docker stop` exits 0.
- **Not published.** Where an image is published, and for which architectures, is up to whoever deploys it.

### 3.5 Frontend and allowed origins (slice 4)

For a browser client that reaches srv through a reverse proxy on one public origin. Headless only; both options are checked before anything is written (a bad value exits 1 and leaves no lock or key), and held in-process like the ports, so nothing inherits them.

- **`--frontend-dir <dir>`** (must contain `index.html`): srv serves the built frontend for every path no route claims, `/` included, and `index.html` for unknown paths, as a single-page app expects.
  - **Health moves to `/health`.** `/health` always exists; without `--frontend-dir`, `/` stays the health endpoint as before.
  - **Unauthenticated**, like the CEF host's static server: the bundle holds no secrets, and every API route still requires the key.
  - **Full router only.** The LAN router never serves it.
- **`--allowed-origin <origin>`** (repeatable): the proxy's public origin, e.g. `https://app.example.com`. It is accepted beside loopback by CORS and by the `/ws` Origin check (`is_allowed_origin`).
  - Written the way browsers send `Origin`: http or https, host, optional port; no path, user or query. Lowercased, a trailing `/` dropped, and the scheme's default port removed, so `https://App.example.com:443/` matches `https://app.example.com`.
  - Exact match only: no wildcards, no subdomains, never `null`.
- **Serving the frontend is not enough on its own** for a browser client to work: the UI still gets its endpoints and key through the desktop host today. A pluggable host is `SPEC_EXTERNAL_HOST_BUILD_2026_10_09.md`; srv's machine facts arrive with `srvinfo` (`SPEC_SRV_INFO_ON_CONNECT_2026_10_09.md`).
- **The key can stay with the proxy.** srv reads `X-AuthKey` before the `?authkey=` fallback on every route, `/ws` included, so a proxy that sets the header on every request (including the WebSocket upgrade) authenticates the browser's traffic. The UI sends `X-AuthKey` only when its host gave it a key (`frontend/app/store/auth-headers.ts`, the one place requests get the header), and opens `/ws` without `?authkey=` when it has none.
- **Live check** (headless, `--frontend-dir` with a fixture, `--allowed-origin https://App.Example.test/`):
  - `GET /` gives the UI; `/assets/app.js` gives 200; `/health` gives the health JSON;
  - `/agentmux/discovery` gives 401 without the key and 200 with it;
  - `/ws` upgrades from `https://app.example.test` and from loopback (101), and is refused from `https://evil.test` (403);
  - a CORS preflight reflects `https://app.example.test` and not `https://evil.test`;
  - `--allowed-origin https://app.example.test/ui` and a `--frontend-dir` without `index.html` both exit 1, writing nothing.

### 3.6 Secrets without a keychain (slice 3)

srv keeps every secret through `identity::secret_store`: Armory API keys, OAuth accounts, MuxBus credentials, browser-pane logins and the dormant model-catalog token. Nothing else touches the keychain. It now has two backends, chosen once at startup:

| Backend | Used by | Where |
|---|---|---|
| **OS keychain** | the desktop app (unchanged); headless with `--secret-store keychain` | macOS Keychain, Windows Credential Manager, Linux Secret Service |
| **File store** | **headless, by default** | `<shared dir>/secrets/` (`~/.agentmux/shared/secrets/`): one per machine, like the keychain, not per version |

- **File store layout:** one file per entry. Each file is named by the SHA-256 of the entry's key, so no id or hostname reaches a file name and no key can escape the directory.
  - The directory is `0700`, and is tightened if it already existed wider. Files are `0600`.
  - Writes go through a fresh temp file and a rename, so a reader never sees a partial secret.
  - A missing entry reads as absent, and deleting one is idempotent, as with the keychain.
- **Not encrypted by srv.** Protection is file permissions plus the volume's own encryption. A key stored beside the files would add nothing, and a machine-derived key means nothing in a container. That matches what CLIs do without a keychain (gh, the AWS CLI, Docker, Claude Code's own credentials file on Linux). Encryption with a key supplied from outside, such as a mounted secret, is a possible follow-up.
- **`SecretRef::Keychain { service, account }`** keeps its name and now means "an entry in srv's secret store". Secrets written by a headless srv live in its file store, and the refs in the database point there. There is no silent fallback between backends: a missing entry is an error.
- **Cloud subscriber:** headless still turns it off by default. A server doesn't connect to the AgentMux relay on its own. That was originally about the keychain; with the file store it's the only reason.
- **Startup log:** `backend directories initialized` carries `secret_store="file"` or `"keychain"`.
- **Live check** (headless, `AGENTMUX_HOST_REG_SECRET` set so the credential service accepts calls):
  - `credential.Save` succeeds and `Lookup` finds it;
  - `shared/secrets` is mode `700` and holds one file, mode `600`;
  - the password appears nowhere else under the home;
  - after a restart, `Lookup` still finds it;
  - `Delete` removes the file;
  - `--secret-store keychain` logs `secret_store="keychain"`.

## 4. Slices

| Slice | Content |
|---|---|
| **1** | §3: `--headless`, `prepare_env`, per-channel lock, auth-key file, fixed loopback ports, no stdin/parent watchers, cloud subscriber off by default |
| **2** | §3.4: `docker/Dockerfile.srv` and its CI smoke test |
| **3** | §3.6: a file-backed secret store, headless by default (`--secret-store keychain` to opt out) |
| **4** | §3.5: `--frontend-dir` serves the built frontend (health at `/health`); `--allowed-origin` for CORS and the `/ws` check |

## 5. Security notes

- **The auth key grants everything srv can do,** shell creation included. Every agent srv spawns also holds it (`pane_env.rs`). Headless mode doesn't widen that; it only changes who creates the key.
- **srv stays on loopback.** To use it from another machine, put it behind a reverse proxy that terminates TLS and authenticates users, and have the proxy send the key. Don't publish srv's ports directly.
- **Windows:** `MuxLock` does not exclude on non-Unix platforms (it only checks the file can be created), so goal 4 holds on Linux and macOS only.
- **Secrets:** headless keeps them in the file store (§3.6), protected by file permissions. With `--secret-store keychain` on a machine without a working Secret Service, reads fail fast with an error and writes can hang if a D-Bus session exists without a secret agent.

## 6. Testing (slice 1)

- **Unit tests (`headless.rs`):**
  - flag and env detection;
  - `--flag value` and `--flag=value` parsing, through the same `CliArgs` parser `Config` uses;
  - data, config and `--wavedata` paths that aren't valid UTF-8 are kept byte for byte (read with `var_os` / `args_os`), so the lock and the stores agree on the directory;
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
