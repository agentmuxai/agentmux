# The Swarm Supervisor: keeping agents going, and other fleet features

**Status:** the Supervisor is being built from this report: [`SPEC_SWARM_SUPERVISOR_2026_10_16.md`](../specs/SPEC_SWARM_SUPERVISOR_2026_10_16.md) is the design as built, and supersedes sections 4 and 7 where they differ. Written 2026-10-16 at the operator's request: name a feature that reviews each agent's output when its turn ends and tells it to keep going (an aggressive mode and a normal mode), and gather other swarm features and industry best practice.

## 1. Summary

- **Name: Supervisor**, the Swarm's supervisor (the operator's choice). Each agent and the Swarm toolbar show "Supervisor: Off / Steady / Drive". Section 2 covers the name, and names to avoid (GitHub already ships this concept as "Autopilot").
- **Where it lives:** in Swarm, entirely: its controls, its "needs you" output and its decision history. The operator: Swarm is active input, Warden is for watching, and the Warden auto-continue "doesn't belong there", so it is removed (section 4.4).
- **A narrow version already exists.** Warden's auto-continue (`db_agents.auto_continue_enabled`, `SupervisorNudge`) sends a fixed "Continue the task you were already doing." to an opted-in agent, at most 5 times in a row. But the judging is done by an ordinary agent the operator has to start and prompt themselves. srv has no watcher. The Supervisor should grow out of it, not sit beside it (section 3).
- **The design the industry has converged on:** after each turn, a cheap judge model returns a structured verdict (continue / done / needs a human / blocked) with a reason. Hard caps on turns, time and cost apply in every mode. Stall detection, judging on evidence rather than the agent's own claims, and a full audit complete it. Claude Code's `/goal` is the closest shipped example (section 5).
- **Two decisions are yours** (section 7):
  1. Drive mode deliberately crosses a guardrail the August design set: "never compose free-form instructions, only a fixed narrow nudge".
  2. Putting the Supervisor in Swarm changes Swarm's documented "watch, don't orchestrate" contract.

## 2. The name

### Chosen: Supervisor (the Swarm's supervisor)

The operator chose this name on 2026-10-16.

- **Its role is clear.** It watches the swarm's agents as each one finishes, keeps them working, and calls the human when they really need one.
- **The code already uses the word.** The existing pieces are named `SupervisorNudge`, `/agentmux/reactive/supervisor-decision` and the Warden Supervisor panel, so the name continues what's there rather than adding a new word.
- **It's a generic term.** It names an architecture pattern (LangGraph's "supervisor", CrewAI's manager), not someone else's product.
- **It reads well as a setting:** "Supervisor: Steady", "Supervisor: Drive", "turn the Supervisor off for Agent5".
- **It reads well as an event:** "Supervisor continued Agent3 (2 of 5)", "Supervisor paused Agent3: needs you".

Two consequences:

1. **Warden's Supervisor panel goes.** The per-agent switch and the decision history move to Swarm, and the old panel, tool and endpoint are removed. Otherwise there would be two things called "Supervisor" in two places.
2. **Expect people to read "supervisor" as an orchestrator that hands out work.** In multi-agent frameworks the supervisor also assigns tasks. V1 doesn't do that, but the task board in section 6 (item 9) would grow into it naturally, so the name has room for it.

### Modes

| Mode | What the judge does when a turn ends | Your words |
|---|---|---|
| **Off** | Nothing. Today's behaviour. | |
| **Steady** | Continues the agent when it stopped only to ask permission, or stopped short of a task it had clearly scoped. Pauses for the human on real unknowns, real choices, risky or irreversible actions, and "done". | "Normal: keep going to complete your work; if there are unknowns or serious questions, pause." |
| **Drive** | Continues the agent through unknowns too. The agent must research them thoroughly, decide on its best judgement, and record what it assumed. It still pauses for risky or irreversible actions, for "blocked", for anything only the human can provide (a credential, a sign-in, a product decision with no sensible default), and when a cap is reached. | "Aggressive: keep working on best recommendations; push forward on unknowns after thorough research." |

"Normal" and "Aggressive" also work as labels. Steady and Drive avoid "aggressive" in the UI, which can read as "ignores safety", and Drive explicitly doesn't.

### Alternatives considered, and names to avoid

| Name | Verdict |
|---|---|
| Momentum | The first recommendation: it names the benefit (keeping agents moving), and no well-known agent product uses it. Dropped in favour of Supervisor. |
| **Second Wind** | Clear of conflicts and charming, but a phrase rather than a noun ("Second Wind: Drive" reads oddly). |
| **Keep Going** | Plain and self-explanatory. |
| Relay, Pacer | Taken (relayevals' agent supervisor) or weak. |
| **Autopilot** | Avoid. GitHub Copilot CLI and VS Code ship exactly this concept under that name, with `--max-autopilot-continues`. |
| **Shepherd**, **Foreman**, **Overseer**, **Watchdog** | Avoid. Open-source agent supervisors with these names already exist (agent-shepherd, thruwire/foreman, agent-overseer, Claude Watchdog). |
| `/goal`, `/loop`, Ralph, YOLO, auto mode | Avoid. Claude Code, Cursor and Cline built-ins and techniques. |
| Warden Auto-Controller | The August design's name. Superseded by Supervisor. |

## 3. What AgentMux has today

**Swarm (observe and broadcast).** `frontend/app/view/swarm/` lists tracked agents and their live state. A broadcast goes out as `[BROADCAST:FROM=user VIA=swarm ...]` with `TurnOrigin::User` (`crates/srv/src/server/app_api/fleet.rs`). `docs/specs/swarm-analysis.md` defines Swarm as a place to watch agents, not orchestrate them.

**Turn end is already detected precisely.** `TurnActivityTracker` and the turn ledger (`crates/srv/src/backend/blockcontroller/health.rs`) end a turn on the provider's own signal: the stream-json `result` frame for Claude, ACP and Codex App Server events. They publish `controllerstatus` (`turn_active`) and `agentturn` (the ledger). The ledger also says who started each turn (`TriggerKind`: User, Broadcast, Agent, Service, Schedule, Task, System). `crates/srv/src/backend/notify/router.rs` already consumes completed turns and filters out system turns, which makes it the template for a Supervisor watcher.

**Warden auto-continue (the existing narrow version).** `docs/analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md` was implemented:
- a per-agent opt-in, `auto_continue_enabled` (off by default), toggled in `frontend/app/view/warden-supervisor/`;
- `SupervisorNudge` (MCP) posts to `/agentmux/reactive/supervisor-decision`, and srv rejects nudges to agents that aren't opted in;
- srv sends only the fixed text `NUDGE_MESSAGE` ("Continue the task you were already doing."), at most `MAX_CONSECUTIVE_AUTO_CONTINUES = 5` in a row, reset after a 30-minute cooldown, and logged to Warden's audit feed (`crates/srv/src/backend/reactive/handler.rs`).

What it lacks: nothing in srv watches turn ends or judges them. The operator has to run an agent that reads transcripts and calls `SupervisorNudge`, so in practice it is rarely on.

**Gap: a prose question looks like "idle".** "Waiting for you" exists only when an `AskUserQuestion` or permission prompt is pending (`term:awaiting_user`, written by the pane). An agent that ends its turn with "Want me to continue with phase 2?" shows as Idle in Swarm, exactly like an agent that finished. That is the case the Supervisor targets, and the same judgement would fix the Swarm status line.

## 4. Proposed design

### 4.1 Shape

1. **A watcher in srv**, modelled on `notify/router.rs`. It subscribes to completed turns of agents whose Supervisor mode is Steady or Drive. It skips turns that are still settling, agents with background work running (subagents, background shells), and turns the Supervisor itself started that produced nothing (see stall detection).
2. **A judge call.** A cheap, fast model (default Claude Haiku 4.5; configurable) receives:
   - the agent's goal, if one is set (see 6.1);
   - the last user or broadcast instruction;
   - the last assistant message;
   - a compact tail of tool calls and their results.

   It returns structured output:

   | Field | Values or meaning |
   |---|---|
   | `verdict` | `continue` / `done` / `needs_human` / `blocked` |
   | `reason` | one or two sentences |
   | `guidance` | Drive only: the recommended next step to take |
   | `risk` | true if the next step is irreversible or outward-facing |

3. **An action.**
   - `continue` (and no cap reached, and `risk` false): inject the continuation.
   - `needs_human` or `blocked`: mark the agent **Needs you** in Swarm with the reason, and notify through the existing notify router (desktop, mobile).
   - `done`: mark it done; optionally hand it to review (6.4).
4. **Record every decision**, including declined ones, in the pane's Supervisor history, shown in Swarm: verdict, reason and what it did.

### 4.2 What the continuation says

- **Steady:** keep the fixed text. "Continue the task you were already doing." Optionally add the judge's one-line reason ("You asked whether to run the tests; run them.").
- **Drive:** a fixed frame plus the judge's guidance, for example:
  > Keep going without waiting for confirmation. Research open questions thoroughly, choose the best option on your own judgement, and note each assumption you made. Stop and ask only for irreversible or outward-facing actions, anything only the user can supply, or if you are blocked. Suggested next step: <guidance>

  The frame is fixed. Only the "suggested next step" is composed by the judge, it is capped in length, and it must stay within the task the user gave.

### 4.3 Guardrails (every mode)

- **A continuation is not the user.** Inject it with its own trigger (`TriggerKind::Service`, or a new `Supervisor`), never as `TurnOrigin::User`. That way nothing that needs the user's own word (self-quit, App upgrade, destructive confirmations) can be authorised by it. The agent sees a header such as `[AgentMux] Supervisor (Drive, 3 of 12): ...`, so it knows the source. That header needs the same anti-forgery rewrite as other `[AgentMux]` notes.
- **Caps, all independent:**
  - consecutive continuations (Steady 5, as today; Drive 15);
  - wall-clock time per run (Drive default 2 h);
  - optionally, tokens or cost when the provider reports usage.

  When a cap is reached, the agent goes to Needs you with a summary, never silently idle.
- **Stall and loop detection.** Pause and escalate if any of these happens:
  - two consecutive continued turns with no tool use;
  - the same tool error repeated three times;
  - near-identical assistant messages;
  - the agent ending turns that only restate its plan.

  (These follow OpenHands' `StuckDetector` patterns and `/goal`'s no-tool-use stop.)
- **Evidence over claims.** The judge prompt treats "done" and "fixed" as unproven unless the transcript shows proof such as test output, a PR URL or a merge. A self-claimed "done" without evidence becomes `continue` with "show the evidence" in Drive, and `needs_human` in Steady. Research cited in section 5 reports most agent self-stops disagreeing with a verifier.
- **Risk is separate from continuation.** The Supervisor never approves a tool permission prompt or an `AskUserQuestion`. Those stay with the existing approval paths and the 30 s question timeout. "Keep going" never means "approve everything" (Copilot keeps Autopilot and Allow-all as separate switches for this reason).
- **Human override always wins.** Any message the user types into the pane resets the count and takes precedence. A visible "Supervisor paused" control on the Swarm row and agent header stops it instantly.
- **Context exhaustion.** If the agent is near its context limit, Drive should ask it to write a handoff note and continue in a fresh session (AgentMux's own continuation mechanism), not push it into compaction loops.

### 4.4 Where it lives

- **Setting:** the existing per-agent `auto_continue_enabled` becomes a mode (`off | steady | drive`), migrated with on → steady. Optionally add a per-workspace default later.
- **Controls:** a Supervisor chip on each Swarm row and the agent pane header. Swarm's fleet toolbar gets "Set Supervisor mode for selection", which fits the existing select-then-act pattern.
- **History:** the last decisions show in the Swarm row's Supervisor bucket. The Warden Supervisor panel is removed.
- **Judge credentials:** the operator's existing provider account, through Identity. If no cheap model is available, fall back to the agent's own provider with a warning.

## 5. Industry practice (summary; sources in the appendix)

| Product / technique | What it does | Lesson |
|---|---|---|
| Claude Code `/goal` | After each turn a Haiku evaluator judges a condition: not met (reason becomes guidance) / met / impossible. Stops after several turns with no tool use; skips evaluation while background work runs; the evaluator sees only the transcript. | Closest match. Three-way verdict, stall stop, evidence in the transcript. |
| Claude Code Stop hooks | `decision: "block"` with a `reason` returns the agent to work. `TeammateIdle` and `TaskCompleted` hooks keep agent-team members going. | The provider-native mechanism. AgentMux installs no Stop hook today; one could be an alternative transport for Claude agents. |
| Ralph loop | A shell loop or Stop-hook plugin that re-feeds the prompt; `--max-iterations` plus an exact "completion promise". | Always cap. A single exact-match "done" string can't express "blocked". |
| GitHub Copilot Autopilot | Continues without new prompts until done, a problem, Ctrl+C or `--max-autopilot-continues`; separate from `--allow-all`. | Continuation and approval are separate switches. |
| Codex CLI | `approval_policy` × `sandbox_mode`; `auto_review` sends approvals to a reviewer model. | An LLM reviewer for approvals is now common. |
| Cursor / Cline / Roo | Auto-run, Auto-review and YOLO modes; Roo caps requests (`allowedMaxRequests`) and cost (`allowedMaxCost`). | Cost and request caps are table stakes; denylists were bypassed. |
| Claude Code auto mode | A classifier reviews each action, and falls back to prompting after 3 consecutive or 20 total blocks. | Escalate to a human after repeated refusals. |
| Factory Droid | `--auto low/medium/high` autonomy tiers; actions above the tier fail fast. | Tiers of autonomy, not one switch. |
| Devin | Idle sessions sleep; per-session and org usage caps; a toast near the limit. | Budgets visible to the user. |
| OpenHands | Headless mode with `max_iterations`, budget caps and a `StuckDetector` (repeated action/observation, repeated error, monologue, alternating patterns, context-error loops). | Concrete stall patterns to copy. |
| AutoGen / LangGraph / OpenAI Agents SDK | Composable termination conditions (text trigger OR message cap), recursion limits, `max_turns`, guardrail tripwires. | A completion signal must always be paired with a hard cap. |
| Anthropic long-running harness | An initializer writes a feature list (all `passes: false`) and a progress file so the agent can't consider the project complete early. | An explicit definition of done beats judging prose. |

The same failure modes come up again and again:
- premature termination (a named failure mode in the MAST taxonomy);
- false "done" claims;
- runaway loops without caps;
- consent-chain degradation, where "keep going" quietly turns into "do anything";
- condensation and compaction loops near the context limit.

## 6. Other swarm features worth building

In rough priority order: value to you first, then how much existing machinery it reuses.

1. **Needs-you inbox.** One Swarm view of every agent waiting on the human, whatever the reason: AskUserQuestion, permission prompt, a prose question (from the Supervisor judge), blocked, cap reached. Each entry has a one-line reason and a reply box. This gap exists today even without the Supervisor.
2. **Goals and a definition of done per agent.** Set a goal when assigning work ("PR merged with muxreview approval and tests green"). The Supervisor judges against it, and Swarm shows progress against it. This is `/goal`'s model, and it makes the judge far more reliable than reading prose.
3. **Budgets.** Per-agent and fleet caps on time, turns and (where reported) tokens or cost, shown on the Swarm row, with a warning before the cap and Needs you at it.
4. **Stall and loop detection on its own.** The patterns in 4.3, shown as a Swarm status ("looping on the same test failure for 20 min") even for agents with the Supervisor off.
5. **Review on done.** When an agent reports done, a second agent or model reviews the diff or result before the human sees it (Jules's "Planning Critic", Amp's Oracle). AgentMux already has the messaging; this is a configured hand-off.
6. **Plan approval with timed auto-approve.** In Drive, the agent posts a plan first. The plan is auto-approved after N minutes unless the human objects (Jules). It catches wrong directions early without blocking.
7. **Fleet digest.** A periodic summary (in Swarm, and pushed to mobile) of what each agent did, what's blocked and what needs you. The ambient Swarm summary already writes per-agent lines.
8. **Collision surfacing in Swarm.** The overlap notes and Presence claims already exist. Show them on the Swarm rows ("Agent4 and Agent5 both editing `crates/x.rs`") and offer "message both".
9. **Task board.** Show the existing `Tasks` queue in Swarm as a board (queued / claimed / done) that the human can drag work into, with idle agents in Drive pulling the next task. This revives the useful part of the superseded `swarm-orchestration.md`.
10. **Best-of-N.** Run the same task on N agents (or models) in separate clones and compare the results side by side (Codex `--attempts`, Conductor).
11. **Checkpoints.** Record a git checkpoint at each Supervisor continuation, so the human can roll an agent back to before a bad direction.
12. **Context handoff.** One button (and a Drive behaviour) to have an agent write a handoff note and continue in a fresh session before its context degrades (Amp's `/handoff`, Ralph's fresh-session loop).

## 7. Decisions (settled 2026-10-16)

The operator chose the name and the placement, and asked for the report's recommendations on the rest ("use best recommendations from your report"):

1. **Name and modes:** Supervisor, the Swarm's supervisor, with Off / Steady / Drive.
2. **Drive goes further than the August guardrail allowed.** The 2026-08-12 design said the nudge must be fixed text, never "do whatever you think is best". Drive deliberately relaxes that, which is what the operator asked for ("aggressive"). The mitigations are in 4.2 and 4.3: a fixed frame, short composed guidance limited to the task, risk gating and caps.
3. **Swarm is where agents are steered.** The operator: "Swarm isn't watch-only... Swarm is active input"; Warden is for watching. The controls, the needs-you output and the history all live in Swarm. The Warden auto-continue is removed, and `swarm-analysis.md` now says Swarm watches and steers.
4. **Transport:** a continuation turn delivered by srv, for every provider the judge can read (Claude in v1), not a Claude Code Stop hook.
5. **Caps:** Steady 5 in a row, Drive 15 in a row or 2 hours, both stopping after two continued turns with no tool use. Tune after use.

## Appendix: sources

- Claude Code hooks (Stop decision control): https://code.claude.com/docs/en/hooks
- Claude Code agent teams: https://code.claude.com/docs/en/agent-teams
- Claude Code `/goal`: https://code.claude.com/docs/en/goal
- Claude Code security / auto mode: https://code.claude.com/docs/en/security
- Ralph Wiggum technique: https://github.com/ghuntley/how-to-ralph-wiggum
- Ralph Wiggum plugin: https://github.com/anthropics/claude-code/blob/main/plugins/ralph-wiggum/README.md
- GitHub Copilot CLI Autopilot: https://docs.github.com/en/copilot/concepts/agents/copilot-cli/autopilot
- Codex CLI approval and sandbox modes: https://inventivehq.com/blog/ai-coding-cli-sandbox-approval-modes-compared
- Cursor YOLO / auto-review: https://forum.cursor.com/t/yolo-mode-is-amazing/36262, https://forum.cursor.com/t/auto-review-run-mode/161922, https://www.theregister.com/2025/07/21/cursor_ai_safeguards_easily_bypassed/
- Cline YOLO mode: https://docs.cline.bot/enterprise-solutions/configuration/infrastructure-configuration/control-other-cline-features/yolo-mode
- Roo Code auto-approve: https://docs.roocode.com/features/auto-approving-actions
- Factory Droid exec: https://docs.factory.ai/droid-exec/overview
- Devin usage policies: https://docs.devin.ai/enterprise/features/usage-policies
- OpenHands SDK and stuck detection: https://docs.openhands.dev/sdk/api-reference/openhands.sdk.conversation, https://github.com/OpenHands/OpenHands/pull/6865, https://docs.openhands.dev/sdk/arch/condenser
- AutoGen termination: https://microsoft.github.io/autogen/stable//user-guide/agentchat-user-guide/tutorial/termination.html
- LangGraph recursion limits: https://machinelearningplus.com/gen-ai/langgraph-cycles-recursion-limits-agent-loops/
- CrewAI hierarchical process: https://docs.crewai.com/en/learn/hierarchical-process
- OpenAI Agents SDK: https://openai.github.io/openai-agents-python/guardrails/, https://openai.github.io/openai-agents-python/running_agents/
- Anthropic, effective harnesses for long-running agents: https://anthropic.com/engineering/effective-harnesses-for-long-running-agents
- MAST failure taxonomy: https://arxiv.org/pdf/2503.13657
- Agent supervisor budgets (Relay): https://relayevals.com/blog/what-is-an-ai-agent-supervisor
- agent-shepherd: https://github.com/CallMeHFK/agent-shepherd; thruwire/foreman: https://github.com/thruwire/foreman; agent-overseer: https://github.com/troyshu/agent-overseer
- Parallel agent orchestration: https://addyosmani.com/blog/code-agent-orchestra/
- Jules plan review: https://jules.google/docs/review-plan/

Some third-party sources (blog posts, recent preprints) were not independently verified. Where they disagree on a detail, the report doesn't depend on it.
