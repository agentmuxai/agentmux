# CLI Wrapper Resolution — Status Report

**Date:** 2026-04-08
**Issue:** agentmuxai/agentmux#314
**Related branches:** `agenta/codex-gemini-fixes-v2`, `agenta/fix-cli-exe-cmd-windows`
**Session:** `a912e4d7` (v0.33.73 instance, Apr 8 04:18–10:15 UTC)

---

## Background

AgentMux installs AI CLIs (Claude Code, Codex, Gemini) into per-version isolated directories (`~/.agentmux/<version>/cli/<provider>/`) via npm. On Windows, npm produces `.cmd` batch wrappers (e.g., `claude.cmd`) that call `node cli.js %*`. These wrappers are invoked from Rust via `std::process::Command`.

## The Problem: `cmd.exe /C .cmd` Is Broken

When Rust spawns `.cmd` files, it wraps them with `cmd.exe /C <path>.cmd <args>`. This pattern is **fundamentally broken** when combined with piped stdio:

1. **Arguments get dropped** — `cmd.exe /C claude.cmd --version` prints the Windows banner instead of the version. The `--version` arg is silently lost.
2. **No output captured** — with `Stdio::piped()`, `cmd.exe` captures its own banner but the actual CLI never runs.
3. **Browser doesn't open** — `claude auth login` never executes, so the OAuth flow never starts.
4. **`CREATE_NO_WINDOW` + `Stdio::null()` double-blocks** — the CLI process can't produce any visible output.

## Work Done in Session `a912e4d7`

The agent (running in v0.33.73 portable) made significant progress across versions v0.33.66 through v0.33.73:

### Phase 1: CLI Resolver Rewrite (v0.33.66–67)
- **Removed the system-scanning anti-pattern** — previously the resolver would scan system PATH, find CLIs, and copy them to the versioned dir. This defeated isolation and produced version-mismatched binaries.
- **All providers now use npm-only install** — `npm install --prefix <versioned_dir> @anthropic-ai/claude-code@latest` (or `@openai/codex@latest`, etc.)
- **Removed `windowsInstallCommand` and `installCommand` from provider configs** — no more provider-specific install scripts
- **Removed `bin/` candidate path** — only `node_modules/.bin/` is checked now, eliminating stale artifacts from the old copy era
- **Net: -356 lines, +106 lines** in `cli_handlers.rs`

### Phase 2: `.cmd` Wrapper Fix (v0.33.68–70)
- **Auth login**: Resolved `.cmd` → `node cli.js` directly in `platform.rs` (CEF host auth handler), bypassing `cmd.exe /C` entirely
- **Version detection**: Fixed `get_cli_version()` to pass args correctly for `.cmd` files
- **Browser open**: Added `start <url>` (Windows) / `xdg-open` (Linux) fallback when CLI can't open browser itself
- **Auth URL capture**: Changed from stderr to stdout capture (Claude prints URL to stdout)
- **Frontend**: Auth URL now displayed in agent view log with clickable link

### Phase 3: UI Polish (v0.33.71)
- **Processing indicator**: Added pulsing dot/bar in agent view footer when the agent is thinking (tracks `controllerstatus` events)
- **Codex/Gemini readiness**: Provider configs verified complete, same npm resolver works for all three

### Filed: Issue #314
Documents the full `cmd.exe /C` problem, all affected code paths, and the proposed centralized fix via a `make_cli_cmd` rewrite.

## Current State: INCOMPLETE

### What Works
- CLI resolution via npm install to versioned dir
- Claude Code version detection (via `node cli.js --version` bypass)
- Auth login browser open + URL display (for Claude, via `node cli.js` bypass)
- Processing indicator in agent view

### What's Still Broken (Issue #314)
- **`make_cli_cmd` in `cli_handlers.rs` (sidecar)** still uses `cmd.exe /C` for `.cmd` files — this affects the **subprocess launch** path (not just auth). The agent pane spawns the CLI via the sidecar's `ControllerResync`, which calls `make_cli_cmd`. If the `.cmd` wrapper doesn't pass args correctly, the agent session itself may not work.
- **Fix is only applied in two spots** (CEF host auth login + version detection). The sidecar's `make_cli_cmd` needs the same treatment — resolve `.cmd` → `node <entry_script>` centrally.
- **Codex and Gemini untested** — the npm install path should work, but the `.cmd` launch path hasn't been verified for these providers.

### Proposed Centralized Fix (from Issue #314)
Rewrite `make_cli_cmd` to detect `.cmd` wrappers and resolve to the underlying `node cli.js` invocation:

```rust
fn make_cli_cmd(cli_path: &str) -> Command {
    if cli_path.ends_with(".cmd") {
        // Parse the .cmd file to find the node entry script
        // e.g., claude.cmd contains: @node "cli.js" %*
        // Spawn: node <dir>/cli.js <args>
        let entry = parse_cmd_wrapper(cli_path);
        let mut cmd = Command::new("node");
        cmd.arg(entry);
        cmd
    } else {
        Command::new(cli_path)
    }
}
```

## Branch Status

| Branch | Base | Status | Contains |
|--------|------|--------|----------|
| `agenta/codex-gemini-fixes-v2` | v0.32.92 | **Stale** — based on old main, do not merge | Early Codex/Gemini work |
| `agenta/fix-cli-exe-cmd-windows` | v0.32.104 | **Stale** — based on old main, do not merge | Auth URL display, exe/cmd fix |
| (session work) | v0.33.65 main | **Not on a named branch** — changes are in dev builds v0.33.66–73 | Full rewrite |

**Important:** The session's work (v0.33.66–73) was done via `task dev` hot-reload iterations. The commits exist locally but may not be on a pushed branch. The agent built incrementally (v0.33.66 → 67 → 68 → 69 → 70 → 71 → 73) with each version fixing issues found in the previous one.

### To recover the work:
```bash
# Check which local branches have the latest commits
git log --all --oneline --since="2026-04-08T04:00:00" | head -20

# Or check the reflog for the session's commits
git reflog --since="2026-04-08T04:00:00" | head -30
```

## Recommendations

1. **Recover the session's commits** — find the final state (v0.33.73) on whatever local branch the agent left it on, or cherry-pick from reflog
2. **Rebase onto current main** (v0.33.65) — the CLI resolver rewrite is the big win
3. **Implement the centralized `make_cli_cmd` fix** per Issue #314 — one function, all paths fixed
4. **Test all three providers end-to-end** — Claude, Codex, Gemini: install → auth → launch → send message → receive response
5. **PR the work** once the centralized fix is in and all three providers pass

## Related Files

| File | Role |
|------|------|
| `agentmux-srv/src/backend/cli_handlers.rs` | CLI resolution, npm install, `make_cli_cmd`, version detection |
| `agentmux-cef/src/platform.rs` | CEF host auth login handler (`.cmd` → `node cli.js` bypass) |
| `frontend/app/view/agent/agent-view.tsx` | Launch flow UI, auth URL display, processing indicator |
| `frontend/app/view/agent/agent-model.ts` | Agent model, CLI path computation |
| `frontend/util/cef-api.ts` | IPC bridge for `run_cli_login`, `resolve_cli_command` |
| `frontend/app/store/providers.ts` | Provider definitions (npm packages, args, env vars) |
