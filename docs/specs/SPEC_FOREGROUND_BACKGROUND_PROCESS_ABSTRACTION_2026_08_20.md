# Spec: Foreground/Background Process Abstraction for Agent-Run Commands

> **Tracked family — canonical status: [`TRACKING_AGENT_AVAILABILITY_AND_BACKGROUNDING_2026_09_17.md`](TRACKING_AGENT_AVAILABILITY_AND_BACKGROUNDING_2026_09_17.md) (issue #3338).**
> This document is accurate as of its own date. Parts may be superseded; check the tracking doc before acting on it.

**Date:** 2026-08-20
**Author:** AgentA
**Status:** Proposed — research-grounded, but §1 identifies an unresolved feasibility question that should be answered before committing to an implementation plan
**Depends on:** the background-task registry work (Phase A/B/C: PID capture #2681, teardown survival #2683, dashboard intelligence #2685 — all merged) as the foundation for tracking/surviving any task this feature backgrounds
**Origin:** live-verification testing of the merged registry work surfaced a real gap — a plain long-running Bash call (no `run_in_background: true`) blocks the agent's turn for its full duration; only dock-promoted for visibility at 30s, never actually freed. The user asked for AgentMux to "commandeer" obviously-long-running commands and abstract a virtual foreground/background process model.

## 0. Scope, precisely

Two related but distinct asks came out of the triggering conversation — keep them separate:

1. **Auto-backgrounding "obviously long-running" commands** (e.g. `sleep`, a dev-server invocation) — proactively set `run_in_background: true` before dispatch, so the existing (already-shipped) background machinery handles it from the start. This is a **hook-side heuristic**, not a new process-lifecycle primitive — see §5.1.
2. **A general virtual foreground/background toggle** the user (or AgentMux itself) can apply to an *already-running* command, mid-flight — mirroring Claude Code's own native `Ctrl+B` feature. This is the harder, more novel piece, and where §1's open question lives.

Explicitly **not** in scope (already answered/rejected in the conversation that led here): making *all* tool calls async by default. Most tools (`Read`, `Edit`, `Grep`, etc.) require a real result for the model to reason correctly on its next step; the "return a placeholder, notify later" trick only makes sense for commands whose real-time result the model doesn't need to proceed. Backgrounding is deliberately an exception applied to Bash-style commands, not a default for tool use in general.

## 1. Critical open question — does this even apply to how AgentMux hosts Claude?

Claude Code's own native `Ctrl+B` background-toggle (§2 below) is documented specifically as an **interactive-mode** feature: a raw keypress (byte `0x02`, or the `Ctrl+X Ctrl+B` chord) consumed by the CLI's own TUI input loop while a `Task` context (a Bash tool call actively running) is active.

AgentMux's own PTY-input infrastructure research (this repo, current code) found:

- Sending an arbitrary raw byte to a hosted CLI's stdin **is possible today** — but only through `ShellController`, which manages a real PTY (`agentmux-srv/src/backend/blockcontroller/shell/lifecycle.rs`'s input task does a literal `writer.write_all(&data)` to the PTY).
- AgentMux's actual Agent panes — the ones this whole feature would need to target — are hosted via `SubprocessController`/`PersistentSubprocessController`, which have **no PTY at all**. They communicate with `claude` over structured stream-json (`--output-format stream-json`-style programmatic mode), and their `send_input` implementations **explicitly reject raw byte input** with an error ("does not accept raw input; use send_message()") — confirmed by reading both implementations directly. The existing SIGINT/Stop handling for these controllers doesn't send a control byte at all; it hard-kills the OS process via `kill_tx`.

**This means the two hosting modes are fundamentally different, and `Ctrl+B`'s applicability to the second one (the one that actually matters) is unverified:**

- If a persistent/subprocess-hosted `claude` is running in a non-interactive, structured-protocol mode, there may be no "TUI input loop" listening for keystrokes at all — the whole `Ctrl+B` mechanism may simply not exist as a recognized input in that mode, regardless of what bytes AgentMux sends.
- It's also possible Claude Code's Agent SDK / stream-json control protocol has its **own**, different mechanism for the same underlying capability (background an in-flight tool call) — exposed as a structured control message rather than a keypress, analogous to how `COMMAND_AGENT_ANSWER` already delivers an `AskUserQuestion` response via "the Agent SDK **control protocol** (a `control_response`)" rather than raw PTY input (see `agentmux-srv/src/server/websocket.rs`'s existing `agent.answer` handler for the precedent — Claude Code does have a structured control-request/control-response channel independent of the PTY-keystroke world). This is worth investigating directly, since it may be the *actual* intended path for anything hosted this way.

**Recommendation: before any implementation work, spike this specifically.** Options, roughly in order of how directly they answer the question:

1. Check Claude Code's own Agent SDK / stream-json protocol documentation and message schema for a control-request type related to backgrounding a tool call (parallel to how `can_use_tool`/`control_response` already work for permission prompts). If one exists, it's almost certainly the correct, supported mechanism — cleaner than simulating a keypress into a channel that may not even be listening for it.
2. If no protocol-level mechanism is found, empirically test whether writing byte `0x02` to the persistent controller's stdin (even though `send_input` currently rejects raw bytes for this controller type — that rejection would need to be deliberately bypassed for the spike) has *any* observable effect when a Bash tool call is in flight. A `PersistentSubprocessController` likely still has a real stdin pipe to the child process even without a PTY; whether the CLI's own input-reading logic (if any exists outside interactive-TUI mode) does anything with unsolicited bytes on that pipe in stream-json mode is knowable only by trying it.
3. If neither pans out, this specific approach (retroactively backgrounding an in-flight Bash call for AgentMux's primary Agent-pane hosting mode) may not be buildable without a Claude Code CLI feature request to Anthropic. In that case, fall back to shipping only §5.1 (auto-backgrounding obviously-long commands proactively, before dispatch) — which doesn't depend on this open question at all, since it works entirely through the existing `run_in_background: true` pre-declaration path.

This spec's remaining sections synthesize the research either way, since the design patterns are useful regardless of which path turns out feasible — but §6 (the phased plan) puts the spike first and gates everything downstream on its answer.

## 2. Claude Code's own native mechanism (for context/precedent)

Officially documented (code.claude.com/docs/en/interactive-mode, /keybindings, /tools-reference):

- **Trigger:** `Ctrl+B` (byte `0x02`), or the chord `Ctrl+X Ctrl+B` (v2.1.169+, added specifically to avoid tmux's own `Ctrl+B` prefix collision). Action id `task:background`, active only in the `Task` context ("Background task is running"). Rebindable via `~/.claude/keybindings.json`.
- **Effect:** identical outcome shape to the model-driven `run_in_background: true` parameter — an early placeholder tool_result with a task ID, later resolved by a `<task-notification>`-style message. stdout/stderr redirect to a file; retrieved via `Read`/`BashOutput`. This is why, if this mechanism does apply to AgentMux's hosting mode, no changes to the already-shipped Phase A/B/C registry infrastructure should be needed — a `Ctrl+B`-backgrounded task produces the same `"Command running in background with ID: ..."` acceptance text `isAcceptedBackgroundLaunch()` already detects.
- **Related, separate mechanism — auto-backgrounding on timeout:** when a command hits its timeout without finishing, Claude Code moves it to the background automatically instead of stopping it, *except* for commands starting with `sleep`, any command containing `git`, or compound commands the CLI can't parse into simple commands. (Note the irony for this spec's own origin story: `sleep 900` is explicitly on the *exclusion* list for automatic backgrounding — testing with `sleep` specifically avoids the one automatic path that might otherwise have masked the real question.)
- **No documented reverse action.** There is no `task:foreground` keybinding and no documented way to re-attach synchronously to an already-backgrounded Bash task. The only operations on it are: read output, list it (`/tasks`/`/bashes`), stop it.
- **Known fragility, even in its native interactive habitat:** GitHub issues report `Ctrl+B` silently not working on some terminal/shell combinations (readline binding conflicts on macOS/Linux — anthropics/claude-code#5376, #5391, #5638), closed without a fix. This is a reason for caution even if the mechanism does apply to AgentMux's hosting mode — it may need a fallback/confirmation UX rather than assuming the toggle always lands.

## 3. Cross-industry survey — recurring patterns

Researched: Unix job control, tmux/screen, systemd/systemd-run, Docker, VS Code tasks, and how other AI coding agents (Codex CLI, Cursor, Windsurf/Cascade, Aider, Replit Agent, GitHub Copilot, Gemini CLI, Devin) handle this same problem. Full findings available on request; the actionable synthesis:

### 3.1 Two structurally different kinds of "background," worth naming distinctly

- **Job-control backgrounding** (Unix `bg`/`&`, Claude Code's `Ctrl+B`/`run_in_background`): the *job itself* changes state — it stops being synchronously waited-on by whatever dispatched it, but nothing about its own execution environment changes. The controller (shell, or Claude's own turn loop) simply stops blocking on it.
- **Session/attachment backgrounding** (tmux/screen detach-reattach, Docker attach/logs): the job **never changes state at all** — it was never blocking anything to begin with (it runs under a persistent server/daemon/shim that owns its stdio independent of any viewer). What changes is purely whether a *viewer* is currently watching. tmux/screen's own research finding is explicit about this: "the job never changes state — it is not stopped, not signalled, not reparented, and its controlling terminal never disappears... Only the viewer comes and goes."

AgentMux's own background-task registry (Phase A/B/C) is actually already closer to the **second** model in spirit — Phase B's whole point was making sure a declared-background task's process survives independent of whatever's "watching" it (a specific controller generation), which is conceptually the tmux/systemd pattern (durable, watcher-independent liveness) layered under Claude Code's own job-control-style backgrounding UX. Recognizing this: **the "toggle to background" feature this spec is about is a job-control-style state change (§2), but the reason it survives afterward is because of the session/attachment-style durability this session's earlier work already built.** Both patterns are legitimately in play, at different layers.

### 3.2 The recurring hard problem: readiness/liveness, not detachment

Detaching a process from its launcher is the easy, well-solved part (every system in the survey has some primitive for it — `setsid`, a persistent server, a daemon, a shim, cgroup membership). The **actually hard, repeatedly-reinvented part** is knowing when a detached process is *ready* or *has meaningfully changed state* without polling or guessing:

- systemd: explicit `sd_notify(READY=1)` protocol (`Type=notify`), or the cruder `Type=forking`/`PIDFile=` heuristics for processes that don't cooperate.
- VS Code: regex-scraping stdout for `beginsPattern`/`endsPattern` — fragile by the docs' own admission ("state tracking depends entirely on the tool printing exactly the strings your patterns expect... may spin forever").
- Claude Code / AgentMux's existing `isAcceptedBackgroundLaunch`: matching a literal string prefix in the tool result.
- Codex CLI (until recently): no real signal at all — documented workaround was "append `&` and guess a `sleep` duration," explicitly called out in the research as clumsy.

Takeaway for this spec: whatever readiness signal a foreground/background toggle relies on (e.g., "is this Bash call still legitimately blocking, or should the toggle option be offered") should be built on the **most structured signal available**, not a regex/string-match if a better one exists — see §4's finding that per-tool-call running state already exists (in the document store) more precisely than the pane-level atoms most naturally reached for.

### 3.3 The recurring pattern for *why* backgrounding needs a handle and cleanup discipline

Every system in the survey pairs "detach" with an **ID** (job number, session name, unit name, container ID, task ID) and warns explicitly about orphaned/leaked processes when that discipline is skipped (Claude Code's own docs literally warn "forgetting to stop a task leaves an orphaned process running"). AgentMux's Phase A (PID capture) already provides exactly this ID; Phase B (teardown survival) already provides the cleanup discipline (kill on genuine pane close, survive on restart). This is further confirmation that **any new foreground/background toggle should route through the existing registry, not invent a parallel one.**

## 4. AgentMux-specific technical grounding (current code, verified)

- **Raw byte injection into a PTY already works**, end to end: `ControllerInputCommand` (frontend) → `CommandBlockInputData` (wire) → `COMMAND_CONTROLLER_INPUT` handler → `parse_block_input()` → `BlockInputUnion::data(bytes)` → `blockcontroller::send_input()` → `ShellController::send_input` → the PTY input task's `writer.write_all(&data)`. This is the exact mechanism that would carry a `Ctrl+B`-equivalent byte, **for `ShellController`-hosted blocks only**.
- **`SubprocessController`/`PersistentSubprocessController` (the controllers that actually host Agent-pane `claude` sessions) have no PTY and explicitly reject raw input bytes.** Their existing "stop" handling (SIGINT via `ControllerInputCommand`) doesn't write a control byte anywhere — it hard-kills the OS process (`stop_subprocess`/`stop_process` via `kill_tx`), with `session_id` retained so a fresh message resumes the conversation. **There is no existing precedent anywhere in this codebase for injecting a synthetic control sequence into a non-PTY, stream-json-hosted `claude` process.** This is the crux of §1's open question — whatever mechanism turns out to work will likely need genuinely new plumbing, not an extension of something that already does something similar.
- **Per-tool-call "is this specific call still running" tracking already exists, but at the wrong layer for a UI trigger.** The pane-level atoms most naturally reached for building a UI feature (`currentToolAtom`, `TurnPhase.Streaming.toolsActive`) are coarse — single-slot, name-only, no `tool_use_id`, can't distinguish "multiple tools active" from "which specific one." The precise, per-call, `tool_use_id`-keyed "is it still running" signal lives one layer down, in the document store's `ToolNode.status` field (`"running"` until the matching `tool_result` arrives). A "background this specific call" UI action would need to read from the document layer, not the coarser pane atoms.
- **No existing per-tool-call interactive UI to model this after.** `ToolBlock.tsx`'s only interactive elements are pin-toggle and a hover peek — both presentational, not RPC-backed. A prior action-bar concept (open-in-pane/open-in-window/new-agent-here) was explicitly removed for being non-functional stubs. A "move to background" button would be **new UI surface**, not an extension of an existing pattern — budget design/review time accordingly.

## 5. Design directions

### 5.1 Auto-backgrounding "obviously long-running" commands (the lower-risk half — does NOT depend on §1)

The `PreToolUse` hook (`agentmux-mcp`/wherever the hook rewrite lives) already reads and rewrites the full tool input **before** Claude Code's own harness dispatches it. Since `run_in_background` is read from `tool_input` at that same point (already confirmed: `hook.rs::PreToolUseInput` deserializes the full raw params), the hook could pattern-match commands that are unambiguously long-running by construction (e.g. `sleep N` for large N, known dev-server launch patterns, or a user-configurable command-pattern list) and **force `run_in_background: true` into the tool input it hands back**, before Claude's own harness ever decides synchronous-vs-async. Claude's native harness would then treat it exactly as if the model had requested backgrounding — no interception of an in-flight call needed, no PTY/stream-json ambiguity, and it plugs directly into everything already shipped.

Design considerations for a future spec, not resolved here:
- False-positive risk: forcing background on a command the user actually wanted to watch synchronously (e.g., a `sleep 5` used deliberately for pacing between two dependent steps) changes observable behavior in a way the model didn't ask for — needs a conservative, probably user-configurable pattern list, not a broad heuristic.
- Whether this should be a global default or an opt-in setting.
- Interaction with the model's own judgment: if the model already knows to set `run_in_background` for the commands that matter, how much residual value does a hook-side heuristic add versus the false-positive risk it introduces?

### 5.2 Retroactive mid-flight toggle (the harder half — blocked on §1)

Assuming §1's spike finds a viable mechanism (either a stream-json control message, or byte-injection genuinely working against the persistent controller's stdin), the remaining pieces, informed by §3/§4:

1. **Trigger surface:** a per-tool-call UI action (new surface, per §4) offered only while `ToolNode.status === "running"` for a Bash call specifically — read from the document layer, not the pane-level atoms.
2. **Mechanism:** whatever §1 validates — a structured control-request analogous to `COMMAND_AGENT_ANSWER`'s existing pattern is architecturally preferable to raw byte injection if available, since it doesn't require punching a new hole through `PersistentSubprocessController`'s deliberate "no raw input" contract.
3. **Downstream handling:** once backgrounded via either path, everything from Phase A/B/C (PID capture, teardown survival, dashboard read-source) should apply unchanged, *provided* the resulting tool_result matches the same `"Command running in background with ID: ..."` shape `isAcceptedBackgroundLaunch()` already detects. If the stream-json path produces a differently-shaped signal, that detection function (and its `#2519` fast-finish-misclassification fix) would need a second recognized shape — flag this explicitly as a likely follow-on cost, not a free extension.
4. **No reverse (background → foreground) action** — matches Claude Code's own documented absence of one; not worth inventing a capability the underlying CLI itself doesn't support.

## 6. Phasing

1. **Spike (§1):** resolve whether any retroactive backgrounding mechanism applies to `PersistentSubprocessController`-hosted sessions at all. This determines whether §5.2 is buildable as scoped, needs a different mechanism, or should be dropped in favor of §5.1 alone. Time-boxed — this is a feasibility question, not an open-ended research task.
2. **§5.1 (auto-backgrounding heuristic)** can proceed independently of the spike's outcome, as its own small, separately-reviewable feature, once scoped (false-positive policy, pattern list, config surface) in a follow-up spec.
3. **§5.2 (retroactive toggle)** — only scoped further once the spike lands; this spec deliberately stops short of a concrete implementation plan for it given the unresolved feasibility question.

## 7. Non-goals

- Making all tool calls async by default (explicitly rejected in the conversation leading to this spec — breaks the model's own reasoning for tools where a real result is needed now, and most tools have no underlying "return early" protocol support to begin with).
- A foreground-restore ("bg → fg") action for an already-backgrounded task — no precedent in Claude Code itself, not worth inventing independently.
- Rebuilding AgentMux's existing background-task registry (Phase A/B/C) — this spec's job is routing a NEW trigger (a UI-initiated toggle, or a hook-side heuristic) into that EXISTING, already-shipped infrastructure, not replacing any part of it.
