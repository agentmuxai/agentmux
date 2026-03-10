# Agent Build Diagnostics & Self-Kill Investigation

## TL;DR

**The kill is usually unnecessary.** AgentMux uses version-scoped Tauri identifiers that make every
build a separate app with its own data directory. Multiple instances genuinely coexist. The last
unexpected close (session `1c360dbd`, v0.31.100 build) was caused by an agent following a **wrong
rule in MEMORY.md** that said to always run `taskkill /IM agentmux.exe /F` before building. That
rule has been removed.

---

## Why Multiple Instances Work

Each production build gets a unique Tauri app identifier:

| Build | Identifier | WebView2 UDF |
|-------|-----------|--------------|
| `task dev` | `ai.agentmux.app.dev` | `%LOCALAPPDATA%\ai.agentmux.app.dev\` |
| v0.31.74 | `ai.agentmux.app.v0-31-74` | `%LOCALAPPDATA%\ai.agentmux.app.v0-31-74\` |
| v0.31.100 | `ai.agentmux.app.v0-31-100` | `%LOCALAPPDATA%\ai.agentmux.app.v0-31-100\` |

The dev config override (`src-tauri/tauri.dev.conf.json`) contains only:
```json
{ "identifier": "ai.agentmux.app.dev" }
```

WebView2 enforces single-process access per UDF. Different identifiers → different UDFs → no conflict.
Backend instances also namespace under `instances/v{version}/`, preventing SQLite collisions.

## Why the Kill Is Not Needed for Builds

When a user runs the portable build, `agentmux.exe` is in their **extracted folder** (e.g., `Downloads/agentmux-0.31.99-x64-portable/`). The Cargo linker writes to `target/release/agentmux.exe` in the repo — **a completely different file**. No Windows file lock conflict.

`package-portable.ps1` then copies from `target/release/` into a new zip. Also fine.

**The only scenario requiring a kill:** the user runs agentmux.exe directly from `target/release/` (unusual, typically only during manual testing).

---

## What Caused the Unexpected Close (Session 1c360dbd)

The agent followed an incorrect rule in `MEMORY.md`:
> "Kill running agentmux.exe before repackaging: `cmd //c "taskkill /IM agentmux.exe /F"`"

This rule was added early in development before instance isolation existed. It is now **wrong** and
has been **removed from MEMORY.md**.

The agent explicitly ran:
```
taskkill //IM agentmux.exe //F
```
...killing the host process it was running inside.

---

## When a Kill IS Legitimately Needed

Only in these cases:
1. **Cargo link fails with `access denied` on `agentmux.exe`** — means the process being built is the running one (user launched from `target/release/`). Ask user to close that instance.
2. **Repackaging the same version over itself** — copying a new `agentmux.exe` on top of a running one would fail. But version bumping before every build means this can't happen in normal workflow.

In both cases: **confirm with user before killing, never do it silently.**

---

## Diagnostics: What to Add

The real gap isn't prevention — it's that the user couldn't tell what happened after the fact.

### 1. Pre-kill audit log (if kill ever IS needed)

Before any explicit `taskkill`, write to `~/.agentmux/agent-build-log.jsonl`:
```json
{
  "timestamp": "2026-03-10T14:23:01Z",
  "event": "pre_kill",
  "agent_id": "AgentA",
  "reason": "Cargo link failed: access denied on agentmux.exe",
  "build_version": "0.31.100",
  "branch": "agenta/drag-drop-files"
}
```
AgentMux should read this on next startup and show: _"AgentA killed AgentMux at 14:23 to build v0.31.100"_

### 2. Self-detection guard

Before any kill, check env vars to detect if running inside AgentMux:
```bash
if [ -n "$AGENTMUX_BLOCKID" ]; then
  echo "⚠ About to kill the AgentMux session you are running in."
  echo "  Confirm? (this will close your current session)"
  # Do NOT proceed automatically — wait for user confirmation
fi
```

### 3. Post-build notification

After a successful build, log a `build_complete` event to the same file so the next startup
can display: _"v0.31.100 was built successfully at 14:31 by AgentA."_

---

## Can Agents Log Before Killing?

Yes. A wrapper script `scripts/agent-kill.sh` could enforce this:

```bash
#!/usr/bin/env bash
# Usage: ./scripts/agent-kill.sh "reason" "version"
REASON="${1:-no reason given}"
VERSION="${2:-unknown}"
LOG="$HOME/.agentmux/agent-build-log.jsonl"
mkdir -p "$(dirname "$LOG")"
printf '{"timestamp":"%s","event":"pre_kill","agent_id":"%s","reason":"%s","build_version":"%s","branch":"%s"}\n' \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  "${AGENTMUX_AGENT_ID:-unknown}" \
  "$REASON" "$VERSION" \
  "$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)" >> "$LOG"

if [ -n "$AGENTMUX_BLOCKID" ]; then
  echo "⚠ WARNING: Killing AgentMux (your current session). Logged to $LOG"
fi
cmd //c "taskkill /IM agentmux.exe /F" 2>/dev/null || true
```

Agents should be instructed to use this script instead of raw `taskkill`. The key is that the
log file persists on disk even after the process is gone.

---

## Summary of Changes Made

| Change | Status |
|--------|--------|
| Removed incorrect "always kill before build" rule from MEMORY.md | Done |
| Added correct rule: "do NOT kill unless Cargo explicitly fails, confirm with user" | Done |
| This spec documents root cause and prevention | Done |
| `scripts/agent-kill.sh` wrapper with audit logging | Proposed — implement if kill is ever needed |
| AgentMux startup notification reading build log | Proposed — UI feature |
