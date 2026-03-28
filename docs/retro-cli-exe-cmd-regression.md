# Retro: Claude CLI Reverted to npm Install (`.exe` → `.cmd` regression)

**Date:** 2026-03-28
**Commit that introduced it:** `cf1710f` — _fix(windows): prefer .cmd wrapper for Node.js CLI detection_
**Symptom:** Claude CLI shows `(unknown)` version; auth browser opens but CLI invocation broken; full package install ignored in favour of npm

---

## What Happened

### Before `cf1710f`
`cli_bin` (the versioned copy destination) was `<bin_dir>/claude.exe` on Windows.
Fast-path: copies `~/.local/bin/claude.exe` → versioned dir as `.exe`.
`make_cli_cmd("claude.exe")` → runs directly as native PE. Works.

### After `cf1710f`
`cli_bin` was changed to `<bin_dir>/claude.cmd` on Windows (to fix npm-based providers).
Fast-path: now copies `~/.local/bin/claude.exe` → versioned dir as `claude.cmd`.
The file on disk is a PE binary with a `.cmd` filename.
`make_cli_cmd("claude.cmd")` → routes through `cmd.exe /C claude.cmd`.
`cmd.exe` tries to parse a PE binary as a batch script → fails.
`get_cli_version` returns `"unknown"`. CLI is effectively broken.

### Why It Was a Silent Failure
- The versioned dir **file exists** (`claude.cmd`) so Step 1 (already installed check) returns early
- The path is logged as `[local install]` — looks healthy
- Only symptom visible to users: `(unknown)` version and broken CLI invocation

---

## Root Cause

`cf1710f` correctly observed that npm-installed CLIs on Windows are `.cmd` batch wrappers,
and hardcoded the versioned bin path to always use `.cmd`. But this assumption is wrong for
`claude` (Claude Code), which is a native Node.js binary installed by the Claude Code
installer at `~/.local/bin/claude.exe` — **not a `.cmd` batch wrapper**.

The fix for codex/gemini (npm → `.cmd`) conflated **all** Windows CLI targets.

---

## Affected Paths

| Provider  | Install method          | Real extension | Broken as |
|-----------|-------------------------|---------------|-----------|
| `claude`  | Full package installer  | `.exe`        | `.cmd`    |
| `codex`   | npm install             | `.cmd`        | `.cmd` ✓  |
| `gemini`  | npm install             | `.cmd`        | `.cmd` ✓  |

---

## Fix

`cli_bin` must respect the **source extension**, not assume `.cmd`.

Two separate versioned paths on Windows:
- `<bin_dir>/claude.exe` — for native-exe providers (detected via known_paths or `where`)
- `<bin_dir>/claude.cmd` — for npm-installed providers (produced by npm install step)

The fast-path copy should determine the destination filename based on the source extension:

```rust
// Derive destination extension from the source binary
let dest_ext = std::path::Path::new(source)
    .extension()
    .and_then(|e| e.to_str())
    .unwrap_or("exe");
let cli_bin_for_copy = format!("{}/{}.{}", bin_dir, cmd.cli_command, dest_ext);
```

`cli_bin` (the "already installed?" check) needs to check **both** extensions:
```rust
let cli_bin_exe = format!("{}/{}.exe", bin_dir, cmd.cli_command);
let cli_bin_cmd = format!("{}/{}.cmd", bin_dir, cmd.cli_command);
```

`npm_bin` (npm install output) stays as `.cmd` — npm always produces `.cmd` on Windows.

---

## Lesson

When adding support for a new install method (npm → `.cmd`), the existing install method
(full package → `.exe`) must be kept in parallel. A single `cli_bin` variable cannot
serve both; they need separate paths or the copy destination must mirror the source extension.
