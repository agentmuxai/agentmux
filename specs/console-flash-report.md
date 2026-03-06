# Console Window Flash Report

## Problem

When clicking a provider button (Claude/Codex/Gemini) in the Agent pane, a console window briefly flashes on screen before disappearing. This is a deal-breaker UX issue on Windows.

## Root Cause

On Windows, `std::process::Command` spawns child processes with a visible console window by default. The fix is to set the `CREATE_NO_WINDOW` (0x08000000) creation flag via `CommandExt::creation_flags()`. This flag is **missing** from multiple subprocess calls in the click-to-connect flow.

## Affected Code Paths

When a user clicks a provider button, the following subprocess calls happen in sequence:

### 1. `get_cli_path` → `detect_on_system_path()` in `cli_installer.rs:72-86`

Runs `where <binary>` to check if the CLI is on PATH.

```rust
// MISSING creation_flags — flashes a console
let output = std::process::Command::new(find_cmd)
    .arg(bin_name)
    .output()
    .ok()?;
```

**Status:** NO `CREATE_NO_WINDOW` flag set.

### 2. `check_cli_auth_status` → `check_claude_auth()` in `providers.rs:320-323`

Runs `claude auth status --json` to check if the user is logged in.

```rust
// MISSING creation_flags — flashes a console
let output = std::process::Command::new(cli_cmd)
    .args(["auth", "status", "--json"])
    .output()
    .map_err(|e| format!("Failed to run `{cli_cmd} auth status`: {e}"))?;
```

**Status:** NO `CREATE_NO_WINDOW` flag set.

### 3. Same issue in `check_codex_auth()` (line 352) and `check_gemini_auth()` (line 368)

### 4. `detect_cli()` in `providers.rs:94` and `providers.rs:109`

Runs `where <name>` and `<name> --version` — both missing the flag.

### 5. `verify_cli()` in `cli_installer.rs:228`

Runs `<binary> --version` after install — missing the flag.

## Already Fixed

The following calls in `cli_installer.rs` DO have `CREATE_NO_WINDOW`:

- `install_claude_native()` line 125: `cmd.creation_flags(0x08000000)`
- `install_via_npm()` line 204: `cmd.creation_flags(0x08000000)`

## Fix

Every `std::process::Command` that is NOT a user-visible PTY session needs `creation_flags(0x08000000)` on Windows. This requires:

1. `use std::os::windows::process::CommandExt;` (already imported in `cli_installer.rs`, needs adding to `providers.rs`)
2. Add `.creation_flags(0x08000000)` to every `Command` builder, gated with `#[cfg(windows)]`

### Files to Fix

| File | Functions | # of Command spawns |
|------|-----------|-------------------|
| `src-tauri/src/commands/cli_installer.rs` | `detect_on_system_path()`, `verify_cli()` | 2 |
| `src-tauri/src/commands/providers.rs` | `detect_cli()` (x2), `check_claude_auth()`, `check_codex_auth()`, `check_gemini_auth()` | 5 |

**Total: 7 subprocess calls missing the flag.**

## Pattern

To avoid this in the future, consider a helper function:

```rust
fn hidden_command(program: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd
}
```

Then all background subprocess calls use `hidden_command("where")` instead of `Command::new("where")`.
