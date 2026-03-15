# Agent Pane -> Terminal Switch Spec

## Overview

When a user clicks a provider button (Claude, Gemini, Codex) in the agent pane,
the pane converts into an embedded terminal running that provider's CLI.
No external windows, no custom renderers — just xterm.js with a PTY.

## Current Architecture

### Block lifecycle (block.tsx:257)

```
blockData.meta.view changes -> Block component detects mismatch
  -> disposes old ViewModel -> creates new ViewModel -> re-renders
```

- `view: "agent"` -> AgentViewModel -> provider selection UI
- `view: "term"` -> TermViewModel -> xterm.js terminal

### Shell controller spawn logic (shell.rs:348-398)

Three code paths based on meta values:

| Path | Condition | What happens |
|------|-----------|-------------|
| **Direct spawn** | `cmd` non-empty AND (`cmd:args` non-empty OR `cmd:interactive`) | `CommandBuilder::new(cmd)` with args |
| **Shell-wrapped** | `cmd` non-empty, no args, not interactive | Windows: `cmd.exe /C <cmd>` / Unix: `sh -c <cmd>` |
| **Interactive shell** | `cmd` empty | Detects pwsh/bash, injects shell integration |

### Windows CLI resolution problem

npm-installed CLIs on Windows have three files:
- `gemini` — Unix shell script (NOT executable on Windows)
- `gemini.cmd` — Windows batch shim (works with cmd.exe)
- `gemini.ps1` — PowerShell shim

`CommandBuilder::new("gemini")` (direct spawn) resolves to the Unix shell script
via PATH and fails with "os error 193: not a valid Win32 application."

`cmd.exe /C gemini` works because cmd.exe searches for `.cmd`/`.bat` extensions
automatically.

## Design

### Step 1: Click provider -> SetMetaCommand

When user clicks a provider button, set these meta keys on the block:

```json
{
  "view": "term",
  "controller": "cmd",
  "cmd": "<cli_command>",
  "cmd:args": [],
  "cmd:interactive": false,
  "cmd:runonstart": true
}
```

Key decisions:
- **`view: "term"`** — triggers React to swap AgentViewModel for TermViewModel
- **`controller: "cmd"`** — runs a command (not interactive shell)
- **`cmd:interactive: false`** — forces the **shell-wrapped** path, so
  `cmd.exe /C gemini` properly resolves `.cmd` shims on Windows, and
  `sh -c gemini` works on Unix
- **`cmd:args: []`** — empty; all args go through the shell wrapper
- **`cmd:runonstart: true`** — ensures the command runs on controller start

### Step 2: ControllerResyncCommand

After SetMetaCommand completes, call `ControllerResyncCommand` with
`forcerestart: true`. The backend:

1. Loads block from store (now has `view: "term"`, `cmd: "gemini"`, etc.)
2. Creates/resets ShellController
3. Opens ConPTY (Windows) or pty (Unix)
4. Spawns `cmd.exe /C gemini` (Windows) or `sh -c gemini` (Unix)
5. PTY I/O flows to FileStore -> WPS events -> frontend

### Step 3: React re-render

The Block component (block.tsx:257) detects `meta.view` changed from `"agent"`
to `"term"`:

1. Disposes AgentViewModel (cleans up subscriptions)
2. Creates TermViewModel for this blockId
3. TermViewModel initializes xterm.js, connects to PTY file subject
4. Terminal renders the CLI output

### What the user sees

1. Agent pane with three provider buttons
2. Click "Gemini"
3. Pane smoothly transitions to a terminal
4. Terminal shows Gemini CLI (interactive TUI with colors, cursor, etc.)
5. User can type, interact, authenticate — all within the pane

## Provider CLI commands

| Provider | `cmd` value | Notes |
|----------|------------|-------|
| Claude | `claude` | Heavy TUI (Ink/React), high CPU is expected |
| Gemini | `gemini` | Standard Node.js TUI |
| Codex | `codex` | Standard Node.js TUI |

All three work identically through the shell-wrapped path. No special handling
per provider.

## CLI resolution by platform

### Windows (shell-wrapped path)
```
cmd.exe /C gemini
  -> cmd.exe searches PATH for gemini.cmd
  -> runs: node ... @google/gemini-cli/dist/index.js
```

### macOS/Linux (shell-wrapped path)
```
/bin/sh -c gemini
  -> sh searches PATH for gemini
  -> runs: #!/bin/sh script -> node ... @google/gemini-cli/dist/index.js
```

### Locally-installed CLIs (~/.agentmux/cli/<provider>/)

If the CLI is installed locally by AgentMux, the full path is used:
- Windows: `cmd.exe /C "C:\Users\x\.agentmux\cli\gemini\node_modules\.bin\gemini.cmd"`
- Unix: `/bin/sh -c "/home/x/.agentmux/cli/gemini/node_modules/.bin/gemini"`

The shell-wrapped path handles both bare commands and full paths.

## Terminal behavior after switch

Once the pane is a terminal:

- **Header**: Shows "gemini" (from `cmd` meta) via TermViewModel.viewText
- **Icon**: Terminal icon (from TermViewModel.viewIcon)
- **Exit handling**: When CLI exits, terminal shows exit code in header
  (green check for 0, red X for non-zero)
- **No auto-close**: `cmd:closeonexit` defaults to false — pane stays open
  so user can see output and restart
- **Restart**: Terminal's existing restart button works (resyncs controller,
  re-runs the command)

## What is NOT in scope

- **Switching back to agent view**: Once terminal, always terminal. User closes
  the pane and opens a new agent pane to pick a different provider.
- **Structured output parsing**: No NDJSON/stream-json parsing. The terminal
  renders whatever the CLI outputs.
- **Auth pre-checking**: No `check_cli_auth_status` call before launching.
  The CLI itself handles auth prompts in the terminal.
- **CLI installation**: The install flow (cli_installer.rs) is separate.
  This spec only covers what happens after a CLI path is known.

## Code changes required

### frontend/app/view/agent/agent-model.ts

**`connectWithProvider()`**: Strip down to just call `startSession()`.
Remove auth state management, translator creation, header updates —
none of that matters when we're switching to a terminal.

**`startSession()`**: SetMetaCommand with `view: "term"`, `controller: "cmd"`,
`cmd: cliPath`, `cmd:interactive: false`. Then ControllerResyncCommand.
Remove `connectToTerminal()` call — xterm.js handles PTY I/O.

### frontend/app/view/agent/agent-view.tsx

**`handleProviderSelect()`**: Needs to resolve the CLI path before calling
`connectWithProvider()`. Two options:

1. **Simple**: Just pass the bare command name (`"gemini"`). The shell wrapper
   resolves it via PATH. Works if CLI is on system PATH.
2. **With local install**: Call `get_cli_path` Tauri command first to check
   system PATH and local install dir. If not found, call `install_cli`.
   Pass the resolved path to `connectWithProvider()`.

Option 2 is the current approach and should be kept for the install flow.
But the actual terminal launch only needs the command string.

### Dead code to clean up (future)

After this change, the following become unused in agent-model.ts:
- `rawOutputAtom` handling
- `handleTerminalData()` — xterm.js reads the PTY directly
- `handleCliEvent()` — no structured parsing
- `ClaudeCodeStreamParser` import
- `translator` field and `createTranslator()` calls
- `connectToTerminal()` / `disconnectTerminal()` — raw file subject subscription
- Auth state atoms and login flow

These should be removed in a follow-up cleanup, not in the initial fix.

## Edge cases

### CLI not on PATH
If the CLI isn't installed, `cmd.exe /C gemini` will fail with
"'gemini' is not recognized as an internal or external command."
This shows up in the terminal as an error message — the user sees it clearly.
The install flow should prevent this case.

### CLI crashes immediately
The terminal shows the error output. The exit code appears in the header.
User can close and retry.

### ConPTY not available (older Windows)
portable-pty falls back to winpty. Should work but may have rendering quirks.
Not a new issue — same as regular terminals.

### Multiple provider clicks before meta update completes
The `SetMetaCommand` is async. If the user clicks rapidly, multiple SetMeta
calls could race. In practice the last one wins, and the ControllerResync
with `forcerestart: true` kills any existing process. Not a real concern.
