# Spec: a Kanban section in Warden, where agents keep their work current and the operator follows it

**Status:** active. srv and MCP (the task model, the agents' `Tasks` actions, the reminder, the operator's RPCs) in #5231; the Warden Kanban section, its palette command and widget-bar button in #5235. The Swarm's task chip (§6.14) and Codex plans as checklists (§6.12) in #5248. Phase 4 (§9): lanes by repository (§6.11) and Review's soft limit (D6) in #5260; the rest remains.
**Date:** 2026-10-20
**Author:** Loap-3@narko, at the operator's request
**Verified against:** `agentmux` `main` @ `30daa7c28` (2026-10-20), by reading code. The survey of other products in §3 is from public sources read the same day (§12).
**Builds on:** the shared task queue (`Tasks` tool, "Muxqueue": `docs/reports/REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md`), `SPEC_WARDEN_WIDGET_2026-05-25.md`, `SPEC_WARDEN_HISTORY_SECTION_2026_10_18.md`, `SPEC_AGENT_COORDINATION_CHANNEL_2026_10_15.md`, `SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md`, `SPEC_AGENT_WORKERS_2026_10_15.md`.

## 1. Ask

The operator, 2026-10-20: "I have a new idea for a new type of pane that is a kanban, it could be central to the app. Essentially, as agents work through their work, they update the kanban, so operators can keep track of whats happens. research online best practices of latest agent kanbans, consider a couple different words, pull in latest from github main agentmux and make sure you understand the code, then design a best practice implementation, write report to file."

Then: "i think this can go inside of Warden, under a Kanban section."

Then, on how the backlog fills: "actually, agents will create tasks, and add them to the backlog. I have a habit of grabbing the nearest agent, even if they are on another train of work, to write a spec or do something, so that's a good place for the backlog, in case the agent needs to finish whatever he is doing first."

## 2. Summary

A **Kanban** section in Warden, beside Host, LAN, Internet, Audit and History. One card is one **task**. Agents create, claim, update, ask about and hand in tasks through the `Tasks` tool they already have. srv moves cards on facts it already knows: an agent exits, asks a question, fails, opens a PR. The operator starts tasks on an agent, answers, accepts or sends back.

Columns: **Backlog, In progress, Needs you, Review, Done.**

**The backlog is filled by agents, mostly on the operator's behalf.** The operator asks whichever agent is nearest for something ("write a spec for X"), often while that agent is on another train of work. The agent doesn't drop what it's doing: it files the request as a task in **its own backlog**, with the operator's words, answers "filed as #12, I'll take it when I've finished this", and carries on. When its current work is done and it goes idle, srv reminds it of its backlog and it takes the next task. The operator sees every such request on the board, can move one to another agent that is free, and loses none of them (§6.5).

The design rests on six choices, each taken from what the products surveyed in §3 learned the hard way:

1. **One source of truth, already built.** Cards are the rows of the existing task queue (`db_work_queue`), which already has atomic claims, leases, attempts, fence tokens, and releases an agent's claims when its pane closes. Nothing is invented beside it; it gains a few states, a side table and a history.
2. **srv owns the facts, agents own the intent.** Whether an agent is working, waiting on a question, errored or gone comes from srv (`agent_state.rs`, the turn ledger), never from the agent remembering to say so. Agents only say what srv can't know: what they're doing, that they need something, that they're done and where the result is. Stale cards are the main failure of every board that relies on agents reporting (§3.2).
3. **"Needs you" is its own column**, separate from "Review". It holds every card a person has to act on: a question, a failed task, a holder that crashed. It is the inbox, and its count is the one number on the Warden tab.
4. **Work nobody filed still shows.** An agent that is working without a card appears in In progress as an *untracked* card built from what srv already knows of it (its summary, current tool, checklist). One click tracks it. The board is useful on the first day, before any agent has learned to use it, and stays honest when one forgets.
5. **Only a person finishes a reviewed task.** An agent can hand in (Review); it can't accept its own work. Accept, Send back and Cancel come only from the UI.
6. **The words match the protocols.** "Task" is the unit in Claude Code, MCP, A2A and AgentMux's own `Tasks` tool, and the states map one-to-one onto A2A's (§6.2), so a later bridge costs nothing.

Four phases (§9); the first is a read-only board over today's queue and agents, with no schema change.

## 3. What others do

### 3.1 The products closest to this

| Product | Unit | Columns or states | Who moves cards | How the agent writes |
|---|---|---|---|---|
| Vibe Kanban (BloopAI; community-maintained since bloop closed in April 2026) | task, with attempts in their own worktrees | todo, in progress, in review, done, cancelled | moves itself on start, finish, merge; a person can drag | it is an MCP server |
| Cline Kanban (March 2026) | task | Backlog, In Progress, Review, Trash | a card moves to Review when it finishes **or when it waits for the user's approval** | built in; dependencies by dragging one card onto another |
| Hermes Agent Kanban (Nous Research, May 2026) | task, in SQLite | triage, todo, ready, running, blocked, review, done, archived | a dispatcher every 60 s reclaims stale claims, respawns crashed workers | tools: show, list, complete, request_review, block, comment, heartbeat, create, link |
| Claude Code Tasks and agent teams | task | pending, in_progress, completed (and deleted) | teammates claim under a file lock; hooks `TaskCreated`, `TaskCompleted`, `TeammateIdle` can veto | `TaskCreate`/`TaskUpdate` (TodoWrite is now off by default) |
| Linear for Agents | issue with an agent session | session: pending, active, error, awaitingInput, complete, stale; plan steps: pending, inProgress, completed, canceled | Linear **derives** session state from the agent's last activity | activities: thought, elicitation, action, response, error |
| GitHub Copilot coding agent, Agent HQ | issue assigned to an agent | agent activity: queued, working, waiting for review, completed (separate from the project's own Status column) | GitHub, from the session and the draft PR; the agent never merges | draft `[WIP]` PR body |
| Superset, Conductor, Nimbalyst, Devin | workspace or session | e.g. Needs attention, Working, Needs review, Idle, Merged (Superset); Working, Ready, Blocked, Inactive (Devin) | the app | the app |
| Backlog.md, Taskmaster, kanban-md, agent-kanban and other MCP boards | a file or row per task | variants of to do, in progress, review, done, plus blocked or deferred | the agent through a CLI or MCP, the person through the CLI or a web view | e.g. `claim_task`, `post_progress`, `request_review`, `set_task_pr`, `complete_task` |

### 3.2 What they learned

1. **Review is the bottleneck, not generation.** "agents parallelize, humans review sequentially — 8 parallel cards means 8x the diffs" (HN); a critic of one board: "every task just ended up in the 'needs review' column". Show review load (how many, how long waiting), and allow a soft limit on the Review column.
2. **"Needs you" is not "please review".** Superset splits Needs attention from Needs review; Devin split Ready from Blocked. Linear's `awaitingInput`, A2A's `input-required` and MCP's `input_required` are the same idea.
3. **Claim before working, with a lease**, atomically (Claude Code's file lock, kanban-md's `pick --claim`, agent-kanban's `claimed_by` check).
4. **Stale cards are the main failure.** Claude Code's own docs: "Task status can lag: teammates sometimes fail to mark tasks as completed". Linear has an explicit `stale` state and a 10-second acknowledgement rule. Hermes's issue tracker is a record of what goes wrong with heartbeats: a heartbeat that didn't extend the lease, a single long tool call that outlived it, ordinary activity not counting unless the model called the tool, and a new claim inheriting the previous run's heartbeat so live workers were killed and duplicated. **Count real activity as the heartbeat, and reclaim only when the holder is actually gone.**
5. **Derive what you can.** Linear, GitHub, Cline and Vibe Kanban move cards on lifecycle events, not on the agent's word.
6. **Keep the work item separate from the agent run.** One task, zero or more attempts (Linear's assignee vs delegate, GitHub's sessions under the issue). This allows retries and handoffs.
7. **A plan is a checklist on the card, not more cards** (Devin, Antigravity, Linear Agent Plans, ACP `plan`).
8. **Link the card to its branch and PR**, and keep an audit trail (Hermes's comment thread doubles as the handoff record; Jira logs every agent action).
9. **Interrupt only for three things:** finished, needs you, failed. Everything else waits on the board.
10. **Only a person approves.** Copilot never merges its own work; kanbo: "only a person approves or sends a card back".

One fact changes how much can be scraped: on current Claude models the CLI no longer writes a todo list by default (`TodoWrite` is disabled; the `Task*` tools are on only for older models). A board that depends on reading todo lists from transcripts would be empty for most Claude sessions. Checklists are a bonus where they exist (§6.7), never the thing a column depends on.

## 4. Words

| Word | Who uses it | What it suggests |
|---|---|---|
| **Kanban** | Vibe Kanban, Cline Kanban, Hermes Kanban, kanban-md | Columns and flow; instantly recognised. Purists say it implies WIP limits and flow measures ("This is a task board and not a Kanban board"). |
| **Board** | Jira, GitHub Projects, Linear, Superset | Neutral; names the view, not the data. |
| **Tasks** | Claude Code, Codex, A2A, MCP, AgentMux's `Tasks` tool | The unit, in every protocol. |
| **Sessions** | Linear, Devin, GitHub agent activity | One agent run, not a piece of work. |
| **Mission control** | GitHub Agent HQ, Google Antigravity | An oversight dashboard across agents; grand, and about watching rather than planning. AgentMux's Swarm already plays this part. |
| **Inbox** | Antigravity, LangChain Agent Inbox | Only what needs a person. |
| **Queue / Backlog** | AgentMux's Muxqueue, Backlog.md | Work not started; "queue" implies pulling. |
| **Plan / Checklist** | Devin, Linear, ACP | The steps inside one task. |

**Recommendation:**
- The section is **Kanban**: the operator's decision (2026-10-20, "we'll call it Kanban"), and the name used everywhere it shows (the Warden rail, `open:kanban`, the widget-bar button, the tab title). The Review column's soft limit (§6.10) and the time a card spends in each column make it a kanban in the purists' sense too.
- A card is a **task**: the same word as the `Tasks` tool and the protocols, so an agent told "update your task" knows which tool to reach for.
- The person's column is **Needs you**, the phrase AgentMux already uses for the top of its agent-status ranking (`frontend/app/store/agent-status.ts`).
- The steps on a card are its **checklist**.
- "Mission control" stays unused: Swarm is the who, Kanban is the what (§6.14).

## 5. What AgentMux already has

### 5.1 The task queue

- **Agents' tool:** `Tasks` with `enqueue, claim, heartbeat, complete, release, list` (`crates/mcp/src/tools/tasks_tool.rs`, schema `crates/mcp/src/tool_schemas.rs` `TASKS_TOOL`), over HTTP to srv (`crates/mcp/src/tools/work.rs`; routes `/agentmux/work…` in `crates/srv/src/server/routes.rs`, handlers `crates/srv/src/server/work_queue.rs`).
- **Store:** `db_work_queue` in the identity store `~/.agentmux/shared/identity-store.db` (`crates/srv/src/backend/storage/migrations.rs`, `run_identity_store_schema`; logic in `crates/srv/src/backend/storage/work_queue.rs`). Fields: `id, title, payload, kind, target_agent, target_group, priority, state, claimed_by, claim_expires, attempts, max_attempts, created_by, created_at, updated_at, not_before, result`, and UIDs beside the names.
- **States:** `open, claimed, done, failed, cancelled`. Claims are one `UPDATE … RETURNING`, by priority then age; the `attempt` number is a fence token, so a superseded holder's heartbeat or completion is refused. Default lease 2 minutes. The reaper runs before every claim and reopens an expired item, or parks it as `failed` after `max_attempts` (3).
- **Teardown:** closing an agent releases its claims (`crates/srv/src/sagas/agent_teardown.rs`, `release_claims`).
- **Scope:** one queue for the whole computer, every channel and instance (deliberately; `migrations.rs` doc comment). No repository column.
- **Event:** `WorkQueueChanged` (`workqueue:changed`, no payload; `crates/srv/src/backend/mps_events.rs`, `frontend/app/store/mps-events.ts`) on every change.
- **UI:** none. No frontend code reads the queue or subscribes to its event, and there is no RPC stub for it.

### 5.2 What srv knows about each agent

- **State:** `AgentState {Working, Waiting, Idle, Stopped, Error}` (`crates/srv/src/backend/agent_state.rs`): a turn in progress is Working, or Waiting when the pane set `term:awaiting_user`; otherwise the process decides, a non-zero exit being Error. Published to the UI through `ControllerStatus` and `AgentTurn` (`mps_events.rs`), failures through `AgentFailed`.
- **Progress:** `crates/srv/src/backend/reactive/progress_watcher.rs` reads each agent's new transcript lines every 3 s and publishes `agent:progress` = `{todos, todos_truncated, todos_partial, current_tool}`. Todos come from `TodoWrite`, `TaskCreate`, `TaskUpdate` (`TODO_TOOL_NAMES`); `current_tool` is the oldest tool call without a result. Codex's plan updates are not parsed (`frontend/app/view/agent/providers/codex-translator.ts` only shows them as thinking).
- **PRs:** the same watcher spots `gh pr create` / `gh-agent pr create` in an agent's shell calls (`pr_create`, `NewPr`), which feeds the overlap notes (`crates/srv/src/backend/work_facts/overlap_notes.rs`). Open PRs are credited to an agent by their `<Agent>@<host>: ` title prefix (`crates/mcp/src/tools/open_prs.rs`).
- **Repository and branch** of each agent's working folder (`crates/srv/src/backend/work_facts/git.rs`); `WorkFacts::repo_key()` gives `owner/repo`, or `local:<clone root>`.
- **Who is calling:** each agent's MCP sends `X-Agent-Token`; srv resolves it to `Caller::Agent{uid}` (`crates/srv/src/server/caller.rs`). Calls from the UI come over the window's websocket, not through the MCP.

### 5.3 Neighbours

- **Presence** claims (`db_work_claims`) and the **coordination channel** (`db_coord_posts`, per repository, 7 days) live in the same store and publish no events.
- **Swarm** (`frontend/app/view/swarm/`, about 10.7k lines) shows agents and their subagents, shells, crons and background tasks, with the todos from `agent:progress` and the status chip from `agent-status.ts`.
- **Workers** (`Agents` `spawn`, `SPEC_AGENT_WORKERS_2026_10_15.md`): an agent can start another agent in a pane with a brief.

### 5.4 Warden, where the section goes

- Manifest `frontend/app/view/warden/warden.tsx`; sections in `warden-model.ts` (`WardenSection = "host" | "lan" | "internet" | "audit" | "history"`, `WARDEN_SECTION_LABELS`, meta key `warden:section`) and `warden-view.tsx` (`RAIL`, a `TabbedPane`). `openWarden(section)` (`warden/open-warden.ts`) opens or focuses the tab's Warden on a section.
- **Every section stays mounted** and owns its own polling (`warden-view.tsx`); History loads only when shown (`visible` prop). Kanban must follow History, not Host: fetch and subscribe only while visible.
- Shared chrome: `frontend/app/view/warden-shared/` (`warden-manager-chrome.scss`, `formatAge`).
- In `widgets.json`, Warden is not pinned (`defwidget@warden`, `display:pinned: false`).

### 5.5 Frontend plumbing to use

- SolidJS signals and memos (no React, no jotai); the object store `frontend/app/store/mos.ts` for block meta.
- Events: `onMuxEvent(WpsEvent.X, handler)` (`frontend/app/store/mps.ts`), unsubscribed in `dispose`.
- RPC: generated stubs only (`frontend/app/store/rpc-api/generated.ts`): register the srv handler with `engine.register_typed`, run the `export_bindings` test, add the manifest entry, run `scripts/gen-rpc-stubs.mjs` (§8.2).
- Drag and drop: `@atlaskit/pragmatic-drag-and-drop` (already a dependency), with drag tags in `frontend/app/drag/drag-types.ts`. Tile drags start only from pane headers, so drags inside the section are free.
- Commands: `frontend/app/store/command-registry.ts` (`open:swarm` is the pattern).

## 6. Design

### 6.1 Where it lives and how it's opened

- **A sixth Warden section**, `kanban`, labelled "Kanban", icon `table-columns`, placed first in the rail. Kanban is the section an operator will open most; Host, LAN, Internet, Audit and History stay as they are.
- **`open:kanban`** in the command palette: `openWarden("kanban")`.
- **A pinned widget-bar entry** `defwidget@kanban` whose `blockdef.meta` is `{ view: "warden", "warden:section": "kanban" }`. This is what makes it "central": one click from the widget bar, while Warden itself stays unpinned.
- **The Warden tab's title** while on Kanban shows the Needs-you count: "Kanban · 2 need you" (`liveTitle`, which already follows the section).
- **Agents** can bring it up with `UI` `command` `open:kanban`.

### 6.2 States

The queue's states, plus three. Each maps onto A2A's task states, so a bridge later is a rename.

| State | Meaning | Claimable | A2A |
|---|---|---|---|
| `open` | Waiting to be started: by the agent it is for (`target_agent`), or by any eligible agent when it is for no one | yes | submitted |
| `held` *(new)* | Waiting, and not to be started until the operator says so | no | submitted |
| `claimed` | An agent holds it under a lease | no | working |
| `blocked` *(new)* | Its holder asked a question; the lease is paused, the holder keeps it | no | input-required |
| `review` *(new)* | Handed in; waiting for a person | no | completed, not yet accepted |
| `done` | Finished | no | completed |
| `failed` | Gave up, or out of attempts | no | failed |
| `cancelled` | Withdrawn | no | canceled |

Older builds on the same computer share this store (§10). Their claim query takes only `open`, their reaper expires only `claimed`, so the new states are invisible to their claims and safe; their `list` shows the new names as they are.

The claim query changes order: an agent's own tasks (`target_agent` is it) before Anyone's, then priority, then age. Today it orders by priority and age only.

### 6.3 Columns

A column is a function of the stored state and srv's live view of the holder, computed in one place (`columnOf(task, holder)`, a pure function with tests):

| Column | Holds |
|---|---|
| **Backlog** | `open` and `held`, grouped by who each is for: one group per agent ("Loap-3 · 2 waiting, busy on #9"), then "Anyone". Within a group, in the order the agent will take them. A held card is greyed with "on hold". |
| **In progress** | `claimed` whose holder is working or idle; untracked agent work (§6.6) |
| **Needs you** | `blocked`; `claimed` whose holder is waiting on a question, errored or gone; `failed` |
| **Review** | `review` |
| **Done** | `done` and `cancelled` from the last 7 days, newest first (older: a "Show older" link) |

Each card in Needs you says why, in its first line: "asks: …", "question waiting", "agent errored", "agent closed", "failed after 3 attempts".

### 6.4 Who moves what

| Move | Who | How |
|---|---|---|
| new → Backlog, for itself | agent | `Tasks` `enqueue` with `for: "me"`: the operator asked it for something it will do after its current work (§6.5). **The main way the backlog fills.** |
| new → Backlog, for another agent or anyone | agent | `Tasks` `enqueue` (as today) |
| new → Backlog | operator | New task (title, prompt, optional agent, repo, kind, priority, "On hold") |
| within Backlog: reorder, move to another agent or to Anyone, hold, unhold | operator | drag, or the card's menu |
| Backlog → In progress | agent | `claim` (as today): its own tasks first, then Anyone's |
| Backlog → In progress | operator | **Start on…**: pick an agent (or "new worker"); §6.8 |
| In progress → Needs you | agent | `ask` (new): a question; the task becomes `blocked` |
| In progress → Needs you | srv | the holder waits on a question, errors or closes (derived, not stored, except a close, which releases as today) |
| Needs you → In progress | operator | **Answer**: the text goes to the holder as the operator's message; the task returns to `claimed` |
| In progress → Review | agent | `submit` (new): result and links |
| In progress → Done | agent | `complete` (as today), only for a task that needs no review; otherwise srv puts it in Review and says so in the answer. Like `submit`, its answer names the agent's next waiting task (§6.5) |
| Review → Done | **operator only** | **Accept** |
| Review → In progress | operator | **Send back** with a comment, to the same agent; if it is gone, back to Backlog with the comment in the history |
| In progress → Backlog | agent or srv | `release`, lease expiry or the holder closing (as today; uses an attempt). A task the holder filed for itself stays its own; one it took from Anyone goes back to Anyone. |
| any open state → Done (cancelled) | **operator only** | **Cancel** |
| failed → Backlog | operator | **Retry** (attempts reset) |

**Which tasks need review:** by default a task the operator asked for does, whether the operator created it or an agent filed it on the operator's behalf (§6.5); a task one agent enqueued for another on its own initiative doesn't (agent-to-agent handoffs shouldn't wait on a person). Either can be changed on the task. See D2.

### 6.5 "Not now, but don't forget": requests filed for later

This is how the operator works, and the backlog's main source: ask the nearest agent for something, whatever it's busy with.

**What the agent does** (the operator-config guidance says so, §6.12):
1. It judges whether the request can wait. If the operator said "now", or it's a minute's work, it just does it.
2. Otherwise it files it: `Tasks` `enqueue` with `for: "me"`, a title, and the request as a standalone prompt. It answers in one line ("Filed as #12 in my backlog; I'll take it when I've finished the migration") and goes back to what it was doing.
3. When it has finished its current work, it takes its next task with `claim`, which gives its own tasks first.
4. **It keeps going through its backlog** (the operator, 2026-10-20: "if it finishes, it proceeds through the backlog"). Finishing a task (`submit` or `complete`) answers with the next one waiting for it: `Next in your backlog: #15 "…" (requested by the operator at 14:20). Claim it and start.` The agent claims it and carries on, one task after another, until its backlog is empty. A task handed in for review doesn't stop it: review is the operator's part, and the agent moves on meanwhile.

**What srv adds, because the agent can't be trusted to remember or to say who asked:**
- **Who asked.** srv knows what started the agent's current turn (the turn ledger's trigger: the operator's message, another agent's, a schedule, a task). A task filed during a turn the operator started is marked **requested by you, via Loap-3**, and srv keeps the operator's own words from that turn (the pane's last prompt) on the card beside the agent's summary of them. The agent can't set this field; a task filed during a turn another agent started is marked as that agent's.
- **When a send-back arrives mid-task.** A task the operator sends back from Review goes to the front of that agent's backlog, and the comment reaches it as the operator's message. The agent finishes the step it is on, hands back or pauses its current task (`release` with "paused for #12's send-back", which keeps it its own), and takes the send-back first. Corrections to finished work come before new work.
- **Stopping it.** The operator stops an agent going on by putting its remaining tasks on hold, moving them, or switching continuing off for that agent; the next `submit` then answers with nothing to take.
- **The reminder**, for an agent that went idle without finishing through the board (its current work wasn't a task, or it was started by hand). When an agent with tasks waiting for it has been idle for a while (default 2 minutes; no question pending, no turn in progress), srv sends it one system note: `[AgentMux] You have 2 tasks waiting in your backlog: #12 "Write a spec for X" (requested by the operator at 14:02), #15 "…". Take the next with Tasks claim when you're ready.` One note per idle period; none while it's busy; none if the operator has put the task on hold or moved it. The agent then claims it and works, so the request gets done after the current work without the operator having to come back. This can be turned off for all with the setting `kanban:reminders: false`, or for one agent with its block's `kanban:continue: false`, which also stops finishing a task from naming the next (D7). (A reminder that waits while the operator is typing in that pane is not built: srv has no cheap signal for it.)
- **The order** within an agent's backlog is the order filed, unless the operator drags them.

**What the operator can do** with a filed request on the board:
- See it at once, under the agent it's waiting for, with "busy on #9" or the agent's current summary beside it, so it is clear why it hasn't started.
- Drag it to another agent that is free, or to "New worker", which starts it there now (§6.8); or to "Anyone", where the next agent that runs `claim` takes it.
- Put it on hold, reorder, edit or cancel it.

**If the agent closes** before taking it, its waiting tasks stay in the backlog under its name with "agent closed", and the operator moves them (they are not given to Anyone silently: the operator chose that agent). When the agent is reopened, they are its again.

### 6.6 Untracked work

An agent whose turn is active and who holds no task appears in In progress as an **untracked** card: dashed border, the agent's name, its ambient summary (`term:ambient_summary`) or the start of its last prompt, its current tool and checklist. It is not stored; it is built from block meta and `agent:progress` the frontend already receives.

**Track** turns it into a real task, claimed by that agent, titled from the summary (editable). The agent learns of it by a short message ("You're on task 12, '…'; update it with Tasks"), so its later updates land on the card.

A toggle "Show untracked" (on by default) hides them for operators who want only filed work.

### 6.7 A card

- **Title**; **kind** tag; **priority** if not 0.
- **Agent chip**: provider icon, name, live status dot (working, question, idle, error, from `agent-status.ts`'s ranking), and the current tool while working. Click: focus the agent's pane.
- **Repository and branch**; **PR chip** once linked (§6.9).
- **Checklist progress** "3/7": the task's own checklist if the agent set one, else the agent's todos from `agent:progress` while it holds the task.
- **Age in this column**; **attempt** "2/3" after a retry; **lease** warning when a claim is close to expiry with no activity.
- **Creator**: "you" or the agent's name (agent-created tasks are marked; §6.13).

**Opening a card** shows a drawer: the prompt (payload), the checklist, the result, links, and the **history**: every move with who made it and when, every note, question and answer. Buttons by column: Start on…, Answer, Accept, Send back, Cancel, Retry, Open agent.

### 6.8 Starting a task on an agent

"Start on…" lists the agents of this instance with their state, idle first, then "New worker…".

- **An existing agent:** srv claims the task on that agent's behalf (a new attempt) and delivers the prompt to it as the operator's message, framed: `Task 12 "…" (from <creator>):` then the prompt, then one line: "Report on it with Tasks: update, ask, submit." The agent's next `Tasks` call finds its claim.
- **A new worker:** `Agents` `spawn`'s path (`SPEC_AGENT_WORKERS_2026_10_15.md`), with the prompt as the brief and the task claimed for the new agent.
- An agent that is busy can still be chosen; the message waits for its turn, like any other.
- Dragging a Backlog card onto an agent's chip in the section's agent strip (§6.11) does the same.

### 6.9 What srv does by itself

- **Activity is the heartbeat.** Before expiring a claim, the reaper asks whether the holder is alive: registered with this srv and its process running. If so, the lease is extended, not expired (`work_tasks::extend_live_leases`, run before every reap). An agent never has to call `heartbeat` during a long tool call (Hermes's failure), and a dead holder is still reclaimed (by teardown at once, or by the reaper after the lease). A fresh claim starts its own lease; it never inherits the previous attempt's activity time.
- **A question pauses nothing.** While the holder waits on a question, the claim is held (the agent is alive), and the card shows in Needs you.
- **A closed holder** releases its claims (today's teardown), which puts the task back in the Backlog with "agent closed" in the history (under the same agent if it filed the task for itself, §6.5; under Anyone otherwise).
- **PR link.** When the progress watcher sees the holder run `gh pr create` and the holder has exactly one claimed task, the task gets the branch and, once the PR appears in the open-PR list, its number and URL. The history records it. With several tasks held, nothing is guessed; the agent's `submit` names the PR.
- **Every move** (by agent, operator or srv) writes one history row and publishes `WorkQueueChanged`.

### 6.10 Attention

- **Needs you is the only interruption.** Its count goes in the Warden tab title and on the widget-bar Kanban button as a badge.
- A card that enters Needs you because its agent asked a question doesn't add a second flash: the agent pane already flashes for its question.
- An `ask`, a failure and a crashed holder show in the count; ordinary moves don't notify.
- **Review load:** the Review column header shows how many and the oldest wait ("4 · oldest 2 h"). An optional soft limit (D6, the setting `kanban:reviewlimit`, a whole number; unset or 0 is none) turns the count amber past it; nothing is blocked.

### 6.11 Layout and scope

- **Top bar:** filters by repository (default: all), agent and kind; a search box; "Show untracked"; "New task".
- **Agent strip** under it: this instance's agents as chips with their state, a drop target for Start (§6.8).
- **Five columns**, each scrolling on its own (the Backlog grouped by agent, each group collapsible); narrow panes (Warden's container queries) stack the columns into a list grouped by column, Needs you first.
- **Lanes by repository**, a toolbar switch kept on the pane (`kanban:lanes` meta): one row of the five columns per repository, those with unfinished work first, then by name, tasks with no repository last; each lane folds by its header, which counts its open tasks and those that need the operator. Untracked work belongs to no task, so to no repository: it gets its own row above the lanes. The lanes scroll together; within a lane a column is as tall as its cards. Dragging between lanes changes the agent a task waits for, never its repository. Review's soft limit (D6) compares the whole board's Review count, so every lane's Review header turns amber together.
- **Scope:** the queue is shared by every AgentMux on this computer. The section shows all tasks, with the instance's own agents in the strip; a task held by an agent of another instance shows its name with the instance's channel. Other computers (Warden's LAN layer) are Phase 4.

### 6.12 How agents write

The `Tasks` tool keeps its six actions and gains three, all on a claimed task (`id` and `attempt`, as today):

| Action | Input | Effect |
|---|---|---|
| `update` | `note?`, `checklist?` (`[{text, status: pending\|in_progress\|completed}]`, replaced whole), `links?` (`{pr?, branch?, url?}`) | A history row; counts as a heartbeat |
| `ask` | `question` | `blocked`; the question shows on the card in Needs you |
| `submit` | `result`, `links?` | `review` (or `done` for a task that needs no review). The answer names the agent's next waiting task, if any (§6.5) |

`enqueue` gains `for: "me"` (the task waits for the caller; shorthand for `target_agent` set to itself), `repo` (default: the caller's repository from its working folder), `hold` (on hold until the operator releases it) and `review` (whether a person must accept it; default from who asked, §6.4). Its answer gives the task number to quote back to the operator. srv sets who asked from the turn (§6.5); the caller can't.

The operator-config guidance (`crates/srv/operator-config-seed.json`) gets a short entry, aimed at few calls per task: when the operator asks for something while you're in the middle of other work, and it isn't urgent or tiny, file it for yourself (`enqueue` `for: "me"`, the request as a standalone prompt), say its number, and finish what you were doing; when you've finished, `claim` your next task; claim (or be started on) a task before larger work an operator asked for; `update` at milestones only; `ask` instead of waiting silently on something only the operator can decide; `submit` with the PR when done. A task's checklist is optional; an agent whose CLI keeps a todo list shows it anyway (§6.7).

**Codex** gets its plan parsed: the progress watcher reads Codex's plan updates into the same `todos`, so its checklists show like Claude's.

### 6.13 Trust

- **Operator-only moves** (Accept, Cancel, Send back, Answer, Start, moves from Backlog) are websocket RPCs from the UI. No HTTP route an agent's MCP can reach performs them. `DELETE /agentmux/work/:id`, which cancelled without checking who asked, now refuses a call carrying an agent's token.
- **An agent can't accept its own work**: `submit` stops at Review for a task that needs review.
- **A task's prompt is a prompt.** Agent-created tasks are marked with their creator on the card and in the drawer. When the operator starts an agent-created task, the prompt is delivered quoted, with its creator named (§6.8), so the operator's click endorses "do this task", not the raw text as the operator's own words. The claimer's own confirmation rules still apply to what the prompt asks (as the `Tasks` tool's description says today).
- **Card text is text**: titles, notes and questions render as plain text (or the app's sanitised markdown), never HTML.
- Nothing about credentials goes on a card; the existing sensitive-text check used by the coordination channel (`is_sensitive_message`) applies to notes, questions and results.

### 6.14 Kanban and Swarm

Swarm answers "what is each agent doing right now" (agents, subagents, shells, crons, background tasks). Kanban answers "what work is there and where does it stand". They link, they don't merge:
- A Swarm row shows its agent's task ("#12 …", marked when it waits on the operator's answer), which opens the Kanban with that task's drawer open (block meta `kanban:open` on the Warden pane, cleared once shown).
- A card's agent chip opens the agent's pane; its drawer has "Show in Swarm".
- Swarm's subagents, shells and crons stay out of Kanban.

## 7. Data

### 7.1 Storage (identity store, no schema version bump, per its rule)

- `db_work_queue.state` takes the three new values; no column changes.
- New `db_work_cards (num INTEGER PRIMARY KEY AUTOINCREMENT, work_id UNIQUE → db_work_queue.id, repo, branch, pr_url, rank, review_required, checklist JSON, question, note, requested_by, requested_by_kind [operator|agent|other|unknown], request_text, filed_by, last_reminded_at, updated_at)`; `num` is the task's `#12`. A task enqueued by an older build gets its card (and number) the first time the board lists it: the board's own fields, kept apart so older builds' queries never see them.
- New `db_work_events (id, work_id, at, actor, actor_kind, kind, from_state, to_state, body)`: the history. `kind` is one of `created, filed_for_later, reminded, moved, assigned, held, unheld, claimed, released, expired, asked, answered, updated, submitted, accepted, sent_back, cancelled, retried, linked`. Kept as long as the task is, plus 30 days after Done.
- Indexes on `db_work_events(work_id, at)` and `db_work_cards(repo)`.

### 7.2 RPCs (typed, generated stubs)

Three websocket commands (`crates/srv/src/server/app_api/kanban.rs`), the operator's by construction: no agent's MCP reaches them.

| Command | For |
|---|---|
| `kanban.list` → `{tasks: [task + card + holder], agents, stamp, now}` | the board: every unfinished task and those finished in the last 7 days (cap 500), each holder's live state, this instance's agents |
| `kanban.task` `{id}` → `{task, events}` | the drawer |
| `kanban.act` `{action, id?, …}` | the operator's moves: `create` (title, payload, agent, hold, review, repo, kind, priority), `edit`, `move` (agent: `""` for anyone, hold, rank), `start` (agent), `answer` (text), `accept`, `send_back` (text), `cancel` (text), `retry`, `track` (block_id, title) |

`id` is a task's id or its number. The agents' side is HTTP, beside the queue's routes: `POST /agentmux/work/:id/update`, `/ask`, `/submit` (`crates/srv/src/server/work_tasks.rs`).

### 7.3 Events and freshness

- `WorkQueueChanged` stays the trigger; the section refetches `work.list` debounced 150 ms, only while visible.
- **Across instances:** the store is shared but events are not (each srv publishes only to its own windows). An agent in a dev build moving a task wouldn't reach the stable build's Kanban. While visible, the section also asks srv for the queue's newest `updated_at` every 10 s (one indexed read) and refetches when it changed.
- Holder state comes from `ControllerStatus`, `AgentTurn`, `AgentFailed` and `agent:progress`, which the frontend already receives; block meta from the object store.

## 8. Files

### 8.1 Frontend

- `frontend/app/view/warden-kanban/`: `warden-kanban-manager.tsx` (the section, `visible` prop), `kanban-model.ts` (signals, fetch, subscriptions, `dispose`), `kanban-columns.ts` (`columnOf`, pure, tested), `kanban-card.tsx`, `kanban-drawer.tsx`, `kanban-new-task.tsx`, `kanban.scss`, tests. Each file under the repo's size limit (`scripts/check-file-sizes.mjs`); Swarm's 2,000-line model is the example not to follow.
- `frontend/app/view/warden/warden-model.ts` (`WardenSection`, label), `warden-view.tsx` (`RAIL`, the section's pane), `warden-view.test.tsx`.
- `frontend/app/drag/drag-types.ts`: a `KANBAN_CARD` tag, so the tile and tab drag monitors ignore cards.
- `frontend/app/store/command-registry.ts`: `open:kanban`.
- `frontend/app/view/swarm/`: the "on #12" link on a row (Phase 2).

### 8.2 srv and MCP

- `crates/srv/src/backend/storage/work_queue.rs` (new states, own-tasks-first claiming), new `work_board.rs` (cards, history, the board's transitions), `store.rs` (tables).
- `crates/srv/src/server/work_queue.rs` (cards, history rows, numbers as ids, the cancel guard), new `server/work_tasks.rs` (update, ask, submit; who asked; the next task; live leases), new `server/app_api/kanban.rs` (the typed RPCs), new `backend/kanban.rs` (the reminder, the PR link), `backend/rpc_types/commands.rs` (command names).
- `crates/srv/src/backend/reactive/progress_watcher.rs` (PR link; Codex plans).
- `crates/mcp/src/tools/tasks_tool.rs`, `work.rs`, `tool_schemas.rs` (`update`, `ask`, `submit`; `enqueue`'s new fields).
- `crates/srv/src/config/widgets.json` (`defwidget@kanban`); `crates/srv/operator-config-seed.json` (guidance).
- Generated: `frontend/types/rpc/rpc-schema.json`, `rpc-api/generated-manifest.json`, `rpc-api/generated.ts` (`export_bindings` test, `node scripts/gen-rpc-stubs.mjs`).

## 9. Plan

1. **A Kanban over what exists, and requests filed for later.** No schema change. `work.list`; the Warden section with Backlog (grouped by agent), In progress, Needs you and Done derived from today's states and the holders' live state; untracked cards; `open:kanban` and the pinned widget; the Needs-you count in the title. With it, the part of §6.5 that today's queue already supports: `enqueue` `for: "me"` (today's `target_agent`), own-tasks-first claiming, the idle reminder, and the operator-config guidance. From this phase on, the operator's "do this when you're done" requests land on the board and get done.
2. **The operator's hands, and who asked.** Who asked and the operator's words on the card (§6.5); the three new states, `db_work_cards`, `db_work_events`; New task, drag, Start on…, Answer, Accept, Send back, Cancel, Retry, Track; the drawer and history; the cancel guard; Swarm's link.
3. **The agents' words.** `update`, `ask`, `submit`; activity as heartbeat; PR link; Codex plans; the operator-config guidance.
4. **Later, each its own decision:** close a reviewed task when its PR merges (D5); dependencies between tasks (Cline's and Claude Code's "blocked by"); swimlanes by repository and the Review limit (D6), both built in #5260; other computers through Warden's LAN layer; a `tasks:read` permission for sandboxed widgets.

## 10. Risks

- **Reminder loops.** An agent reminded of a task it can't do would release and be reminded again. One reminder per idle period, none for a task the agent released in the last hour, and a released task says why on the card, so it lands in front of the operator instead of cycling.
- **A reminder that interrupts.** An idle agent may only be waiting on a review or a CI run; taking a backlog task meanwhile is the intended use. The reminder never arrives mid-turn, with a question pending, or while the operator is typing in that pane.
- **Agents don't use it.** Mitigated by design: columns come from srv's facts and untracked cards, so the board is right without a single `Tasks` call; agent updates add detail, not truth.
- **Mixed versions on one computer.** The identity store is shared by every installed and dev build. The new states are never claimed or expired by older builds (§6.2), and the new tables are ignored by them. An older build's `complete` on a task that needs review would set `done` directly; acceptable, since the operator chose to run that build.
- **Too many cards.** Done is windowed to 7 days; untracked cards can be hidden; the list is capped at 500 with filters.
- **Delivering a task as the operator's message** carries an agent's words with the operator's weight. Mitigated by quoting with the creator named (§6.13), and only on the operator's click.
- **Cost of a mounted section.** Warden keeps every section mounted; Kanban fetches and subscribes only while visible.

## 11. Decisions for the operator

- **D1.** *Name decided:* Kanban (the operator, 2026-10-20). Kanban as a Warden section, first in the rail, with its own pinned widget-bar button and `open:kanban`. *Recommended* (the operator's direction; the button is what makes it central while Warden stays unpinned).
- **D2.** A task the operator asked for needs review by default, whether the operator created it or an agent filed it on the operator's behalf; one an agent enqueues on its own initiative doesn't. *Recommended.*
- **D3.** "Start on…" delivers the task's prompt to the chosen agent at once, as the operator's message (quoted when an agent wrote it). *Recommended.* The alternative, only setting the target and waiting for the agent to claim it, leaves idle agents idle.
- **D4.** Untracked work shows as cards by default. *Recommended.*
- **D5.** A merged PR closes its task from Review without a click. *Not now*: until the review flow is used, Accept stays a person's.
- **D6.** A soft limit on Review, default off. *Built, off by default* (#5260): `kanban:reviewlimit` in settings turns it on.
- **D7.** *Decided* (the operator, 2026-10-20: "if it finishes, it proceeds through the backlog"): an agent that finishes a task goes on to the next in its backlog, and one that goes idle with tasks waiting is reminded once, after 2 minutes idle, and takes the next itself. On by default, with a per-agent switch. The alternative, waiting for the operator to say "go", is what the backlog is meant to save.

## 12. Sources

Read 2026-10-20. Some details could not be confirmed and are left out above: Codex cloud's state values, Vibe Kanban's MCP tool names, Hermes's default claim timeout (its docs and its issues disagree).

- Vibe Kanban: https://www.vibekanban.com/, review https://elite-ai-assisted-coding.dev/p/vibe-kanban-tool-review, HN https://news.ycombinator.com/item?id=44533004, status https://first-tree.ai/compare/vs-vibe-kanban
- Cline Kanban: https://azukiazusa.dev/en/blog/cline-kanban/
- Hermes Agent Kanban: https://hermes-agent.nousresearch.com/docs/user-guide/features/kanban; heartbeat issues https://github.com/NousResearch/hermes-agent/issues/21147, /25517, /31752, /119155
- Claude Code: https://code.claude.com/docs/en/agent-sdk/todo-tracking, https://code.claude.com/docs/en/agent-teams, https://code.claude.com/docs/en/tools-reference
- Linear: https://linear.app/developers/agent-interaction, https://linear.app/developers/agent-best-practices
- GitHub Copilot coding agent: https://github.blog/news-insights/product-news/agents-panel-launch-copilot-coding-agent-tasks-anywhere-on-github/, https://github.blog/changelog/2025-12-18-assigning-github-copilot-to-an-issue-now-adds-you-as-an-assignee/, https://github.com/orgs/community/discussions/190731
- Superset https://superset.sh/, Nimbalyst https://docs.nimbalyst.com/session-management/agent-window-and-session-management, Devin https://docs.devin.ai/release-notes/overview, Google Antigravity https://antigravity.google/blog/introducing-google-antigravity/
- Backlog.md https://github.com/MrLesk/Backlog.md, Taskmaster https://github.com/eyaltoledano/claude-task-master/blob/main/docs/task-structure.md, kanban-md https://github.com/antopolskiy/kanban-md, agent-kanban https://github.com/hedburgaren/agent-kanban, kanbo https://github.com/Krowli/kanbo
- Review load and notifications: https://news.ycombinator.com/item?id=48239413, https://addyosmani.com/blog/code-agent-orchestra/, https://junctionpanel.dev/blog/what-ai-coding-agent-notifications-should-actually-tell-you/; "not a Kanban board": https://news.ycombinator.com/item?id=47307169
- Protocols: A2A https://raw.githubusercontent.com/a2aproject/A2A/main/specification/a2a.proto; MCP Tasks https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/tasks; ACP plans https://agentclientprotocol.com/protocol/agent-plan; AG-UI https://docs.ag-ui.com/spec/1.0/events
