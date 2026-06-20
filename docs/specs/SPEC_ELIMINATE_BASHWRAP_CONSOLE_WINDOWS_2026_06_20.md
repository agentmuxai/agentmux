# SPEC: Eliminate Transparent Console Windows on Windows

**Date:** 2026-06-20
**Status:** Draft
**Scope:** Windows only. Two Rust changes, no frontend changes.

---

## 1. Problem

On Windows, every Bash tool call and every persistent Shell tool execution
spawns a visible transparent/blank console window that accumulates in the
taskbar and on the screen. After a session with many tool calls, dozens of
orphaned windows remain even after the shells finish.

**Why they appear:**  
Windows auto-creates a visible console window for any process in the
`IMAGE_SUBSYSTEM_WINDOWS_CUI` (console) subsystem **unless** the spawner
passes `CREATE_NO_WINDOW` (`0x08000000`) in `CreateProcess`. By default,
Rust binaries compiled without `#![windows_subsystem = "windows"]` are
console-subsystem apps, so every spawn produces a window.

---

## 2. Two Spawn Sites

### 2.1 `agentmux-bashwrap` — Bash tool hook

`agentmux-bashwrap exec ...` is spawned by Claude Code's PreToolUse hook
for **every Bash tool call** in every agent pane. It streams the tool output
back to the WPS broker and to Claude's tool result. Each exec lives for the
duration of the tool call, then exits — but its console window stays until
the process is explicitly waited on. Orphaned windows accumulate if the
parent (Claude Code) doesn't clean up promptly.

**Fix:** Add `#![windows_subsystem = "windows"]` to
`agentmux-bashwrap/src/main.rs`. This changes the PE subsystem from
`CUI` → `GUI`, so Windows never creates a console window when spawning it.
The binary doesn't use a console at all (it communicates via HTTP/WPS and
stdout pipe, not a terminal), so the subsystem change is safe and correct.

### 2.2 `ShellNodeRunner` — Shell tool backend

`shell_node.rs` spawns `cmd /C <command>` (Windows) without
`CREATE_NO_WINDOW`. Any Shell tool call — `task dev`, `ping`, `npm run
build` — creates a visible cmd.exe console window for the duration of the
shell's lifetime.

**Fix:** Add `creation_flags(0x08000000)` (CREATE_NO_WINDOW) to the
`tokio::process::Command` in `ShellNodeRunner::run()` on Windows. The
process's stdout/stderr are already piped (`Stdio::piped()`), so no console
I/O is lost — the window was doing nothing anyway.

---

## 3. Changes Required

### 3.1 `agentmux-bashwrap/src/main.rs` — add Windows subsystem attribute

```rust
// Add at the very top of the file, before any `use` statements:
#![cfg_attr(windows, windows_subsystem = "windows")]
```

`cfg_attr` limits the attribute to Windows only so Unix builds are
unaffected (Unix has no concept of console vs. GUI subsystem).

**Why not `#![windows_subsystem = "windows"]` directly?**  
The bare attribute would cause a linker warning on Linux/macOS. `cfg_attr`
is the canonical conditional approach.

### 3.2 `agentmux-srv/src/backend/shell_node.rs` — add CREATE_NO_WINDOW

```rust
// After `child_cmd.stdin(Stdio::null());`, add:
#[cfg(windows)]
{
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    child_cmd.creation_flags(CREATE_NO_WINDOW);
}
```

The `CommandExt` trait from `std::os::windows::process` provides
`creation_flags()` on Windows only — no-op on other platforms.

---

## 4. What Does NOT Change

- No behavior change on Linux/macOS.
- `agentmux-bashwrap` still writes to stdout (Claude reads it as tool
  result) and HTTP (WPS streaming). Neither requires a console.
- `ShellNodeRunner` still captures stdout/stderr via pipes. The shell
  process runs correctly — it just has no window.
- PTY-backed terminal panes (`ShellController`) are unaffected — ConPTY
  already owns the terminal allocation for those.
- `agentmux-srv` itself, `agentmux-cef`, `agentmux-launcher` — untouched.

---

## 5. Implementation Steps

1. Edit `agentmux-bashwrap/src/main.rs` — add `#![cfg_attr(windows, windows_subsystem = "windows")]` at line 1.
2. Edit `agentmux-srv/src/backend/shell_node.rs` — add `creation_flags(CREATE_NO_WINDOW)` after `stdin(Stdio::null())`.
3. `task build:backend` — rebuilds both `agentmux-srv` and `agentmux-bashwrap`.
4. `task dev` — verify: run a Bash tool call and a Shell tool call; confirm no new console windows appear.
5. Add changeset: `patch "fix(win32): suppress console windows for bashwrap and persistent shell spawns"`.

**Total diff:** ~8 lines across 2 files.

---

## 6. Cleanup of Existing Orphans

Existing orphaned windows must be killed manually once (they predate the
fix). From any terminal OUTSIDE AgentMux:

```
taskkill /F /IM agentmux-bashwrap.exe
```

After the fix ships, no new windows will appear, so this is a one-time
cleanup.
