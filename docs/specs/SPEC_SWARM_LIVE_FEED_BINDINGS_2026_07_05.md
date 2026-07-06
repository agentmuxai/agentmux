# SPEC: Swarm Live Feed — backend code bindings

**Date:** 2026-07-05
**Status:** Draft (PR-1 scope partially implemented in working tree, see §6)
**Owner:** camper
**Companion spec:** `SPEC_SWARM_LIVE_FEED_UI_2026_07_05.md`

## Goal

Give the frontend everything it needs to render a real-time swarm feed:
per-agent subagent activity (exists), **workflow run grouping + progress**
(new), and **pushed haiku summaries** of what each agent is doing (new).

## What exists (verified 2026-07-05)

- `backend/subagent_watcher.rs` — watches `<claude-config>/projects/**` for
  `agent-*.jsonl` (notify crate, recursive, 200ms debounce, incremental
  offset reads). Emits `subagent:spawned` / `subagent:activity` (with parsed
  `SubagentEventType::{Text,ToolUse,ToolResult,Progress}`) /
  `subagent:completed` directly on the EventBus as `eventrecv`.
  RPC: `subagent.ListActive`, `subagent.GetHistory`, `subagent.WatchAgent`.
  Auto-wired per reactive agent registration (`server/reactive.rs`).
- **No workflow tracking.** Old `db_workflow_*` tables are dead (Drone is the
  DAG successor). Workflow tool runs exist only as files:
  `projects/<ws>/<session>/subagents/workflows/wf_<runid>/agent-<n>.jsonl`
  plus a `journal.jsonl` per run. Verified journal format (live run):
  one JSON object per line, `{"type":"started","key","agentId"}` and
  `{"type":"result","key","agentId","result":{...}}`.
- Haiku summarizer exists but is pull-only: `server/app_api.rs`
  `session:activity_summary` RPC — tail-reads last 32KB of the block's
  `output` FileStore, prompts `claude-haiku-4-5-20251001` (15s timeout),
  returns a ≤N-word summary. Frontend stores it in block meta `term:activity`.
- Event transport: WPS `Broker` → `EventBusBridge` → two-lane WebSocket.
  The subagent watcher bypasses the Broker and broadcasts on EventBus
  directly.

## Design

### 1. Workflow grouping in subagent_watcher

- `SubagentInfo.workflow_id: Option<String>` — parsed from the JSONL path:
  a `subagents/workflows/<id>/` segment ⇒ `Some(id)`. The recursive watch
  already delivers these files; no new watcher.
- Session-id derivation walks ancestors to the dir containing `subagents/`
  (workflow members are nested two levels deeper than direct subagents, so
  the old fixed-depth `parent().parent()` derivation breaks on them).
- Watch `journal.jsonl` inside each `wf_*` dir. Tally `started` / `result`
  records incrementally (offset-based, same pattern as agent files):
  `agents_total = max(journal started, member files seen)`,
  `agents_done = max(journal results, members completed)` — either source
  can lag the other.
- Startup scan extended: `subagents/` may sit directly under the project dir
  or one level deeper under a session dir; also descend into
  `subagents/workflows/<wf>/` for member files + journal.

### 2. New/changed events (EventBus `eventrecv`, same envelope as today)

- `workflow:updated` — `{workflowId, parentAgent, parentBlockId, sessionId,
  agentsTotal, agentsDone, status}`; emitted on member spawn/completion and
  journal changes.
- `subagent:spawned` / `subagent:activity` / `subagent:completed` payloads
  gain `workflowId` (null for direct subagents).
- `status` semantics: `running` until counts-complete AND 60s quiet. There
  is no timer — the flip happens lazily at the next event or
  `ListWorkflows` read. Frontend treats it as advisory (it can render
  "n/n done" from counts regardless).

### 3. New RPC

- `subagent.ListWorkflows` → `Vec<WorkflowInfo>` sorted by recency, for
  backfill when the swarm pane opens.

```rust
pub struct WorkflowInfo {
    pub workflow_id: String,
    pub parent_agent: String,
    pub parent_block_id: String,
    pub session_id: String,
    pub agents_total: usize,
    pub agents_done: usize,
    pub status: WorkflowStatus, // running | completed
    pub last_event_at: u64,
}
```

### 4. Pushed haiku summaries (PR-2, not yet implemented)

Reuse the existing `session:activity_summary` machinery, but push:

- srv task: for every registered reactive agent whose controller status is
  `working`, run the existing digest+haiku path every 20s. Skip when the
  output FileStore offset is unchanged since the last summary (idle agents
  burn zero tokens).
- Publish `agent:summary` WaveEvent `{agentId, blockId, summary, ts}` on
  scope `block:<id>` via the Broker (normal WPS path, not the EventBus
  bypass).
- Guardrails: one in-flight haiku call per agent, global concurrency 2,
  15s timeout (existing), word_target 12.
- Out of scope: changing the summarizer model (currently pinned
  `claude-haiku-4-5-20251001`; verify against the live model list if
  touched).

### 5. Testing

- Unit: path parsing (`parse_workflow_id`, `derive_session_id`), journal
  incremental counting (`read_journal_counts`).
- Integration: fake `wf_*/agent-1.jsonl` + `journal.jsonl` tree → assert
  `workflow:updated` counters and `ListWorkflows` output.
- Manual: run a real Workflow tool invocation, open swarm pane, watch
  grouping + counters live.

### 6. Implementation status (working tree, not committed)

PR-1 scope is implemented and passing in the local working tree:
- `subagent_watcher.rs`: `workflow_id` field, path helpers, journal watch +
  incremental counts, `WorkflowInfo`/`WorkflowStatus`, workflow aggregate
  state + `workflow:updated` broadcast, extended startup scan, 7 unit tests
  (all green; `cargo check` clean).
- `service.rs`: `subagent.ListWorkflows` dispatch.

Remaining for PR-1: integration test. PR-2 (pushed summaries) not started.

## Phases / PRs

1. **PR-1 (srv):** workflow grouping + `workflow:updated` + `ListWorkflows`.
2. **PR-2 (srv):** pushed `agent:summary` (periodic haiku, offset-gated).

Each PR reviewed by reagent, merged only on approval.

## Open questions

1. Haiku push cadence 20s ok? (~3 haiku calls/min/active agent, tiny prompt.)
   Alternative: only on turn-phase transitions.
2. Should workflow nodes carry the phase titles from the script's
   `meta.phases` (requires locating + parsing the script file) or counts
   only? Counts-only for v1.
