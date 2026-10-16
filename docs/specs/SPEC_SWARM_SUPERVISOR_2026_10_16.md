# Spec: the Swarm Supervisor keeps agents working until they are done or need the user

**Date:** 2026-10-16
**Status:** implemented — #5036, and checked live in a dev build on 2026-10-16 (§5)
**Author:** Camper@narko, at the operator's request
**Research:** [`REPORT_SWARM_SUPERVISOR_AND_FLEET_FEATURES_2026_10_16.md`](../analysis/REPORT_SWARM_SUPERVISOR_AND_FLEET_FEATURES_2026_10_16.md) (the name, the modes, industry practice, other fleet features)
**Replaces:** the Warden Supervisor auto-continue ([`ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md`](../analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md)), removed in the same PR
**Related:** [`SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md`](SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md) (the turn ledger and `TriggerKind`), [`SPEC_SWARM_BROADCAST_AS_USER_MESSAGE_2026_10_01.md`](SPEC_SWARM_BROADCAST_AS_USER_MESSAGE_2026_10_01.md) (a header only srv writes), [`SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md`](SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md) (the Haiku call path)

## 1. Problem

Agents often end a turn asking "shall I continue?" or stop short of a task they had clearly scoped, and wait. In a swarm of agents, each such stop sits idle until the user notices. Swarm then shows that agent as idle, the same as one that finished: the "Waiting for you" state exists only for a pending `AskUserQuestion` or permission prompt.

AgentMux had a narrow answer: Warden's auto-continue. An agent opted in (`auto_continue_enabled`) could receive a fixed "Continue the task you were already doing." from the `SupervisorNudge` tool, at most 5 times in a row. But nothing decided when to send it: the operator had to run a watcher agent of their own. It lived in Warden, which is for watching, not acting, so it was rarely used.

The operator, 2026-10-16:
- it is "the swarm's supervisor";
- "aggressive: it tells them keep working using best recommendations and push forward with best judgement on unknowns after thorough research";
- "normal: keep going to complete your work, if there are unknowns or real serious questions, pause";
- "Swarm isn't watch-only... Swarm is active input"; the Warden work "doesn't belong there".

## 2. What it does

Each agent pane has a Supervisor mode: **Off** (the default), **Steady** or **Drive**. When a supervised agent ends a turn, the Supervisor:
1. reads the agent's recent conversation;
2. asks a cheap judge model (Haiku) for a verdict;
3. either sends the agent a continuation, or stops and tells the user why.

| Mode | Continues the agent when | Stops for the user when |
|---|---|---|
| Steady | it stopped only to check in, or stopped short of work it had already scoped | it is done, blocked, asks a real question or a choice only the user can make, or proposes something risky or irreversible |
| Drive | as Steady, and also through open questions: the agent is told to research them, decide on its best judgement, and say what it assumed | it is done, blocked, needs something only the user has (a credential, a sign-in, a product decision with no sensible default), or proposes something risky or irreversible |

Both modes stop when a limit is reached (§3.4). Every decision is recorded and shown in Swarm.

### 2.1 Non-goals (v1)

- **Providers other than Claude.** The judge reads the agent's conversation from the block's `output` file, which srv can read for Claude's stream (`ambient::digest::reads_output_format`). The judge also runs on the Claude CLI. Swarm offers the Supervisor only for Claude agents and says so for the rest.
- **Agents on other machines.** A fleet action that sets the mode reaches this instance's agents only, as Stop does today.
- **Approving tools or answering questions.** The Supervisor never answers an `AskUserQuestion` or approves a permission prompt: those keep their own paths and the 30 s question timeout. "Keep going" never means "approve everything".
- **Assigning new work.** It continues the task the agent already has. The task board in the report (§6, item 9) is a later step.

## 3. Design

### 3.1 The mode

- **Where it lives:** block meta `supervisor:mode`, `"steady"` or `"drive"`. Absent or anything else is Off.
- **Why on the pane:** a Swarm row is a pane, and "set these agents to Drive" is about running agents. It needs no schema change, it reaches Swarm through the block updates Swarm already reads, and a newly opened agent starts Off.
- **How it is set:** one RPC, `supervisor.set-mode { block_ids, mode }`, from the Swarm row's chip and from the fleet toolbar.
  - It writes the meta, resets the run state (§3.5), and records the change.
  - If the agent is idle at that moment, its last turn is judged at once. Turning on Drive for an agent that just asked "shall I continue?" then gets it going without waiting for another turn.
  - Setting Off clears the state.

### 3.2 When a turn is judged

The Supervisor watches the turn ledger (`agentturn`) for completed turns, as the notification router does (`notify::router::parse_ledger_end`). For a turn on a supervised pane, it decides in code, before any model call:

| Condition | What happens |
|---|---|
| Mode is Off, or the pane is not a Claude agent | Nothing. |
| The turn was AgentMux's own hidden turn (`TriggerKind::System`) | Nothing. |
| The turn ends in a question tool (`AskUserQuestion`, `ExitPlanMode`), or the newest message is the user's | Stop: needs the user. No model call. |
| The agent has background tasks still running (`db_background_tasks`) | Wait: the CLI wakes the agent when they finish, and that turn is judged then. |
| A limit is reached (§3.4) | Stop: limit, with the reason. |
| Otherwise | Judge (§3.3). |

One judgement per pane runs at a time. A turn that ends while one is running supersedes it (the ambient gateway cancels the older call).

### 3.3 The judge

A new ambient purpose, `supervisor_verdict` (background class, 90 s), on the existing gateway, so it gets the same caps, cancellation, spend accounting and outcome log as every other Haiku call. It uses the pane's own CLI and auth (`CliTarget::from_meta`).

**The prompt** gives:
- the mode and what it allows;
- the agent's recent activity digest (`read_recent_activity`, the same bounded digest the Swarm summaries use);
- how many continuations this run has had.

It asks for exactly one line:

```
VERDICT: <continue|done|needs_user|blocked> | REASON: <one sentence> | NEXT: <one sentence, Drive only>
```

**Rules the prompt states:**
- "done" needs evidence in the transcript, such as tests passing, a PR merged or opened, or a result shown. A bare claim is not enough: in Drive it becomes `continue` with "show the evidence", and in Steady `needs_user`.
- Risky, irreversible or outward-facing next steps are `needs_user`: deleting, force-pushing, deploying, spending money, publishing, or messaging people outside the swarm.
- A question only the user can answer is `needs_user` in both modes. In Drive, a question the agent can research or decide itself is `continue`.
- A turn that ends by asking the user to approve, choose or decide something is never `done`, even when the user told the agent to ask.

**Time and retries.** A verdict took 14 to 27 s on a busy shared login in the dev-build test, and 45 s timed out, so the limit is 90 s, like the continuity state's. A call that fails or times out is tried once more, if the turn is still the latest and the agent is still idle.

**Parsing** is strict. A malformed reply, an empty reply, or a call that failed twice is treated as "no verdict". The Supervisor stops, says it couldn't judge, and the user gets the ordinary "finished" notification. It never continues on a guess.

### 3.4 Limits

| Limit | Steady | Drive |
|---|---|---|
| Continuations in a row since the user last spoke | 5 | 15 |
| Time since the first continuation in the run | none | 2 h |
| Continued turns in a row with no tool use (ledger `steps <= 1`) | 2 | 2 |
| The same final message twice in a row | stop | stop |

Reaching a limit stops the agent with "needs you" and the reason (for example "5 continuations in a row"). Any turn the user starts resets the run, whether typed in the pane or broadcast from Swarm. So does changing the mode. A turn another agent or a service started is judged like any other, but it doesn't reset the run.

### 3.5 The continuation

It is delivered through the same route a broadcast takes (`bootstrap::route_agent_message_from`), as **automated** input (`TurnOrigin::Automated`), never as the user's. Nothing that needs the user's own word can be authorised by it, such as an agent quitting itself or an AgentMux upgrade. The text is:

```
[SUPERVISOR:MODE=drive N=3 OF=15]
<fixed instruction for the mode>
Why: <the judge's reason>
Suggested next step: <the judge's NEXT, Drive only>
```

- **The fixed instruction:**
  - Steady: "Continue the task you were already doing. Ask the user only if something truly needs them."
  - Drive: "Keep going without waiting for confirmation. Research open questions thoroughly, choose the best option on your own judgement, and say what you assumed. Stop and ask only for something irreversible or outward-facing, for something only the user can provide, or if you are blocked."
  - Both end: "If the work is done, say so and show the evidence."
- **Only srv writes the header.** `[SUPERVISOR:` joins the marker words `neutralize_markers` quotes inside every delivered body, so a jekt or a broadcast can't forge one.
- **Model text is quoted.** The judge's reason and next step come from a model reading the agent's own transcript. They go through `quote_in_system_note`, flattened to one line and capped at 300 characters, so they can't open a marker or add lines of their own.
- **The turn is labelled.** `TurnTrigger::from_input` classifies an automated input that starts with the header as a new `TriggerKind::Supervisor`. It is external, so its ending doesn't fire the generic "finished" notification (§3.7). The pane and Swarm can say a turn was started by the Supervisor.
- **No route, no continuation.** If the agent's process isn't running (the route answers "start a turn"), the Supervisor doesn't start one. It stops with "the agent isn't running".

### 3.6 State and history

The Supervisor keeps per-pane run state in memory and mirrors it to block meta `supervisor:state` after every decision, so Swarm renders it and it survives an srv restart:

```json
{
  "mode": "drive",
  "status": "judging | continued | paused | done | waiting | off",
  "reason": "…",
  "continuations": 3,
  "limit": 15,
  "run_started_at_ms": 0,
  "updated_at_ms": 0,
  "history": [{ "at_ms": 0, "verdict": "continue", "action": "continued", "reason": "…" }]
}
```

- **History** keeps the last 10 decisions, each with the reason, for the row's detail list. The state is rewritten whole each time, and is small.
- **"paused"** means the Supervisor stopped and the user is needed. Swarm ranks it as "needs you" while the agent is idle. A new turn makes the agent live, which outranks it.

### 3.7 Notifications

For a supervised pane, the router skips its generic "finished" notification, because the Supervisor speaks instead:
- **Paused:** `InputWaiting`, with "Supervisor paused: <reason>", through the router's own redaction.
- **Done:** `TurnCompleted`.
- **No verdict:** `TurnCompleted`, as if the Supervisor were off.
- **Continued:** nothing.

### 3.8 Swarm

- **On each Claude agent row:** a Supervisor chip showing Off, Steady or Drive and the current state ("judging", "continued 3/15", "paused"). Clicking it opens Off / Steady / Drive.
- **On the row's line:** a paused Supervisor shows "Needs you: <reason>", ranked like a pending question.
- **In the row's children:** a "Supervisor" bucket listing the recent decisions (time, verdict, reason).
- **In the fleet toolbar:** "Supervisor" with Off / Steady / Drive for the selected agents, reporting each agent's outcome like Broadcast and Stop do.
- **In the agent pane:** a continuation renders like a broadcast: the instruction, with a "Supervisor" label in place of the `[SUPERVISOR:…]` header.

### 3.9 What is removed

The Warden Supervisor panel (`frontend/app/view/warden-supervisor/`), the `SupervisorNudge` MCP tool (the `Agents` tool's `nudge` action), `POST /agentmux/reactive/supervisor-decision`, the nudge counters and fixed nudge in the reactive handler, and the `auto_continue_enabled` field everywhere in code. The database column stays, unused, because older builds share the data directory and expect it.

## 4. Tests

- **Pure policy** (`backend/supervisor/policy.rs`):
  - the pre-judge table in §3.2;
  - every limit in §3.4;
  - the user-turn and mode-change resets;
  - the verdict-to-action table.
- **Judge reply parsing:** well-formed replies in each mode, malformed and empty replies, and an over-long reason.
- **Continuation text:**
  - the header shape;
  - a forged `[SUPERVISOR:` inside a jekt or broadcast body is quoted;
  - the judge's text can't add a line or a marker.
- **Turn labelling:** `TurnTrigger::from_input` labels the header as `Supervisor` only for automated input.
- **Router:** a supervised pane gets no generic "finished" notification.
- **Frontend:**
  - the mode and state readers;
  - the "needs you" ranking for a paused, idle agent;
  - the row chip and menu;
  - the fleet toolbar action;
  - the pane rendering of a continuation.
- **Live, in a dev build:** a Claude agent asked to do a multi-step task and to "ask before each step" is continued in Steady and in Drive, stops at the limit, and stops for a risky step.

## 5. Live check (dev build, 2026-10-16)

A throwaway Claude agent (Haiku) in an isolated `task dev` build, driven over its DevTools port through the same RPCs the Swarm uses:

1. **Steady, "ask me after each of three files":** after step 1 it asked to continue; the judge said `continue`, the Supervisor sent continuation 1 of 5 (shown in the pane as "Supervisor · Steady · 1 of 5", the footer "Worked on a Supervisor continuation"), the agent made the other two files, and the judge said `done`, with the tool results as evidence. Swarm showed "Steady · done".
2. **Drive, an open choice and then a delete:** the agent asked both questions with the question tool, so AgentMux's 30-second question timeout answered them, delete included, before the Supervisor was involved (the Supervisor never answers that tool, §2.1). Its one continuation then asked for evidence, and it judged the result `done`.
3. **Drive, permission to delete asked in plain text:** the first run timed out at 45 s and, by hand, the judge called the turn `done` ("it asked as told"). Fixed: 90 s with one retry, and a turn ending in a question is never `done` (§3.3). After the fix: `needs_user`, "asking permission to delete … a risky, irreversible action"; nothing was continued, the files were kept, and Swarm showed "Drive paused" and "Needs you: …".

The judge prompt is also checked against real Haiku by `live_supervisor_judge_answers_in_format_and_judges_the_clear_cases` (ignored by default; nine cases, all in format and right as of this date).
