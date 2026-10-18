# SPEC: agent history in Warden — the History section

**Status:** active — phase 1 (§4: Agents and Storage, the status bar link, the history tab fix) ships in PR #5116; phases 2 and 3 (§5) are not started.
**Date:** 2026-10-18
**Author:** Agent5@narko, at the repo owner's direction
**Decided by the repo owner (2026-10-18):** history lives in Warden, as a section beside Audit, not as a page of its own ("build it into warden").
**Builds on:** `SPEC_WARDEN_WIDGET_2026-05-25.md` (Warden), `SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md` (the per-agent Agent History view), `SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md` (the history tab), `SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md` (the agent's record), and the status bar history total (#5081, `backend/history_usage.rs`).

## 1. Why

AgentMux keeps agent history forever: rigorous tracking and auditing of agent behaviour is part of what it is for, and visibility features are built on it. Until now there was no place to see that record as a whole:

- the per-agent Agent History tab shows one agent's conversation, and only from that agent's own pane;
- the status bar total says how much is kept, in a read-only popover;
- an agent that was deleted keeps its history, but nothing listed it.

Warden is already the oversight surface: Host, LAN and Internet say what agents may reach, Audit what they did. History is the full record of what agents did, so it belongs beside Audit rather than in Memory (which holds what agents are given, and has edit and revert controls history must never have).

## 2. Where it lives

| Surface | What it is for |
|---|---|
| **Warden → History** | the whole record: every agent with history, its sessions, where it is kept, how long |
| An agent's **Agent History** tab | reading one agent's conversation (unchanged); Warden opens it |
| The status bar total | how much is kept; its popover's **Open in Warden** goes to History |

Opening Warden at a section is `openWarden(section)` (`frontend/app/view/warden/open-warden.ts`), the same shape as `openConnectors` and `openMemory`.

## 3. The section

A segmented control picks one of two views.

### 3.1 Agents

Every agent whose history the global transcript store (`~/.agentmux/shared/agents/transcripts/filestore.db`) holds, most recently active first:

- **forks under the agent they were forked from** (a fork is a definition with a branch label, `forkagentdefinition`; an agent made from a template is not a fork of it);
- **deleted agents included**, marked as such: their history is kept;
- columns: agent (name, short id, "fork of …"), provider, history size (current plus archived), archived conversations, last active;
- a filter by name, id, provider or parent (it lists the matches flat);
- **Show sessions** reads that agent's sessions from its record (`AgentRecords`): id, first and last time, size, current or archived, working directory;
- **Open history** opens the agent's Agent History tab in the Warden pane.

Data: `history.agents` (`backend/history_registry.rs`) reads the store's file records only, never the transcripts, so it stays fast however large they are. `history.agent_sessions` reads one agent's record on demand; the first read of a zone scans it, later reads only what was appended. Both are window APIs and refuse agent connections. The section loads when first shown and on **Refresh**; it doesn't poll.

### 3.2 Storage

The status bar total's numbers (`history.usage`), larger: the total, one row per store with its share, size and file count, when it was measured, and **Measure again**. Below it, the retention policy in words: history is kept forever; Claude Code's own cleanup is set to 36,500 days unless its config already sets another value; archived conversations and archived pane transcripts are never pruned; the record of messages between agents is written to disk and kept.

## 4. Phase 1 (this PR)

1. Warden's fifth section, `history` (`warden-model.ts`, `warden-view.tsx`, `frontend/app/view/warden-history/`).
2. `history.agents` and `history.agent_sessions` (`server/app_api/history_agents.rs`, `backend/history_registry.rs`, `backend/history/agent_sessions.rs`).
3. The status bar popover's **Open in Warden**.
4. **A history tab opened from a pane that isn't the agent's own reads the agent's history.** `openOrFocusHistoryTab` read the transcript through the pane it joined; the server finds the agent's shared transcript from that block's `agentId`, which Warden's block (and the agent picker's) doesn't have, so the tab showed the pane's own empty output. It now reads through the anchor only when the anchor is one of the agent's own panes, and otherwise through the history tab's own block, which carries `agentId`. Callers that aren't the agent's pane pass its provider and name for the tab's display fields. This also fixes the agent picker's **View History**.

## 5. Later phases

- **Phase 2, the per-agent view's last phase** (`SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md` §4.5): an archives section in the Agent History view, jump to date, and a session count on the link row. With archives now kept in the global store (#5100), the view can list them.
- **Phase 3, with the history store** (`REPORT_AGENT_HISTORY_STORE_DEDUP_COMPRESSION_2026_10_17.md`, not in the repo): Storage shows duplication and what compression saves; an **Integrity** view ("verify history", the last signed checkpoint, what was redacted and when); Audit reads its events from the same store so they survive a restart; deletion, when the user chooses it, is confirmed and recorded.

## 6. Out of scope

Deleting history from this section: the policy is to keep everything, and any deletion must be deliberate, confirmed and recorded (phase 3). Searching message text across all agents (agents have `SearchHistory`; a window search is its own design).
