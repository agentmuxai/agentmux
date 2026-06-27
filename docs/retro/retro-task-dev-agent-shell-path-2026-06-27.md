# Retro: `task dev` via Agent MCP Shell — Four Consecutive Failures on Windows

**Date:** 2026-06-27
**Severity:** Medium — blocked agent-driven dev loop; user had to run manually
**Observed by:** lzop-06239 (Claude agent) during zoom-persistence fix session
**Related retros:** RETRO_BASHWRAP_STALE_BUNDLE_2026_06_13, retro-task-dev-isolation-multi-agent-2026-06-23

---

## TL;DR

An agent session attempted to launch `task dev` four times via the MCP Shell tool
so the user could test a fix without switching to a terminal. All four failed, each
for a distinct but related reason rooted in the same structural gap: **the agent's
shell environment is not the same as the user's Git Bash terminal.**

Server log (`agentmuxsrv-v0.49.5.log.2026-06-27`) confirmed all four via
`shell.exit` events with `line_count: 2` or `line_count: 53` — both signatures of
immediate failure before cargo even started. Diagnosis used `muxlog ls` to confirm
a dev instance eventually did come up (user ran it themselves in Git Bash).

---

## What Happened

Session lzop-06239 was implementing zoom-persistence for agent panes (PR #1810).
After pushing the fix, the agent attempted to start `task dev` so the user could
test immediately, without needing to open a terminal. Four shell attempts were
made via `mcp__agentmux__Shell`. All failed. The user eventually ran `task dev`
themselves in Git Bash.

### Attempt 1 — `bash --login -c '... task dev ...'`

**Shell ID:** `6b31042b-2086-4bfd-a417-1bc1ba868690`
**Lifetime:** ~20 ms | **Exit:** 1 | **Lines:** 2

```
cmd: "C:/Program Files/Git/bin/bash.exe" --login -c
     'cd /c/.../agentmux && task dev TITLE="zoom-fix: PR #1810"'
```

**Root cause:** MSYS2 bash does not auto-resolve `.cmd` files without their
extension. On the system PATH, `task` exists only as `task.cmd` (a Windows batch
file at `C:\Systems\node-v22.13.0-win-x64\task.cmd`). When bash evaluates `task`,
it looks for an ELF binary or shell script named `task` — finding nothing, it
exits with "command not found" (exit 1, 2 lines of output: the error message and
a newline).

The `--login` flag made things slightly worse by sourcing `.bash_profile` (adding
latency and potentially altering PATH unpredictably) but the core failure was the
`.cmd` extension issue, present with or without `--login`.

---

### Attempt 2 — `task.cmd dev` (no PATH augmentation)

**Shell ID:** `54ece475-4737-4b12-9b9c-a649ef8aff66`
**Lifetime:** ~400 ms | **Exit:** 200 | **Lines:** 53

```
cmd: C:\Systems\node-v22.13.0-win-x64\task.cmd dev TITLE="zoom-fix: PR #1810"
```

The agent switched to calling `task.cmd` directly (which the MCP Shell spawns via
`cmd.exe`). go-task started and printed ~53 lines — its preamble and the first
failing step — then exited with 200 (go-task's "task failed" exit code).

**Root cause:** `build:host:windows` in the Taskfile calls:

```bash
bash -c 'cargo build --release -p agentmux-cef … || { bash scripts/repair-cef-extract.sh … }'
```

go-task invokes this via cmd.exe. cmd.exe looks for `bash.exe` in the **Windows
registry PATH** — not in the MCP Shell's inherited env. The production Windows
PATH contains only `C:\Program Files\Git\cmd` (the Git Bash launcher shims), not
`C:\Program Files\Git\bin` (where `bash.exe` lives). Result: cmd.exe cannot find
`bash.exe` → "The system cannot find the file specified" → go-task exits 200.

The 53 lines are go-task's task header output plus the cmd.exe error for the
failing `bash -c` line.

---

### Attempt 3 — `task.cmd dev` with env PATH (Unix-style)

**Shell ID:** `8f2a9bdc-fcf8-4d1d-aa55-8a96e78d7c2a`
**Lifetime:** ~300 ms | **Exit:** 200 | **Lines:** 53

Same command as attempt 2, but the agent passed an `env` override to the MCP
Shell containing PATH prefixed with the Git bin directory — in **Unix path format**:

```
PATH = /c/Program Files/Git/bin:/c/Program Files/Git/usr/bin:/c/Systems/node-v22.13.0-win-x64:…
```

**Root cause:** cmd.exe processes Windows paths (`C:\Program Files\Git\bin`),
not POSIX paths (`/c/Program Files/Git/bin`). The env override was formatted
for MSYS2's internal representation — cmd.exe cannot use it for `bash.exe`
lookup. Identical failure to attempt 2: same exit code, same line count.

This was the subtle part: the agent's Bash tool *does* accept `/c/...`-style
paths and works correctly, because the Bash tool's shell is managed by a harness
that bridges MSYS2 ↔ Windows path translation. The raw MCP Shell → cmd.exe
path has no such bridge.

---

### Attempt 4 — `bash.exe -c "export PATH='…' && task dev"`

**Shell ID:** `467a54e3-f4ea-4c42-b404-cfdea8be3d81`
**Lifetime:** ~20 ms | **Exit:** 1 | **Lines:** 2

The agent tried a hybrid: spawn `bash.exe` explicitly, set PATH inside bash (to
include `/c/Systems/node-v22.13.0-win-x64`), then call `task dev`.

```bash
"C:\Program Files\Git\bin\bash.exe" -c \
  "export PATH='/c/Program Files/Git/bin:…:/c/Systems/node-v22.13.0-win-x64:…:$PATH' \
   && cd /c/.../agentmux && task dev TITLE='zoom-fix: PR #1810'"
```

**Root cause:** Same as attempt 1. Even with `/c/Systems/node-v22.13.0-win-x64`
on PATH (which contains `task.cmd`), MSYS2 bash will not execute `task.cmd` when
invoked as bare `task`. The `.cmd` extension must be specified explicitly. Without
it: "command not found", exit 1, 2 lines.

A secondary issue: `$PATH` inside the single-quoted export string is not expanded
(single quotes in bash suppress variable expansion), so the existing PATH was not
appended — only the newly listed directories were active. This didn't change the
outcome (the needed directories were listed explicitly) but was an additional
latent bug.

---

## Diagnosis Method

The four failures were diagnosed retrospectively using the server log:

```
muxlog srv grep "shell"   # or grep directly:
grep "shell\." ~/.agentmux/logs/agentmuxsrv-v0.49.5.log.2026-06-27
```

Each shell left a `shell.create` / `shell.spawn` / `shell.exit` triplet. The
`line_count` field was the key signal:

| Pattern | Meaning |
|---------|---------|
| `line_count: 2, exit_code: 1` | bash "command not found" — 2 lines (error + newline) |
| `line_count: 53, exit_code: 200` | go-task printed its preamble then hit `bash` not found in cmd.exe |
| lifetime < 500 ms | Cargo never started — build didn't even begin |

`muxlog ls` confirmed that a dev instance *did* eventually appear — on the
`fix-ts-errors` branch — after the user ran `task dev` themselves in Git Bash.

---

## Root Cause Summary

Two independent but compounding gaps:

### Gap 1: MSYS2 bash ignores `.cmd` extension in bare command lookup

MSYS2's bash resolves commands by scanning PATH for files matching the bare name
(ELF binaries, shell scripts). It does **not** try `.cmd`, `.bat`, or other Windows
PATHEXT extensions for executables in dynamically-added directories. `task.cmd`
on a POSIX PATH entry is invisible to `task` as a bare command.

The Bash tool works around this because the harness bakes a PATH that includes the
Git Bash interop directories where PATHEXT resolution is pre-handled. A fresh
`bash.exe -c` subprocess doesn't inherit that interop.

### Gap 2: cmd.exe (spawned by go-task) uses Windows registry PATH, not the agent's env

When `task.cmd` runs, it spawns cmd.exe. cmd.exe resolves `bash.exe` from the
Windows registry PATH — specifically `HKCU\Environment` and `HKLM\SYSTEM\...\Environment`.
The production Windows PATH includes `C:\Program Files\Git\cmd` (shim launchers)
but not `C:\Program Files\Git\bin` (where `bash.exe` lives). An env override on
the MCP Shell call propagates only as far as the first process — cmd.exe may
spawn further child processes that re-read the registry PATH.

Passing Unix-style paths in env doesn't help because cmd.exe can't translate them.

---

## Historical Chain

This episode is **distinct** from prior `task dev` / bash / PATH retros, but
shares the same root structural tension: Windows has two bash-resolution domains
(MSYS2 bash and cmd.exe) that don't automatically bridge.

| Retro | Date | Problem | Fix |
|-------|------|---------|-----|
| RETRO_CEF_BUILD_RACE_2026_04_24 | Apr 2026 | Windows Defender races CEF extraction → `cargo build` gets `ERROR_ACCESS_DENIED` | `repair-cef-extract.sh` + Taskfile retry |
| 2026-05-11-live-log-streaming-wrapper-failures | May 2026 | `agentmux-bashwrap` resolve failed on Windows (ConPTY handle closure pattern) | `tokio::process::Command` + `Stdio::piped()`; document fallback paths |
| RETRO_BASHWRAP_STALE_BUNDLE_2026_06_13 | Jun 2026 | Dev builds didn't populate `tools/bin/`; agents resolved stale portable's bashwrap (exit-130) | Bundle freshly-built `agentmux-bashwrap.exe` into dev's `runtime/tools/bin/` |
| retro-task-dev-isolation-multi-agent-2026-06-23 | Jun 2026 | Second `task dev` in same session failed: Windows EXE lock on re-run | Timestamp-stamp `dist/cef-dev-<epoch>` per run |
| **This retro** | Jun 2026 | Agent MCP Shell cannot reliably launch `task dev` on Windows: MSYS2 bash ignores `.cmd`, cmd.exe PATH missing `bash.exe` | See below |

The recurring theme: **every layer (agent shell, go-task, cargo, CEF extraction)
has a distinct PATH or process-spawn context on Windows**, and fixes at one layer
don't propagate to others.

---

## What Worked (and Why)

The **Bash tool** in the agent worked correctly because:
- Its shell is spawned and managed by the Claude Code harness
- The harness pre-configures PATH with MSYS2 interop including PATHEXT bridging
- `task` → `task.cmd` resolution works in that context

The **user's Git Bash terminal** worked correctly because:
- Git Bash's login environment sets up full PATHEXT interop
- `C:\Program Files\Git\bin` is in the shell's PATH via `/etc/profile.d/` scripts
- go-task can find `bash.exe` because the terminal's PATH inherited the full env

---

## Correct MCP Shell Invocation (for future reference)

To invoke `task dev` from an MCP Shell on Windows, two things must both be true:

1. **Use `task.cmd` explicitly** (not bare `task`) inside any bash subprocess
2. **Pass a Windows-style PATH** in the env override so cmd.exe can find `bash.exe`:

```
cmd:  "C:\Program Files\Git\bin\bash.exe" -c "task.cmd dev TITLE='...'"
env:  { "PATH": "C:\\Program Files\\Git\\bin;C:\\Program Files\\Git\\usr\\bin;C:\\Systems\\node-v22.13.0-win-x64;..." }
```

Or, invoke directly through `bash.exe` and call `task.cmd` by extension:

```
cmd:  "C:\Program Files\Git\bin\bash.exe" -c "export PATH=...; task.cmd dev"
```

---

## Prevention / Follow-ups

### Immediate

- When an agent needs `task dev` running for testing, prefer asking the user to
  run it in their own terminal. It takes 5 seconds and avoids this entire class of
  failure. Agent-launched dev servers are best-effort on Windows.

### Longer term

- **Add a `task dev:agent` wrapper** that handles Windows PATH setup internally:
  a thin `.cmd` or PowerShell script that prepends `Git\bin` before invoking the
  Taskfile, so agents can call it with a single cmd.exe-friendly path.
  
- **Add a `dev:running?` health-check task** that exits 0 if a dev Vite is
  already up on the expected port, 1 otherwise — agents can probe before launching.

- **Document in CLAUDE.md**: the correct MCP Shell invocation pattern and the note
  that bare `task` fails in MSYS2 bash without extension.

- **`muxlog` as first diagnostic tool**: the `shell.exit` triplet with `line_count`
  and `exit_code` in the server log is a reliable, fast way to classify MCP Shell
  failures post-hoc. `muxlog srv grep shell` is the first command to run when an
  agent-spawned shell shows as "failed" in the activity dock.
