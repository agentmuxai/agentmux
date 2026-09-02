# Spec: Cross-Shell Bootstrap for CLI Isolation

**Status:** Ready for implementation
**Date:** 2026-03-08
**Fixes:** Bootstrap script injected wrong syntax (bash into PowerShell)

## Problem

The bootstrap script in `bootstrap.ts` uses `getApi().getPlatform()` to pick between PowerShell and bash syntax. This fails because:

1. **Platform !== shell** — Windows can run pwsh, powershell, cmd.exe, bash (WSL/Git Bash), or zsh
2. **Remote backends** — an SSH connection from Windows to Linux runs bash, not PowerShell
3. **No recovery** — if the wrong syntax is injected, the user sees a parser error and is stuck

The first attempt injected `[ -x "$CLI_BIN" ]` (bash) into PowerShell, producing:
```
ParserError: Missing type name after '['.
```

## Goal

A single bootstrap mechanism that:
1. **Detects the actual shell** running in the PTY (not just the OS)
2. **Works in all shells**: pwsh, powershell 5.1, cmd.exe, bash, zsh, sh
3. **Recovers from errors** — if detection fails, falls back gracefully
4. **Shows install progress** in the terminal (user sees everything)
5. **Supports all platforms**: Windows, macOS, Linux, remote SSH backends

## Shell Detection

### Where the shell is chosen

The shell controller (`shell.rs`) picks the shell at PTY spawn time:

**Windows priority:**
1. `pwsh` (PowerShell 7) — if `where pwsh` succeeds
2. `powershell` (PowerShell 5.1) — if `where powershell` succeeds
3. `cmd.exe` — fallback

**Unix priority:**
1. User's `$SHELL` environment variable
2. `/bin/bash` fallback

### How to detect at injection time

The frontend doesn't know which shell was actually spawned. Two approaches:

**Approach A: Backend reports the shell (recommended)**

When the shell controller starts, it already knows which shell it spawned (it runs the detection logic). Store this in block meta:

```rust
// shell.rs — after detecting the shell
meta.insert("shell:type".to_string(), shell_type.into());  // "pwsh" | "powershell" | "cmd" | "bash" | "zsh" | "sh"
```

The frontend reads `block.meta["shell:type"]` before building the bootstrap script.

**Approach B: Polyglot script**

Write a single script that is valid in both PowerShell and bash/sh. This is fragile and hard to maintain, but avoids the need for shell detection.

**Approach C: Shell-probe injection**

Before injecting the bootstrap, inject a probe command that prints the shell type:
```
echo $PSVersionTable.PSVersion 2>$null || echo "bash:$BASH_VERSION" || echo "sh"
```
Then parse the PTY output to determine the shell. This is complex and timing-sensitive.

**Decision: Approach A** — backend reports the shell. It's the most reliable and simplest.

## Bootstrap Scripts Per Shell

### PowerShell (pwsh / powershell 5.1)

```powershell
$d="$HOME/.agentmux/instances/v<VERSION>/cli/<PROVIDER>"; $b="$d/node_modules/.bin/<BINARY>.cmd"; if(!(Test-Path $b)){Write-Host "Installing <DISPLAY_NAME>..."; npm install --prefix $d <NPM_PACKAGE>@<PINNED_VERSION> --no-fund --no-audit}; <ENV_PREFIX>& $b <ARGS>
```

Notes:
- Works in both pwsh 7 and powershell 5.1
- `$HOME` resolves correctly in both
- `.cmd` extension required for npm bin shims on Windows
- `& $b` invokes the resolved path (required for paths with spaces)

### cmd.exe

```cmd
@set "d=%USERPROFILE%\.agentmux\instances\v<VERSION>\cli\<PROVIDER>" && @set "b=%d%\node_modules\.bin\<BINARY>.cmd" && @if not exist "%b%" (echo Installing <DISPLAY_NAME>... && npm install --prefix "%d%" <NPM_PACKAGE>@<PINNED_VERSION> --no-fund --no-audit) && <ENV_PREFIX>"%b%" <ARGS>
```

Notes:
- `%USERPROFILE%` instead of `$HOME`
- Backslashes for paths (cmd doesn't handle forward slashes well in all contexts)
- `@` prefix suppresses echo of each command
- `if not exist` instead of `Test-Path`

### bash / zsh / sh (Unix + WSL + Git Bash)

```bash
CLI_DIR="$HOME/.agentmux/instances/v<VERSION>/cli/<PROVIDER>" && CLI_BIN="$CLI_DIR/node_modules/.bin/<BINARY>" && { [ -x "$CLI_BIN" ] || { echo "Installing <DISPLAY_NAME>..." && npm install --prefix "$CLI_DIR" <NPM_PACKAGE>@<PINNED_VERSION> --no-fund --no-audit; }; } && <ENV_PREFIX>"$CLI_BIN" <ARGS>
```

Notes:
- `[ -x "$CLI_BIN" ]` checks existence and executable bit
- Works in bash, zsh, and POSIX sh
- No `.cmd` extension (Unix binaries)
- Forward slashes

## Implementation

### 1. Backend: Report shell type in block meta

**File:** `agentmuxsrv-rs/src/backend/blockcontroller/shell.rs`

After detecting the shell (in `start_shell_proc` or equivalent), write the shell type to block meta:

```rust
// After shell detection
let shell_type = if shell_path.contains("pwsh") {
    "pwsh"
} else if shell_path.contains("powershell") {
    "powershell"
} else if shell_path.contains("cmd") {
    "cmd"
} else if shell_path.contains("zsh") {
    "zsh"
} else if shell_path.contains("bash") {
    "bash"
} else {
    "sh"  // conservative fallback
};

// Write to block meta so frontend can read it
// (via SetMeta RPC or direct meta update)
```

The meta key `shell:type` is readable by the frontend via `useAtomValue(model.blockAtom)`.

### 2. Frontend: Read shell type and build correct script

**File:** `frontend/app/view/agent/bootstrap.ts`

Replace the `isWindows` boolean with the actual shell type:

```typescript
type ShellType = "pwsh" | "powershell" | "cmd" | "bash" | "zsh" | "sh";

interface BootstrapOptions {
    version: string;
    provider: ProviderDefinition;
    shellType: ShellType;
    args: string[];
}

export function buildBootstrapScript(opts: BootstrapOptions): string {
    const { shellType } = opts;

    switch (shellType) {
        case "pwsh":
        case "powershell":
            return buildPowerShellBootstrap(opts);
        case "cmd":
            return buildCmdBootstrap(opts);
        case "bash":
        case "zsh":
        case "sh":
        default:
            return buildBashBootstrap(opts);
    }
}
```

### 3. Frontend: Wait for shell type before injecting

**File:** `frontend/app/view/agent/agent-model.ts`

Instead of a blind 500ms `setTimeout`, wait for the `shell:type` meta key to appear:

```typescript
// After ControllerResyncCommand, poll for shell:type
const waitForShellType = async (blockId: string, maxWaitMs = 3000): Promise<ShellType> => {
    const start = Date.now();
    while (Date.now() - start < maxWaitMs) {
        const block = globalStore.get(WOS.getWaveObjectAtom<Block>(`block:${blockId}`));
        const shellType = block?.meta?.["shell:type"];
        if (shellType) return shellType as ShellType;
        await new Promise((r) => setTimeout(r, 100));
    }
    // Fallback: guess from platform
    return getApi().getPlatform() === "win32" ? "pwsh" : "bash";
};
```

This replaces the `setTimeout(async () => { ... }, 500)` pattern with a more robust polling approach that:
- Waits for the backend to report the actual shell
- Falls back to platform-based guess after 3 seconds
- Still works if the backend doesn't set the meta key (backwards compatible)

### 4. Error recovery

If the bootstrap script fails (e.g. npm not found, network error), the user sees the error in the terminal and can:
1. Fix the issue (install npm, check network)
2. Press Enter and re-run the command (it's in shell history)
3. Click the provider button again (spawns a new shell with fresh bootstrap)

No special error recovery code is needed — the terminal itself is the recovery mechanism.

### 5. Environment cleanup per shell

| Shell | Unset env var | Example (CLAUDECODE) |
|-------|--------------|----------------------|
| pwsh / powershell | `$env:VAR=$null` | `$env:CLAUDECODE=$null` |
| cmd | `set "VAR="` | `set "CLAUDECODE="` |
| bash / zsh / sh | `unset VAR` | `unset CLAUDECODE` |

## Edge Cases

| Scenario | Behavior |
|----------|----------|
| Shell type not reported (old backend) | Falls back to platform guess (win32 → pwsh, else → bash) |
| Git Bash on Windows | Backend reports "bash" → bash script injected (correct) |
| WSL shell | Backend reports "bash"/"zsh" → Unix script (correct) |
| SSH to Linux from Windows | Remote shell is bash/zsh → Unix script (correct, $HOME is remote) |
| SSH to Windows from macOS | Remote shell is pwsh/cmd → Windows script (correct) |
| npm not installed | Error visible in terminal, user can fix and retry |
| PowerShell 5.1 execution policy | `npm` and `Test-Path` don't require scripts — just inline commands |
| cmd.exe with spaces in path | Paths wrapped in `"%d%"` quotes |
| Shell env var overrides (SHELL=/bin/fish) | Backend detects fish → should fall back to sh-compatible syntax |

## Files Modified

| File | Change |
|------|--------|
| `agentmuxsrv-rs/src/backend/blockcontroller/shell.rs` | Write `shell:type` to block meta after shell detection |
| `frontend/app/view/agent/bootstrap.ts` | Three script builders (PowerShell, cmd, bash), `ShellType` parameter |
| `frontend/app/view/agent/agent-model.ts` | Wait for `shell:type` meta instead of blind setTimeout |

## Testing Matrix

| OS | Shell | Expected Script | Test |
|----|-------|----------------|------|
| Windows | pwsh 7 | PowerShell | Click provider → installs via npm, launches CLI |
| Windows | powershell 5.1 | PowerShell | Same |
| Windows | cmd.exe | cmd | Same (with %USERPROFILE% paths) |
| Windows | Git Bash | bash | Same (Unix-style paths) |
| macOS | bash | bash | Click provider → installs, launches |
| macOS | zsh | bash | Same (zsh-compatible) |
| Linux | bash | bash | Same |
| SSH (Linux) | bash | bash | Remote install, remote $HOME |

## Verification

1. Launch AgentMux on Windows (default pwsh)
2. Open agent pane, click Claude (raw) → PowerShell bootstrap runs, installs, launches
3. Open another agent pane, verify the `shell:type` meta is set to "pwsh"
4. Change default shell to cmd.exe, repeat → cmd bootstrap runs correctly
5. Connect to Linux SSH backend, click provider → bash bootstrap runs on remote
