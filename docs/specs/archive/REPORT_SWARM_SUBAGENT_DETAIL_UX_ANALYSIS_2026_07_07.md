# Report: Subagent detail view — inline expand, no timestamps, Haiku-generated names

**Status:** Implemented (branch `agentx/subagent-detail-ux`, stacked on
`agentx/swarm-workflow-grouping` / PR #2018).
**Author:** AgentX
**Date:** 2026-07-07
**Triggered by:** user usability feedback after the workflow-grouping fix
(Finding 4 of `REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md`): "when
clicking the subagent ID, we need it to expand with the content .. we
dont need a timestamp for every line, remove that .. also, can we get the
ambient haiku to efficiently rename the line to a 5 word concise. when
expanding, you can [see] the subagent's name and other meta data, and
below the log, no extra spaces or timestamps."

## Requirements, restated precisely

1. Clicking a subagent row in the Swarm tree expands **inline** (same
   pattern as the new `WorkflowGroupRow`) instead of opening a new split
   pane.
2. Per-event timestamps are removed from the log.
3. Each subagent gets a concise, Haiku-generated ~5-word name instead of
   its raw slug/hash — used both as the collapsed row's label and the
   expanded header's title.
4. Expanded layout: name + metadata at the top, log content below, no
   timestamps, tightened spacing (no "extra spaces").

## Current state (grounded in the code)

**Today's flow**: `SubagentRow` (`swarm-view.tsx:298-320`) `onClick` calls
`openSubagentPane` (`subagent-pane-manager.ts:28-87`), which creates a
**new split-pane block** (`view: "subagent"`) running the standalone
`SubagentView`/`SubagentViewModel` (`frontend/app/view/subagent/`). That
component is a real `ViewModel` — constructor takes `(blockId, nodeModel)`,
reads `subagent:id` off **block meta** (`subagent-model.ts:77-79`), and
ties its 3 event subscriptions + `loadHistory()`
(`subagent.GetHistory`/`GetInfo` RPCs) to pane lifecycle via `dispose()`.

**Today's header** (`subagent-view.tsx:88-117`): icon, `slug || agent_id`,
7-char id, status badge, `event_count` + "events", `elapsed()` string,
model. **Today's event rows** (`SubagentEventItem`, L153-166): a
`.subagent-event-time` timestamp span, then `EventContent` — switches on
event type (`text`/`result` → `<pre>`; `tool_use`/`tool_result` →
collapsible headers; `progress` → spinner + text).

**Raw display names are ugly by design, not by bug**: `slug` comes from
the subagent's own JSONL first line's `"slug"` field
(`subagent_watcher.rs:1004-1012`); if the CLI didn't write one, it falls
back to the raw `agentId` hash (`:1013-1027`) — confirmed live: Lark's 15
loose subagents (spread across June 22-30, genuinely unrelated individual
Task-tool calls, not a workflow) all show as bare hashes for exactly this
reason.

## Proposed design

### 1. Inline expand (replaces the new-pane navigation)

Reuse `WorkflowGroupRow`'s pattern exactly (`swarm-view.tsx:266-294`):
local `const [expanded, setExpanded] = createSignal(false)` on
`SubagentRow`, toggled on click, `<Show when={expanded()}>` revealing the
detail content beneath the row instead of calling `openSubagentPane`.

The data-fetching logic in `SubagentViewModel` is reusable but not as-is —
it's a `ViewModel` shell (needs `blockId`, implements `dispose()` tied to
pane unmount, reads `subagentId` from block meta instead of a prop).
Extract the substance (3 `waveEventSubscribe` handlers +
`loadHistory()`/`subagent.GetHistory`+`GetInfo`) into a plain function
taking `subagentId: string` directly — e.g.
`useSubagentDetail(subagentId): { events, info, status }` — callable from
a `SubagentRow`'s local scope, cleaned up via Solid's `onCleanup` instead
of a `dispose()` method.

**Open question — what happens to the old pane path?** Once `SubagentRow`
expands inline, `openSubagentPane`/`subagent-pane-manager.ts`/the
standalone `SubagentView` pane type have no remaining caller (per
`CLAUDE.md`'s own "Not widgets" table, clicking a subagent in Swarm was
already the *only* documented entry point). Recommend retiring them in the
same change rather than leaving now-dead code — but flagging this
explicitly since it's a deletion, not purely additive.

### 2. Remove per-line timestamps, tighten spacing

Delete `.subagent-event-time` (`subagent-view.tsx:156` /
`subagent-view.scss:142-149`) from the row template. Drop the `.subagent-event`
flex-row wrapper's now-unneeded time column and its `min-width: 56px`
reservation; reduce `.subagent-event` padding
(`subagent-view.scss:131-140`) and `.subagent-events` container padding
(`:113-117`) to remove the "extra spaces" called out.

### 3. Expanded layout: name + metadata header, log below

Matches today's structural intent already (`subagent-view.tsx:88-133`
already puts a header above the scrollable event list) — just needs the
timestamp/spacing trims above, plus swapping the displayed name for the
Haiku-generated one (§4) and confirming which metadata fields stay: slug/
name, short id, status, event count, model — parent agent + session id
are already fetched (`ActiveSubagent`/`SubagentInfo` both carry them) and
could be added to the header now that there's room without a full pane's
worth of chrome.

### 4. Haiku-generated concise name

**Trigger: on-demand, cached — not eager for every backfilled subagent.**
This is the load-bearing decision. A single agent's *current* session can
carry hundreds of subagents (confirmed live: Lark has 227 total — 15 loose
+ 2 workflow runs of 106 each). Naming all of them eagerly on every pane
reopen would mean up to 227 Haiku CLI spawns per reopen for one agent —
directly reintroducing the cost/latency problem
`REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md` Finding 3 (ambient
concurrency cap) was written to prevent, just from a new call site. Naming
**lazily, on first expand, then caching the result** means a Haiku call
only ever fires for a subagent a human actually looked at — matches "get
the ambient haiku to *efficiently* rename" directly.

**Source content: the subagent's own initial task prompt**, not a
transcript summary. The subagent's first JSONL line already carries the
task description handed to it (the same content `read_recent_activity_digest`-style
extraction reads for the existing ambient summaries) — cheap, available
immediately even for a still-running subagent (no need to wait for
output), and a closer match to "what was this subagent *for*" than a
summary of what it produced.

**Backend**:
- New `AMBIENT_PURPOSE_SUBAGENT_NAME` purpose tag, admitted through the
  existing `crate::ambient::gateway()` — `AmbientCallKey{entity_id:
  agent_id, purpose: "subagent_name"}`. Since a name is generated once and
  never needs regenerating, `generation` can be a constant (e.g. `1`) —
  admission is inherently one-shot per subagent, no supersede-on-retry
  complexity to design for.
- Reuses `invoke_ambient_haiku_call` unchanged (same CLI-spawn/timeout/
  cancel-race machinery `session.rs` already has) — new prompt: "Give a
  concise ~5-word name for this task. No punctuation, no quotes." + the
  subagent's first-line content.
- Route through the same `pull_call_semaphore`-style cap as the two
  existing pull RPCs (`session.rs`'s `MAX_CONCURRENT_PULL_CALLS`) — a user
  rapidly expanding several subagent rows in a row shouldn't spawn
  unbounded concurrent Haiku CLIs either.
- New field `SubagentInfo.display_name: Option<String>`, set once the
  Haiku call resolves; broadcast via a new `subagent:named { agentId,
  displayName }` WS event so every client watching that session (not just
  the one that triggered the naming) picks up the result — the collapsed
  row's label updates too, not only the expanded view, and a second
  expand-click by anyone doesn't refire the call (cached on the backend
  struct, `GetInfo`/`ListActive` return it going forward).
- New RPC: `subagent.GenerateName(agent_id)` (or piggyback on `GetHistory`/
  `GetInfo`'s existing call site if simpler) — fired once, the first time
  a given subagent is expanded client-side.

**Frontend**:
- `ActiveSubagent`/`SubagentInfo`-derived types gain `display_name: string
  | null`. Row label preference: `display_name ?? slug ?? agent_id.slice(0,7)`.
- `swarm-model.ts` subscribes to the new `subagent:named` event and
  patches the matching entry in `subagentsAtom` in place (same pattern
  already used for `subagent:spawned`/`subagent:completed`).
- The inline-expand handler (§1) fires the naming RPC once, only if
  `display_name` isn't already set.

## Implementation plan (file-by-file, once approved)

**Backend**:
- `agentmux-srv/src/backend/subagent_watcher.rs` — add `display_name`
  field to `SubagentInfo`; new method to set it + broadcast
  `subagent:named`.
- `agentmux-srv/src/server/app_api/session.rs` or a new
  `subagent_naming.rs` — new ambient-purpose constant, `pull_call_semaphore`-gated
  handler calling `invoke_ambient_haiku_call` with the new prompt.
- `agentmux-srv/src/server/service/misc.rs` — register
  `subagent.GenerateName`.

**Frontend**:
- `frontend/app/view/swarm/swarm-model.ts` — `display_name` field;
  `subagent:named` subscription + in-place patch.
- `frontend/app/view/swarm/swarm-view.tsx` — `SubagentRow` gains local
  `expanded` signal (mirrors `WorkflowGroupRow`); on first expand, calls
  the new naming RPC if unnamed, and `useSubagentDetail(agent_id)` for the
  event log; renders the trimmed header + no-timestamp log inline.
- New `frontend/app/view/swarm/useSubagentDetail.ts` (or inline in
  `swarm-view.tsx` if small enough) — extracted data-fetching logic from
  `SubagentViewModel`, taking `subagentId` directly.
- `frontend/app/view/swarm/swarm-view.scss` — trimmed spacing, no
  timestamp column, name/metadata header + log-body layout for the
  expanded state.
- Retire (pending confirmation, see §1's open question):
  `frontend/app/view/subagent/subagent-view.tsx`,
  `frontend/app/view/subagent/subagent-model.ts`,
  `frontend/app/store/subagent-pane-manager.ts`.

## Resolution of the open questions (implemented per "use best judgment")

1. **Standalone `SubagentView` pane: kept, not retired.** Discovered during
   implementation that `agent-view.tsx`'s `handleSubagentClick` (a subagent
   link rendered *inline in an agent's own transcript*, e.g. a Task-tool
   invocation) also calls `openSubagentPane` — a second, independent entry
   point beyond the Swarm tree that `CLAUDE.md`'s "Not widgets" table didn't
   account for. Retiring the pane would have broken that click-through, so
   `subagent-view.tsx` / `subagent-model.ts` / `subagent-pane-manager.ts`
   are untouched; only the Swarm tree's own row click now expands inline
   instead of calling `openSubagentPane`.
2. **Naming trigger: on-demand at first expand**, as recommended — implemented
   exactly as designed (§4).
3. **Header metadata**: name, short id, event count, model (when present),
   parent agent. Elapsed-time and a separate status badge were dropped from
   the inline header (the row's own `AgentStatusChip` already shows status
   one line above; repeating it in the expanded panel added noise without
   new information).

## Implementation notes (post-hoc)

- **Row remount instability discovered and fixed.** `SwarmViewModel.buildTree()`
  reconstructs fresh `WorkflowGroup`/`ActiveSubagent`-wrapping objects on
  every recompute (`groupSubagentsByWorkflow`'s `.map()`), and `tree()`
  recomputes on every `agentStatusesAtom` tick (i.e. very often during an
  active turn — every `controllerstatus` event). SolidJS's `<For>` diffs
  list items by reference, so this was silently remounting every row
  (including the just-shipped `WorkflowGroupRow`) far more often than any
  actual data change — collapsing any row a user had expanded within
  roughly one status tick. Fixed by lifting expand state and per-subagent
  detail fetch/subscribe state off the row components and onto
  `SwarmViewModel` (`expandedIdsAtom` keyed by `workflowId`/`agent_id`,
  `getSubagentDetail` cache keyed by `agent_id`), plus
  `mergeSubagentsPreservingIdentity` so `loadSubagents()`'s full-list
  reload reuses old object references for unchanged subagents instead of
  handing `<For>` a fresh object per entry every time.
