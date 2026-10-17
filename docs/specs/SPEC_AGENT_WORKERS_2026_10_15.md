# Spec: an agent launches and coaches worker agents in panes

**Date:** 2026-10-15
**Status:** active. Phase A (§3.1, §3.2's pane mark, §3.5, §3.6) implemented in #5020 (as `Agents` `fork`) and #5023 (renamed `Agents` `spawn`; a worker is a new agent, not a fork; its account, §3.5): `app_api/agent_spawn.rs`. Phase B (§3.3, §3.4) implemented in #5031: `backend/worker_notes.rs`, `Agents` `workers`; a question mid-turn (§3.3) in #5035. Left: Phase C (§3.7); §3.2's table was not needed (Phase A's marks serve the counts and the list).
**Author:** Loap-3@narko, at the operator's request
**Scope:** srv (`app_api/agent_open.rs`, `server/agent_handlers/template.rs`, `backend/reactive/handler.rs`), the MCP `Agents` tool (`crates/mcp/src/tools/agents_tool.rs`, `panes.rs`), the agent pane header.
**Related:** `SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md` (the UI's fork), `SPEC_JEKT_DELIVERY_STATES_AND_MAILBOX_2026_10_01.md`, `SPEC_AGENT_COORDINATION_CHANNEL_2026_10_15.md`, the Warden Supervisor (`frontend/app/view/warden-supervisor/`).

## 1. What the operator wants

A planner on a strong model (say Opus) writes a spec, then, asked to, launches a worker: a new agent built from its own definition, on a cheaper model (say Sonnet), with a name (`Loap-4`), in a new pane next to it. The planner coaches the worker through implementing the spec. The operator watches both and can type into either. It is a subagent the operator can see and talk to, and one planner can run a few of them, on different projects.

The operator's words: "launch a fork (loap-4) in a new pane, sonnet, that you coach through implementing your spec … the leader could also manage a couple projects."

Below, the planner is the **leader** and the agent it launches is its **worker**.

**Fork and spawn, in one line each** (the glossary "Fork, spawn and worker", `server/agent_handlers/mod.rs`): a *fork* is user-initiated, an exact copy of an agent with the same history; a *spawn* is agent-initiated, a new agent (a *worker*) with the leader's definition and a brief, and no history. Currently, a fork of a worker carries the worker's conversation (the brief with it) but not its `agent:worker_of` mark, so it isn't the leader's worker (whether a fork of a worker should stay one is open: it bends "a fork is an exact copy"; the operator decides) and doesn't count toward its limit; it is not refused as a worker is, and a spawn it makes asks the user unless their own turn asked for it.

**A worker is not a fork** (the operator, 2026-10-16: "the agent fork feature is different from the Leader/follower work, where an agent spawns other agent panes. in that latter, there is no history"). The pane's Fork (`SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md`) copies an agent with its whole conversation and records it in the source's lineage. A worker is built from the leader's definition and starts with the leader's definition, Global Memory and the brief only (no conversation, no Personal Memory) and no lineage: it isn't among the leader's forks, and Claude's resume gate, which copies a conversation only into a definition recorded as a fork (`continuity_segments.rs`, `fork_source_head`), never gives it the leader's. #5020 called the action `fork` and made the worker through the fork RPC; #5023 renamed it `spawn` and builds the worker with `copy_definition(.., as_fork: false)`.

## 2. What exists, and what's missing

| Need | Today | Gap |
|---|---|---|
| Create the worker | The pane's "Quick Fork" (`frontend/app/view/agent/quick-fork.ts:108-274`): `ForkAgentDefinitionCommand` (srv `template.rs:395-532`), then the frontend links the parent's account (`agent-model.ts:936-943`), carries the runtime over, and opens it in the pane's stack. | Only from the UI. srv's fork links the source's accounts (`inherit_source_account_links`) but not the runtime; the frontend carries the runtime over at launch. No agent can call it. |
| Open it in a pane | `Agents` `open` (`crates/mcp/src/tools/panes.rs:255-297` → `/api/v1/agent/open`, `app_api/agent_open.rs:319`) | Opens existing agents only. Takes no model or effort (the runtime is the remembered one, `agent_open.rs:565-579`). Doesn't read who is calling (`http_open.rs:101-109` has no `Caller`). Places the pane in the first workspace's active tab, appended, not next to the caller (`app_api/mod.rs:222`). |
| Coach it | `Message` `send`: arrives as a jekt, mid-turn included (`blockcontroller/mod.rs:759-799`), `TRUST=host-verified` both ways (`server/reactive.rs:229-253`). | Nothing. |
| Know how it's doing | `Agents` `transcript` (`turn_active` plus the transcript tail), `Agents` `conversations` | Poll only. srv knows every turn end, question, failure and exit (`backend/agent_state.rs:80-99`, the `agentturn` ledger, `health.rs:323-327`), but nothing tells another agent. |
| Know it's a worker | Fork lineage on the definition (`parent_id`, `branch_label`) | Nothing records who launched which pane. |
| Stop it | `Stop` `pane` (`server/app_api/pane.rs:615-641`, through the user's 15 s override window) | Nothing. |

## 3. Design

### 3.1 `Agents` `spawn`: one call creates, opens and briefs the worker

A new action on the consolidated `Agents` tool:

```
Agents { action: "spawn", name?: string, model?: string, effort?: string, account?: string, brief: string, user_instruction?: string }
```

- `name`: the worker's name, its slug derived from it. If left out, the first free "<leader> worker N". A name already in use is refused.
- `model` and `effort`: validated as a launch reads them back (`Remembered::parse`, `backend/agent_runtime.rs`: `opus`, `sonnet`, `haiku` or a `claude-…` id; `low` to `max`). If left out, the leader's own. Only Claude's runtime is remembered across a launch today, so for another provider both are refused and the worker runs on the leader's flags.
- `brief`: the worker's first message: what to do, where the spec is, how to report. Required: a worker with no task only spends tokens.
- The worker opens in the leader's own tab (`resolve_tab_id_for_block`, `app_api/mod.rs:264`), not the first workspace's active tab; Phase A takes no `tab_id`.

srv does all of it in one handler (`POST /api/v1/agent/spawn`), so the result doesn't depend on a frontend being open:
1. **Admission** (§3.5).
2. **Build the worker's definition** from the leader's (`copy_definition`, shared with the fork RPC: copies content, skills, provider and `provider_flags`; resets the workspace, auto-start and bus id; makes a new memory bundle), with no lineage (`parent_id`, `branch_label` empty).
3. **Link the account** chosen for it (§3.5's account rule: the one asked for, else the leader's, else another of the user's that can run), with `agent_identity_link`. A fork copies its source's links (`inherit_source_account_links`); a worker's is chosen, not copied.
4. **Set the runtime:** the remembered runtime `{model, effort, permissionMode}` (`agentlastruntime`, `normalize_last_runtime`), the leader's permission mode with the requested model and effort. The launch then uses it as for any agent (`seed_launch`).
5. **Open it** with the existing `open_agent_inner` path, placed **to the right of the leader's pane**, not stacked under it: the operator wants to watch both. `agent.open` took `split_direction` and `split_reference_block_id` but ignored them; it now places the pane as `pane.open` does (`pane::resolve_placement`).
6. **Mark it** (§3.2).
7. **Brief it:** the brief goes as a jekt from the leader, sent by the leader's own MCP server after srv answers (`Message` `send`'s path, which queues it while the worker starts), signed as the leader's own messages are, so it arrives `TRUST=host-verified` and the worker can answer with `Message` `send`. AgentMux adds one line to the brief: "You are a worker of `<leader>`. Report to it with Message send when you finish, get stuck, or need a decision."

It answers `{agent_id, name, message_to, block_id, tab_id, provider, model, effort, account, account_id, account_note, asked_user, brief}`, or an error naming the step that failed. A failure after step 2 deletes the new definition and any pane the open made before it failed (`discard_worker`), as quick-fork removes its pushed block on failure (`quick-fork.ts:151-163`).

**A fresh session, not the leader's conversation** (the operator's decision, 2026-10-16; it is what makes a worker different from a fork). The pane's Fork continues the parent's session (`--fork-session`, Claude only). A worker starts empty:
- the leader's context is long and expensive, worst of all on the model the worker is meant to save on;
- the spec, not the conversation, is the hand-off: if the brief can't stand alone, the spec is missing something.

Fresh means without the leader's conversation, not without context. The worker launches like any agent, so it gets:
- **Global Memory**: the workspace rules and AgentMux's Operator Config, which srv writes into every agent's startup file at launch (`inject_global_bundles`, `backend/agent_config.rs`). The rules on branches, PR titles, changesets, public repos and credentials apply to the worker unchanged.
- **The leader's own definition**: its soul, instructions and skills, which `copy_definition` copies (`template.rs`, the content and skills copy after `agent_def_insert`).

It doesn't get the leader's Personal Memory (it gets its own, empty bundle: `agent_def_provision_and_bind_bundle`) or the CLI's own memory folder, which belongs to a workspace (Claude's `~/.claude/projects/<workspace>/memory`). Personal Memory stays private to the agent that wrote it; what the worker needs from the leader goes in the spec or the brief.

**Its own workspace.** As for any new agent, a fresh folder under `~/.agentmux/agents/` (`agent-model.ts:491-506`). The workspace rules already say how the worker gets the code: it clones into its own workspace (`git clone --reference` the leader's clone saves disk), reads the leader's uncommitted spec where it is, and never edits the leader's workspace. The brief gives the spec's absolute path.

**How it reports.** AgentMux's line ahead of the brief names the tool: "Report to <leader> with the agentmux Message tool (action send, to <leader>), not a built-in SendMessage". In the first run (2026-10-16, a Haiku worker) the line said only "Message send", and the worker used Claude Code's own cross-session `SendMessage`, which reached the leader unverified instead of as a `host-verified` jekt. A second dev-build run (2026-10-16, after #5023) showed the reason: AgentMux's MCP tools are deferred in Claude Code, the worker searched its tools for "SendMessage", and found Claude Code's own. The line now names the tool exactly (`mcp__agentmux__Message`, action "send"); and a report that doesn't go through AgentMux isn't counted as one, so the leader is told the turn ended without a report (§3.3).

### 3.2 The link: who coaches whom

- **On the worker's pane:** a srv-only block meta `agent:coached_by = {agent, id, block_id}` (the leader's name, definition id and pane), set when the worker's block is created (`open_agent_with_meta`), stripped from a client's new block and refused from a client's meta update, as `browser:owner_agent` is (`server/browser_owner.rs`). The id is the leader's verified identity's definition, not a claimed name.
- **On the worker's definition** (#5023): agent content `agent:worker_of = {agent, id}`, written by the spawn. A worker's pane closes and is reopened from My Agents like any agent, and the ordinary open writes no `agent:coached_by`; the definition still says whose worker it is, so it still can't spawn and still counts toward its leader's limit (`live_workers_of` counts a pane by its mark or its definition's). `setagentcontent` refuses it from a client (`client_may_write_content`), and a fork of a worker (the pane's Fork) doesn't inherit it.
- **In srv's store** (Phase B; Phase A counts the panes that carry the mark, in a tab, `live_workers_of`): a table `db_agent_workers (worker_def_id, worker_block_id, leader_uid, leader_name, created_at, ended_at)`. It is what the limits count and what `workers` lists. `ended_at` is set when the worker's pane closes or its definition is deleted.

### 3.3 The leader hears from its workers without polling

srv already computes each agent's state (`working | waiting | idle | stopped | error`, `agent_state.rs`). For a worker, a change the leader needs becomes an `[AgentMux]` note to the leader (`Handler::deliver_system_note`, `reactive/handler.rs:1551-1579`, the overlap notes' path):

| Worker event | Note to the leader |
|---|---|
| A turn ended, and the worker sent the leader no message during it | `[AgentMux] Your worker Loap-4 finished a turn without reporting. Its last words: "<the last assistant line, 200 chars at most>".` |
| It asked the user a question (`term:awaiting_user`) | `... Loap-4 is waiting for the user to answer a question.` |
| It failed (`agentfailure`) | `... Loap-4 stopped with an error: <class>.` |
| Its process exited, or its pane closed | `... Loap-4 stopped.` |

- A turn in which the worker messaged the leader gets no note: the message is the report. A report is a successful AgentMux delivery from the worker (its agent id) to one of the leader's open panes since the turn began, from the handler's delivery log (`get_audit_log`), which is a 100-entry ring: a task copies its successful deliveries into `worker_notes`' own table every two seconds, so a report isn't gone before its turn ends. A turn end published twice (when its settle window lapses and at the tracker's close) is handled once, by its turn and end time. The sender's name there is the one it gave; a false claim can only hide a note, never send one.
- The quoted words are the worker's: they pass through `quote_in_system_note`, so they can't open a marker of their own. They are information, not instructions, like the channel's posts.
- At most one note per worker per minute; a held one is counted in the next ("2 earlier updates from it were held back").
- Implemented in `backend/worker_notes.rs`, installed at boot (not with the notify Router, which starts only when a window connects): a broker observer takes `agentturn` (`end: completed` or `exited`; AgentMux's own hidden turns skipped) and `agentfailure`; a task does the store reads and delivers to the leader's open pane, found by the leader's definition id (a restored pane's `block_id` in the mark is stale).
- Waiting on a question: a turn whose last message ends in a question, or that ended calling `AskUserQuestion`, is reported as waiting (`ambient::digest`). A worker blocked inside a question dialog mid-turn ends no turn; the agent pane sets `term:awaiting_user` on its block while the dialog is open (`view/agent/hooks/awaiting-user.ts`). `worker_notes` subscribes to the reducer's events for that flag being set, waits 30 s (a user at the pane usually answers first), and notes the leader only if the worker is still `waiting` (a turn in flight and the flag still set, `agent_state::current_state`): `... Loap-4 is waiting for the user to answer a question in its pane; it can't go on until they do.` The flag is written by a mounted pane, so a worker whose pane isn't on screen in any window isn't seen until it is.

### 3.4 `Agents` `workers`: the leader's view

```
Agents { action: "workers" }
→ { workers: [{ name, message_to, agent_id, block_id, state, since_ms, last_report_ms }] }
```

From the panes that carry the leader's mark, or whose definition does, and the agent state tracker (`agent_status_of_block`); `last_report_ms` from the delivery log. With this a leader can run a few workers on different projects and see them all in one call. It reads only the caller's own workers; a worker gets an empty list.

### 3.5 Limits

Launching an agent spends tokens until someone stops it. The rules:

- **The user asked for it.** A spawn goes ahead at once only when the leader's current turn was started by the user's own message, the rule `QuitSelf` already applies (`sagas/self_quit.rs:43-79`). Any other turn (one a jekt, a schedule or a worker's report started) asks the user first, in the host's approval window (`connections::ask_user`, which no agent can answer), naming the leader, the worker's name, model and account, and the brief's start. So a message from another agent can't make an agent spawn agents.
- **An identified leader.** The handler requires `Caller::Agent` (the agent token, `server/caller.rs`); an unidentified call is refused. Container agents are refused, as they are for `open` (`backend/container_credential.rs:88-111`).
- **No chains.** A worker (`agent:coached_by` set) can't spawn.
- **A few at a time.** At most 3 live workers per leader (`MAX_LIVE_WORKERS`; a setting can come with Phase B), counted from the panes that carry the leader's mark. One spawn per leader runs at a time (`leader_lock`), from the count through the user's answer to the open, so parallel calls can't each see room for one more.
- **An account that can run.** `account` names one of the user's accounts for the leader's provider (id, name or email). Left out, the worker runs on the leader's. An account can't take a worker when it is signed out (status `expired`, `needs_reauth`, `invalid`) or a pane whose run was on it last ended on the plan's usage limit or a refused sign-in (`agent:last_failure`, cleared by that pane's next clean turn; a rate limit is transient and doesn't count). The account a run was on is noted on the pane at spawn (`agent:run_account`, `build_persistent_spawn_env`), so relinking the agent afterwards doesn't move the failure to its new account; a pane from before that falls back to its agent's current link. Candidates and their status come from this channel's accounts (`id_store`), the ones a launch can use. Then: one asked for is refused (`ACCOUNT_CANT_RUN`, with why); the leader's is swapped for the first of the user's other accounts that can run, and `account_note` (and the user's prompt) says so; with none, nothing starts (`NO_ACCOUNT`). AgentMux's failure classifier now reads Claude's "You've hit your weekly limit" (and "session limit") as the usage limit; before, it recognized only "usage limit".

### 3.6 Ending a worker

- The leader closes the worker's pane with `Stop` `pane`, as today: the user's 15 s override window applies, and `ended_at` is set.
- The worker's definition stays in My Agents, as an agent of its own (not among the leader's forks). Deleting it is the user's call; a leader can't.
- A worker can't quit itself on its leader's word: a jekt-started turn waits out the override window (`self_quit.rs`). This stays.

### 3.7 What the user sees

- The worker's pane header shows `coached by Loap-3`, a chip from `agent:coached_by`; clicking it focuses the leader's pane.
- In the worker's conversation, the leader's messages already show as jekt bubbles from `Loap-3` (`JektBubble.tsx`).
- Later: the Swarm groups workers under their leader, as it groups a CLI's own subagents under their parent pane (`swarm-model.ts:2019-2055`).

## 4. Phases

- **A: spawn and brief** (§3.1, §3.2, §3.5, §3.6). With this alone the operator can say "launch Loap-4 on Sonnet and coach it through the spec", and the leader coaches with `Message` `send` and checks in with `transcript`.
- **B: the leader hears from its workers** (§3.3, §3.4).
- **C: in the UI** (§3.7).

## 5. Tests

- **Spawn handler:**
  - creates the definition, links the leader's account, sets the requested model and effort, and opens the pane right of the leader's in the leader's tab;
  - refused without a token, from a container agent, from a worker, past the limit, with an unknown model, with a name in use;
  - a failure after the definition is made deletes it, and the pane if there is one;
  - the worker is not a fork (no lineage, its own slug, no copied account);
  - the account: the leader's while it can run; another, with a note, when it is out of quota or signed out; one asked for, or refused with why; none, and nothing starts.
- **Consent:** a user-started turn spawns at once; a jekt-started one asks, and a decline creates nothing.
- **`agent:coached_by`:** written by the handler, refused from a client's meta update.
- **Brief:** arrives at the worker `TRUST=host-verified`, from the leader.
- **Notes:** a turn end with no report notes the leader, and one with a report doesn't; a question, a failure and a stop each note; at most one per minute; markers in the quoted words are neutralized.
- **`workers`:** lists only the caller's workers, with their state.
- **MCP:** schema/handler parity (the existing test).

## 6. Open questions

1. ~~**Fresh session or history.**~~ Decided 2026-10-16 by the operator: a worker starts fresh (§3.1), always. Carrying the conversation is what the pane's Fork does; a worker that needs more context gets it from the spec and the brief.
2. **Model ceiling.** Should a worker be refused a model stronger than its leader's? Not in this version: the user asked for it in their own turn, or approved the prompt.
3. **Another provider.** The worker keeps the leader's provider. A Claude leader with a Codex worker would need the user to pick that worker's account; left out.
