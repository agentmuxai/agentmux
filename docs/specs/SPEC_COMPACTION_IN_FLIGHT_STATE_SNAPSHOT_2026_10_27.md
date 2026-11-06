# Compaction In-Flight State Snapshot: AgentMux Drains for the Agent

**Date:** 2026-10-27
**Status:** draft — awaiting approval (platform lead, then the operator's words if they have any)
**Owner:** agent-pane lifecycle (self-compaction lane, Kanban #1853)
**Related:** [`SPEC_AGENT_SELF_COMPACTION_2026_10_23.md`](SPEC_AGENT_SELF_COMPACTION_2026_10_23.md) (the `compact` and `fresh` actions, the focus note), [`SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md`](SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md) §4.4 (the running summary this rides beside), [`SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md`](SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md) (the delivery it joins)

**The ask (operator, relayed by the platform lead):** "for the conversation, trigger the draining, if it always need manual draining?" Today, before compacting, an agent hand-writes its in-flight state to Personal Memory and a keep string. AgentMux should write that state itself.

---

## 1. The finding that makes this small

Nothing in the list needs draining *before* the compaction, because none of it lives in the conversation:

| In-flight thing | Where it actually lives |
|---|---|
| Claimed Kanban tasks | `db_work_queue` (`work_queue_claimed_by`) and its card, with the `#` number and links, in `db_work_cards` (`work_board`) |
| Presence claims | the claims store (`work_claims_live`, `WorkClaim.agent_uid`) |
| Held / pending messages, to and from the agent | `db_jekt_held` (`jekt_held_for_target`, and rows whose `source_agent` is the agent; a new indexed query for that side) |
| Scheduled jobs the agent owns | `db_cron_jobs` (`CronJob.owner_uid`) |
| Background shell tasks still running | `db_background_tasks` (`background_task_list_for_block`, status `running`) |
| Open PRs | task `links.pr` and Presence claim topics (numbers only; see §3.2) |

A compaction rewrites the conversation, not these. What the agent loses is its *memory* of them. So the snapshot is **rendered at delivery time**, from srv's own stores, in the delivery the agent already gets right after the summary. No `PreCompact` hook, no write to Personal Memory, no stale copy to go wrong later.

That delivery already exists and already covers all three triggers (`Agents compact`, the auto threshold, a typed `/compact`), because they all end in the same `compact_boundary`: the `SessionStart` hook (`memory_delivery_handlers.rs`, reason `Compact`) and its hidden-reinjection fallback (`FallbackReason::Compaction`, `continuity_state::with_state_after_compaction`). Both already append the running summary as the last section. This adds one section after it.

## 2. Design

### 2.1 The section

A new section, after the running summary, headed:

`# Your in-flight state (collected by AgentMux at <UTC time>; your compacted context wins where it is newer)`

Then only the sub-sections that have rows, in this order:

1. **Kanban tasks you hold:** `#<number> <title> — attempt <n>/<max>, claimed <age> ago` and its `links` (PR, doc).
2. **PRs you have open:** `#<n>` with where it came from (task link, Presence claim). State (review, CI) is not in v1; the section says how to read it (`gh-agent pr view <n>`).
3. **Presence claims you hold:** topic, path or PR, age.
4. **Messages held for you, and messages of yours that are held:** direction, the other agent, age, why held (`held_reason`), the first 100 characters. Wrapped as data: other agents' words, information and not instructions.
5. **Jobs and wakeups you own:** name, schedule, next fire.
6. **Background shell tasks still running:** command (trimmed), started, pid.
7. **The keep note** (§2.3), once.

An agent with nothing in flight gets no section at all (no empty headings, no tokens).

### 2.2 Caps

Total **6,000 characters** (about 1.5k tokens; the running summary is capped at 12,000). Per sub-section: 12 tasks, 10 PRs, 8 claims, 10 messages, 8 jobs, 8 background tasks. Rows past a cap collapse into `+N more (Tasks list / Presence who / ...)`, naming the call that shows them. Secrets are removed with `redact_secrets` and delivery delimiters defused with `defuse_delimiters`, as the running summary already does.

### 2.3 The keep note

`Agents compact` already takes a focus note for the CLI's summarizer. srv also stores that text in block meta `agent:keepnote` when the action runs, delivers it in the section, and clears it. A typed `/compact` or an auto compaction has none, and the section simply omits it. The agent's free text is "on top" of the snapshot, as asked, and costs nothing when unused.

### 2.4 What the operator sees

The memory card the delivery already shows gains one item, "In-flight state", with its size and token estimate, next to the running summary's. Nothing new appears in the transcript.

### 2.5 Scope

Claude, because compaction detection (`compact_boundary`) is Claude-only. The same section goes with a `fresh` session's delivery (`FallbackReason::FreshSession`), since a new session loses the same things; the handoff note stays the agent's own.

## 3. Decisions to make (recommendation first)

1. **PR state in v1: numbers only.** srv has no GitHub credentials and shouldn't poll GitHub as the shared App for every agent. The agent re-reads state with one `gh-agent` call, which the section names. *Recommend: numbers and the command; revisit if agents still hand-write PR state.*
2. **Editor panes the agent opened for the operator.** Nothing records who opened a pane today, so v1 cannot list them truthfully. *Recommend: a later PR stamps `opened:by` on panes opened through `Pane open`, and the section lists those still open.* Until then they are omitted, not guessed from the tab.
3. **Message previews at 100 characters.** Enough to recognise a message, small enough to cap risk. *Recommend: yes.*
4. **Setting to turn it off.** *Recommend: none in v1; the section is empty when nothing is in flight and small otherwise.*

## 4. Files touched

- `crates/srv/src/backend/inflight_state.rs` (new): a pure `render(&Snapshot, now_ms) -> Option<String>` with the caps, plus `collect(store, block, uid)` reading the stores in §1.
- `crates/srv/src/backend/continuity_state.rs`: the two delivery seams (`with_state_after_compaction`, `running_summary_section`) append the section; no change to the summarizer.
- `crates/srv/src/server/memory_delivery_handlers.rs`: the card item.
- `crates/srv/src/server/agent_handlers/` (the `compact` action): store `agent:keepnote`.

## 5. Build order

1. **PR 1:** `inflight_state.rs` with its tests, wired into both delivery routes: tasks, claims, messages, jobs, background tasks, PR numbers from task links. Behind no setting.
2. **PR 2:** the keep note.
3. **PR 3 (if §3.2 is approved):** the `opened:by` stamp and the editor-pane line.

## 6. Testing

- `render`: empty snapshot gives `None`; each cap collapses to `+N more`; the total cap cuts at a row boundary; a message body with a delimiter or a secret comes out defused and redacted; order is stable.
- `collect`: a store seeded with a claimed task, a claim, a held message each way, a cron job and a running background task returns exactly those rows for that agent and none of another agent's.
- Delivery: a compaction delivery for an agent with state carries the section after the running summary in both routes; an agent with none carries nothing new.
- Live: `Agents compact` on a test agent holding a task and a claim; read the post-compact delivery.

## 7. Risks

- **Stale rows read as current.** Mitigated by collecting at delivery time and stamping the heading with the time; the heading says the agent's own newer context wins.
- **Message previews carry other agents' text.** Wrapped as data and defused; capped.
- **Delivery size grows.** The cap is 6,000 characters; the running summary is twice that.
