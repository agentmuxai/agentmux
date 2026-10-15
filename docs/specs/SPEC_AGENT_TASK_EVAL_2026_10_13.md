# Agent task eval: what AgentMux costs and adds per coding task

**Status:** draft
**Date:** 2026-10-13
**Author:** Agent5

## 1. Why

AgentMux is a performance tool, but nothing measures what it does to a coding task's outcome. The repo has UI and latency benches (`tools/tests/bench-*.mjs`, `SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md`) and two desk analyses of the token overhead (`docs/analysis/TOKEN_TAX_ANALYSIS_2026_06_19.md`, `TOKEN_TAX_FOLLOWUP_2026_07_04.md`). `docs/reports/REPORT_COMMENT_COMPRESSION_WORTH_IT_2026_10_02.md` notes the missing agentic-task benchmark directly.

This spec defines two evals:

- **A, overhead (single agent).** The same tasks, model and effort, with and without what AgentMux puts around Claude Code. Does AgentMux change pass rate, and what does it cost in tokens, time and turns? Which layer costs what?
- **B, multi-agent value.** Several agents on overlapping tasks, AgentMux against plain parallel `claude` processes. Does coordination (overlap notes, WhoIsWorkingOn, messaging, continuation) save time or prevent duplicated and conflicting work?

A ships first. B reuses its runner and grader.

## 2. What the treatment is

What AgentMux changes about a Claude Code session, from the launch code:

| Layer | AgentMux | Where |
|---|---|---|
| MCP tools | `agentmux-mcp`, 95 tools, about 88K characters of schema; the CLI may defer them behind ToolSearch | `crates/mcp/src/tool_schemas.rs`, `backend/agent_config.rs:220-275` |
| Memory | `CLAUDE.md` with the Operator Config entries (about 18 KB) and Global Memory; SessionStart delivery up to 8 × 9,000 characters | `backend/agent_config.rs:60-207`, `backend/memory_delivery.rs:20-27` |
| Skills | six starter skills | `crates/srv/src/config/starter-skills.json` |
| Shell | every Bash command runs through `agentmux-bashwrap` (a PTY) | `server/agent_handlers/bash_prefix.rs:40-52` |
| Prompt layout | `--exclude-dynamic-system-prompt-sections` | `backend/providers.rs:305-333` |
| Defaults | model `sonnet`, effort `high`, every tool auto-allowed, an isolated config dir | `backend/agent_runtime.rs:38-40, 392` |
| Runtime | mid-turn message injection, overlap notes, continuation after a failed resume | `blockcontroller/mod.rs:673-729`, `backend/work_facts/overlap_notes.rs` |

The defaults are confounds, not treatment: both arms pin the same model, effort and permission mode, and both run without user-level config.

## 3. Eval A: overhead

### Arms

| Arm | Session |
|---|---|
| `vanilla` | Claude Code on the task repo, nothing added |
| `agentmux` | the same, plus AgentMux's `CLAUDE.md` memory, `.mcp.json` (agentmux-mcp against a headless srv), `.claude/settings.json` hooks, starter skills and bashwrap |
| `agentmux-no-mcp`, `agentmux-no-memory`, `agentmux-no-bashwrap` | ablations: `agentmux` with one layer removed |

The ablations run only if `agentmux` and `vanilla` differ by more than noise on cost or pass rate.

### Where trials run

Each trial is one Claude Code cloud session (`claude --cloud "<task prompt>" --environment <arm env>`). Both arms then run on the same harness, so the comparison is fair, and the runs draw on cloud-session credit rather than plan usage. Each arm is a cloud environment whose setup script lays down that arm's files; the `agentmux` arm's script also builds or downloads Linux `agentmux-srv` and `agentmux-mcp` and starts `agentmux-srv --headless`.

What this does not capture: the launch flags (`--exclude-dynamic-system-prompt-sections`, stream-json, the permission bridge), since cloud sessions use their own launch. A later local run through `inspect_swe`'s `claude_code()` agent, or through AgentMux itself, can cover those.

### Tasks

Mined from merged agentmuxai/agentmux PRs that changed code and added or changed tests:

- The task is the PR's linked issue or a prompt written from the PR body, with the solution removed.
- The repo is checked out at the PR's base commit.
- The hidden check is the PR's tests: they fail on the base commit and pass on the merge commit. A task where that doesn't hold is dropped.
- Prefer tasks whose tests run in minutes: frontend vitest and single Rust crates, not a full `agentmux-srv` build.
- All tasks postdate the models' training data.

Pushes go to a private eval repo, never to the public repo.

### Metrics

Per trial: pass (hidden tests), input, output, cache-read and cache-write tokens, turns, tool calls and tool errors, wall time, and whether the session finished on its own. Token usage comes from the session transcript (`claude --teleport <id>` loads it).

Analysis is paired by task. Pass rate gets a task-bootstrap 95% interval and a paired test; cost and turns get the median paired difference and its interval. With about 50 tasks a pass-rate difference under 10 points is unlikely to show, while a cost difference usually will: published harness comparisons find small pass-rate effects and large token effects.

### Budget

Cloud-session credit, two accounts, expiring in early November 2026. A pilot of 5 tasks × 2 arms measures cost per trial; the full run is sized from it. At $2–5 a trial, the budget covers 100–250 trials, for example 50 tasks × 2 arms × 1–2 runs.

## 4. Eval B: multi-agent value (later)

N agents, M tasks chosen so some touch the same files. AgentMux arm: N agents in one headless AgentMux. Plain arm: N `claude` processes in separate clones. Metrics: wall time until all tasks pass, merge conflicts, duplicated work, and recovery after a restart mid-task. Smaller n; reported as cases, not intervals.

## 5. Build

| Piece | What it does |
|---|---|
| `evals/agent-task/mine.py` | lists candidate PRs and writes task rows (base commit, prompt, test command) |
| `evals/agent-task/verify.py` | keeps a task only if its tests fail on the base commit and pass on the merge commit |
| `evals/agent-task/launch.py` | starts one cloud session per (task, arm, run) and records the session ids |
| `evals/agent-task/grade.py` | checks out each session's branch and runs the hidden tests |
| `evals/agent-task/usage.py` | reads token usage and turns from each transcript |
| `evals/agent-task/report.py` | paired statistics and a table |

## 6. Open questions

1. Can a cloud environment's setup script build or fetch Linux `agentmux-srv` and run it headless? The pilot answers it.
2. Do teleported transcripts carry per-message usage, including cache tokens?
3. Does a cloud session's spend come only out of the credit, leaving plan usage alone? Check plan utilization before and after the pilot.
4. Does the cloud harness load a repo's `.mcp.json` and `.claude/settings.json` hooks? The docs say it reads committed `.claude/settings.json`; verify the hooks run.
