# SPEC: Agents drive terminals through MuxAPI: keys, screen, wait, commands, terminal panes

**Status:** active — P1 partly shipped in #5001 with the `Terminal` tool (SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md S1): keys, submit, plain-text reads from a cursor, waits, and `run` with an exit code (§4.7). Remaining: terminal panes (P1), the emulator (P2), shell integration (P3), P4.
**Author:** Agent5@narko, 2026-10-15, at the operator's request
**Trigger (operator, 2026-10-15):** "can u run it through an agentmux terminal? … can we explore deep agent integration into the terminal pane? we want to be able to efficiently drive work there, when it is needed, like now. research best practices online, write a spec to file" … "via the MuxAPI"
**Builds on:** `SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md` (PtyShell; its §5 deferred a rendered screen), `SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md` (Shell), the browser-pane agent model (`SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md`: an agent drives only panes it opened, a "driven by" bar, Take over, handoff).

## 1. Summary

1. **The incident.** An agent needed a real terminal to start `claude --cloud` (the CLI refuses without a TTY). It opened its PtyShell, typed the command and sent `\n`: nothing ran. On Windows the PtyShell is `cmd.exe` under ConPTY, which submits a line only on CR, and the tool sends `text` as raw bytes, so an agent whose tool call carries a literal backslash-r has no way to press Enter at all. It then fell back to opening a visible console window it couldn't read.
2. **What agents get today** (§2): one PTY per agent pane (its shell drawer), raw-byte input, a raw log tail with ANSI escapes, no rendered screen, no way to wait for output or for a command to finish, no exit codes per command, and no access to terminal panes at all.
3. **One contract on every platform** (§4.0): the agent never needs to know whether it is typing into cmd.exe under ConPTY, a macOS or Linux shell, WSL, or a host over SSH. Enter, keys, encodings, line endings, "is it done?" and secret prompts are handled in srv, and each result says which platform and shell it ran on for the rare case the agent needs it.
4. **What this adds**, all through MuxAPI (MCP tools in `agentmux-mcp`):
   - **Keys by name and a submit flag** (§4.1): `Enter` is CR on every platform; Ctrl+C, arrows, Tab, Esc by name.
   - **A rendered screen** (§4.2): a headless terminal emulator per PTY in srv; read the screen as plain text, page the scrollback, get only what changed.
   - **Server-side waits** (§4.3): block until a pattern appears, output goes quiet, the prompt returns, or the command ends, with a timeout. No sleep-and-poll loops.
   - **Commands as records** (§4.4): shell integration (OSC 133) in AgentMux's shell scripts gives each command its output range, duration and exit code; `TerminalRun` types a command and returns its own output and exit code.
   - **Terminal panes** (§4.5): an agent can open a terminal pane and drive it with the same tools; the user sees "driven by" and can take over, as with browser panes.
   - **Safety** (§5): never type secrets, hand a password prompt to the user, treat screen text as untrusted, and keep the user in control of their own terminals.
5. **One `Terminal` tool with an `action`, replacing the six `PtyShell*` tools** (§4.0.1): every tool costs context (a name and a lookup where the CLI defers tools, the whole schema where it doesn't), so the capability goes into actions and parameters, and one call can type, wait and read.
6. **Phases** (§7): P1 fixes the incident and works on the drawer and on terminal panes the agent opens itself (keys, submit, ANSI-stripped and delta reads, a pattern/quiet wait), without an emulator. P2 adds the emulator and screen. P3 shell integration and `TerminalRun`. P4 the user handing their own terminals to an agent, and secret-prompt enforcement.

## 2. What exists (verified 2026-10-15 at `b118943b8`: main plus the browser nav-bar PR #4981, which touches none of these files)

| Piece | State |
|---|---|
| Tools | `Shell`, `ShellInput`, `ShellStatus`, `ShellStop`; `PtyShell`, `PtyShellInput`, `PtyShellResize`, `PtyShellRead`, `PtyShellStatus`, `PtyShellStop`; `ConnList` (`crates/mcp/src/tool_schemas.rs:17-147`, handlers in `tools/shell.rs`, `tools/pty_shell.rs`, routes `crates/srv/src/server/routes.rs:224-241`). |
| PtyShell input | `text` passes through unchanged (`tools/pty_shell.rs:85`), becomes bytes (`http_pty_shell.rs:576`) and is written as is (`blockcontroller/shell/lifecycle.rs:1305-1307`). No key names, no escape handling, no LF→CR. The docs disagree: `PtyShellInput` suggests `"y\n"` for Enter (`tool_schemas.rs:87`), `PtyShell` says `"exit\r"` (`:80`), srv tests send `"\r\n"` (`server/tests.rs:4491`). |
| PtyShell shell | Windows: forced to `cmd.exe` (`http_pty_shell.rs:157-161`) because PSReadLine's `ESC[6n` cursor query hangs with no emulator to answer it; srv answers ConPTY's own startup query once (`:499-527`). Unix: the automatic shell. |
| PtyShell output | Appended to the block's `term` file (`shell/file_ops.rs:99-201`). `PtyShellRead` returns the last N lines of that raw log, ANSI escapes included (`http_pty_shell.rs:659-687`). No emulator, no screen. |
| PtyShell status | Whether the shell process runs, and its exit code (`:690-709`). Nothing about a command inside it. |
| User lock | Each input or resize takes a 4 s lease that blocks the user's typing (`http_shell.rs:291`, `agent_lock.rs`, `server/websocket.rs:1072,1797`). |
| Which terminal | Only the agent pane's own shell drawer (`term:shellsubblockid`, `http_pty_shell.rs:9-23`). |
| Terminal panes | `/api/v1/pane/open` accepts `view:"term"` (`app_api/pane.rs:247-253`), but no MCP tool asks for it. No tool writes to or reads a terminal pane. `RunCommand term:paste` and `PressKeys` act on the focused UI, not a chosen terminal. |
| Shell integration | AgentMux's scripts emit OSC 7 (cwd) and OSC 16162;E (`backend/shellintegration/bash.sh:44,70`; zsh, fish, pwsh alike). The frontend's OSC 16162 handler defines A/C/D (D carries an exit code) but ignores them "until something consumes them" (`frontend/app/view/term/termosc.ts:180-182`). No OSC 133 or 633 anywhere; srv parses no OSC. |
| Shell (pipes) | Exit code known (`backend/shell_node.rs:517-533`), output streamed to the UI as a ShellNode, but the agent has no tool to read that output back (`http_shell.rs:256-271`). |
| Precedents | `BrowserWaitFor`, `BrowserSnapshot`, `BrowserHandoff`, the browser pane's driven-by bar and Take over (`browser-view.scss` `.browser-driven-by`). |

## 3. What the field converged on (research, 2026-10-15)

Agent harnesses and terminal MCP servers have settled on the same small set of ideas:

1. **Send text and keys separately.** Literal text in one call, named keys (`Enter`, `C-c`, arrows, F-keys) in another, as tmux's `send-keys -l` then `send-keys Enter` ([mager.co](https://www.mager.co/blog/2026-08-23-orchestrating-agents-with-tmux/), [tmux for agents](https://blog.philz.dev/blog/tmux-for-agents/)). Terminal-Bench found a model that seldom ended keystrokes with a newline typed 119 of 127 commands without running them ([Terminus-2 notes](https://deepwiki.com/harbor-framework/harbor/4.2-terminus-2)).
2. **Enter is CR.** Windows console hosts end a line only on CR; a lone LF stays in the input buffer, and cmd.exe appends the next write to it. CR is also what a Unix tty expects from a Return key (ICRNL). Send `\r`, keep `\n` inside content ([iterm2-control-mcp #11](https://github.com/dude84/iterm2-control-mcp/issues/11), [sageport #75](https://github.com/joygqz/sageport/pull/75)). One report also measured cmd.exe under ConPTY reading back only 511 of a 3,000-character line, so long input must be chunked or pasted.
3. **Read a rendered screen, not a byte log.** Keep a headless emulator (node-pty + headless xterm, `pyte`) and return the screen as text; page the scrollback; detect the alternate screen so a full-screen program (vim, htop, a TUI) is read as a screen ([termmirror](https://github.com/Ar9av/termmirror), [mcp-ssh-session](https://glama.ai/mcp/servers/@devnullvoid/mcp-ssh-session/blob/f828e4f76f5589615ec724f4af169cb4ff7cfe99/IMPLEMENTATION_SUMMARY.md)). Raw logs full of escape codes are for grep, not for reading ([tmux for agents](https://blog.philz.dev/blog/tmux-for-agents/)).
4. **Wait on the server, with a timeout.** Block until a regex appears, output settles, or the command exits, instead of `sleep 30 && check` loops that burn calls ([pty-mcp](https://github.com/raychao-oao/pty-mcp), [Terminal Use MCP](https://mcpservers.org/servers/computer-agent-labs/terminal-use)).
5. **Don't let the echo match.** A pattern must not match the command the agent just typed ([interactive-terminal-mcp](https://github.com/sairaph/interactive-terminal-mcp)).
6. **"The screen stopped changing" is not "done".** Quiet can mean finished, thinking, or waiting at a prompt; spinners and counters change without progress. Prefer explicit signals, then prompt patterns, then quiet as a last resort ([mager.co](https://www.mager.co/blog/2026-08-23-orchestrating-agents-with-tmux/)).
7. **Use shell integration where it exists.** OSC 133 marks prompt start (A), command start (B), output start (C) and command end with exit code (D); VS Code's OSC 633 adds the exact command line (E) with a nonce so output can't fake it ([VS Code shell integration](https://code.visualstudio.com/docs/terminal/shell-integration), [OSC 133](https://contour-terminal.org/vt-extensions/osc-133-shell-integration/), [Windows Terminal](https://learn.microsoft.com/en-us/windows/terminal/tutorials/shell-integration), [the quiet handshake](https://elyracode.com/blog/the-quiet-handshake-osc-133-and-633)). Treat a bare `D` as finished with unknown status, and keep a fallback for shells without it.
8. **Wait for readiness before typing.** Keystrokes sent before the shell is at a prompt land somewhere else; Claude Code's own tmux backend hit this ([claude-code #25315](https://github.com/anthropics/claude-code/issues/25315)).
9. **Save tokens.** Diff reads (only changed lines), tail limits, and batching input with one read back ([terminal-mcp](https://github.com/mkpvishnu/terminal-mcp), [Terminal Use MCP](https://mcpservers.org/servers/computer-agent-labs/terminal-use)).
10. **Share the terminal with the human.** Warp's Full Terminal Use runs the agent in the same visible PTY as the user, who can watch, steer and take back control; permissions still apply ([Warp docs](https://docs.warp.dev/agent-platform/capabilities/full-terminal-use)). AgentMux already has the pieces: the shell drawer is the user's own visible shell, and browser panes have driven-by and Take over.

## 4. Design

### 4.0 One contract on every platform

**The agent never has to know which OS, console or shell it is typing into.** The same calls behave the same on Windows, macOS and Linux, in WSL, and over SSH (where the remote OS can differ from this machine's). Every platform difference is handled in srv's terminal layer, once, behind the tools:

| Concern | The agent says | srv does |
|---|---|---|
| Submit a line | `submit: true` / `keys: ["Enter"]` | sends CR; never relies on LF reaching Enter (ConPTY ignores a lone LF, a Unix tty maps CR to NL) |
| Named keys | `Ctrl+C`, `Up`, `F5`, … | maps to the right bytes for the terminal's current mode; on Windows, Ctrl+C reaches the program as an interrupt only if it processes input, so srv also sends a console Ctrl event to the foreground process when the byte alone doesn't interrupt (measured per shell in P1) |
| Long or multi-line text | `text` | bracketed paste when the program supports it; otherwise lines with Enter; chunked under the Windows console line limit |
| Output text | read calls | one encoding (UTF-8) in every read: the Windows PtyShell runs with code page 65001 (`chcp 65001`) and srv decodes what it can't; one line ending (`\n`) in what the agent reads, whatever the console wrote |
| What's on screen | `TerminalScreen` | the same emulator renders every platform's output, so ConPTY's repainting and a Unix program's cursor movement read back the same way |
| Is it done? | `TerminalWait`, `TerminalRun` | OSC 133 marks where the shell has AgentMux's integration; otherwise the known prompt for that shell (cmd, PowerShell, bash, zsh, fish, sh); otherwise quiet. Each result says which method decided (`how`) |
| Which shell | nothing, or `shell` when it matters | picks the default per target and reports it: `TerminalOpen` and every result carry `{ platform, shell, connection }`, so an agent that does need shell syntax (`&&` vs `;`, quoting) reads it instead of guessing |
| Secret prompts | nothing | detects them per platform (tty ECHO off on Unix and over SSH; prompt patterns everywhere) and refuses to type, returning `needs_user` |
| Paths | `cwd` | accepts either slash style and the `~` home on every target; converts for WSL (`/mnt/c/…` ↔ `C:\…`) |

Two rules follow:

1. **Nothing platform-specific in a tool's schema or docs.** The PtyShell docs today tell agents to send `\r` here and `\n` there; that kind of advice moves into srv. A tool description may say what a platform can't do (cmd.exe has no exit code per command before P3), never how to work around it.
2. **Every platform is tested.** CI runs the terminal integration tests (§6) on Windows, macOS and Linux runners, plus a WSL and an SSH target where the runner can host one. A behaviour that only one platform's test covers isn't done.

### 4.0.1 Wide in capability, not in tool count

Every MuxAPI tool costs context. Where the CLI defers MCP tools (Claude Code loads a schema only when the agent asks for it with ToolSearch, as it does for AgentMux's 95 tools today), each tool still costs a name on every turn and a lookup on first use; where it doesn't, the whole schema is paid up front (about 88K characters today, the overhead `SPEC_AGENT_TASK_EVAL_2026_10_13.md` measures). So the terminal surface is **one tool, `Terminal`, with an `action`**, the shape of Anthropic's own text-editor (`command`) and computer-use (`action`) tools. It replaces the six `PtyShell*` tools.

| `action` | Does | Replaces |
|---|---|---|
| `open` | open a terminal pane (`cwd`, `connection`, `command`, `split`, `rows`, `cols`), or attach to the agent's drawer (the default when no pane is asked for); returns the terminal's block id and `{ platform, shell, connection }` | `PtyShell` |
| `type` | `text`, `keys`, `submit`, optionally followed in the same call by a `wait` and a `read` | `PtyShellInput`, `PtyShellResize` (`rows`/`cols` on `type` resize first) |
| `read` | `view: "log"` (since a cursor, ANSI stripped), `"screen"` (rendered; `page`, `diff`), `"commands"` (recent command records) | `PtyShellRead` |
| `wait` | `until`: pattern, quiet, prompt, command, exit; a secret prompt ends any wait with `needs_user` and raises the pane's attention (the handoff) | `PtyShellStatus` |
| `run` | waits for a prompt, types `command`, submits, waits for that command's end, returns its own output and exit code | — |
| `close` | close a pane the agent opened; for the drawer or a granted terminal, release it to the user | `PtyShellStop` |

Schema rules:

- **One flat parameter object.** Every parameter is optional and its description names the actions it applies to. No `oneOf`/`anyOf` per action: some providers' function-calling schemas reject or flatten them. srv validates the combination and returns an error that names what applies (`"pattern" applies to action "wait" and to "type" with "wait"`).
- **Compose in one call.** `type` takes `wait: { until, pattern, timeout_ms }` and `read: "screen" | "log"`, so starting a dev server and waiting for it is one call: `{ action: "type", text: "npm run dev", submit: true, wait: { until: "pattern", pattern: "ready on" }, read: "screen" }`. Round trips cost more than tool count: each call is a model turn.
- **Every result carries** `terminal`, `{ platform, shell, connection }`, the `since` cursor, and what decided the outcome (`how`).

Cost: Claude Code permission rules match tool names, so a user can't allow `read` but ask before `type`. AgentMux auto-allows its tools, and srv's own rules (§5: secrets, ownership, Take over) apply whatever the harness allows; this only matters to someone running `agentmux-mcp` under a stricter harness.

The `PtyShell*` names go in the same release (an alias the model can't see helps no agent; one it can see costs what this saves; see `SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md` §6). That spec also folds the pipe `Shell` tools (as `run` with `background: true`) and `ConnList` (as `connections`) into `Terminal`: eleven tools become one.

The sections below describe each capability. Where a heading or example names `TerminalOpen`, `TerminalType`, `TerminalRead`, `TerminalScreen`, `TerminalWait`, `TerminalRun`, `TerminalCommands`, `TerminalHandoff` or `TerminalClose`, read it as the `Terminal` action (and parameter) above.

### 4.0.2 The tools in detail

The `Shell*`, `PtyShell*` and `ConnList` tools are no longer listed; calling one returns an error naming the `Terminal` action to use instead (§4.0.1). Their srv endpoints stay, and `Terminal` calls them.

A **terminal** is a `term` block with a PTY: the agent pane's shell drawer, or a terminal pane. Every tool takes `terminal` (a block id from `TerminalOpen` or `PtyShell`), defaulting to the agent's own drawer.

### 4.1 Input: text, keys, submit (P1)

`TerminalType { terminal?, text?, keys?, submit? }`

- `text` is typed literally. A string with a line break is sent as a bracketed paste when the program asked for it (`ESC[?2004h` seen), so a multi-line script isn't run line by line by accident; otherwise line by line, each ended with Enter.
- `keys` is a list of named keys, sent after `text`: `Enter`, `Tab`, `Esc`, `Backspace`, `Up`/`Down`/`Left`/`Right`, `Home`/`End`, `PageUp`/`PageDown`, `F1`-`F12`, and `Ctrl+<letter>`, `Alt+<key>`, `Shift+Tab`. srv maps each to its bytes, for the terminal's mode (application cursor keys when the program set them). Unknown names are an error, never typed.
- `submit: true` is `keys: ["Enter"]`. **Enter is `\r` on every platform and connection.**
- Long input is chunked below 500 characters per write on Windows (cmd.exe's measured line limit) and written with a short gap, as paste already does.
- The tool returns what was typed (with secrets masked, §5) and the screen's last lines after a short settle (default 300 ms), so a typo or an unexpected prompt shows at once. This is the "batch input, one read back" pattern.

`PtyShellInput` gains the same `keys` and `submit`, and its docs drop the `"y\n"` example. `ShellInput` (pipes) keeps appending `\n`; a pipe isn't a console.

### 4.2 Reading: screen, scrollback, changes (P1 for text, P2 for the screen)

P1, without an emulator: `TerminalRead { terminal?, since?, tail_lines? }` returns the log **with ANSI escapes stripped**, from a byte cursor (`since`, returned by every read and input) or the last N lines. Reading from the cursor gives only the output since the last look, which is most of the token saving.

P2 adds a headless emulator per terminal in srv (`alacritty_terminal`, the emulator Zed and others embed; `vt100` as the lighter alternative), fed by the same PTY output that goes to the `term` file:

- `TerminalScreen { terminal?, page?, mode? }` returns the rendered screen as plain text (rows × cols), the cursor position, whether the alternate screen is active, the window title and the cwd (OSC 7). `page: 1, 2, …` steps back through the scrollback a screen at a time. `mode: "diff"` returns only rows changed since this agent's last screen read.
- The emulator answers the queries a program makes of its terminal (`ESC[6n` cursor position, device attributes), which is what makes **PowerShell usable as the Windows PtyShell** (§2 says it was ruled out because nothing answered PSReadLine's queries). The default stays `cmd.exe` until that is measured.

### 4.3 Waiting (P1)

`TerminalWait { terminal?, until, pattern?, timeout_ms, quiet_ms? }` blocks in srv and returns what it saw:

| `until` | Returns when |
|---|---|
| `"pattern"` | `pattern` (a regex, or `fixed: true`) appears in output written after the call started, not in the echo of the agent's own last input |
| `"quiet"` | no output for `quiet_ms` (default 1500); stated as a guess, not completion |
| `"prompt"` | P3: an OSC 133 A mark; before P3, the shell's prompt pattern for known shells (cmd `^[A-Z]:\\.*>$`, `$ `, `# `, `PS …>`) on the last line |
| `"command"` | P3: the OSC 133 D mark of the command the agent started |
| `"exit"` | the terminal's process ended |

Every wait has a timeout (default 30 s, max 10 min), returns `{ reason: "matched"|"quiet"|"prompt"|"done"|"exited"|"timeout", match?, exit_code?, since, tail }`, and never fails silently. A long wait doesn't hold the user lock (§5).

### 4.4 Commands as records (P3)

AgentMux's shell-integration scripts (`backend/shellintegration/{bash,zsh,fish,pwsh}`) emit OSC 133 A/B/C/D around each command, with D carrying `$?`, and OSC 633 E with the command line and a per-terminal nonce, so a program's output can't forge a command record. The existing OSC 16162 A/C/D become aliases or are retired. cmd.exe can emit A/B through `PROMPT` (`$e]133;A$e\`) but has no hook for D; it falls back to prompt detection.

srv parses these marks in the emulator's input stream and keeps the terminal's recent commands: command line, start, duration, output range, exit code, cwd. Then:

- `TerminalRun { terminal?, command, timeout_ms }` waits for a prompt, types the command, submits, waits for its D mark, and returns `{ exit_code, output, duration_ms, truncated }` with only that command's output. Without integration it falls back to prompt or quiet and says so (`exit_code: null, how: "prompt"`).
- `TerminalCommands { terminal? }` lists the recent records, so an agent can ask "did the build I started earlier finish, and how?"
- The terminal pane's UI can use the same records (marks in the scrollbar, jump to previous command); that is a UI follow-up, not part of this spec.

### 4.5 Terminal panes (P4)

- `TerminalOpen { cwd?, connection?, command?, split? }` opens a terminal pane next to the agent's (`/api/v1/pane/open` with `view:"term"`, which exists) and returns its block id. `connection` is `ConnList`'s set: local, `wsl://…`, an SSH host (first use asks the user, as `PtyShell` does today).
- **Ownership, as browser panes:** an agent drives terminals it opened, and its own drawer. A terminal the user opened is driven only after the user grants it to that agent (a "Let <agent> drive this terminal" action in the pane's menu), and the grant ends when the user types in it or takes over.
- **Visible state:** a driven terminal shows a "Driven by <agent>" bar with **Take over**, like `.browser-driven-by`. The per-keystroke 4 s lease stays for the drawer; a pane the agent opened is the agent's until the user takes over.
- `TerminalClose { terminal }` for a terminal the agent opened.

### 4.6 What it would have done for the incident

```
Terminal({ action: "open", cwd: "<checkout>" })
  → { terminal: "blk_…", platform: "windows", shell: "cmd", connection: "local" }
Terminal({ action: "type", terminal: "blk_…",
           text: "claude --cloud \"<task prompt>\"", submit: true,
           wait: { until: "pattern", pattern: "https://claude.ai/code/(\\S+)", timeout_ms: 120000 },
           read: "screen" })
  → { how: "matched", match: ["https://claude.ai/code/session_…"], screen: "…" }
```

Two calls instead of a console window the agent couldn't read: the user watches the same terminal in a pane, and the agent reads the session id off the screen. The same two calls work unchanged on macOS and Linux (§4.0).

### 4.7 As built in P1 (2026-10-15)

`crates/mcp/src/tools/terminal.rs`, with srv's `ptyshell/read` taking `since` and returning `end`:

- **The drawer only.** `open` attaches the agent pane's shell drawer; terminal panes (`TerminalOpen` for a pane) come later. `terminal` is the drawer's id (the old `shell_id` is accepted too).
- **`run` with an exit code before shell integration.** Not in the original P1 plan: `run` clears any half-typed line (Esc in cmd, Ctrl+U in a Unix shell), types the command followed by a marker in the shell's own syntax (`& call echo __AMX_RC_<nonce>_%^ERRORLEVEL%__` in cmd, `; echo "__AMX_RC_<nonce>_$?__"` elsewhere), and waits for the marker. The nonce keeps an earlier run's marker from answering; the echoed line can't match, since it has `%^ERRORLEVEL%` or `$?`, not digits. The command's output is what came between the echo and the marker. OSC 133 (P3) replaces this where the shell has AgentMux's integration.
- **Reading.** ANSI stripped, CRLF as LF, a lone CR starting the line over, and a cursor move to another row (CUP, HVP, CNL, VPA, CUD) read as a line break: ConPTY often starts a new line that way instead of writing CRLF, which otherwise glued lines together.
- **Measured live** on Windows cmd.exe against a running srv: `run` returned exit codes 3 and 0 for consecutive commands; `type` with `submit` and `until: "pattern"` returned the capture groups; `wait` with `until: "prompt"` found cmd's prompt; an unknown key name and an old tool name each returned an error that names the fix. macOS and Linux run the same code paths (POSIX marker, Ctrl+U) and are covered by the unit tests, not yet live.

## 5. Safety

1. **No secrets typed.** An agent never types a password, passphrase, token or one-time code. When the screen shows a secret prompt (Unix: the tty's ECHO is off, which srv can read with `tcgetattr` on the PTY; any platform: the line ends in `password:`, `passphrase`, `PIN`, `token:` and similar), `TerminalType` refuses text and returns `needs_user`. `TerminalHandoff { terminal, reason }` raises the pane's attention (as `BrowserHandoff` does) and the user types it. `ssh` already routes its prompts to the user through bashwrap's askpass; this covers everything else.
2. **Output is data, not instructions.** Tool results carry screen text from programs and remote hosts. The tool descriptions say so, as `OpenBrowser`'s do, and the agent's instructions treat it like web content.
3. **Masking.** Reads mask strings that look like credentials (API keys, bearer tokens, private key blocks, the common provider key formats) before they reach the model, and say how many were masked.
4. **The user stays in control.** Every driven terminal is visible; Take over always works and wins; a wait or a long command never holds the user's keyboard lock, only an active `TerminalType` does.
5. **Destructive commands are the agent harness's problem, not the terminal's.** MuxAPI doesn't try to classify commands; the agent's own permission rules (Claude Code's allow list, Warden) apply to what it asks for, and `TerminalRun` names the command in the pane so the user sees it.

## 6. Tests

- **Unit (srv):** key-name → bytes table (normal and application cursor mode); Enter is `\r` on Windows, Unix, WSL and SSH targets; chunking at the Windows limit; ANSI stripping; `since` cursors across reads; wait reasons including timeout and echo exclusion; the secret-prompt detector; OSC 133/633 parsing with BEL and ST terminators, a bare D, a forged E without the nonce.
- **Integration (srv, real PTY):** cmd.exe on Windows: type + submit runs a command (the incident's regression test); `TerminalWait pattern` across a slow command; Ctrl+C interrupts `ping -t`; bash/zsh/pwsh with integration: `TerminalRun` returns the right exit code and only its own output; a full-screen program (`vim`, `less`) reads back as a screen with `alt_screen: true`.
- **MCP:** schema tests that every documented key name is accepted and an unknown one is refused.
- **Eval:** a small terminal task set (start a dev server and wait for "ready", answer an installer's prompt, run a REPL session, debug with gdb/lldb) run with and without these tools, counting tool calls, tokens and success. It plugs into `SPEC_AGENT_TASK_EVAL_2026_10_13.md`'s harness.

## 7. Phases

| Phase | Scope | Size |
|---|---|---|
| **P1** | the `Terminal` tool (§4.0.1), replacing the `Shell*`, `PtyShell*` and `ConnList` tools and every reference to them; the §4.0 contract for input and text reads (Enter, keys, Ctrl+C, encoding, line endings, `platform`/`shell` in results), tested on all three OSes; Windows chunking; `TerminalRead view:"log"` with ANSI stripping and `since`; `TerminalWait` for `pattern`, `quiet`, `exit`; `TerminalOpen` for the drawer **and for panes the agent opens itself**, with a minimal "Driven by" bar and Take over on those panes. No emulator. | small-medium; fixes the incident, and agents get visible terminal panes from the start |
| **P2** | headless emulator per terminal; `TerminalRead view:"screen"` (screen, scrollback pages, diff, alt screen, cursor, title, cwd); emulator answers terminal queries; measure PowerShell as the Windows default | medium |
| **P3** | OSC 133/633 in AgentMux's shell scripts with a nonce; srv parses marks; command records (`view:"commands"`); `TerminalRun`; `TerminalWait` `prompt`/`command` | medium |
| **P4** | the user hands one of their own terminals to an agent (grant from the pane's menu, ended when they type or take over); secret-prompt refusal and the `needs_user` handoff; credential masking in reads | medium |

## 8. Open questions

1. **Secret-prompt refusal in P4 or P1?** P1 lets an agent drive its own panes, which can reach a `sudo` or `ssh` prompt. P1 relies on the agent's instructions ("never type passwords") and on bashwrap's askpass for ssh; P4 enforces it in srv. Move the refusal into P1 if that gap is too wide.
2. **Emulator crate.** `alacritty_terminal` (full, maintained, heavier) or `vt100` (small, enough for screen text). The screen read and alt-screen detection are the requirements; pick by binary size and Windows ConPTY behaviour.
3. **Default Windows shell.** Stay on `cmd.exe` (no exit codes per command, prompt detection only) or move to PowerShell once the emulator answers its queries (full OSC 133 with exit codes). Measure in P2.
4. **Granting a user's terminal.** A per-terminal grant until the user types, or a standing per-agent setting. Start with per-terminal.
