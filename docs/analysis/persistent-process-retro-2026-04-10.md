# Persistent Process Mode — Debug Retro (2026-04-10)

**Goal:** Replace per-turn subprocess with persistent long-running CLI process
using `--input-format stream-json` for bidirectional communication.

**Spec:** `docs/specs/persistent-process-mode.md`
**Implementation:** `agentmux-srv/src/backend/blockcontroller/persistent.rs`

---

## Issues Found (chronological)

### 1. System-scanning CLI resolver copies .exe as .cmd (PR #327, v0.33.77)
- **Symptom:** Agent pane subprocess exits with code 1 immediately
- **Root cause:** Resolver found `~/.local/bin/claude.exe` (Bun-bundled binary),
  copied it to `bin/claude.cmd`. The `.cmd` parser (`parse_cmd_wrapper`) couldn't
  parse a PE binary, fell back to `cmd.exe /C` which crashed.
- **Fix:** Removed entire system-scanning copy path. All providers use npm install
  exclusively to `node_modules/.bin/`.
- **Lesson:** Never copy system binaries — version isolation requires independent installs.

### 2. npm install gate on install_cmd string (PR #328, v0.33.78)
- **Symptom:** "claude not found and npm install is not configured for this provider"
- **Root cause:** Resolver checked `install_cmd.contains("npm install")`. Claude's
  provider had an official installer string, not "npm install", so it skipped npm
  entirely.
- **Fix:** Gate on `npm_package` field instead — all providers with a package name
  get npm installed.
- **Lesson:** Don't branch on string matching of config values; use explicit type fields.

### 3. persistent.rs used raw Command::new() instead of make_cli_cmd() (PR #329, v0.33.79)
- **Symptom:** Process spawned as `cmd.exe` (pid visible in tasklist), no stdout
  output, exits with code 1.
- **Root cause:** `persistent.rs` used `Command::new(&config.cli_command)` which
  on Windows spawns `cmd.exe /C claude.cmd`. `subprocess.rs` already used
  `make_cli_cmd()` which parses `.cmd` → `node <script>`. Stdin/stdout piping
  through `cmd.exe /C` doesn't reliably work for persistent processes.
- **Fix:** One-line change: `Command::new()` → `make_cli_cmd()`.
- **Lesson:** Any new controller that spawns CLI processes must use the shared
  `make_cli_cmd()` helper, not raw `Command::new()`.

### 4. Stale portables built before fix commits (v0.33.79, v0.33.80)
- **Symptom:** Portable shows correct version but doesn't have the fix
- **Root cause:** `task cef:package:portable` was run on the branch before the
  fix commit was pushed. The version bump happened first, then the fix, but the
  binary was compiled from pre-fix code.
- **Fix:** Rebuild v0.33.80 from main after all PRs merged.
- **Lesson:** Always verify the fix is in the built binary, not just in source.
  `git log --oneline -1` before building.

### 5. No stderr capture — blind to exit code 1 cause (v0.33.80)
- **Symptom:** Process spawns, stdout reader finishes in ~40ms, exit code 1. No
  error info because stderr was `Stdio::null()`.
- **Root cause:** Original `persistent.rs` set `stderr(Stdio::null())` with a
  comment about SIGPIPE/EPIPE. On Windows there's no SIGPIPE — the real effect
  was hiding all error messages.
- **Fix:** Changed to `Stdio::piped()` with a background tokio task draining
  stderr lines to `tracing::warn!`.
- **Lesson:** Never null stderr on a process you need to debug. Pipe + drain.

### 6. Windows canonicalize() returns \\?\C:\... — Node.js can't parse it (v0.33.81)
- **Symptom:** `Error: EISDIR: illegal operation on a directory, lstat 'C:'`
- **Root cause:** `parse_cmd_wrapper` calls `resolved.canonicalize()` which on
  Windows returns `\\?\C:\Users\...` (UNC extended-length path). Node.js
  `realpathSync` in `run_main` can't handle the `\\?\` prefix and interprets
  `C:` as a directory.
- **Fix:** Strip `\\?\` prefix after `canonicalize()`:
  `if path_str.starts_with(r"\\?\") { path_str = path_str[4..].to_string(); }`
- **Lesson:** Always strip Windows UNC prefix when passing paths to external
  programs (Node, Python, etc.). Rust's `canonicalize()` is the only stdlib
  function that produces these paths.

### 9. Stale instances stacking up — port file collisions (v0.33.86-91)
- **Symptom:** "Sent new-window request to existing instance" — new versions
  refuse to launch, or open windows in a broken older instance.
- **Root cause:** Test instances (.86, .87, .88, .89) were never closed between
  builds. Each version left an `ipc-port` file in its data dir. New launches
  connected to the stale port and delegated, or opened a window in a broken
  instance whose frontend was from a different branch.
- **Fix:** Discipline — always kill the previous test instance before launching
  the next. Only `.78` (our running instance) and ONE test instance at a time.
- **Lesson:** `tasklist | grep agentmux | grep -v "0.33.78"` before every launch.
  Clean up stale port files. Never let instances stack up.

### 10. "Failed to load AgentMux frontend" — stale instance windows (v0.33.86-91)
- **Symptom:** Multiple windows show "Failed to load AgentMux frontend" with
  `ERR_CONNECTION_REFUSED` to `localhost:5173`.
- **Root cause:** These were windows from stale .86/.89 instances that were never
  closed. The "Sent new-window request to existing instance" mechanism opened
  new windows in these broken instances. The frontend couldn't load because
  `frontend/index.html` wasn't found relative to the exe in those builds
  (branch mismatch or build artifact issue).
- **Fix:** Killed all stale instances. The .88 build that was properly launched
  worked fine — frontend loaded correctly.
- **Lesson:** "Failed to load" errors during testing are usually stale instances,
  not build bugs. Close everything and retry with a clean state.

### 11. agent.open creates block but pane not visible in UI (v0.33.88-89)
- **Symptom:** `agent.open` returns success, `agent.list` shows the agent, but
  no pane appears in the UI.
- **Root cause:** `create_block` only adds the block to the tab's `blockids` and
  stores it in the database. The layout tree (`LayoutState.rootnode`) is a
  separate data structure that defines what the frontend renders. Without a
  layout node, the block is invisible.
- **Fix:** After `create_block`, insert a layout node into the tab's `LayoutState`:
  - Empty tab: set as sole root node
  - Existing panes: wrap old root + new leaf in a row container
  - Update `leaforder` array
  - Broadcast layout + tab + block updates via event bus
- **Lesson:** The layout tree is the source of truth for what's visible. Creating
  a block without a layout node is like adding a row to a database table without
  updating the UI's data source. The frontend's `createBlock()` uses
  `layoutModel.treeReducer(InsertNode)` which handles all the edge cases
  (splitting, focusing, flex sizing). The backend layout manipulation is simpler
  but functional.

---

## Architecture Decisions

### Why npm install over system copy
- Version isolation: each AgentMux version gets its own CLI install
- No binary type confusion (.exe vs .cmd vs node script)
- Reproducible: `npm install @anthropic-ai/claude-code@latest` always produces
  a proper `node_modules/.bin/claude.cmd` wrapper

### Why make_cli_cmd() over Command::new()
- On Windows, npm `.cmd` wrappers must be parsed to extract the node entry script
- `cmd.exe /C` drops arguments and breaks stdin/stdout piping for long-running processes
- `make_cli_cmd()` centralizes this logic in `agentmux-common` for all crates

### Why persistent over per-turn subprocess
- No process startup latency per message
- Session continuity without `--resume`
- Mid-turn interruption (send while streaming)
- Simpler state machine (no respawn logic)

---

## Files Modified

| File | Changes |
|------|---------|
| `agentmux-srv/src/server/cli_handlers.rs` | Removed system scanner (-219 lines), npm gate fix |
| `agentmux-srv/src/backend/blockcontroller/persistent.rs` | make_cli_cmd, stderr capture, args logging |
| `agentmux-common/src/cli.rs` | Shared make_cli_cmd + .cmd parser |
| `frontend/app/view/agent/providers/index.ts` | Claude → controllerType: "persistent" |
| `frontend/app/view/agent/agent-model.ts` | persistentLaunchArgs routing |
| `frontend/app/view/agent/agent-view.tsx` | Loading spinner clears on failure |
