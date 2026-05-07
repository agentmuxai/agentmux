# Dev-Build Env Isolation

**Status:** Implemented (PR #717)
**Date:** 2026-05-06
**Owner:** AgentX

## Problem

Running `task dev` from inside an existing AgentMux terminal pane silently
routes the new dev build's data dir to the **parent** instance's
version-isolated directory — so the new code never actually runs in
isolation.

### Concrete failure observed

Parent: AgentMux portable v0.33.669 running on the host.
Child: `task dev` invoked from inside one of v0.33.669's terminal panes.
Child's compiled-in version: 0.33.680.

Logs from the child host:

```
AgentMux host starting           version="0.33.680" ...
Initializing CEF browser process version="0.33.680"
                                 runtime_mode=Some("portable")
                                 data_dir=C:\Users\asafe\.agentmux\versions\0.33.669\cef-cache
...
CEF early exit (process singleton or similar) — exiting cleanly exit_code=24
Opening in existing browser session.
```

The child resolved its `cef-cache` dir to **0.33.669**'s path, collided
with CEF's process-singleton lock on that dir, and forwarded its open
request to the running 0.33.669 window. The user's "see the new code"
loop silently no-ops.

### Why version isolation didn't kick in

The post-#695/#696 unification routes data via:

```
DataPaths::from_env()                 ← env-provided paths win
  .or_else(|| {
      RuntimeMode::current(exe_dir)   ← path-based fallback
      DataPaths::resolve(version, &mode)
  })
```

The child inherited `AGENTMUX_DATA_DIR=...versions/0.33.669/data` (and
the rest of the `AGENTMUX_*` family) from the parent process's env.
`DataPaths::from_env()` returned `Some` with the parent's stale paths,
short-circuiting the path-based detection that would have selected
`RuntimeMode::Dev { branch }` and routed to `~/.agentmux/dev/<branch>/`.

`RuntimeMode::current` itself has the same blind spot: step 1 honors
`AGENTMUX_RUNTIME_MODE` from env *before* checking the exe path, so even
if `from_env()` had returned `None`, an inherited `AGENTMUX_RUNTIME_MODE`
value would override path detection.

## Constraint

`task dev` is run routinely from inside an AgentMux session — agents
running in panes are expected to iterate on the codebase via `task dev`
without first closing the host they're embedded in. We can't assume a
clean environment.

We cannot ask agents to kill or restart the host they're running inside
(see `CLAUDE.md` "Host Process Safety").

## Resolution

**Dev builds never inherit `AGENTMUX_*` env vars from a parent process.**
Path-based detection is authoritative when the binary lives in a
recognized dev-output directory.

### Public API additions (`agentmux-common`)

```rust
// runtime_mode.rs
impl RuntimeMode {
    /// Path-only detection — skips the AGENTMUX_RUNTIME_MODE env step.
    pub fn current_path_only(exe_dir: &Path) -> Self { ... }
}

/// True when this binary lives in a known dev-build output directory
/// (`dist/cef-dev/`, `target/debug/`, `target/release/`).
pub fn is_dev_build_exe(exe_dir: &Path) -> bool { ... }
```

`current_path_only` mirrors `current`'s priority order minus step 1
(env override): portable marker → dev exe path → installed.

### Call-site changes

**`agentmux-cef/src/main.rs`** (boot path):

```rust
let common_paths = if agentmux_common::is_dev_build_exe(&host_exe_dir) {
    let mode = RuntimeMode::current_path_only(&host_exe_dir);
    DataPaths::resolve(version, &mode).ok()
} else {
    DataPaths::from_env().or_else(|| {
        let mode = RuntimeMode::current(&host_exe_dir);
        DataPaths::resolve(version, &mode).ok()
    })
};
```

**`agentmux-cef/src/sidecar.rs`** (`spawn_backend` / `restart_backend`):
Same guard — even on the restart path, a dev host re-derives paths from
its own exe location, never trusting inherited env.

**`agentmux-launcher/src/data_dir.rs`** (`resolve_paths`):
Same guard for symmetry — when the launcher itself is a dev build,
ignore env override.

The installed/portable paths are unchanged. The launcher remains
authoritative there: it computes paths once and propagates them to host
+ srv via `to_env_vars()`.

## Why this is safe

- **Dev binaries are never the launcher target on installed/portable
  systems.** `dist/cef-dev/`, `target/debug/`, `target/release/` are
  developer-only locations; an installed user's binaries live under the
  portable extract or the OS install dir, neither of which trips
  `is_dev_build_exe`.
- **The path check is structural, not heuristic.** It walks ancestors
  for an exact `parent_name="dist", name="cef-dev"` (or `target/debug`,
  `target/release`) match. An installed binary whose path *happens* to
  contain the substring "cef-dev" elsewhere would not match.
- **Portable marker still wins for portable layouts.** A portable
  build dropped into a path that also matches `target/release` is still
  detected as portable because `is_portable_marker_present` is checked
  first inside `current_path_only`.
- **The host still receives correct paths to forward to srv.**
  `DataPaths::to_env_vars()` is called on the resolved paths, so the
  spawned `agentmux-srv` inherits dev paths regardless of what env the
  host itself was invoked with.

## Behavioral summary

| Build        | Parent env present | Resolved data dir              |
| ------------ | ------------------ | ------------------------------ |
| Installed    | yes (launcher)     | env-provided (unchanged)       |
| Installed    | no                 | path detection → installed     |
| Portable     | yes (launcher)     | env-provided (unchanged)       |
| Portable     | no                 | marker → portable, version dir |
| **Dev**      | **yes (parent)**   | **path-only → `dev/<branch>/`** ← FIX |
| Dev          | no                 | path-only → `dev/<branch>/`    |

## Verification

After rebuild, launch `task dev` from inside an existing AgentMux pane.
Expected: log line shows
`data_dir=...\.agentmux\dev\<git-branch>\cef-cache`, NOT
`...\versions\<parent-version>\cef-cache`. A fresh CEF window opens
(no process-singleton collision because the dev cache dir is its own).

## Related

- #695 — `RuntimeMode` + `DataPaths` primitives
- #696 — switch launcher/host/srv onto the new primitives
- `docs/specs/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md` — the original
  unification spec; this is a follow-up.
