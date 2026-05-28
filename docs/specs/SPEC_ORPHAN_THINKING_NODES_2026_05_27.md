# SPEC: Orphan in-progress nodes — cancel + collapse on session reopen

**Date:** 2026-05-27
**Author:** AgentA
**Status:** Design — bug fix for resumed-session UI inconsistency. No tracking discussion yet.

---

## The bug

Open an agent pane, the agent starts thinking (`thinking` stream events arrive, the parser produces a markdown node with `metadata.thinking = true`, the UI renders "Thinking..."). Close the conversation (close the pane, close the tab, close the app) **before the thought completes**. Re-open the same conversation later.

**Current behavior:** the node renders exactly as it did mid-thought — expanded, with the "Thinking..." spinner label, as if the agent is still actively thinking. It isn't. The session ended. The node is *orphaned*.

**Expected behavior:** the node should be marked as **canceled** (or *interrupted*, *abandoned* — pick the term in §5.2) and should render **collapsed** by default. The user can expand it to see the partial thought, but the UI should not falsely suggest active work.

The same bug applies to any node-level "in progress" state that depends on a closing stream event to complete:
- **Thinking markdown** (`metadata.thinking = true`) — no explicit "done" flag; UI infers active-thinking from `metadata.thinking` + presence of the live spinner.
- **Tool blocks** (`type: "tool"`, `status: "running"`) — explicit status field. If the session closes between `tool_call` and `tool_result`, the tool stays at `running` forever.
- **Pending message rows** in the composer's pending zone — already handled (`PendingMessageExpired` after 30s); not in scope here.

---

## Where it comes from

1. The agent emits stream events; the NDJSON file is append-only.
2. The stream parser folds deltas into nodes (`currentTextNode`, `currentThinkingNode`).
3. Tool nodes start with `status: "running"` on `tool_call`, transition to `success` / `failed` on `tool_result`.
4. Snapshots (`output.state.json`) are saved periodically and on certain events.
5. **If the session ends without the closing event**, the snapshot captures the dirty state: a thinking node mid-stream or a tool with `status="running"`.
6. On re-open, the snapshot loads as-is. The reducer has no signal to mark anything as terminated.

The cause is missing terminal-state cleanup at session close. The fix is to detect dirty nodes and rewrite their status at the appropriate moment.

---

## Design

### Where to fix it — three candidate trigger points

| Trigger | Fires when | Pros | Cons |
|---|---|---|---|
| **A. SessionEnd reducer** | The pane's stream ends (clean or dirty) | Single chokepoint; runs on every session boundary | Doesn't fire when the **app** is killed mid-thought before any save — the snapshot already on disk is dirty |
| **B. Snapshot load (SessionStart / HistoryLoaded)** | When a pane mounts and loads a snapshot | Catches the killed-mid-thought case; runs exactly once per mount | The first turn after mount immediately runs cleanup, which is fine but feels like "fixing on read" — slightly weirder mental model |
| **C. Snapshot save** | Right before writing `output.state.json` | Snapshot on disk is always clean | Snapshot save is owned by the host (Rust), and only the frontend knows about thinking-state semantics. Crossing that boundary widens scope |

**Recommended: A + B.** SessionEnd is the primary path (covers clean stream ends + most normal closes). SessionStart-load is the safety net for app-kill scenarios where SessionEnd never ran. Both call the same scrub helper. The reducer dispatches a new `ScrubOrphanedInProgress` command in both cases.

### What gets scrubbed

For each node in the document:

| Node type | Detection | Action |
|---|---|---|
| `markdown` with `metadata.thinking === true` AND no follow-up text/tool/message node in the same turn boundary | "Last in its turn AND turn is closed" | Add `metadata.canceled = true`; render shows "_Canceled — partial thought, click to expand_" + collapsed-by-default |
| `tool` with `status === "running"` | Status field directly | Set `status = "canceled"`; render shows ⏹ icon + "Canceled" label + collapsed by default |
| any node whose `streaming` flag is set (currently rare, but reserved) | flag check | Clear `streaming`; mark canceled |

Detection for "last thinking in its turn":
- A turn is bounded by `user_message` nodes (one user_message starts a turn; the next user_message ends the prior turn) and by `session_end` events.
- A thinking node is **orphaned** if it's not followed by any other node (text, tool, user_message, etc.) within the same turn AND the turn has ended.
- In practice: walk the document tail-first, find the latest `user_message`, examine nodes after it; for the most recent thinking node, if it's followed by nothing of higher precedence, it's orphaned.

For the simpler heuristic (which we can ship first): **any `metadata.thinking === true` node that is the document's last node when SessionEnd fires is orphaned.** This catches the common case. The "turn boundary" refinement is a follow-up if needed.

### UI rendering changes

In `MarkdownBlock` (or wherever the thinking node renders):

```ts
const isCanceled = props.node.metadata?.canceled === true;
const isThinking = props.node.metadata?.thinking === true;

// Label
const label = isCanceled
    ? "⏹ Canceled — partial thought"
    : (isThinking && isCurrentTurnActive
        ? "⏳ Thinking..."
        : "💭 Thought");

// Default-collapsed state
const defaultCollapsed = isCanceled || !isCurrentTurnActive;
```

`isCurrentTurnActive` reads from the pane state's `turnPhase.kind === "Streaming"` or similar. We already have this signal — it's what drives the agentActivity busyCount.

Tool blocks (`ToolBlock`): if `status === "canceled"`, render the ⏹ icon, label as "Canceled", collapsed by default. Existing status-icon machinery (success / failed / running) extends with one new case.

### Persistence

Adding `metadata.canceled` and the new `tool.status === "canceled"` value means:
- Existing snapshots (no canceled marker) → load fine; nodes without the marker render as today (which is the bug we're fixing). The fix only takes effect on **new** SessionEnd / SessionStart cycles.
- Loaded snapshots get scrubbed on next mount via the SessionStart hook; the document is rewritten with the canceled markers and re-saved on the next snapshot.

Migration: none required. The new fields are additive.

### Reducer action

```ts
// in agent-document/types.ts
{ type: "ScrubOrphanedInProgress"; at: number }
```

Reducer handler:
1. Walk `state.nodes`.
2. For each `markdown` node with `metadata.thinking === true` AND (heuristic): is the document's last in-progress thinking → set `metadata = { ...metadata, canceled: true, thinking: false }` (remove the thinking flag so the UI doesn't keep showing the spinner; preserve canceled for the label).
3. For each `tool` node with `status === "running"` → set `status: "canceled"`.
4. Lazy-clone: only allocate a new array if anything changed.
5. Emit a `scrubbed` event with counts for diagnostics.

Called from:
- `SessionEnd` reducer handler — at session boundary.
- `SessionStart` / `HistoryLoaded` reducer handler — after the snapshot is loaded.

---

## Out of scope

- Distinguishing **user-canceled** (Esc pressed) from **session-ended-mid-stream** (close, crash). The PR can label all of them as "canceled" — a follow-up can split into "interrupted" (user) vs "ended" (session). Current code already has `Interrupting` phase for user stops; we just don't propagate that into node metadata yet.
- Cleaning up the **streaming buffer** when the session ends. That's a render-side concern handled by the pane lifecycle.
- Recovering the *real* completion of a thought when the session is resumed via `--continue`. The agent might continue thinking on resume; if so, the canceled marker would be misleading. Decision: leave it canceled. If the new turn produces fresh thinking, it's a new node. Old canceled nodes stay as-is in history.

---

## Acceptance criteria

- [ ] When SessionEnd fires, any `markdown` node with `metadata.thinking === true` that is the most recent thinking in its turn gets `metadata.canceled = true` and `metadata.thinking = false`.
- [ ] When SessionEnd fires, any `tool` node with `status === "running"` gets `status: "canceled"`.
- [ ] When a snapshot is loaded (SessionStart with prior nodes present), the same scrub runs.
- [ ] Re-opening a previously-orphaned conversation renders the thinking node as "Canceled — partial thought", collapsed by default. Click-to-expand still works.
- [ ] Re-opening a previously-orphaned conversation with a running tool renders the tool with the ⏹ icon and a "Canceled" label, collapsed.
- [ ] Reducer test: scrub on SessionEnd transforms in-progress nodes; no-op when nothing is in progress.
- [ ] Reducer test: scrub is idempotent (running it twice doesn't double-modify).
- [ ] Snapshot round-trip: save → load → scrub → save → load → identical document (canceled marker preserved).
- [ ] Existing tests that assert `metadata.thinking === true` after a thinking event are unchanged (scrub is post-session, doesn't affect the parser's live behavior).

---

## Files likely to change

- `frontend/app/store/agent-document/types.ts` — add `ScrubOrphanedInProgress` command + the `canceled` metadata field + the `canceled` tool status.
- `frontend/app/store/agent-document/reducer.ts` — handler for `ScrubOrphanedInProgress`; call from `SessionEnd` and `SessionStart`/`HistoryLoaded` handlers.
- `frontend/app/view/agent/components/MarkdownBlock.tsx` (or equivalent thinking renderer) — render path for `canceled`.
- `frontend/app/view/agent/components/ToolBlock.tsx` — render `canceled` status (icon + label + default-collapsed).
- `frontend/app/view/agent/components/ToolOverlayLog.tsx` — the inline "Thinking..." label currently lives here; gate on turn-active.
- Tests: `agent-document/reducer.test.ts`, `MarkdownBlock.test.tsx`, `ToolBlock.test.tsx`.

Estimated scope: ~150 LOC + tests. Single PR.

---

## Open questions

- **Label wording.** "Canceled" vs "Interrupted" vs "Abandoned" vs "Stopped". Suggest "Canceled" for the user-facing label — matches the icon ⏹ and is the most neutral.
- **Should we preserve the in-progress timestamp?** Optional `metadata.canceledAt: number` for future audit / hover-tooltip. Probably yes; trivial to add.
- **Should the scrub also clear `pending` user messages?** Already handled by `PendingMessageExpired` (30s timeout). Out of scope for this spec.
- **Cross-talk with the new turn after resume.** If the user reopens a conversation with a canceled thought and the agent immediately resumes thinking (via `--continue`), do we want to *merge* the canceled thought with the new continuation? Decision: no. The canceled thought stays as historical artifact. New thinking nodes are new.
