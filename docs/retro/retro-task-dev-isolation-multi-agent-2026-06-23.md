# Retro: `task dev` Launch Failure — Orphaned Dev Process Within Same Agent Session

**Date:** 2026-06-23  
**Severity:** Medium — subsequent `task dev` run fails if a prior one was not cleanly terminated  
**Observed by:** Mazs during strip-redesign implementation session

---

## What Happened

Two `task dev` runs were attempted in the same agent session (`mazs-0527n`). The second failed:

```
task: Failed to run task "dev:serve": coreutils:: open
  dist/cef-dev/agentmux-launcher.exe: The process cannot access the file
  because it is being used by another process.
```

A `pwsh Stop-Process -Id <launcher-pid> -Force` was required before the second run could proceed.

---

## Initial Misdiagnosis

The first failure was attributed to a *different agent* sharing the same clone. This was wrong.

Each agent in the fleet has **its own clone**:

```
/c/Users/asafe/.agentmux/agents/mazs-0527n/agentmux    ← my clone
/c/Users/asafe/.agentmux/agents/korp-0620g/agentmux    ← korp's clone
/c/Users/asafe/.agentmux/agents/smike-06122/agentmux   ← smike's clone
...
```

Data dirs, Vite ports, and `dist/cef-dev/` are all **naturally isolated** because each clone has a unique filesystem path:
- Data dir: `~/.agentmux/dev/<branch>/<hash-of-clone-root>/`
- Vite port: `cksum(CLONE_ROOT) % 200 + 5173` — deterministic per path, different per clone
- `dist/cef-dev/`: inside each clone's own working tree

The "other dev instance" (`dev:main`, v0.48.2, hash `3eaacaa32634b401`) was a **different agent's clone** at a different path — no shared resources at all.

---

## Actual Root Cause: Orphaned Dev Process Within My Own Session

The sequence:

1. First `task dev` was run as a background task (`b002d81bg`). Output file showed 0 bytes; task was assumed to have failed silently.
2. The build actually succeeded. The launcher launched and stayed running.
3. Second `task dev` was run. `dev:serve` tried to wipe and rebuild `dist/cef-dev/`, hit the Windows file lock on the running `agentmux-launcher.exe`, and exited 201.

The 0-byte output file was a red herring — the bashwrap background task wrapper collects output differently; the process ran to completion and stayed alive even though the wrapper reported "failed."

---

## Why This Happens on Windows

On Linux/macOS, a running process holds a file descriptor to its own executable but a new `cp -f` succeeds by replacing the inode — the old process keeps reading from the old inode. On Windows, the OS locks the EXE image for the lifetime of the process; `cp -f` (or `Copy-Item -Force`) cannot overwrite a running EXE.

`dev:serve` has a stale-Vite reaper for the port collision case, but has no equivalent for a locked launcher EXE.

---

## Impact

- ~4 minutes of redundant Rust recompile (second build succeeded before hitting the lock).
- Required manual process discovery and `Stop-Process`.
- No data loss; no state corruption across agents.

---

## Fix Options

### Short-term (pre-flight in dev:serve)

Before the `rm -rf "$DEV_DIR"` step, check if the launcher is running and terminate it:

```bash
if [ "windows" = "windows" ]; then
  pid="$(tasklist /FI "IMAGENAME eq agentmux-launcher.exe" ... | ...)"
  # verify it's OUR clone's launcher (CommandLine check)
  # if so: taskkill /PID $pid /T /F
fi
```

This mirrors the existing stale-Vite reaper logic and applies the same ownership guard (CommandLine must reference `$CLONE_ROOT`) to avoid killing a different agent's launcher.

### Medium-term (agent self-cleanup)

When an agent's tool-call session ends, automatically run a cleanup that terminates any background `task dev` processes spawned during that session. The bashwrap job tracking already has PIDs; expose a `cleanup` command.

### Documentation

Add a note to CLAUDE.md: when running `task dev` as a background task, the launcher stays alive after the task command returns. Before re-running `task dev`, verify no prior dev process is holding the launcher: `pwsh -Command "Get-Process | Where-Object { \$_.Modules | Where-Object { \$_.FileName -like '*cef-dev*' } }"`.

---

## Action Items

- [x] Eliminate EXE lock on re-launch: `dev:serve` now uses a timestamp-stamped `dist/cef-dev-<epoch>` dir on Windows so each run gets a fresh dir and never tries to overwrite the running launcher. No prune loop — `rm -rf` on a live dir partially succeeds (unlocked files deleted, locked DLLs survive), breaking tool paths in the running session; old stamped dirs accumulate until `task clean:host` wipes them. `clean:host` now globs `dist/cef-dev-*`. `exe_dir_is_dev_build` updated to `starts_with("cef-dev")` so the timestamped dir is still classified as Dev. Unix keeps fixed `dist/cef-dev` for stable desktop entry. (Taskfile.yml + agentmux-common/src/runtime_mode.rs, PR #1742)
- [ ] Add CLAUDE.md note about orphaned dev launcher on Windows
- [ ] Investigate why the background-task output file showed 0 bytes while the process ran successfully — possible bashwrap buffering issue
