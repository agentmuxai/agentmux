# REPORT: Agents start without their AgentMux tools on v0.59.19 (the shell prefix refuses `agentmux-mcp`)

**Date:** 2026-10-11
**Author:** AgentY@narko
**Status:** analysis. The cause is fixed in the same PR; self-heal is designed in [`docs/specs/SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md`](../specs/SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md).
**Affects:** every Claude Code agent started or restarted on v0.59.19, on every platform.

## 1. What the operator saw

After moving to v0.59.19, an agent came up with this in its session start:

> The following MCP servers are configured but failed to connect: agentmux (CONNECTION_CLOSED): "Connection closed"

The agent had none of its AgentMux tools: no `SendMessage`, `WhoIsWorkingOn`, `ClaimWork`, `Shell`, memory, work queue or scheduling. Nothing retried, nothing told the operator, and the agent did not treat it as urgent. The operator noticed only because they were watching. A restart (the model switch that followed) came up the same way.

## 2. Timeline (UTC, 2026-10-11)

| Time | Event |
|---|---|
| 05:57:37 | srv for v0.59.19 boots (migrations applied, 1.1 s). |
| 06:00:40.59 | The agent's pane opens: `WriteAgentConfig` writes its `.mcp.json`. |
| 06:00:40.93 | The CLI starts (resume of the existing session). |
| 06:00:41.50 | The CLI's `agentmux` MCP connection fails after 113 ms: `CONNECTION_CLOSED`. |
| 06:00:46.65 | Resume falls back to a fresh session; the second attempt fails the same way after 571 ms. |
| ~06:10 | Diagnosis (below). By hand, `agentmux-mcp` starts and answers an `initialize` handshake, so the binary and the config are fine. |

## 3. Cause

Claude Code keeps a log per MCP server (on Windows under `%LOCALAPPDATA%\claude-cli-nodejs\Cache\<project>\mcp-logs-agentmux\`). Both attempts:

```
Server stderr: error: unrecognized subcommand 'agentmux-mcp'
Usage: agentmux-bashwrap.exe <COMMAND>
Connection failed after 113ms (CONNECTION_CLOSED): Connection closed
```

v0.59.19 sets `CLAUDE_CODE_SHELL_PREFIX` to `agentmux-bashwrap` so Bash output streams live (#4592, `SPEC_BASH_STREAMING_VIA_SHELL_PREFIX_2026_10_10.md`). That spec expected the CLI to use the prefix only for Bash tool calls. The CLI's own code (2.1.288) uses it in three places:

| Where | What the prefix receives |
|---|---|
| Bash tool calls | one argument: the CLI's script (`source … && eval '<command>' … && pwd -P >| …`) |
| Hook commands (unless the hook's shell is PowerShell) | one argument: the hook's command, e.g. `agentmux-bashwrap hook` |
| **stdio MCP servers** | one argument: `[command, ...args].join(" ")`, unquoted |

So the `agentmux` server was started as `agentmux-bashwrap agentmux-mcp`. `prefix::prefix_script` took a lone argument as a script only if it contained whitespace (a real CLI script always does), so `agentmux-mcp` fell through to `clap`, which refused it as an unknown subcommand. The process exited and the CLI recorded `CONNECTION_CLOSED`.

Hooks happened to work: their command has a space, so prefix mode found no Bash call to link and ran it with `bash -c` on its own stdio. bashwrap's debug log shows this for nearly every unlinked prefix call (148 of 150 on this machine had no Bash script in them).

## 4. A second problem found on the way

When a prefixed script has no `eval` in it (a hook, or an MCP server with arguments), `claim` still took the session's **only** pending Bash call record (`records.len() == 1`, meant for a CLI format change). Parallel Bash calls make that reachable: the CLI runs the second call's `PreToolUse` hook through the prefix while the first call's record is still waiting. The hook would then take the first call's record and run inside the streaming PTY. That garbles the hook's JSON answer and leaves the first call without live output. Not observed (the log can't tell), but the window is real.

## 5. Why it wasn't caught

- #4592's tests covered Bash calls through the prefix and the hook's two modes. Nothing started an MCP server through the prefix, and the unit test listed `bw nospaces` as deliberately *not* a prefix call.
- No agent was restarted on a build with #4592 until the release: running agents keep the MCP connection they made at launch. On this machine only two agents had launched on such a build (this one and QTest's `prefixtest` agent), and both show the failure.
- Nothing in AgentMux watches whether the `agentmux` MCP server connected (§7).

## 6. The fix (this PR)

In [`crates/bashwrap/src/prefix.rs`](../../crates/bashwrap/src/prefix.rs):

1. **A lone word is a prefix call too.** Any single argument that isn't a subcommand or a flag is now taken as the CLI's prefix call.
2. **Only a Bash call's script claims a Bash call's record.** A script without the CLI's `eval` runs as it is and never claims: a lone word is started directly as a program (found through `PATHEXT` on Windows, so a `.cmd` shim starts; on Unix by `exec`, so the CLI's signals reach it), and a command line goes to `bash -c` as before. On Windows, a command line whose program is a Windows path (`C:\tools\server.exe --port 1`) is split back into words and started directly instead, because bash would strip its backslashes; only an MCP server's line can have one, since the CLI makes a hook's paths POSIX.
3. **Tests:** unit tests for the new shapes and for a non-Bash script leaving a waiting call alone; an integration test (`tests/prefix_runs_mcp_servers.rs`) that starts the real binary the way the CLI starts an MCP server and checks that stdin reaches the program and its stdout comes back byte for byte, with its exit code.

Checked by hand on Windows: the fixed binary started as `agentmux-bashwrap agentmux-mcp` completes `initialize` and lists all 95 tools. The v0.59.19 binary prints the error above.

**Known limit, unchanged by this fix:** the CLI joins an MCP server's command line without quoting, so a server whose command path contains a space can't be started through any shell prefix. That is the CLI's behaviour; AgentMux's own entry (`agentmux-mcp`) has no space.

## 7. Follow-ups

- **Self-heal** ([`docs/specs/SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md`](../specs/SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md)): srv notices a dead `agentmux` MCP server, tells the operator and the agent, and repairs it in bounded steps, ending with a CLI restart without the shell prefix (the step that would have fixed this incident with no code change). `agentmux-mcp doctor` gives an agent with no MCP tools the diagnosis above in one command.
- **Hooks through the prefix** each start a `bash -c` just to run `agentmux-bashwrap hook`. Dispatching a prefixed `agentmux-bashwrap <subcommand>` in-process would save a bash start per Bash call. Not needed for correctness.
- **Users on v0.59.19** need a build with this fix. Until then, the workaround is to start AgentMux with `AGENTMUX_BASHWRAP_MODE=rewrite` in its environment: srv then sets no shell prefix, the hook rewrites Bash commands as it did before #4592, and agents started after that connect to their MCP server ([`crates/srv/src/server/agent_handlers/bash_prefix.rs`](../../crates/srv/src/server/agent_handlers/bash_prefix.rs)).
