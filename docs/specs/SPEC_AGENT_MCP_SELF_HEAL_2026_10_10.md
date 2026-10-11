# SPEC: Agent MCP self-heal — notice when an agent's AgentMux tools are down, and repair it without the operator

**Date:** 2026-10-10
**Status:** proposed. Phase 0 (the bashwrap fix) ships with this spec; Phases 1–4 are next.
**Author:** AgentY@narko
**Trigger:** repo owner, 2026-10-11, after an agent came up on v0.59.19 with its `agentmux` MCP server dead: "this gives us an opportunity to add a feature. In case such a situation happens again, agents need a way to self-heal."
**Grounded in:** `main` at `75fdca23b`, plus the Claude Code MCP logs of the affected agents. File references are as of that commit.
**Related:** `SPEC_AGENT_SELF_QUIT_2026_09_24.md` (an agent acting on its own pane, and the trust model for it), `SPEC_RUNTIME_MENU_REMAINING_GAPS_2026_10_01.md` (control requests to a running CLI), `SPEC_MCP_INTEGRATION_PARITY_ABLETON_PILOT_2026_07_08.md` §4.4 (`mcp_probe.rs`), `CLAUDE.md` "Jekt security rules".

---

## 0. Summary

On v0.59.19 an agent can start with **no AgentMux tools at all**: no `SendMessage`, no `WhoIsWorkingOn`, no `Shell`, no memory, no work queue. Nothing tells the operator, nothing tells the agent, and nothing retries. The agent stays that way for the whole session.

Two things are needed, and they are separate:

| | What | Why separate |
|---|---|---|
| **Phase 0: fix the bug** | `agentmux-bashwrap` must pass a plain program invocation through instead of failing it (§2) | It is a regression that hits every Claude agent launched on this build. It ships first, as its own PR, whatever happens to the rest of this spec. |
| **Phases 1–4: self-heal** | srv notices a dead MCP server, tells the agent and the operator, and walks a bounded repair ladder (§4–§6) | The next failure will have a different cause. The mechanism must not depend on knowing it. |

> [!IMPORTANT]
> The incident's cause is deterministic, not a race. Re-connecting, re-writing `.mcp.json` and restarting the CLI would **all have failed the same way**. Only one rung of the ladder would have healed it: relaunching the CLI in a *safe profile* without the shell prefix (§5.4). A ladder made only of "try again" rungs would have been useless here.

## 1. What happened

### 1.1 Evidence

Claude Code keeps a log per MCP server. For the affected agent, both launch attempts (the resumed session at 06:00:41Z and the fresh one at 06:00:46Z) ended the same way:

```
Server stderr: error: unrecognized subcommand 'agentmux-mcp'
Usage: agentmux-bashwrap.exe <COMMAND>
Connection failed after 113ms (CONNECTION_CLOSED): Connection closed
```

The last good connection, on v0.59.16, was `Successfully connected (transport: stdio) in 39ms`.

Everything else was healthy at the time:
- srv had been up for three minutes (booted 05:57:37Z);
- `.mcp.json` was correct, with the right agent id, channel and keys;
- `agentmux-mcp` resolves on `PATH`, and run by hand it completes an MCP `initialize` handshake and reports version 0.59.19.

### 1.2 Cause

AgentMux sets `CLAUDE_CODE_SHELL_PREFIX` to `agentmux-bashwrap` (PR #4592). The CLI's own code applies that prefix to stdio MCP servers too, passing `[command, ...args].join(" ")` as one argument, so it ran `agentmux-bashwrap agentmux-mcp`. `prefix::prefix_script` accepted a lone argument only if it contained whitespace, because a real CLI shell script always does. `agentmux-mcp` has none, so it fell through to `clap`, which rejected it as an unknown subcommand. The process exited and the CLI recorded `CONNECTION_CLOSED`. Full analysis: `docs/reports/REPORT_AGENT_MCP_DOWN_THROUGH_SHELL_PREFIX_2026_10_11.md`.

### 1.3 Reach

Checked on this computer: the two agents launched on a build with #4592 (AgentY and QTest's `prefixtest` agent) both show the failure. Every other agent's last launch pre-dates it and connected normally. So **every Claude agent that starts or restarts on 0.59.19 loses its MCP tools.**

### 1.4 Why nothing noticed

- The CLI does not retry a failed stdio MCP server, and does not report it to AgentMux in any frame srv reads today (`system/init` handling in `persistent/stdout_reader.rs:176` looks only at the CLI version and the turn-active flag).
- srv has `mcp_probe.rs`, but it probes servers an operator configured in Connectors, on request. It never probes the `agentmux` server it injects itself.
- The agent only learns through the CLI's own "failed to connect" line, which a model reads as a configuration detail. It has no instruction to treat it as an emergency and no tool to do anything about it.
- The operator saw it only because they were watching the first message.

## 2. Phase 0: the bashwrap fix

`agentmux-bashwrap` as a shell prefix must behave like a prefix. The contract of a prefix is "run what comes after me". It understood one shape of what comes after it, a CLI Bash script; the CLI also gives it hook commands and MCP command lines. **Shipped with this spec:**

- Any lone argument that is not a subcommand or a flag is a prefix call.
- Only a script with the CLI's `eval` (a Bash call) may claim a Bash call's record. Anything else runs with **inherited, unmodified stdio** (no PTY, no output rewriting): a lone word directly as a program (`exec` on Unix), a command line through `bash -c`. An MCP server speaks newline-delimited JSON-RPC on stdin/stdout, so a single added byte breaks it. This also closes a race where a prefixed hook took a waiting Bash call's record (report §4).
- **Regression test:** `crates/bashwrap/tests/prefix_runs_mcp_servers.rs` starts the real binary the way the CLI starts an MCP server and checks the stream byte for byte and the exit code.

Still to do in Phase 1:
- **Launch-time self-test (cheap, useful beyond this bug):** when srv materializes an agent's config, it already writes the `agentmux` MCP entry. It can run `mcp_probe` against that entry once per agent per AgentMux build, with the same environment the CLI will give it. A failure is logged and, from Phase 1, shown (§4.3). This catches the *config* half of this class of bug before the first message.

## 3. Goals and non-goals

**Goals**
1. A dead `agentmux` MCP server is noticed within seconds of the CLI starting, whatever the cause.
2. The operator and the agent are both told, in plain words, with the cause when it is known.
3. Most failures repair themselves with no one involved; a persistent one stops retrying and says so.
4. An agent that has no MCP tools can still start the repair itself, using only what it always has (its shell).
5. Repair is bounded, audited, and can never interrupt work in progress.

**Non-goals**
- Healing a *user-configured* MCP server (Connectors). The same detection can later show them; repair stays the operator's.
- Fixing the CLI's own MCP client.
- Healing a stopped or quit agent. Self-heal never starts a process that the operator or `QuitSelf` stopped.
- Other providers in the first cut (§8).

## 4. Detect and tell

### 4.1 Three signals, any one is enough

| Signal | Source | Note |
|---|---|---|
| **A. CLI-reported status** | The CLI's `system/init` frame and the `mcp_status` control request report each server as connected, failed or pending. | The CLI binary contains `mcp_status`, `mcp_reconnect`, `mcp_toggle` and `reconnectMcpServer`. *Spike:* confirm the request payloads and whether `init` carries `mcp_servers` on the CLI versions AgentMux supports. srv already sends `get_settings` and `get_context_usage` the same way (`agent_runtime.rs`, `agent_context_usage.rs`). |
| **B. MCP hello** | `agentmux-mcp` already calls srv at startup (`self_keys::refresh_if_stale`). srv records the first call per block and spawn generation. | Independent of the CLI's frame format. A spawn with no hello within `MCP_HELLO_GRACE` (10 s, long enough for a slow disk) is suspect. It also detects the case where the CLI says "connected" but the process cannot reach srv. |
| **C. Launch self-test** | §2, config-time probe of the exact entry. | Fails early, before any spawn. |

A and B together give the cause: *the CLI says failed, and there was never a hello* means the process never ran; *hello then silence* means it died or lost srv.

### 4.2 State

One record per block, kept in the persistent controller's health state next to `turn_active`:

`mcp: { state: ok | pending | failed | healing | gave_up, since, attempts, last_rung, last_error }`

Published as an event so the pane, the dock and the Swarm can show it, and persisted so a pane that mounts late still gets the latest (the pattern `agentruntime` uses).

### 4.3 Tell the operator

A notice in the pane, in the same channel as the CLI-version notice (`cli_notice::append_to_pane`), once per state change, not per retry:

- `healing`: "This agent's AgentMux tools did not connect. Retrying (step 2 of 5)."
- `ok` again: "AgentMux tools are back."
- `gave_up`: "This agent has no AgentMux tools and AgentMux could not repair it. <cause, one line>. What was tried: …. Copy the report" with the doctor output (§7) attached.

### 4.4 Tell the agent

The agent cannot call a tool to learn this, so srv puts it in the conversation, in the vetted `[AgentMux] ` system-note channel overlap notes use (`CLAUDE.md` global memory, "system notes"). Delivered with the next input srv forwards to that agent, never as its own turn:

> `[AgentMux] Your AgentMux tools (MCP) are not connected: <quoted cause>. AgentMux is retrying. Until it says they are back, do not rely on SendMessage, memory or the work queue. If this note repeats, run: agentmux-mcp doctor --heal`

Rules:
- Any text that came from a process or a log (the MCP's stderr line) goes through `sanitize::quote_in_system_note`, and is length-capped. A failing server's output is untrusted text and must not be able to forge a marker.
- Rate-limited: one note per state change, plus one reminder after `gave_up`.
- When the state returns to `ok`, one short note says so.

## 5. Repair: the ladder

Run by srv, per block, one rung at a time, with a pause between rungs. **A rung is skipped if the previous failure has the same error text and the rung cannot change it** (the deterministic-failure rule, §1: do not retry the same thing).

| # | Rung | Fixes | Cost |
|---|---|---|---|
| 1 | **Wait and re-check** (`MCP_HELLO_GRACE`) | slow start, antivirus scan of a new binary | none |
| 2 | **`mcp_reconnect` for `agentmux`**, up to 2 tries with backoff | a transient crash, srv briefly unreachable | none, works mid-session |
| 3 | **Re-materialize config**: rewrite `.mcp.json` through `write_mcp_json_respecting_user_servers` and re-inject the signing keys, then rung 2 | a missing or stale entry or key, a file the user edited | none, keeps the user's own servers (the existing manifest rules) |
| 4 | **Alternate launch shape**: the entry's `command` as the absolute path to the `agentmux-mcp` that ships with this install, then rung 2 | `PATH` resolution problems | none |
| 5 | **Safe profile restart**: restart the CLI process with `--resume`, with every feature that changes how the CLI launches child processes switched off. Today that is the shell prefix: the per-block equivalent of srv's `AGENTMUX_BASHWRAP_MODE=rewrite` in `bash_prefix::apply_for_spawn`, under which the Bash hook rewrites commands as it did before #4592 (`prefix_active()` already tells the two apart). Only between turns. | a wrapper or prefix that breaks the launch. This is the rung that heals the §1 incident. | restarts the CLI; the conversation resumes (`SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md`) |

### 5.1 Safe profile

A named set of launch choices with a sentinel in the controller state ("this block runs the safe profile, because MCP failed at <time> with <error>"). Remembered per block until the AgentMux version changes or the operator clears it, so a restart does not walk the ladder again. It is a degraded mode, shown as such in the pane (§4.3). It exists so that a bug in any new launch-time feature costs a restart, not an unusable agent.

### 5.2 Guard rails

- **Never mid-turn.** Rungs 1–4 do not interrupt a turn. Rung 5 waits for turn end (the controller already has the signal) with a cap, the way `QuitSelf` does (`SELF_QUIT_TURN_GRACE`), and asks the operator first if the turn does not end.
- **Never against the operator.** No rung runs for a block that is `mark_closing` or `mark_quitting`, was stopped by the user, or whose pane is closed.
- **Bounded.** At most 5 rungs, 2 `mcp_reconnect` tries, 1 restart per block per hour. After that: `gave_up`, no more automatic attempts until the operator or the agent asks (§7).
- **One at a time.** A per-block lock; a second trigger while `healing` is ignored.
- **Setting.** A user setting turns the automatic ladder off (default on). Detection and notices stay on regardless. The name follows the existing `agent:` settings conventions in `settings-template.jsonc`.
- **Audited.** Every rung is one log line with block, rung, result and cause, and appears in the doctor report.

## 6. Why not just a retry loop

Because the first real failure was one a retry cannot fix, and a loop that reconnects forever on a deterministic failure is worse than none: it hides the problem and burns the CLI's connection budget. The ladder's order is cheapest first, but each rung must be able to change an outcome the previous one could not.

## 7. Agent-initiated repair: `agentmux-mcp doctor`

The agent has no MCP tools, but it still has a shell, and `agentmux-mcp` is the binary on its `PATH`. So the diagnosis and the "heal now" button live there, as subcommands that need no MCP session:

```
agentmux-mcp doctor           # diagnose, read-only
agentmux-mcp doctor --heal    # diagnose, then ask srv to run the ladder now
```

`doctor` checks, and prints one line each with a pass or fail:
1. the binary itself: path, version, and that it completes an `initialize` handshake (what I did by hand);
2. the same handshake **through `CLAUDE_CODE_SHELL_PREFIX`, as the CLI would launch it**;
3. the agent's `.mcp.json` entry and that the keys it needs are present (never printing them);
4. srv reachable at `AGENTMUX_LOCAL_URL`, and whether srv has seen a hello from this block;
5. the tail of the CLI's own per-server log, best effort. Its location is a CLI internal and can change, so a miss is reported, not an error;
6. what srv has already tried (the ladder state in §4.2).

`--heal` sends one request to srv, `POST /api/v1/agent/self/mcp-heal`, carrying `AGENTMUX_BLOCKID` and `AGENTMUX_AUTH_KEY`, both already in the agent's shell environment. Trust and scope follow `SPEC_AGENT_SELF_QUIT_2026_09_24.md` §0.1, where local agents are trusted:
- there is no target parameter; it acts on the caller's own block only;
- it can only run the §5 ladder (and, by that spec's rules, nothing else), which is bounded and idempotent;
- it is **not** a jekt and carries no free text into a system note. A message from another agent can ask nothing of it, and the note in §4.4 is the only place it is suggested.

The report is the same text the operator gets under `gave_up`, so a person can paste it into an issue.

## 8. Other providers

First cut: Claude Code (the only provider with a control protocol for MCP). The hello signal (B) and the launch self-test (C) are provider-neutral and run for any agent whose config includes the `agentmux` entry; their failures raise the §4.3 notice and the §4.4 note. The repair rungs that need the CLI (2 and 5) are Claude-only until another provider has an equivalent.

## 9. Phases

| Phase | Ships | Depends on |
|---|---|---|
| **0** | bashwrap passthrough, MCP-through-prefix contract test (§2) | nothing; urgent |
| **1** | detection (A, B, C), state and event, operator notice, agent note (§4) | spike results |
| **2** | the ladder, safe profile, guard rails, setting (§5) | 1 |
| **3** | `agentmux-mcp doctor` and `--heal` (§7) | 2 |
| **4** | dock and Swarm chip; other providers (§8) | 3 |

**Spike, before Phase 1** (a day, in an isolated `task dev` instance, never in a live one):
1. ~~Does the current CLI add `CLAUDE_CODE_SHELL_PREFIX` to MCP launches?~~ Yes, read from the CLI's code (2.1.288): stdio MCP servers and non-PowerShell hooks both get it (report §3).
2. The exact `mcp_status` and `mcp_reconnect` payloads and answers, and whether `reconnect` re-resolves the launch command.
3. Does `system/init` carry `mcp_servers` with a status?
4. Does the safe profile (no prefix) actually bring MCP back on the incident build?

## 10. Tests

- **Ladder** as a pure state machine with an injected clock: every rung, the deterministic-failure skip, the caps, `mark_closing` suppression, turn-end waiting.
- **Fake CLI** that reports `failed` until it receives `mcp_reconnect`, then `connected`: ends `ok`. A second that never recovers: ends `gave_up` after the caps.
- **Safe profile:** with the Phase 0 fix reverted in the test build, the ladder reaches rung 5 and the agent comes back.
- **Notes:** a failing server whose stderr contains `[AgentMux]`, `[JEKT:` and a line break yields a one-line, quoted note.
- **Doctor:** golden output for each failing check; `--heal` returns `already_healing` on a second call.
- **Contract (Phase 0):** MCP handshake through the prefix.

## 11. Decisions for the repo owner

1. **Safe profile restarts the CLI.** Is an automatic restart (between turns, once an hour, conversation kept) acceptable, or should rung 5 ask the operator first?
2. **`doctor --heal` trust.** It uses the agents-are-trusted rule from the self-quit spec. Is that the right level for a bounded, own-block repair?
3. ~~**Phase 0 timing.**~~ Shipped first, with this spec and the report (repo owner, 2026-10-11: "write report to file and proceed with implementation").
