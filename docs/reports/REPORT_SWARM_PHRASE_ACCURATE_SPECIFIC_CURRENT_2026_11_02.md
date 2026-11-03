# Report: The Swarm's one-line phrase per agent: how to make it accurate, specific and current

**Date:** 2026-11-02 (research done against AgentMux `origin/main` at `cea4fdd`, the 0.60.12 line)
**Task:** #20815 (Research). **Author:** Agent2-5@narko. **Status:** draft for the operator's decision; research only, no code.
**Audience:** AgentY (front door), Camper (owns the Swarm and builds from this).
**Publishing note:** this report stays uncommitted. The pilot table in section 2 names agents and public-repo tasks only; rows about private-repo work are described generally. Check that again before it ships in any public PR.

The operator's words (2026-11-02): "we also want to refine how the swarm gets its short phrase. it isnt accurate often, too general, or just wrong. lets research best practice on how this should be done, including how claude and the major labs do it, I dont think we ever did real research."

---

## 0. The decision, and the recommendation (revised 2026-11-03)

**The decision:** how AgentMux produces the one-line phrase the Swarm row and `Agents conversations` (`work.goal`) show for each agent.

The operator's direction (2026-11-03): "i notice the summaries in the agent pane my agents are much better than the ones in swarm. the haiku method may not be ideal. did we do research there, i asked earlier. I think directing the actual model to set it self." Approved for building as #20816.

**Recommendation: the agent's own model sets its phrase. Haiku leaves the live line.**

1. **How it sets it: one new MuxAPI action, `Presence` `status`** (`line`, at most 80 characters; optional `state`: `working`, `blocked`, `waiting`, `done`). Not `Presence claim`: a claim is scoped to a path or repo, lapses on a 2-hour TTL and exists to warn about overlapping edits; a status is per agent and has no scope. The server stores the line on the agent's pane block as `term:live_phrase`, with `term:live_phrase_at` and `term:live_phrase_state`. The call is a tool call inside the agent's own turn.
2. **When it sets it:** one Operator Config entry, delivered to every agent at launch: set it at the start of a task, when the plan changes, when blocked or waiting for an answer (give the question), and when done (give the result). Not per step. Verb first, name the artifact, 5 to 10 words (wording rule in 6.4). `Tasks claim` and `Tasks complete` answers end with a one-line reminder, so an agent that forgot is told at the moment it matters.
3. **What the row shows**, first match wins: (a) the agent's own line, if it is newer than its last task claim or release; shown muted with an age once it is more than 30 minutes behind the agent's last activity; (b) the title of the Kanban task it holds (with several, the one whose number is in its branch name, else the latest touched); (c) a deterministic line from branch, current tool and last edited file; (d) the existing ladder underneath (generated title, last prompt, status word), as today. `work.goal` reads the same order instead of "title, else last prompt". Waiting-on-user stays on top.
4. **Haiku:** dropped from the live line. No new model rung is built. The existing titler stays only because the agent pane's header uses its title as the session name; its title becomes a muted, last-resort fallback in (d). If the measurement below shows many agents never set a line, a model rung can be added later behind (c); nothing in this design depends on it.
5. **Cost:** one tool call per task change in the agent's own turn (about 60 tokens out, a short answer in), plus about 150 tokens of instruction in each agent's startup context. No extra process, no extra model, nothing for an idle agent. **Failure case:** an agent that never sets a line falls to (b) and (c), which are specific for any agent with a claimed task or a branch and the least informative for a leader doing conversation. A stale line cannot lie for long: a new task claim retires it at once, and age mutes it.
6. **Why "My agents" reads better today.** Its tile preview is not a summary. It is the newest real human-typed prompt from the agent's transcript, verbatim, collapsed to one line of 240 characters, read when the list is built (`session.rs:456` takes `h.last_user_message`; `history_summary.rs:259` skips tool results, sub-agent turns, jekts, broadcasts and startup text). So it is accurate (it is exactly what was asked), current to the last human message, and in the operator's own words, which they recognise at a glance. Its weakness is the opposite of the Swarm's: it says what the operator last asked, not what the agent is doing now, and for an agent driven by jekts it can be hours old. The Swarm line is a model's paraphrase of an old goal. The self-set line keeps the first property (the words of the one who knows) and fixes the second (it is written at each change of work).

**Evidence that today's phrase is broken (pilot, section 2):** in one snapshot of 21 agents on this host, 10 of the 17 agents that had a phrase showed one that was wrong or stale against their branch, edited files and claimed Kanban task; 4 were partly right, 2 plausible, 1 could not be judged, and 4 had no phrase. This is my own rough read, not the measurement.

**The one measurement (after it ships):** one hour with the fleet working, counting for each working agent whether a self-set line exists, its age against the agent's last activity, and whether it names the real work (branch and claimed task as the check). Pass bar: at least three quarters of working agents have a line under 30 minutes old. If not, the reminder wording or a model rung is the next step. Cost figures in section 5.2 are now the cost of the option NOT chosen.

**Sections 5 and 6 below are the first draft (Haiku rung as option D).** Where they differ from this section, this section wins: the ladder order in 6.2 is replaced by item 3 above, and the model rung 6.3 is not built.

---

## 1. How AgentMux makes the phrase today

### 1.1 The pipeline

| Step | What happens | Where |
|---|---|---|
| Trigger | The agent pane's `useAgentActivitySummary` fires when a turn enters `Submitting`, that is, when a message arrives. Hidden reinjection turns and AgentMux's own "# Session Context" startup message are skipped. **A jekt, a broadcast or a Supervisor message counts like a human message**: only the `last_prompt` fallback filters them (`isInjected`). | `frontend/app/view/agent/hooks/useAgentActivitySummary.ts:87,91,103` |
| Schedule | Each counted message increments `term:human_turns`. With **no** title the pane asks every time. With a title it asks only on turns **2, 5, 8, then every third** (11, 14, 17, ...). | `frontend/app/store/title-schedule.ts:45,54` |
| Input | The model sees **only the newest message** (verbatim) and the **current title**. No recent activity, no tool calls, no assistant text, no todo, no task. The activity digest is used only when no message is passed. | `crates/srv/src/server/app_api/session.rs:372` |
| Prompt | "Maintain a short running TITLE ... similar to a git pull-request title - it describes the **OVERALL GOAL** of the session, not the current micro-step or the most recent tool call." Reply `SKIP` if the current title still fits. | `crates/srv/src/ambient/prompt.rs:69-120` |
| Model | `claude-haiku-4-5-20251001` through a one-turn `claude -p` process with no tools and its own system prompt. | `crates/srv/src/ambient/cli.rs:22,64` |
| Store | A new title replaces the old one only if it is **news**: word overlap (Jaccard over non-stop words) below 0.5. A rewording is discarded. | `crates/srv/src/ambient/title.rs:77,91`; stored with `Replace::IfNews` at `session.rs:422` |
| Recovery | A 20 s sweep gives an agent that has **no** title one, from the last 96 KB of its output (14 entries, 6,000 characters). It never replaces an existing title and gives up after 3 attempts. Only Claude-shaped output is readable. | `crates/srv/src/backend/reactive/activity_watcher.rs:58,62`; `ambient/digest.rs:222-224` |
| Display, Swarm | A ladder: generated title, restored title, a heuristic from the last prompt, a status word ("Working"). Waiting-on-user outranks all. | `frontend/app/store/swarm-line.ts:175` |
| Display, `work.goal` | `goal_of`: the generated title, **else the last prompt**. No ladder, no status, no age. | `crates/srv/src/backend/work_facts/mod.rs:107` |

### 1.2 Why the operator's four examples go wrong

| Example (2026-11-02 ~23:59Z) | Mechanism |
|---|---|
| Korp: "Implement card tags and filter chips feature" while writing up a streamed-browser spike | **Stale by design.** The title was set when that was the goal. Every later task arrives as a jekt or a Kanban claim, which the model is told to treat as a continuation (`SKIP`) unless it is a new top-level goal, and the pane only asks on 1 of 3 messages after turn 8. A task switch inside the same repo and area also often overlaps in words with the old title. |
| QTest: "Set up GitHub Vault integration for secrets" while reading upgrade logs | Same. The model never sees what QTest is doing (no activity in the pane call), only the message that just arrived. |
| AgentY: "Complete Store 0.60.12 deployment and spec approvals" hours later | Same, plus: nothing ever *expires* a phrase. There is no age on it and no event (turn end, task done) that retires it. |
| Camper: "Diagnose and fix missing transcript block bugs" while on pane menus | Same; the live Swarm session moved on to other work inside one session. |
| (also seen) "Clone repository and understand project plan" for an agent editing console code | **Too early.** The first message of a session is the only moment the schedule always asks. |
| (also seen) "Merge and coordinate browser tool updates" | **Too general.** The prompt asks for an *overall* goal; the model obliges with a bucket. |

Four structural causes, in order of weight:

1. **Wrong job.** A stable goal-title is the specified output. The Swarm needs "what now".
2. **Wrong input.** One message and the old title; the evidence of what the agent is *doing* (tools, edits, branch, claimed task) is never read in the pane path.
3. **Wrong trigger.** Messages (1 in 3 once past turn 8), not events that mark a change of work: task claim, turn end, a long quiet turn.
4. **No decay.** A stored title lives until it is judged to be a different goal. Nothing shows its age.

A fifth, smaller one: `work.goal` falls back to `term:last_prompt`, which deliberately excludes jekts and nudges, so for a jekt-driven agent the fallback is an old human message.

---

## 2. Pilot: today's phrases against what agents are actually doing

One snapshot, taken with `Agents conversations` and `Tasks list` at 03:27Z on 2026-11-03 (the evening of 11-02 in Pacific time), host narko, 21 agents. "Signals" are the agent's branch, the files it has just edited or has dirty, its claimed Kanban tasks and its current tool. **The verdicts are my proxy judgement from those signals, not ground truth**: no one asked the agents. They are here to size the problem and to rehearse the scoring in section 7.

| Agent | Phrase shown today | Signals (what it is actually on) | Read |
|---|---|---|---|
| Agent2-5 (me) | Research Anthropic Scion Git replacement | Claimed #20815, this report | Stale |
| agent2 | Add reply capability to broadcast schema | Branch `agent2/pane-menus`; pane chrome files; #20805 pane context menus | Wrong |
| Klon | Clone repository and understand project plan | Edits console code; private repo | Stale / too early |
| korp | Implement card tags and filter chips feature | Branch `korp/stale-site-limit-restore-loop`; browser allowlist files | Wrong |
| agent5 | Implement history store compaction and clean streaming deltas | Branch `agent5/identity-live-signin`; #18967 expired sign-in | Wrong |
| qtest | Set up GitHub Vault integration for secrets | Branch `qtest/mcp-phase-2b`; #120 MCP servers phase 2b | Wrong |
| camper | Diagnose and fix missing transcript block bugs | Branch `camper/swarm-sections-live`; #20807 Swarm sections | Wrong |
| agent2-6 | Debug contributors tally display discrepancy | Branch `agent2-6/strong-password-signup`; Vault sign-in files; #6761 | Wrong |
| loap-3 | Coordinate widget PR merge and rebase workflow | Branch `loap-3/audience-tag`; #20792 audience tag | Wrong |
| loap | Test agent duplicate busy state and reuse PRs | Branch `loap/settings-per-key-validation`; #16468 settings.json | Wrong |
| AgentY | Store deployment and phrase refinement | Branch `agenty/release-patch-0.60.12`; spec on long-running animations | Partial |
| Clamk | Verify AWS authentication standard compliance | Edits a console in a private repo | Partial / stale |
| Camper-2 | Merge and coordinate browser tool updates | Branch `camper-2/browser-set-files-iframe`; #20738 | Partial (right area, too general) |
| agent1 | Add motion settings and pulse loop fixes | #20799 Animations Off; #20812 toast Copy button | Partial |
| Applo | Submit enrollment documents to Apple | Current tool is the Browser | Plausible |
| agentx | Implement pane-less mode and shutdown behavior | Dirty spec on closing parks agents | Plausible |
| Naki-2 | Fix email mailbox verification | Idle; claimed #16990 (Google Phase B); no branch | Can't judge |
| JCont, KCont, Pulp, Winny | (none) | Idle | No phrase |

Read from the table:

- **Almost every wrong phrase is a *previous* task of the same agent**, not an invention. The model is not hallucinating; the pipeline is retaining.
- **A free signal would have been right in every wrong row.** The branch name or the claimed task title names the work: `korp/stale-site-limit-restore-loop`, `qtest/mcp-phase-2b`, `camper/swarm-sections-live`, `loap/settings-per-key-validation`.
- **Todos are empty for all 21** (`todos.total` is 0 everywhere). Newer Claude models do not keep a todo list unless opted in (Claude Code docs, "Track todos"), so "derive the line from the current todo item" is not available for most of this fleet.
- **`current_tool` is null for 20 of 21** in a snapshot; it is a momentary signal. It helps a "now" line only when sampled continuously.
- **An agent with several claimed tasks** (korp holds three, agent2 three) cannot name its work from the task list alone; the ladder needs a tie-break (the task whose branch or files match, or the most recently touched).

---

## 3. How others do it

Evidence quality is marked: **Doc** = the vendor's own documentation or changelog, **Code** = open-source code, **Issue** = a maintainer or user thread, **Not published** = the vendor does not document it and I could not find a primary source. I fetched the docs for every product below; where nothing is said, I say so rather than guess.

| Product | What the one-liner is | Signal it summarises | Model | When it refreshes | Wording / length | Evidence |
|---|---|---|---|---|---|---|
| **Claude Code, agent view** (`claude agents`) | **Two fields**: a *session name* and a one-line *summary* | Working: "what the session says it's doing"; blocked: "the question it's asking"; finished: "its result" | Haiku-class; the main model if no Haiku is configured | Working: updated **at most every 15 s from the session's own recent output with no model request**; a **fresh model summary when each turn ends**; **rewritten every few minutes during a long turn** so a busy row "doesn't keep showing an outdated one" | Fills the row; the peek panel shows the full sentence; grouped views lead with the state word (`Needs input · double jump or wall climb?`) | Doc ([Agent view](https://code.claude.com/docs/en/agent-view)) |
| **Claude Code, session name** | A stable handle | The first prompt; a plan, once accepted | Haiku-class background request | Once. `/rename` or a plan replaces it. It does not follow the conversation; an open request asks it to track the recap | A short summary of the first prompt | Doc ([Sessions](https://code.claude.com/docs/en/sessions)); Issue ([#75166](https://github.com/anthropics/claude-code/issues/75166)) |
| **Claude Code, recap** | One line "what happened so far" shown on return | The session so far | Not stated | After 3+ minutes idle with the terminal unfocused; at least 3 turns; never twice in a row; `/recap` on demand | Capped at **400 characters** | Doc ([Interactive mode](https://code.claude.com/docs/en/interactive-mode)) |
| **Claude Agent SDK, subagents** | `task_progress.summary`, one line per subagent | The subagent's activity | Model-generated; off by default (`agentProgressSummaries`) | Periodic while running | One line | Doc ([SDK reference](https://code.claude.com/docs/en/agent-sdk/typescript)) |
| **Claude Code, todo items** | `activeForm`, a present-continuous label per item | The agent's own plan | The working model | When the agent updates the item | Present continuous, one per in-progress item ("Running tests") | Doc ([Track todos](https://code.claude.com/docs/en/agent-sdk/todo-tracking)); not available by default on newer models |
| **Claude Code, status line** | A user script | Whatever it computes, including `session_name` | None | Event-driven, debounced 300 ms; optional timer | Free | Doc ([Status line](https://code.claude.com/docs/en/statusline)) |
| **claude.ai, ChatGPT** | Conversation title | The first exchange | Not published | Once after the first reply; does not change when the topic does (users ask for retitling) | Community reports "1 to 4 words" for ChatGPT | **Not published**; community threads only |
| **OpenAI Codex** (CLI, app) | Thread name | The first prompt (an LLM title is reported after the first prompt) | Not published | Once per thread; a user rename always wins. Requests for retitling "only when the task meaningfully changes, not after every small follow-up" | Previously the whole first prompt (a 131,003-character title was reported) | Issue ([#24289](https://github.com/openai/codex/issues/24289), [#24060](https://github.com/openai/codex/issues/24060)); a third-party test says the title is written about 0.6 s after the first prompt (unverified) |
| **Codex cloud tasks, Cursor cloud agents, GitHub Copilot coding agent, Jules** | A task or agent label, a session log | Not documented how it is made | **Not published** | Jules documents an *activity feed* "as each step completes" with inline explanations and a mini diff per file. Copilot documents session logs showing "the work and tools used". Cursor's chat title is made from the first message and overwrote a hand-set title once (a bug report) | | Doc (Jules, Copilot) for the feed/log; the rest **Not published** |
| **Devin** (CLI, cloud) | Session title | The conversation; `/rename` with no argument "generates a new session title from the conversation so far" | Not published | At creation, and on demand. A fix in May 2026: "**Injected context is no longer included in auto-generated session titles**" | | Doc ([CLI changelog](https://docs.devin.ai/cli/changelog/stable.md)) |
| **Replit Agent** | Checkpoint descriptions; a progress report tool | The agent's own work, at feature granularity | The agent's own model | At logical milestones | A third-party teardown describes a progress tool limited to 5 plain-language items with a mark for done and in-progress; 18 months old | Doc (checkpoints); the tool prompt is a third-party teardown, may be out of date |
| **Windsurf / Cascade** | Not documented | | | | | **Not published** |
| **OpenHands** | Conversation title | The first user message, truncated to 1,000 characters | The user's configured model | Once, at the start; falls back to the first 30 characters if the model call fails | "maximum 50 characters" | Code ([`conversation_summary.py`](https://huggingface.co/spaces/Backup-bdg/OpenHands/blob/main/openhands/utils/conversation_summary.py)) |
| **Open WebUI, LibreChat** | Chat title | The conversation, truncated in the middle if long | A small "task model" is recommended | After the first reply | "3-5 words", language of the chat, no quotes or trailing punctuation, JSON `{ "title": ... }` with examples; LibreChat: "5 words or less ... no punctuation" | Doc ([LibreChat](https://www.librechat.ai/docs/configuration/librechat_yaml/object_structure/shared_endpoint_settings), [Open WebUI](https://github.com/open-webui/open-webui/discussions/1692)) |
| **Anthropic, "Building effective agents"** | Guidance, not a feature | | | | "Prioritize transparency by explicitly showing the agent's planning steps"; pause at checkpoints | Doc ([article](https://www.anthropic.com/engineering/building-effective-agents)) |

I found **no published guidance from Anthropic or OpenAI on writing agent status summaries** beyond the above. The agent-guide sources say show the plan and pause at checkpoints; none say how to word or refresh a one-line status. The best evidence is how the products behave.

### 3.1 What the field converges on

1. **Titles are made once and are about the start.** Every chat product (claude.ai, ChatGPT, Codex, Cursor, OpenHands, Open WebUI) names a conversation from its first message or first exchange. Users of Claude Code, Codex and ChatGPT all ask for retitling because the title goes stale. AgentMux's design has the same property, which fits a name and fails a live line.
2. **The one product built for watching many agents separates the two.** Claude Code's agent view has a name *and* a changing summary, and ties the summary to **state** (working / blocked / finished).
3. **Free text first, a model second, and the model on events.** Claude Code updates the working row from the session's own output every 15 s with no model call, calls the model at turn end, and again on a timer only when a turn runs long.
4. **A small model is the norm; the title call is cheap.** Haiku-class in Claude Code; "a small task model, 8B is probably enough" for Open WebUI. Cost is described as "small" and billed like the session.
5. **A human or agent name always wins** (Codex, Cursor's bug, Claude Code `/rename`, Devin).
6. **Injected text must not become the title** (Devin's May 2026 fix). AgentMux had the same bug with its startup message (already handled in `useAgentActivitySummary.ts:91`) and still counts jekts as messages.
7. **Wording is short and plain.** 3 to 5 words for chat titles; a 400-character cap for a recap; present-continuous for a live item; no quotes or trailing punctuation; reply in the user's language; output only the text.
8. **Fail soft.** A failed title call never blocks the task (Codex discussion, OpenHands fallback to truncation), which AgentMux already does (the ladder never goes blank).

---

## 4. What "accurate, specific, current" mean here, so they can be scored

| Property | Definition | Score 0 | Score 1 | Score 2 |
|---|---|---|---|---|
| **Accurate** | It describes what the agent is doing at this moment | About other work | Right area, wrong job | Correct |
| **Specific** | A reader could tell this agent from another on the same repo | A bucket ("Coordinate browser updates") | Names the area | Names the artifact (PR, file, feature, task) |
| **Current** | It reflects the last ~15 minutes | Older work that has ended | Plausible but unverifiable, or at most an hour old | Matches the latest activity |

An accurate phrase can still be useless (too general) and a specific one can be dead (stale). Each is scored separately.

---

## 5. Options for AgentMux

### 5.1 The options

| Option | Source of the line | Benefit | Cost and weakness |
|---|---|---|---|
| **A. No model** | Claimed task title; else branch + current tool + last edited file; else status word | $0. Latency 0. Always current for the signals it has. Specific whenever a task is claimed or a branch is named. Cannot be "wrong", only absent | Poor for conversational or exploratory work with no task and no branch (the leaders, Applo browsing). Needs a tie-break when several tasks are claimed. Wording is a title, not a sentence |
| **B. Cheap model on triggers** | Haiku reads the last turn's activity (assistant text + tool calls) at turn end, on task claim/release and every ~5 min in a long turn | Reads what the agent did, so it can say "Writing up the streamed-browser spike"; works with no task | Tokens per working agent per hour; latency of a `claude -p` spawn; only Claude-shaped output is readable today (Codex and others have no digest); a summary can be confidently wrong |
| **C. Agent declares itself** | The agent sets the line with a MuxAPI call (`Presence claim` already has a `note`; `Tasks update` has `note`) | Most accurate when done: the agent knows what it is doing. No model call | Costs the agent a tool call and context; agents forget; drifts as soon as work changes; trust: a stale declaration reads as fact. Precedents: Replit's progress tool, Claude Code's `activeForm` |
| **D. Hybrid ladder (recommended)** | C if fresh, else A's task title, else B, else A's deterministic line | Free sources carry most of the load; the model fills the gaps; the agent can correct it | Most moving parts; needs the age/decay rules and a tie-break. Sections 5.3 and 6 |

### 5.2 Cost estimate (arithmetic, not a measurement)

Assumptions, all to be replaced by the measurement in section 7: Haiku list price of $1 per million input tokens and $5 per million output tokens (confirm in the `claude-api` reference before relying on it); the existing digest caps (14 entries, 6,000 characters, about 1,500 tokens) plus a prompt of about 300 tokens; an output of about 15 tokens.

| | Input tokens | Output tokens | Cost per call (list) |
|---|---|---|---|
| Today (pane path): one message + prompt | about 400 to 2,000 (a jekt body can be long) | about 15 | about $0.0005 to $0.0021 |
| Option B: activity digest + prompt | about 1,800 | about 15 | about $0.0019 |

Per hour, for a working agent:

| | Calls per hour | Cost per agent-hour (list) |
|---|---|---|
| Today: past turn 8, 1 in 3 messages is evaluated, and many of those return `SKIP` | about 1 to 5 | about $0.001 to $0.01 |
| B alone: every turn end (a busy agent ends 5 to 20 turns an hour) | 5 to 20 | about $0.01 to $0.04 |
| D: the model runs only when the cheap signals changed (task claim, branch change, new file area), capped at one call per agent per 2 minutes | about 3 to 10 | about $0.006 to $0.02 |

For a fleet of 25 working agents, D is about 15 to 50 cents an hour at list price, and today is already a fraction of that. These figures are list-price arithmetic and say nothing about the operator's subscription limits. The `claude -p` process may add tokens (its own system text, cache creation) that the arithmetic leaves out; `ambient:spent` events (`ambient/spend.rs`) record the real count per call and are the first thing to read.

**Latency:** each call spawns a CLI process. I did not measure it; Codex's title call is reported at about 0.6 s (third-party, unverified). With a turn-end trigger, a few seconds of lag is invisible because the line is read between turns. The deterministic rung is instant and covers the gap.

### 5.3 The decay rule (how to show staleness)

The cheap, honest answer is to carry two timestamps and compare them:

- `phrase_at`: when the line was computed;
- `activity_at`: the agent's last activity; `claim_at`: its latest task claim or release.

Rules:

1. A line with `phrase_at < claim_at` is **retired** at once and the ladder is re-run (a task change always outranks an old line). This single rule would have fixed Korp, QTest and AgentY in the operator's examples.
2. A line with `activity_at - phrase_at` over 15 minutes while the agent is working is **muted** and shows its age ("1h ago"). The existing `source` tag and muted style from `swarm-line.ts` carry it.
3. An idle agent keeps its last line, labelled with when it stopped ("Idle 2h · Fixing the stale-site restore loop"), because "what it was last doing" is useful, but it must not look current.
4. Waiting-on-user stays on top, and its line is the question itself (as Claude Code does), not the word "Waiting".

---

## 6. Recommended design, in the detail Camper needs

### 6.1 Fields

| Field | Meaning | Written by | Read by |
|---|---|---|---|
| `term:ambient_summary` (exists) | The session **name**: a stable goal title | unchanged (the pane, on the existing schedule) | pane header, session lists |
| `term:live_phrase` (new) | The **live line** | the ladder, one writer | Swarm row, `Agents conversations` `work.goal` |
| `term:live_phrase_src` | `declared`, `task`, `model`, `derived` | the ladder | Swarm tooltip/muting |
| `term:live_phrase_at` | Unix ms | the ladder | decay rule |

`work.goal` should return the live line, falling back to the name, **not** to `last_prompt`.

### 6.2 The ladder

1. **Declared:** the agent's `note` on `Presence claim` (or a Kanban `update` note) from the last 15 minutes, if present. Source `declared`.
2. **Task:** the title of the Kanban task the agent claimed. With several claimed, pick the one whose number appears in its branch name or recent messages, else the one touched most recently. Source `task`.
3. **Model:** a Haiku summary of the last turn's activity, triggered by: turn end; task claim or release; a long-turn timer (5 minutes) when the file set or tool set changed. Input: the final assistant text, the tool names and the paths touched. **Not** the user's message. Never include injected text as a goal. Source `model`.
4. **Derived:** branch name humanised + current tool + the last edited file (`korp/stale-site-limit-restore-loop` becomes "stale site limit restore loop"). Source `derived`.

The existing `SwarmLine` ladder (`generated`, `restored`, `heuristic`, `status`) is kept underneath as the last resort.

### 6.3 The prompt for the model rung (proposal)

```
You write the one-line STATUS of an AI agent's work for a dashboard, in 5 to 10 words.
Say what it is doing right now, from the activity below. Start with a verb. Name the
specific thing: the feature, file, PR or task. No generic verbs on their own (implement,
coordinate, work on). If the agent is waiting for an answer, give the question. If its
last turn finished something, say what. Reply SKIP if the activity does not show what the
work is. {PLAIN_TEXT_RULES}

<recent_activity>
...
</recent_activity>
```

Reuse the existing `SKIP`/validation/`ANSWER:` machinery (`ambient::reply`, `validate`). The "OVERALL GOAL" and "micro-step" wording is **removed** for this call.

### 6.4 Wording rule (for the prompt, the validator and a reviewer)

- Verb first: **Fixing / Writing / Reviewing / Waiting / Done:**; present participle for work in progress, a short past form for a result (`Merged PR #6190`).
- Then the object, then the one fact that distinguishes it. 5 to 10 words, at most 70 characters.
- Name the artifact when there is one: a `#` for a task or PR, a feature name, a file stem.
- Banned as the only content: Coordinate, Implement, Work on, Handle, Various, Updates, Misc.
- Never first person, never a quote of the user's message, never an injected marker (`[JEKT`, `[BROADCAST`, `# Session Context`).
- A state word may lead when the state is what the reader needs: `Waiting:`, `Blocked:`, `Done:`.

### 6.5 Rules that carry over from what exists and from the field

- A **user or agent rename always wins** and stops regeneration (matches P7 in the hardening spec; Codex, Cursor, Devin, Claude Code agree).
- **Injected text never becomes the line.** Fix the turn counter to ignore jekts and broadcasts as "human turns" for the *name* schedule, and never pass a jekt body to the live line as a goal.
- **Failure never blanks the row** (existing P1).
- **Non-Claude agents** (Codex and others) get the `declared`, `task` and `derived` rungs; the model rung needs a digest reader for their output format. This is a known gap in `digest::reads_output_format`.
- **Cost gates:** at most one model call per agent per 2 minutes; none while the agent is idle, unavailable or limited; none when the signals are unchanged since the last call; quota mode (`tight`) turns the model rung off and leaves the free rungs.

### 6.6 What this does not change

The name (`term:ambient_summary`), its schedule and its replacement rule can stay as they are. It is a different job and works as a handle. The fix is not to make that title fresher; it is to stop showing it as the live line.

---

## 7. Measurement plan

**Question:** does the recommended live line beat today's phrase on accuracy, specificity and currency, at what cost?

**Sample:** 20 agent-moments. Pick 20 (agent, time) pairs spread over at least 10 agents and 3 hours, deliberately including: a fresh claim, a mid-task moment, a task switch within the last 10 minutes, a long quiet turn, an idle agent, a waiting-on-user agent, a leader doing conversation, and a non-Claude agent if one runs.

**Capture at each moment** (one script, read-only):

1. today's phrase (`work.goal`) and the Swarm row text, with the block's `term:*` meta;
2. the candidate phrases: option A (ladder rungs 2 and 4 only, computed offline), option B (run the proposed prompt on the activity digest), option D (the ladder);
3. ground truth: the last 10 transcript lines (`Agents transcript`), the git state, the claimed tasks.

**Score:** two raters (the operator or a leader, and one agent) score each phrase 0 to 2 on accurate, specific and current (section 4), blind to which candidate wrote it, with the transcript tail in view. Report agreement; resolve the gaps in discussion.

**Automatic checks, for scale:**
- overlap between the phrase and the words of the branch, the claimed task title and the edited file stems (a cheap proxy for "named the real work");
- the share of phrases that contain a banned generic verb alone;
- the share of phrases older than 15 minutes while the agent was working.

**Cost and latency, measured not estimated:** from `ambient:spent` events, tokens per call, per purpose, per agent-hour for today's design and for D run in shadow mode for one working session; and the wall time from turn end to a stored line.

**Pass bar, proposed:** D scores at least 1.5 on every property, beats today's phrase on accurate and current in at least 15 of 20 moments, and its model rung adds no more than a small multiple of today's per-agent-hour tokens. If the free rungs alone (option A) already pass for most moments, ship A first and add the model rung after.

**Shadow mode:** write the candidate to a new meta key and log both; do not change the Swarm until the numbers are in.

---

## 8. Risks, open questions, and what I could not establish

- **The proxy judgments in section 2 are mine.** The agents themselves would be better judges; section 7 fixes that.
- **Cost figures are list-price arithmetic.** I did not read `ambient:spent`; I did not time a call.
- **Three claims rest on weaker sources:** Codex's LLM title timing is one third-party test; ChatGPT's "1 to 4 words" is a forum post quoting a prompt; Replit's progress-tool limits are from an 18-month-old teardown.
- **Not found:** how Cursor, Copilot coding agent, Jules, Codex cloud, Windsurf, claude.ai and ChatGPT produce and refresh their labels. I fetched their docs; they do not say. I did not rely on leaked prompts or reverse-engineered code.
- **Claim ambiguity:** some agents hold three claimed tasks. The tie-break in 6.2 is a heuristic and could name the wrong task.
- **Declared lines can lie.** An agent that sets a note and moves on leaves a confident, wrong line; the 15-minute window and the task-claim rule bound the damage.
- **Privacy:** the model rung reads transcript text. The existing digest already strips hidden reinjection turns; keep that and do not send injected memory or credentials-bearing tool results.
- **Feature freeze:** AgentY has frozen new feature builds for the architecture cleanup. This is a build that must wait for the freeze to lift, or be argued as a bug fix (the Swarm showing wrong information); that is AgentY's call.
- **Tests that must exist:** the retire-on-claim rule; the decay display; the tie-break; the banned-verb validator; jekts never counted for the name schedule; non-Claude agents still get a line.

---

## Sources

AgentMux code (at `cea4fdd`): `frontend/app/view/agent/hooks/useAgentActivitySummary.ts`; `frontend/app/store/title-schedule.ts`; `frontend/app/store/swarm-line.ts`; `crates/srv/src/server/app_api/session.rs`; `crates/srv/src/ambient/{prompt,title,tasks,digest,cli,spend,validate}.rs`; `crates/srv/src/backend/reactive/activity_watcher.rs`; `crates/srv/src/backend/work_facts/mod.rs`. Earlier work: `docs/specs/SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md`, `SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md`.

External (fetched 2026-11-02 to 11-03):
- Claude Code: [Agent view](https://code.claude.com/docs/en/agent-view), [Manage sessions](https://code.claude.com/docs/en/sessions), [Interactive mode (session recap, prompt suggestions)](https://code.claude.com/docs/en/interactive-mode), [Status line](https://code.claude.com/docs/en/statusline), [Track todos](https://code.claude.com/docs/en/agent-sdk/todo-tracking), [Agent SDK reference (TypeScript)](https://code.claude.com/docs/en/agent-sdk/typescript), [issue #75166: auto-update session title from the recap](https://github.com/anthropics/claude-code/issues/75166).
- Anthropic: [Building effective agents](https://www.anthropic.com/engineering/building-effective-agents).
- OpenAI Codex: [#24289 auto-generate a thread name](https://github.com/openai/codex/issues/24289), [#24060 auto-updating thread titles](https://github.com/openai/codex/issues/24060), [#16601 resume picker names](https://github.com/openai/codex/pull/16601).
- Devin: [CLI changelog](https://docs.devin.ai/cli/changelog/stable.md).
- Jules: [Running tasks](https://jules.google/docs/running-tasks). GitHub Copilot: [About the coding agent](https://docs.github.com/en/copilot/concepts/agents/coding-agent/about-coding-agent).
- OpenHands: [`conversation_summary.py`](https://huggingface.co/spaces/Backup-bdg/OpenHands/blob/main/openhands/utils/conversation_summary.py). LibreChat: [title settings](https://www.librechat.ai/docs/configuration/librechat_yaml/object_structure/shared_endpoint_settings). Open WebUI: [better title prompt](https://github.com/open-webui/open-webui/discussions/1692).
- Community and third-party (weak): OpenAI developer forum threads on ChatGPT titles; Cursor forum threads on chat titles; Replit Agent teardown on Medium.

Agent: Agent2-5@narko
