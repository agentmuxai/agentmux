# Analysis: Multi-clone `task dev` isolation

**Date:** 2026-05-26
**Status:** Analysis (no code change yet)

## TL;DR

Two clones of `agentmux` on the same Windows host can each independently
run `task dev` **only if they're on different branches.** On the same
branch, they collide on:

- the single-instance lockfile,
- the named-pipe IPC the launcher uses to receive "open" commands,
- the data dir (`~/.agentmux/dev/<branch>/`) — including logs, CEF cache,
  channel-scoped state.

The Vite port (5173) is also a hard collision, but it's already loud and
predictable: `--strictPort` makes the second clone fail fast at Vite
startup. That's a non-issue (clear error, not silent cross-contamination).

The lockfile + pipe collision is the bad one. The second clone's launcher
sees the first's pipe, treats it as "another instance of myself", and
routes its open-pane intent to the first clone's window. Two agents on
the same branch silently drive the same UI.

**Fix shape:** widen the dev isolation key from `<branch>` to
`<branch>/<clone-id>`, where `clone-id` is a stable hash of the clone's
workspace root path. ~40 lines of code in two files, backward compatible,
reuses the existing FNV-1a hash infrastructure.

This is in keeping with the spirit of the channels design — which already
isolates installed/portable/dev paths via `RuntimeMode` — and is a
natural extension of `RuntimeMode::Dev { branch }` to
`RuntimeMode::Dev { branch, clone_id }`.

---

## Current state | risks | recommended changes (one page)

### Current state

| Resource | Isolation key today | Source |
|---|---|---|
| Data dir | `~/.agentmux/dev/<branch>/` | `agentmux-common/src/data_paths.rs:450-457` |
| Logs / CEF cache / agents | Children of data dir | same |
| Single-instance lockfile | `instance_runtime_dir/lockfile`, name derived from `data_dir_hash16(instance_dir)` | `agentmux-launcher/src/hash.rs:42-51` |
| Named-pipe IPC | `\\.\pipe\agentmux-<hash16>\command`, hash of instance_dir | `agentmux-launcher/src/ipc/mod.rs:37-39` |
| Vite dev port | Hardcoded 5173, `--strictPort` | `Taskfile.yml:622`, `vite.config.ts:114-115` |
| srv TCP ports (web + ws) | OS-assigned via `bind 127.0.0.1:0` | `agentmux-srv/src/main.rs` |
| Cargo `target/` | Per-clone (in working tree) | n/a |
| `dist/cef-dev/` | Per-clone (in working tree) | `Taskfile.yml` |
| mDNS service name | Per-launch UUID-ish `instance_id` | `agentmux-srv/src/backend/lan_discovery.rs:71` |

### Risk profile (two clones, same branch)

| Resource | Collision? | Severity | Why |
|---|---|---|---|
| Data dir + logs + CEF cache | **Yes** | High | Both write to `~/.agentmux/dev/<branch>/` |
| Single-instance lockfile | **Yes** | Critical | Identical instance_dir → identical hash → same lockfile path |
| Named-pipe IPC | **Yes** | Critical | Same hash → same pipe name → second launcher routes opens into first instance |
| Vite port 5173 | Yes (loud) | Low | `--strictPort` fails fast; clear error, no silent damage |
| srv TCP ports | No | — | Dynamic |
| `target/`, `dist/cef-dev/` | No | — | In-tree |
| mDNS | No | — | Per-launch UUID |

### Recommended fix (smallest)

1. Extend `RuntimeMode::Dev` to `Dev { branch, clone_id: Option<String> }`.
2. Populate `clone_id` at runtime from a hash of the workspace root path
   (via existing `data_dir_hash16` / FNV-1a).
3. In `data_paths.rs`, nest dev instances at
   `~/.agentmux/dev/<branch>/<clone-id>/` instead of
   `~/.agentmux/dev/<branch>/`.
4. Export `AGENTMUX_CLONE_ID` in `to_env_vars()` for diagnostics.

That's the whole change. The lockfile and pipe automatically follow
because they both derive from `instance_dir`. No new mechanism — same
isolation discipline that already separates installed / portable / dev,
extended one level deeper for the multi-clone case.

---

## 1. Detailed inventory

### 1.1 Data dir and channel resolution

`agentmux-common/src/data_paths.rs:450-457` — for `RuntimeMode::Dev`:

```rust
if let RuntimeMode::Dev { branch } = mode {
    let safe_branch = sanitize_path_segment(branch)?;
    let channel = format!("dev-{}", safe_branch);
    let dir = root.join("dev").join(safe_branch);  // ← instance_dir
    return Ok((channel, dir));
}
```

`root` = `~/.agentmux/` (or `AGENTMUX_HOME_OVERRIDE` in tests). For
**any** clone on branch `main`, `instance_dir` resolves to
`~/.agentmux/dev/main/`. Every downstream path — logs, CEF cache,
agents, runtime, db — is a child of this.

### 1.2 Single-instance lockfile and named-pipe IPC

The launcher derives both from the **instance_dir** path:

`agentmux-launcher/src/hash.rs:42-51`:

```rust
pub fn data_dir_hash16(data_dir: &std::path::Path) -> String {
    let canonical = data_dir.canonicalize()
        .unwrap_or_else(|_| data_dir.to_path_buf());
    let s = canonical.to_string_lossy().to_lowercase();
    format!("{:016x}", fnv1a_64(s.as_bytes()))
}
```

`agentmux-launcher/src/ipc/mod.rs:37-39`:

```rust
let pipe_name = format!("\\\\.\\pipe\\agentmux-{}\\command", hash);
```

Two clones at `C:\repo1\agentmux` and `D:\repo2\agentmux`, both on
branch `main`, both resolve to instance_dir `~/.agentmux/dev/main/`.
`data_dir_hash16` of that path is **identical** for both clones, so
both pipe names and both lockfile paths are identical.

What happens on a real second launch:

1. Clone A's `task dev` succeeds: lockfile acquired, pipe created at
   `\\.\pipe\agentmux-H1234567890ABCDEF\command`, srv spawned, host
   loads Vite at `localhost:5173`.
2. Clone B's `task dev` runs the launcher. The launcher checks for an
   existing single-instance pipe; **it finds Clone A's pipe** and
   treats it as "I'm already running, route to me." It sends Clone B's
   intended open-pane command to Clone A's running window.
3. Clone B's Vite never starts (port 5173 is taken by Clone A's host
   anyway — that part fails loud).
4. From Clone B's perspective: launcher exits 0, no visible UI, but
   Clone A's window opens an extra pane. Silent cross-contamination.

This is the worst failure mode in the inventory.

### 1.3 Vite dev port

`Taskfile.yml:622` and `vite.config.ts:114-115` pin Vite to 5173 with
`--strictPort`. Second clone gets `EADDRINUSE` and an obvious failure
in the dev terminal. Not silent. Lower priority to fix because the
user immediately knows; multi-clone work likely wants different
Vite ports anyway.

### 1.4 srv TCP ports

`agentmux-srv/src/main.rs` binds the web and ws listeners to
`127.0.0.1:0`. OS assigns. No collision. Each srv writes its actual
port into the data dir (under `instance_runtime_dir`) for the host
to read — also collision-free **once** instance_dir is per-clone.

### 1.5 `target/` and `dist/cef-dev/`

Per-clone (live inside the clone's working tree). Disk cost is
significant — a Rust `target/` is a few GB and CEF release builds are
~400MB extracted — but functional isolation is fine. Could be
optimized with `CARGO_TARGET_DIR` or sccache; out of scope for this
analysis.

### 1.6 mDNS

`agentmux-srv/src/backend/lan_discovery.rs:71` registers as
`agentmux-{instance_id}`, where `instance_id` is generated at srv
boot (not deterministic across launches). Two parallel instances
register under distinct service names. Safe by accident — they'd
both *see* each other on the network as if they were two agentmux
hosts. That might surprise the warden widget's LAN list but it's not
a collision.

### 1.7 VS Code Bridge

Mentioned in the startup guide as a host service on `:3101`. I did
not find an obvious in-repo source for it during the grep pass —
likely an external companion tool (e.g. `@a5af/vscode-bridge`
installed globally per the user-CLAUDE.md). If it routes file-open
commands by workspace path, multi-clone is already differentiated by
path. If it routes by some agentmux-instance identifier, fixing
clone isolation in agentmux will surface a per-clone identifier the
bridge can use.

---

## 2. Why "two clones on the same branch" is the real scenario

The naive expectation is that each agent gets its own branch and
isolation is automatic. In practice:

- A user often has their primary IDE clone on `main` for everyday work.
- An agent's container or host clone is also on `main` until it starts
  a feature branch.
- Two host agents (AgentX, AgentY per CLAUDE.md) might each have their
  own clone, both starting work from `main`.
- An agent reviewing a PR pulls the PR's branch — if two agents both
  review the same PR, they both end up on the same branch.

Same-branch is the steady-state scenario, not the edge case.

---

## 3. Recommended fix

### 3.1 Option A — clone-path hash (recommended)

Extend `RuntimeMode::Dev`:

```rust
pub enum RuntimeMode {
    Installed,
    Portable,
    Dev { branch: String, clone_id: Option<String> },
}
```

When `RuntimeMode::current()` resolves to `Dev`, also resolve the clone
path. The simplest source is `std::env::current_exe()`:

- In `task dev`: the host runs from `dist/cef-dev/agentmux-cef.exe`.
- `current_exe().parent().parent()` is the workspace root.
- Hash that path via the existing `data_dir_hash16` to get a 16-char hex.

Then in `data_paths.rs`:

```rust
if let RuntimeMode::Dev { branch, clone_id } = mode {
    let safe_branch = sanitize_path_segment(branch)?;
    let safe_clone = clone_id
        .as_deref()
        .and_then(sanitize_path_segment)
        .unwrap_or_else(|| "default".to_string());
    let channel = format!("dev-{}-{}", safe_branch, safe_clone);
    let dir = root.join("dev").join(safe_branch).join(safe_clone);
    return Ok((channel, dir));
}
```

Export `AGENTMUX_CLONE_ID` from `to_env_vars()` so the launcher, srv,
and host all observe the same key. The lockfile and pipe re-key
automatically because they derive from `instance_dir`.

**Pros**

- Zero new config files.
- Uses existing FNV-1a infra in `agentmux-launcher/src/hash.rs`.
- Backward compatible: if `clone_id` is `None`, dev mode falls back to
  the current `dev/<branch>/` layout (preserves existing dev sessions).
- Aligns with the channels-design philosophy: identity is path-derived,
  not configured.

**Cons**

- One more dir level in the dev tree (`dev/main/A1B2C3D4.../`). Mildly
  noisier when poking around manually, but acceptable.
- Plumbing through `RuntimeMode` touches every call site that
  constructs `Dev { branch }`. Mostly mechanical — Rust's compiler
  forces the update.

### 3.2 Option B — `.agentmux-clone-id` file

Drop a `.agentmux-clone-id` (gitignored) in each clone root, content
= a stable random 8-char id. Read on every launch.

**Pros**

- Human-readable, can be inspected or reset by the user.
- Doesn't depend on `current_exe()` quirks (e.g. when the exe is run
  from outside `dist/cef-dev/`).

**Cons**

- Requires extra IO per launch.
- User can accidentally edit or commit it.
- Two clones with identical `.agentmux-clone-id` (e.g. cloned by file
  copy) collide silently — same failure mode in disguise.

Option A is the cleaner default. Option B can be a manual override
mechanism if path-based derivation has a corner case we miss.

### 3.3 Out of scope for the fix

- Vite port. Already loud. If multi-clone parallel dev becomes
  routine, add a `AGENTMUX_VITE_PORT` override and let
  `vite.config.ts` honor it. Two-line change but not required for
  isolation — only for *simultaneous* loud-failure-free dev.
- Cargo `target/` deduplication. Significant disk cost. Out of scope.
- VS Code Bridge. Cross-cutting; depends on the bridge's own routing
  model.

---

## 4. Implementation sketch

| Step | File | Approx LOC |
|---|---|---|
| Add `clone_id: Option<String>` to `RuntimeMode::Dev` | `agentmux-common/src/runtime_mode.rs` | 3 |
| Helper `derive_clone_id_from_exe()` (current_exe → workspace root → hash) | `agentmux-common/src/runtime_mode.rs` | 12 |
| Plumb through `RuntimeMode::current()` | same | 4 |
| Update `to_env_vars()` + `from_env()` for `AGENTMUX_CLONE_ID` | `agentmux-common/src/data_paths.rs` | 6 |
| Update `resolve_channel_and_dir()` to nest under clone_id | same | 8 |
| Tests: same-branch-different-clone-paths → distinct dirs; backward-compat when clone_id None | same | 25 |
| `task dev` integration test (optional) | `tools/` or new | n/a |
| Total | | ~58 LOC + tests |

Risk is low because the change is localized to path resolution —
every downstream consumer reads paths from env vars (`AGENTMUX_*_DIR`)
already, so re-keying the instance dir doesn't require touching
launcher / srv / host code.

The first release with the fix should be flagged in `VERSION_HISTORY`
so users who have existing `~/.agentmux/dev/<branch>/` state know it
will be parked in favor of a new `~/.agentmux/dev/<branch>/<clone-id>/`
on first launch (their old state isn't migrated; it's fresh per
clone — which is exactly the point).

---

## 5. References

- `agentmux-common/src/runtime_mode.rs` — `RuntimeMode::Dev` enum
- `agentmux-common/src/data_paths.rs:450-457` — dev path resolution
- `agentmux-common/src/data_paths.rs:240-264` — `to_env_vars()`
- `agentmux-launcher/src/hash.rs:42-51` — `data_dir_hash16`
- `agentmux-launcher/src/ipc/mod.rs:37-39` — pipe name construction
- `agentmux-srv/src/main.rs` — srv listener binds (`127.0.0.1:0`)
- `agentmux-srv/src/backend/lan_discovery.rs:71` — mDNS service name
- `Taskfile.yml:622`, `vite.config.ts:114-115` — Vite `--strictPort`
- `docs/specs/SPEC_DATA_CHANNELS_2026_05_24.md` — channels-design
  precedent for path-keyed isolation
- CLAUDE.md `### Multiple Instances Run in Parallel` — design
  intent statement
