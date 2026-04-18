# SPEC: Test-Harness Access to the App API (+ /wave → /agentmux rename)

Status: draft
Date: 2026-04-18
Owner: AgentA
Motivation: external test harnesses (PowerShell, Node, Rust integration
binaries, future CI jobs) need to call the agentmux-srv service API in
order to drive setup and teardown without touching the UI. Today the API
is fully functional but gated behind a per-process random auth key that
no external process can read.

This spec proposes a narrow, audit-friendly path for dev-mode harnesses
to obtain that key while preserving release-build security.

**Bundled rename.** Every HTTP route is currently prefixed `/wave/*` —
a Wave-Terminal-era name that's been misleading since the AgentMux fork.
This spec also renames the prefix to `/agentmux/*` as part of the same
effort, since every test harness that uses the endpoint has to learn
its URL. Doing both at once avoids teaching harnesses the old name.

## 1. Background

### 1.1 What the API does

`agentmux-srv` exposes an HTTP router on `127.0.0.1:<port>`. The main
RPC endpoint is `POST /wave/service` (rename target: `/agentmux/service`).
Every frontend action goes through this endpoint — `CreateBlock`,
`DeleteBlock`, `UpdateObjectMeta`, `CreateTab`, `UpdateTabIds`,
`SetMeta`, layout tree reducers, etc. The request body carries
`{service, method, args, uicontext}`. Response is `{data, updates, error}`.

Other routes under the same prefix:

| Route | Purpose |
|---|---|
| `POST /wave/service` | Main RPC |
| `GET  /wave/file` | File read for object-id / filename pairs |
| `GET  /wave/stream-file[/*path]` | Stream-read (currently 501 stubs) |
| `GET  /wave/stream-local-file` | Local file stream (501) |
| `POST /wave/reactive/inject` | LAN-awareness reactive API |
| `GET/POST /wave/reactive/*` | Reactive agent registry/audit/poller |

All of these will move to `/agentmux/*`. See §4.7 for the rename details.

### 1.2 Why the auth key exists

`agentmux-srv` listens on a loopback TCP port, not a Unix-domain socket.
Any process on the machine that reaches `127.0.0.1:<port>` could, absent
auth, invoke arbitrary service methods — which includes methods that
spawn shell processes (terminal blocks), delete user data (workspace
delete), or modify layout state. Auth is defense against **same-user
local-process attack**.

The key is a random v4 UUID generated at `agentmux-cef` startup
(`state.rs:241`), stored in `state.auth_key: Mutex<String>`, and passed
to `agentmux-srv` via the `AGENTMUX_AUTH_KEY` env var
(`sidecar.rs:119`). Frontend reads it via the CEF host binding
`getApi().getAuthKey()`.

### 1.3 Current gap

External processes can't obtain the key because:
- It never hits the filesystem.
- The host logs only the first 8 chars (`sidecar.rs:109`) for diagnostic
  purposes — deliberately not the full key.
- The CEF host binding isn't reachable over any documented channel from
  outside the CEF process.

Result: test harnesses have no way to call the app API. They currently
simulate user input via UIAutomation + mouse clicks, which is slow,
flaky, layout-sensitive, and can't reach operations that don't have UI
bindings (bulk data manipulation, fixture setup, etc).

## 2. Goals and non-goals

### 2.1 Goals

1. **Test-only endpoints (eventually, CI-friendly)**: a harness written
   in any language should be able to call `/wave/service` against a
   running `task dev` instance, without polling logs or driving UI.
2. **Release-build parity**: release builds behave exactly as today.
   No auth-weakening, no new endpoints, no file writes that weren't
   already there.
3. **Audit clarity**: any dev-only behaviour is gated on a single
   compile-time feature or env var, visible in `grep` and reviewable
   in one PR.
4. **No new trust model**: we don't invent a second auth layer or a
   token-exchange protocol. The existing `auth_key` is the key; the
   question is just how a dev harness retrieves it.

### 2.2 Non-goals

- Remote test access. This is local-only; `127.0.0.1` stays.
- Changing the service API surface.
- Replacing UI-level test drivers (UIA) — they're still the right tool
  for testing UI behaviour. This spec is about data/setup plumbing.
- Multi-user concurrent dev instances. If two agentmux-cef processes
  run for the same user, they have separate data dirs (`ai.agentmux.cef.v<ver>`)
  and thus separate auth files; that's fine.

## 3. Threat model

The system remains defended against:

| Attacker | Before | After (Option A) |
|---|---|---|
| Remote network attacker (same LAN) | Blocked by 127.0.0.1-only bind | Same |
| Same-user local process (release build) | Needs auth_key (in-process only) | Same |
| Same-user local process (dev build) | Needs auth_key | Can read `authkey.dev` from user-readable path |
| Other-user local process | Blocked by Windows ACL / Linux file perms on the data dir | Same — same ACLs apply to the auth file |

Dev builds weaken the "same-user local process" axis, which is consistent
with how every dev-mode tool (Vite exposing :5173, CEF remote debugging
on :9222, the sidecar's own loopback ports) behaves today. Release
builds are unchanged.

## 4. Design options considered

### 4.1 Option A — dev-only auth file (recommended)

`agentmux-cef` writes the auth_key to a file in the data dir at startup
when `cfg(debug_assertions)` is true. File is gone or stale on next
release-build startup. Harness reads file → calls `/wave/service` with
the key.

- **Path**: `<data_dir>/authkey.dev`
- **Contents**: `{"auth_key":"<uuid>","web_endpoint":"127.0.0.1:<port>","ws_endpoint":"127.0.0.1:<port>","created_at":"<iso8601>","pid":<cef_pid>}`
- **Write timing**: after backend started, so endpoints are known (sidecar.rs line 108 context).
- **ACL**: Windows DACL set to "current user only, no inherited ACEs".
  On Unix: mode `0600`, owner = current uid.
- **Cleanup**: overwritten on every fresh startup (superseded). Not
  deleted on shutdown (next startup owns the overwrite — deletes on
  shutdown can race with harnesses reading).
- **Gate**: `#[cfg(debug_assertions)]` surrounding both the write and
  the ACL code, and the struct that defines the file format.
- **Release behaviour**: file is never written. Harnesses that look
  for it fail fast.

**Pros**
- Works for any language; no SDK required.
- One-file, one-gate — a reviewer can confirm it's dev-only in one grep.
- Consistent with Vite :5173 and CEF :9222 dev-mode exposures.
- No new endpoints, no new auth, no change to the trust model.

**Cons**
- File on disk. Mitigated by owner-only ACL and debug-build-only gate.
- Harness has to know the file path (documented in the harness README).

### 4.2 Option B — dedicated test API with a separate token

Add `POST /test/*` methods with their own `test_token` env var that only
dev builds generate. Sanctioned test methods (create_layout, reset_workspace)
live here, not on the main service.

**Pros**
- Separation of intent: clearly "test-only" surface.
- Test methods can be higher-level (one call = full 3-pane layout).

**Cons**
- Two auth systems to maintain.
- Re-implements methods the main service already has.
- More endpoints is more surface to guard.
- A dev harness still needs a way to fetch `test_token` — same problem
  we started with, one level removed.

### 4.3 Option C — CEF DevTools protocol

Drive the frontend's already-authenticated context via CDP on :9222.
Execute `window.globalAtoms...` / `getApi().getAuthKey()` via
`Runtime.evaluate`.

**Pros**
- No new backend code.
- Works on release builds too (if DevTools port is exposed).

**Cons**
- DevTools is disabled on release by default (and should be).
- CDP websocket protocol is an API surface we don't control.
- Frontend internals (`globalAtoms`) aren't a stable API — refactors
  would break harnesses.
- Slow — CDP round-trips add hundreds of ms per call.

### 4.4 Option D — IPC-channel auth extension

`agentmux-cef` already exposes IPC over a separate port with an
`ipc_token` that's written to the page URL. Add a `get_auth_key` IPC
command that returns `state.auth_key` to the IPC caller.

**Pros**
- Reuses an existing authenticated channel.
- No filesystem.

**Cons**
- `ipc_token` is logged to host log (redactable) and embedded in the
  Vite-served URL — harvestable by any process that can read either.
  So this just moves the auth question from "get the auth_key" to
  "get the ipc_token". No net security improvement.
- Couples frontend IPC and backend service auth semantically.

### 4.5 Option E — unix/Windows named pipe with OS-level auth

Expose a second channel over a named pipe with OS file-system permissions
doing the auth instead of a token.

**Pros**
- "Most secure" on paper; no tokens to leak.

**Cons**
- Cross-platform pipe code (Windows named pipes vs Unix sockets) is
  a non-trivial amount of work.
- Harnesses need pipe-talking code per language.
- Overkill for a dev-time plumbing problem.

### 4.6 Decision

**Option A.** Simplest, smallest surface, matches how other dev-time
tools expose themselves. Each of the alternatives solves a problem we
don't have (B = bespoke test API when the main API is already fine;
C = frontend-internals dependency; D = moved shell game; E = enterprise
security for a dev-time fixture).

### 4.7 Path rename: `/wave/*` → `/agentmux/*`

Lifted with the auth-file change because anyone writing a harness
against `/wave/service` would otherwise learn the old name and have to
relearn it later. Rename now; never document the old name in any
harness doc. Scope from `grep -rn /wave/ .`:

- `agentmux-srv/src/server/mod.rs` — `Router::route("/wave/...")` literals
- `agentmux-srv/src/server/reactive.rs` — internal refs to the reactive path
- `agentmux-srv/src/backend/blockcontroller/shell.rs` — shell-side URL builders
- `agentmux-srv/src/server/tests.rs` — test URIs
- `agentmux-srv/tests/integration_test.rs` — integration test URIs
- `frontend/app/store/wos.ts` — `getWebServerEndpoint() + "/wave/service"`
- `frontend/app/store/global.ts` — `/wave/file?...`
- `frontend/app/view/term/*` — termosc / termsticker / termagent URL builders
- `frontend/app/element/markdown-util.ts`, `frontend/util/waveutil.ts`
- `agentmux-cef/src/client.rs` — one internal URL, if any matches
- Spec-side references (`specs/*.md`, `docs/analysis/*.md`) — docs update only,
  no behaviour change

21 files match `/wave/` today. One commit, one grep-replace pass,
`cargo check` + `npx tsc --noEmit` + `cargo test --features test-authfile`
all pass. No compat shim — both server and client flip atomically
because they ship in the same build.

External integrations? None — the backend is 127.0.0.1-only and the
frontend is the only client. No API consumers to deprecate.

Test: the server should return 404 on any `/wave/*` request after the
rename (canary test in `agentmux-srv/src/server/tests.rs`).

## 5. File format

```jsonc
// <data_dir>/authkey.dev
{
  "version":         1,
  "auth_key":        "f8c9b0e4-1234-4567-89ab-cdef01234567",
  "web_endpoint":    "127.0.0.1:59719",
  "ws_endpoint":     "127.0.0.1:59720",
  "ipc_endpoint":    "127.0.0.1:59718",
  "ipc_token":       "92d136fa-2e14-46d0-9ace-eddee320a35e",
  "service_path":    "/agentmux/service",
  "file_path":       "/agentmux/file",
  "instance":        "v0.33.264",
  "data_dir":        "C:\\Users\\area54\\AppData\\Roaming\\ai.agentmux.cef.v0-33-264",
  "host_pid":        19500,
  "created_at":      "2026-04-18T20:30:15.123Z"
}
```

Including `service_path` / `file_path` (post-rename values) so a
future prefix change doesn't require harnesses to hardcode strings.

Fields chosen so a harness has everything it needs without having to
parse logs:
- `auth_key` / `web_endpoint` — hit `/wave/service`.
- `ipc_endpoint` / `ipc_token` — already-logged values, consolidated
  for convenience (harness can send IPC commands too).
- `host_pid` — lets harness detect stale files from a dead instance.
- `instance` / `data_dir` — diagnostics; useful when multiple dev
  instances exist.
- `version` — schema version. Bump if the file format changes.

## 6. Code changes

### 6.1 `agentmux-cef/src/sidecar.rs`

After `Backend successfully started` log line, when
`cfg(debug_assertions)`:

```rust
#[cfg(debug_assertions)]
{
    write_dev_auth_file(
        &data_dir,
        &auth_key,
        &backend_endpoints,  // web + ws
        &ipc_endpoint,
        &ipc_token,
        &version_instance_id,
    );
}
```

New helper `fn write_dev_auth_file(...)` in the same file (or a new
`src/dev_authfile.rs`), gated by `#[cfg(debug_assertions)]`.

### 6.2 ACL logic

Windows (`windows-sys` crate already in deps):
- Create the file with `GENERIC_READ | GENERIC_WRITE` for current user
  only, no inherited ACEs.
- Use `SetNamedSecurityInfo` or `AddAccessAllowedAce` to construct a
  DACL with one ACE for the current user.

Unix (not currently a target, but for future parity):
- `std::fs::Permissions::from_mode(0o600)` on the file handle.

Both paths verified by a unit test that reads the DACL back and asserts
only the current user's SID is present.

### 6.3 Release build is compile-time unreachable

Both the helper function and its call site are under `#[cfg(debug_assertions)]`.
A release build's binary won't contain the helper at all. A
`grep -R authkey.dev agentmux-cef/target/release/` on the built binary
must return zero hits — verified in CI.

### 6.4 Harness README update

`tools/tests/README.md` adds a section:

> **Getting the auth key.** A dev-mode `agentmux-cef` writes its auth
> key and endpoints to `<data_dir>/authkey.dev`. The dev data dir on
> Windows is `%APPDATA%\ai.agentmux.cef.v<version>` (format: `v0-33-264`
> for version `0.33.264`). Read the file as JSON. The structure is
> documented in `docs/specs/SPEC_TEST_API_ACCESS.md` §5.

### 6.5 Harness library (`tools/tests/authfile.ps1`)

Helper sourced from `pane-focus-stress.ps1`:

```powershell
function Read-AgentmuxAuthFile {
    # Walk APPDATA for the newest ai.agentmux.cef.v* dir
    # Read authkey.dev, validate host_pid is alive, return the object
    # Throw if no file, or if file is stale (pid dead)
}
```

Similar helpers land for Node.js (`tools/tests/authfile.mjs`) and
Python (`tools/tests/authfile.py`) as those ecosystems start needing
the harness.

## 7. Test plan

1. **Unit test** in `agentmux-cef/tests/dev_authfile.rs` — verifies
   file format, ACL (owner-only), and the `#[cfg]` gate (compiles but
   has no side effect in release build). Only run under
   `cargo test --features test-authfile`.
2. **Smoke test** — after `task dev` comes up, `authkey.dev` exists,
   parses as JSON, contains a non-empty `auth_key` field.
3. **Negative test** — a release build produces no `authkey.dev` file.
   Verified in CI by building with `--release` and checking
   `<data_dir>/authkey.dev` is absent after startup.
4. **Harness round-trip** — `pane-focus-stress.ps1` reads the file,
   makes a `POST /wave/service` with service=workspace method=GetWorkspace,
   expects `200 OK` with a Workspace payload.

## 8. Rollout

1. PR 1: land this spec.
2. PR 2: `/wave/*` → `/agentmux/*` rename. Pure grep-replace; backend
   routes + every caller in the same commit. Regression canary: 404
   on `/wave/service`. Lands independently of PR 3 so the rename isn't
   blocked on auth-file work.
3. PR 3: add `authkey.dev` file write + ACL, unit tests, release-build
   no-op verification.
4. PR 4: update `pane-focus-stress.ps1` and `tools/tests/README.md` to
   use the auth file. Retire the "fill in coords manually" step for
   layout setup — harness creates the 3-pane layout via
   `object.CreateBlock` + `workspace.UpdateTabIds` directly.
5. PR 5 (optional): Rust integration test crate at `tests/integration/`
   that uses the auth file to talk to a live dev instance. Replaces
   manual smoke testing for large classes of behaviour.

## 9. Back-compat

File doesn't exist today; harnesses that need it check for it and
gracefully fail with a clear message ("no authkey.dev found — run
`task dev` first, or upgrade to v0.33.X+ which writes the file").
No existing tool or path expects the file, so adding it is purely
additive.

## 10. Not in scope

- Packaging a dedicated "test runner" binary. The PowerShell harness
  is enough for the current load.
- Cross-machine test access (remote CI agents driving a dev instance).
  If that need arises, a port-forward or reverse-tunnel is the orthogonal
  mechanism; the auth key concern is the same either way.
- Auto-cleanup of stale `authkey.dev` files after the host exits.
  Harnesses read `host_pid` and check liveness before using the key —
  that handles the stale case. File cleanup on startup (overwrite) is
  sufficient.
