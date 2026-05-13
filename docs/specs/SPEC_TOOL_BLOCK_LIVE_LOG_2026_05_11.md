# Tool block: live log popout + bottom action bar

**Status:** Proposed
**Owner:** AgentA
**Date:** 2026-05-11
**Driving observation:** *"the hover tool thing currently shows an array of buttons, that doesn't work. We want it to show the actual running result of the command (many have `&&` so we want the log as it is running, and the result once complete). We want reducer arch for this too. The buttons for branching and opening up other stuff, let's move them to the bottom of the log popout."*

---

## 1. Today's behavior

Two surfaces overlap on the same tool row and neither does what the user expects:

| Surface | File | Content | State |
|---|---|---|---|
| **`NodeHoverStrip`** floats over every document row | `frontend/app/view/agent/components/NodeHoverStrip.tsx` | Buttons: bookmark, expand/collapse, open-in-pane, open-in-window, new-agent-from-here | Three of the five are **stubs** logging `console.warn` ("not yet implemented") |
| **`ToolBlock` portal overlay** (`agent-tool-content--portal`) | `frontend/app/view/agent/components/ToolBlock.tsx:336-348` | Per-kind tool content: `BashOutputViewer`, `DiffViewer`, `CompactResult`, etc. | Only renders when pinned (click). Hover doesn't show this content. |

For Bash specifically:
- The stream parser delivers a single `tool_result` event when the command completes (`frontend/app/view/agent/stream-parser.ts:207`).
- `BashResult = { stdout, stderr, exitCode }` lands whole on `ToolNode.result`.
- `BashOutputViewer` renders it as a static `<pre>` once.

Nothing streams. A `cmd1 && cmd2 && cmd3` command leaves the user staring at a spinner with no feedback until every step finishes.

---

## 2. Goal

A single, predictable hover/pin overlay per tool row that:

1. **Shows the tool's actual output, streaming as it runs.** For Bash, append stdout/stderr lines to the visible log as they arrive. For Edit, stream the diff hunks. For Read, stream content if large.
2. **Surfaces the final result and exit status when complete** without changing the overlay's identity (no flicker, no re-mount).
3. **Hosts the branching/contextual actions at the bottom of the overlay** — not in a floating strip — so the user sees the output first and the available actions second. Buttons being "down there" matches every native log viewer (terminal, VSCode output, browser DevTools).
4. **Flows through the reducer family** like the rest of the document state, so the live log is a first-class cell — auditable, replayable, durable across reconnect, not a side store.

---

## 3. Architecture

### 3.1 Two reducer cells per tool

Today the reducer has one cell per tool: `ToolNode.result` (whole-result on completion). We add:

```ts
interface ToolStreamingLog {
    // Append-only log buffer. Lines arrive as deltas.
    chunks: ReadonlyArray<{
        kind: "stdout" | "stderr" | "system" | "diff-hunk";
        content: string;
        timestamp: number;
    }>;
    // True until tool_result lands. Distinct from ToolNode.status
    // because status flips at the network level; chunks may still
    // be in-flight on the reducer queue when status flips.
    open: boolean;
}

// New field on ToolNode
interface ToolNode {
    // ...existing fields...
    log?: ToolStreamingLog;     // live streaming, undefined for tools without partials
    result?: ToolResult;        // existing — whole-result on completion (kept for back-compat)
}
```

`log` is the live, streaming view. `result` is the final snapshot. Both are populated by the time `status === "success"` or `"failed"`.

### 3.2 New stream events

Today the stream parser handles `tool_call` and `tool_result`. Add:

```ts
type ToolChunkEvent = {
    type: "tool_chunk";
    id: string;                                  // tool call id, matches tool_call
    kind: "stdout" | "stderr" | "system" | "diff-hunk";
    content: string;                             // append, may be partial line
    timestamp?: number;                          // optional; defaults to receive time
};
```

The stream parser's `eventToNode` (`stream-parser.ts:118`) gets a new arm:

```ts
case "tool_chunk":
    return this.toolChunkToNode(event as ToolChunkEvent);
```

`toolChunkToNode` mutates the matching pending tool's `log.chunks` and returns the updated ToolNode (same id). The agent-document-store's `StreamFlush` reducer command handles upserts by id, so the existing reconciliation path works without changes.

### 3.3 Reducer changes

New command (`frontend/app/store/agent-document-store.ts`):

```ts
type Command =
    | { type: "SessionStart"; … }
    | { type: "HistoryLoaded"; … }
    | { type: "StreamFlush"; newNodes: DocumentNode[] }
    | { type: "ToolChunkAppend"; toolId: string; chunk: ToolStreamingLog["chunks"][number] }
    | { type: "StreamTruncate"; … }
    | { type: "UserClear" };
```

`ToolChunkAppend` finds the tool node by id, appends to `log.chunks`, leaves all other state untouched. Idempotent: chunks are append-only with timestamps, so re-applying a command set during replay produces the same final state.

On `tool_result`, an existing `StreamFlush` carries the final ToolNode with `log.open = false` + `result` populated. No new command for completion.

Why a separate command instead of `StreamFlush` carrying chunks: chunks fire at high frequency (potentially per-line). Routing each through `StreamFlush` would force the whole node array through `dispatch` and re-trigger every memoization that watches the document length. `ToolChunkAppend` mutates one node in-place (in reducer terms — produces a new state with one node changed) and downstream readers can subscribe to `getToolLog(toolId)` selectors that ignore unrelated changes.

### 3.4 UI: one overlay, log on top, actions on bottom

```
┌────────────────────────────────────────────────┐
│  📌 Bash    cmd1 && cmd2          (3.4s) ✓      │  header (sticky)
├────────────────────────────────────────────────┤
│ $ npm install                                  │
│ added 421 packages in 8s                       │  log body
│ $ npm test                                     │  (scrollable, auto-scroll
│ PASS test/foo.test.ts                          │   to bottom when at bottom)
│ Tests: 12 passed, 12 total                     │
│ ┃ (spinner while running)                      │
│                                                │
├────────────────────────────────────────────────┤
│  Open in pane │ New window │ New agent here    │  action bar (fixed)
└────────────────────────────────────────────────┘
```

**New component:** `frontend/app/view/agent/components/ToolBlockOverlay.tsx` replaces the inline `renderToolContent()` switch and the embedded action button in `ToolBlock.tsx`. Shape:

```tsx
<div class="agent-tool-overlay">
    <ToolOverlayHeader node={node} />
    <ToolOverlayLog
        log={node.log}                  // live; auto-scrolls
        fallback={<ToolOverlayResult result={node.result} />}
    />
    <ToolOverlayActions
        node={node}
        onBookmark={...}
        onOpenInPane={...}
        onOpenInWindow={...}
        onNewAgentHere={...}
    />
</div>
```

**`ToolOverlayLog`** is a virtualized line list (chunks can be 10,000+ for a long build). For Bash it renders monospaced lines; for diff-hunk kinds it routes to `DiffViewer` per hunk. Auto-scroll-to-bottom while the user is at the bottom; sticks when the user scrolls up (same anchor logic as the agent document).

**`ToolOverlayActions`** is the new home for branching actions. **Stub functions removed** — each button gets a real implementation as part of this PR (see §4).

### 3.5 NodeHoverStrip — slimmed, not removed

The hover strip stays but only carries actions that affect the document row itself (bookmark, expand/collapse). Branching actions move into the overlay. The strip's "open in new pane / window" / "new agent" buttons are deleted entirely; users find them in the overlay.

Implication: hover on a row still shows the strip with just **two** buttons (bookmark, expand). Click/pin opens the overlay with the log + bottom action bar. This avoids cluttering every document row with stub buttons; branching actions are explicitly a tool concern.

### 3.6 Trigger model — hover OR pin?

User language is ambiguous between hover and pin. Recommend:

- **Hover:** show the overlay (log + actions) AFTER a 200ms intent-debounce. Overlay is read-only.
- **Pin (click):** persistent overlay, ignore mouseleave. Becomes the "stuck open" version.
- **Click in overlay (anywhere except actions):** maintains hover state until the user moves the mouse outside the overlay AND the row.

This matches VSCode hover-cards-with-buttons. Avoids the menu-chase problem because intent-debounce gives the user a moment to commit, and clicking the overlay pins it.

---

## 4. Implementing the stub branching actions

All three actions in `DocumentRow.onOpenInNewPane / onOpenInNewWindow / onNewAgentFromHere` are currently `console.warn`. As part of this PR, implement each:

- **Open in pane:** call `createBlock({ meta: { view: "tool-detail", "tool:id": node.id } })`. Pane renders the same `ToolBlockOverlay` content stretched to the pane. The tool's log is shared (same reducer node), so additions stream into both.
- **Open in window:** `getApi().openNewWindow({ initialTab: { layout: { tool-detail } } })`. Window opens with the tool overlay as its primary content.
- **New agent here:** spawn a sibling agent in a new pane, pre-seeded with the same prompt context up to and including this tool. Requires a backend RPC (`SpawnAgentFromState`). Out of scope as code in this PR — file a follow-up; show the button disabled with a tooltip *"Coming soon — backend support needed"*.

---

## 5. Edge cases

- **Tool fails mid-stream** (`tool_result` arrives with status: `failed`). Overlay header flips status icon to ✗; log stays scrolled to the bottom of what was captured. No state loss — `log.chunks` holds everything received.
- **Reconnect during running tool.** History replay reconstructs `log.chunks` from the journal. `ToolChunkAppend` is timestamp-keyed; deduplication on replay via timestamp + content hash.
- **Tool produces no output but takes time** (e.g., a SQL migration). `log.chunks` stays empty; the spinner + duration tick in the header keep the user informed. When result arrives, the result body (e.g., "Migration applied") replaces the spinner.
- **Very large log.** Virtualize the log line list. Truncate at 50k lines with a "log truncated" indicator. The full log lives on disk; the action bar can offer "Export full log" later (out of scope).
- **Mixed stdout + stderr ordering.** The reducer preserves chunk arrival order; rendering interleaves both kinds (stderr in dim red) so the user sees the actual emission interleaving, not artificially segregated streams.
- **Pinned overlay during streaming.** Pinned state already exists; live log streams in continuously. Auto-scroll respects user scroll position — if they've scrolled up to read, no jump.
- **Diff-hunk streaming for Edit tools.** Each `tool_chunk` with `kind: "diff-hunk"` adds one hunk to the rendered diff. Same virtualization model — large diffs scroll.
- **Reducer audit ring.** Each `ToolChunkAppend` becomes one entry in the dispatch audit ring (Slice #9 Phase 5). High-volume but bounded by command rate; the ring evicts old entries normally.

---

## 6. Trade-offs vs. simpler alternatives

- **Atom-based partial log (no reducer cell).** Less infrastructure but doesn't survive reconnect, can't be replayed, doesn't show up in the diag panel. The reducer-stack direction is explicit: every cell with cross-cutting consequences goes through the reducer. Live log qualifies.
- **`StreamFlush` carries chunks.** Tempting because it reuses the existing path. But chunk frequency would force every node-id-set-watching memo to recompute on every line. The dedicated command is the right granularity.
- **Hover-only, no pin.** Currently the tool overlay only renders on pin. Adding hover (debounced) trades a tiny render cost for a much faster path to seeing the log. Worth it.
- **Keep branching actions in NodeHoverStrip.** Slot-cleaner architecturally (row-level chrome stays consistent), but the user explicitly wants them in the popout. Doing what the user asks.

---

## 7. Test plan

- [ ] Bash `sleep 2 && echo hi && sleep 2 && echo bye`: while running, overlay shows "hi" after ~2s, "bye" after ~4s; result with exit code 0 appears at end. No flicker, no remount.
- [ ] Hovering opens the overlay after ~200ms intent debounce; mouseleave closes it.
- [ ] Click the overlay → pinned, mouseleave keeps it open until the user clicks outside.
- [ ] Action bar appears at the bottom of the overlay, stays fixed while log scrolls.
- [ ] "Open in pane" creates a new pane showing the same overlay content; chunks continue to stream into both.
- [ ] Tool that produces no output (SQL migration) shows spinner + duration, no empty log mess.
- [ ] Edit tool: each `tool_chunk` with `kind: "diff-hunk"` adds a hunk to the diff viewer incrementally.
- [ ] Disconnect mid-stream → reconnect → log fills with previously-received chunks, no duplicates.
- [ ] Unit tests for `toolChunkToNode`, `ToolChunkAppend` reducer command, append + dedup.
- [ ] Perf: streaming 1000 lines / sec into an open overlay doesn't drop frames in the agent pane document virtualization.

---

## 8. Out of scope (file follow-ups)

- **Backend support for `SpawnAgentFromState`.** Required for "New agent here" to actually work. File as a follow-up; ship the button disabled in this PR.
- **Per-tool log persistence to disk + replay UI.** Logs over 50k lines get truncated in memory; full log lives in the existing tool-execution log files. Adding "Export full log" is a future improvement.
- **Stream events from non-Claude providers.** Codex / OpenAI / Gemini providers vary in whether they emit partial tool output. Adapter work to bridge their formats to `tool_chunk` is per-provider; ship Claude-only in this PR, file follow-ups for the others.
- **Diff-hunk streaming for the Edit tool.** Conceptually fits the design; requires upstream Claude support for partial Edit results. Phase later.

---

## 9. Files touched (estimate)

| Path | Change |
|---|---|
| `frontend/app/view/agent/types.ts` | Add `log?: ToolStreamingLog` to `ToolNode`; add `ToolChunkEvent` |
| `frontend/app/view/agent/stream-parser.ts` | New `toolChunkToNode` + arm in `eventToNode` |
| `frontend/app/store/agent-document-store.ts` (or `agent-document-store/reducer.ts`) | New `ToolChunkAppend` command + reducer arm |
| `frontend/app/view/agent/components/ToolBlock.tsx` | Replace inline `renderToolContent()` and embedded button with `<ToolBlockOverlay>` |
| `frontend/app/view/agent/components/ToolBlockOverlay.tsx` (new) | Header, virtualized log, action bar |
| `frontend/app/view/agent/components/ToolOverlayLog.tsx` (new) | Virtualized chunk renderer |
| `frontend/app/view/agent/components/ToolOverlayActions.tsx` (new) | Bottom action bar |
| `frontend/app/view/agent/components/NodeHoverStrip.tsx` | Remove branching buttons; keep bookmark + expand only |
| `frontend/app/view/agent/virtualization/DocumentRow.tsx` | Wire `onOpenInPane` / `onOpenInWindow` / `onNewAgentFromHere` to real implementations or disabled state |
| `frontend/app/view/agent/styles/_tool-overlay-portal.scss` | Add `--log-body` + `--action-bar` slots |
| Backend (`agentmux-srv`): tool execution providers | Emit `tool_chunk` events as stdout/stderr arrives (Claude provider as starting point) |

---

## 10. Effort

| Component | LOC | Days |
|---|---|---|
| Types + `ToolChunkEvent` + `ToolStreamingLog` | ~50 | 0.25 |
| Stream parser arm + tests | ~80 | 0.5 |
| Reducer command + tests | ~120 | 0.5 |
| `ToolBlockOverlay` + `ToolOverlayLog` + virtualization | ~250 | 1.0 |
| `ToolOverlayActions` + stub implementations | ~120 | 0.5 |
| `NodeHoverStrip` slim + `DocumentRow` wiring | ~50 | 0.25 |
| Backend `tool_chunk` emission (Claude provider only) | ~150 | 0.5 |
| Manual smoke + perf + reconnect test | — | 0.5 |
| **Total** | **~820** | **~4 days** |

---

## 11. Phasing

This is too large for a single PR. Split into phases that each ship something visible:

- **Phase 1 — data shape + reducer + tests.** No UI changes. Adds the `log` cell, the `ToolChunkAppend` command, types, parser. Sets up the contract. Tools accept chunks but the UI doesn't yet read them. Land first to lock the shape.
- **Phase 2 — backend chunk emission (Claude provider).** Connect stdout/stderr line streaming on the host side to emit `tool_chunk` events. Now chunks flow but still aren't rendered.
- **Phase 3 — `ToolBlockOverlay` UI refactor.** New component, virtualized log, action bar at the bottom. Removes branching buttons from `NodeHoverStrip`. User now sees the live log on hover/pin.
- **Phase 4 — stub action implementations.** Wire `onOpenInPane` / `onOpenInWindow` to real `createBlock` / `openNewWindow` calls. "New agent here" stays disabled pending backend.
- **Phase 5 (optional) — other providers, Edit diff-hunk streaming, full log export.** Follow-ups.

Each phase is independently shippable and reviewable. Phase 1 + 2 are invisible foundation; Phase 3 is the user-visible payoff.

---

## 12. Cross-references

- Reducer family: `frontend/app/store/agent-document-store.ts`, master status in `docs/specs/SPEC_MASTER_REDUCER_STATUS_*.md`
- Tool block render: `frontend/app/view/agent/components/ToolBlock.tsx`
- Hover strip: `frontend/app/view/agent/components/NodeHoverStrip.tsx`
- Stream parser: `frontend/app/view/agent/stream-parser.ts`
- Predecessor analysis (tool collapse spec, related): `docs/specs/tool-collapse.md`
- Virtualization predecessor: `docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md`

---

## 13. Driving observation (verbatim)

> "in the meantime, we also want to tweak the hover tool thing. currently it shows an array of buttons, that doesn't work. we want it to show the actual running result of the command (many have && so we want the log as it is running, and the result once complete. we want want reducer arch for this too. the buttons for branching and opening up other stuff, lets move them to the bottom of the log popout, write a spec to file"
