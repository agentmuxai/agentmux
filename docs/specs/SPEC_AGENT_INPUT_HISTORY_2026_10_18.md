# Spec: Agent input history that lives with the agent

**Date:** 2026-10-18
**Author:** QTest
**Status:** active (phase 1 implemented; phases 2 to 4 to follow)
**Origin:** the 0.59.31 → 0.59.32 upgrade lost every pane's ArrowUp history
(`docs/reports/REPORT_UPGRADE_0_59_32_DOGFOOD_FINDINGS_2026_10_18.md` §2.4). The
operator asked for "a robust system to track that data over the long term; it
lives with the agent, so even after many exit rounds users have access to that
history".

## 1. Problem

What a user types to an agent is worth keeping: a prompt they refined, a long
instruction they want to send again, a command they run every morning. Today
it is kept as if it were scratch.

- **Where it lives.** The sent-message history is an array per *block* (pane)
  in the window's `sessionStorage`, at most 200 entries
  (`frontend/app/view/agent/reload-persistence.ts`; recall in
  `AgentFooter.tsx`). The unsent draft is kept the same way.
- **What loses it.** `sessionStorage` survives a reload of the same window
  and nothing else: an app restart, an upgrade, the pane moving to another
  window, and closing the pane all lose it. A restored session gives panes new
  block ids, so even a copy under the old id would not be found.
- **Wrong owner.** History belongs to the agent, not the pane. Open the same
  agent in a new pane tomorrow and its history should be there.
- **No access beyond ArrowUp.** There is no way to search it, see it, or
  delete an entry.

The provider's own transcript does hold the user messages, but it is not a
substitute. It is per session and per provider, mixed with tool traffic,
jekts and system notes, it lives where the provider keeps it, and it ends
when the agent starts a new conversation.

## 2. Goals

1. **Per agent, durable.** An agent's input history survives reloads,
   restarts, upgrades, new conversations (`/clear`, a new session), closing
   and reopening its pane, and moving it between windows.
2. **ArrowUp works as it does now**, against that history, with no
   noticeable delay.
3. **Long term.** Years of use, bounded so it can't grow without limit.
4. **The user can find and manage it.** Search it, send an entry again, delete
   one entry, clear an agent's history.
5. **The draft survives too.** An unsent message in the composer comes back
   after a restart or an upgrade.
6. **A framework, not a composer feature.** One store and one API that any
   surface can use: the composer, a history view, the Swarm view's broadcasts,
   and later sync.

## 3. Non-goals

- Syncing history between machines or through the cloud. Kept local; a later
  spec can add sync with its own privacy design.
- Giving agents access to it. It is the user's own typing: agents already see
  their conversations, and this history adds nothing they need. No MCP action
  reads it.
- Replacing transcripts or `Agents search_history`.

## 4. Design

### 4.1 What is recorded

One entry per message the user sends to an agent from AgentMux's own UI:

| Source | Recorded | Notes |
|---|---|---|
| The agent pane's composer | yes | Plain text, slash commands (`/compact`), bang commands (`!cmd`) |
| A Swarm broadcast the user typed | yes, in each recipient's history | `source: swarm`, so a later view can group them |
| Jekts, `[AgentMux]` system notes, scheduled prompts | no | Not the user's typing |
| Attachments | names only | A reference (`name`, `kind`), never the file's contents |

Consecutive duplicates collapse into one entry (as now) that keeps a send count
and the latest time.

### 4.2 Where it is stored

Two tables in the **channel store** (`objects.db`), next to `db_agents`,
keyed by agent id (`crates/srv/src/backend/storage/agent_input_history.rs`).
Additive and applied on every open, like `db_agent_bundles`, so no schema
version bump; no foreign key, also like it: an agent's rows go with it in
`purge_agent_dependents`.

```sql
CREATE TABLE db_agent_input_history (
    id           INTEGER PRIMARY KEY,
    agent_id     TEXT NOT NULL,        -- db_agents.id
    sent_at_ms   INTEGER NOT NULL,
    text         TEXT NOT NULL,
    kind         TEXT NOT NULL,        -- message | slash | bang
    source       TEXT NOT NULL,        -- composer | swarm | backfill
    session_id   TEXT,                 -- the conversation it went to, if known
    attachments  TEXT,                 -- JSON [{name, kind}] or NULL
    sends        INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX idx_agent_input_history_agent
    ON db_agent_input_history (agent_id, id);

CREATE TABLE db_agent_input_drafts (   -- one unsent draft per agent
    agent_id     TEXT PRIMARY KEY,
    text         TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
```

Why the channel store:

- It is where the agent is (`db_agents`), so the history is created, carried
  and deleted with the agent.
- Upgrades already carry it: carry-forward copies `objects.db` to a new
  version, and the pre-migration snapshot and backups include it.
- It is one SQLite file per channel, already used by both srv and the
  migration framework.

An agent with the same id in two channels has two histories, as it has two of
everything else. Sharing history across channels belongs with the larger
question of agents that span instances, not here.

`db_agent_history` (from the former Forge) is not reused: it has a different
shape and meaning and holds nothing today. It can be dropped by a later
migration.

### 4.3 Retention

- Default: keep the newest **10,000 entries per agent**, with no age limit
  (`INPUT_HISTORY_MAX`). At typical use that is years of history and a few
  MB per agent. An entry is at most 64 KB: a pasted file is not history.
- Setting `agent:inputhistory:max` (0 turns recording off for every agent):
  phase 4.
- Trimming runs on append, in the same transaction, oldest first.
- Deleting an agent deletes its history and draft, at the moment its other
  data is deleted: a deletion that can still be undone keeps them.

### 4.4 API (srv)

Typed RPC commands with generated stubs (`server/app_api/input_history.rs`):

| Command | Does |
|---|---|
| `agent.input-history-append {agent_id, text, kind, source?, attachments?}` | Record one send; a repeat of the newest entry counts `sends` instead; trims. Nothing for an unknown agent |
| `agent.input-history {agent_id, before_id?, limit?}` | Newest first, paged (200 by default, at most 500) |
| `agent.input-history-search {agent_id?, query, limit?}` | Case-insensitive (ASCII) substring match; `%` and `_` are literal; one agent, or all |
| `agent.input-history-delete {id}` / `agent.input-history-clear {agent_id}` | Remove one entry, or an agent's whole history |
| `agent.input-draft {agent_id, text?}` | The unsent draft; with `text`, keep that one (empty removes it) |

Each change publishes `agent:inputhistory {agent_id, what}` (`history` or
`draft`), so two panes showing the same agent stay in step.

The writes come from the frontend, which knows what the user typed. srv does
not infer history from what it forwards to the provider, so jekts and system
notes can't leak into it.

### 4.5 Frontend

- `reload-persistence.ts` keeps its role for the attachments draft. The text
  draft and history move behind an `agent-input-history.ts` module keyed by
  agent id.
- **On composer mount:** load the newest 200 entries and the draft. ArrowUp
  walks the loaded entries. Past the oldest one, it fetches the next page.
- **On send:** append locally first, then `agent.input-history-append`
  without awaiting, so sending is never slower. A failed append is logged. A
  send never fails because history couldn't be written.
- **Draft:** debounced writes (0.5 s) to `agent.input-draft`, flushed on
  `pagehide`; the window's own per-block draft still comes back first after a
  reload, and srv's when there is none (a restart, an upgrade, a new pane).
- **One cache per agent** (`agent-input-history.ts`), shared by every pane
  showing it. Entries arrive before the oldest shown (srv's pages) or after
  the newest (another pane's sends, on the event); each composer is told
  where, so its cursor stays on the same entry.
- **The existing ArrowUp rules stay:** queued-message recall first, selection
  and caret gating, soft-wrap handling
  (`SPEC_COMPOSER_SHIFT_UP_SELECTION_VS_HISTORY_RACE_2026-08-11.md`).
- A pane without an agent id (a quick-launch provider pane) keeps the current
  per-block behaviour.

### 4.6 Seeing and managing it

- **Reverse search in the composer** (Ctrl+R, as in a shell): type to filter
  this agent's history, Enter puts the entry in the composer, Escape returns.
- **A History tab** in the agent's settings or Agent History pane: the list
  with times, search across all agents, send again, copy, delete, clear.
- **Command palette:** "Search input history".

### 4.7 Backfill from existing conversations

So that history starts full instead of empty, a one-time backfill per agent
reads the user messages from the agent's past provider sessions (the session
files `Agents search_history` already reads) and records them with
`source: backfill` and their original times.

- Only messages the user typed are imported: never jekts (`[JEKT:` markers),
  `[AgentMux]` notes, Swarm broadcast headers, tool results, or provider
  system messages.
- It runs in the background after the first composer mount for that agent,
  never during startup. It caps at the retention limit and records that it
  ran (a column on the agent, not a marker file), so it runs once.
- The `sessionStorage` histories of the current window are also imported once
  where the block's agent is known, so nothing typed in this release is lost.

### 4.8 Privacy

- Kept on this machine only, in the same store as the agent's other data. Not
  synced, not sent to any service.
- What the user types may include secrets. Deleting one entry is always one
  click away, the history view says what is kept, and
  `agent:inputhistory:max = 0` turns recording off.
- Exports of an agent (bundles, sidecars) leave history out unless asked for.

## 5. Phasing

| Phase | Scope | Fixes |
|---|---|---|
| 1 (done) | Tables, append/list/search/delete/clear/draft API, composer reads and writes it, events | The loss the operator reported: ArrowUp and the draft survive restarts and upgrades |
| 2 | Backfill from past sessions, import of `sessionStorage` | History starts full |
| 3 | Reverse search, the History tab, delete and clear, palette entry | Finding and managing it |
| 4 | Retention setting in Settings, Swarm broadcasts recorded | Completeness |

## 6. Testing

- **srv:** append, collapse of duplicates, trim at the cap, paging, search,
  delete, clear; deletion with the agent; the migration on a store from
  before it.
- **Frontend:** ArrowUp across the page boundary; draft restore; a send while
  an append fails still sends; two panes of one agent stay in step.
- **End to end, isolated build:**
  1. Send a few messages and leave a draft.
  2. Quit and relaunch; then also upgrade with the simulated-release harness.
  3. The history and the draft are both there.
  4. `/clear`, then reopen the agent in a new pane: the history is still
     there.
- **Backfill:** a fixture session file with jekts, notes and tool results
  mixed in; only the typed messages come out.

## 7. Open questions

1. Is 10,000 entries per agent the right default, or should it be by age
   (for example two years)?
2. Should a Swarm broadcast also appear in a separate "broadcasts" history,
   or only in each recipient's?
3. Should the History tab search across agents by default, or start with the
   current agent?
