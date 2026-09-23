# SPEC: Agent pane bounded live window — migration plan

**Date:** 2026-09-23
**Status:** proposed — no code yet; Phase 0 (measurement) is the first PR.
**Author:** Manoz
**Priorities (set by the user, 2026-09-23):** performance and robust stability
above everything else. Engineering cost and time are not constraints. Nothing
in this plan is optional for cost reasons; every "measure first" step below is
there because a decision depends on data, not to save work.
**Related:**
`docs/analysis/ANALYSIS_AGENT_PANE_FLUSH_REMOUNT_CHURN_2026_09_23.md` (§6 is the starting point),
`docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md` (introduced the 50-node streaming buffer this spec replaces),
`docs/specs/SPEC_REPLACECHILD_CRASH_FULL_ANALYSIS_AND_FIX_2026-06-06.md` (the crash class every tail change must respect),
`docs/specs/SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md` (the input-path rules §6.5 implements for streaming),
`docs/specs/SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md` (session-scope clamp),
`docs/specs/SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md` (the History tab this spec makes follow the live pane),
`docs/specs/SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md`,
`docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md`.

---

## 1. Summary

Typing in an agent pane gets slow again as a conversation grows, even after
#3521, #3536, #3555 and #3559. The old messages themselves are not the cause —
they are already virtualized and out of the DOM. The causes are what the pane
keeps **always mounted**, what it **recomputes per stream flush**, and the fact
that **stream work and keystrokes compete for the same frames with no
priority**:

1. The last **50 nodes** are rendered unvirtualized (`STREAMING_BUFFER_SIZE`),
   capped by *count*, not size. In a long conversation those 50 are large:
   five 20–30 KB messages already put the page at ~58k DOM nodes.
2. Pin-to-bottom **forces layout up to 3× per flush** and reads every tail
   row's `offsetHeight` on each call. Cost scales with (1).
3. Every flush **copies the whole node array plus its id Set and index Map**;
   every layout change recomputes **O(n) prefix sums**; the stream hook's
   dedup set grows forever. All grow with total history within a session.
4. Stream flushes from every visible pane run to completion in the frame they
   land in. A keystroke that arrives during them waits (measured: "typing
   adds nothing on top of streaming — keystrokes queue behind flushes").

**Is a major architecture change needed?** The data model, the layout store's
design, the row renderers, the session-scope clamp and the History tab stay.
The changes are:

| # | Change | Kind |
|---|---|---|
| A | Pin-to-bottom reads layout once per frame, after layout, never in a microtask | local rewrite |
| B | The always-mounted tail becomes "the turn in flight" instead of "the last 50 nodes" | **redesign of the render split** |
| C | The live document keeps a bounded window; the History tab is the archive and follows the live pane | new subsystem boundary |
| D | Per-flush and per-layout work becomes O(batch + log n), not O(n) | data-structure replacement in two stores |
| E | One cross-pane stream scheduler with a frame budget that yields to input | **new architectural layer** |
| F | Off-main-thread markdown parsing and highlighting | new architectural layer, decided by §7 Phase 8's criterion |

A–D remove costs that grow with history. E removes the remaining contention
between streaming and typing regardless of history. F is the next lever if
main-thread parse cost still breaks the budget after A–E.

## 2. Background

### 2.1 What is already fixed (2026-09-22/23)

| PR | Fix |
|---|---|
| #3521 | incremental markdown parse |
| #3536 | dormancy gate (hidden panes don't render) |
| #3555 | finished tool results no longer rebuilt on every flush |
| #3559 | markdown keeps the frozen prefix's DOM; processor built once |

Four visible panes streaming 20k-word documents, 15 s window, no typing:

| | `main` before | #3555 | #3555 + #3559 |
|---|---|---|---|
| Frames | 35 (2.3 fps) | 565 (38 fps) | 647 (43 fps) |
| Time in long frames | 100 % | 38 % | 27 % |
| `flushPendingNodes` per flush | ~144 ms | ~45 ms | ~25 ms |

Real keystrokes under the heaviest load of the day: key→paint p50/p95/max
64/184/240 ms (was 88/472/552 on `main` at about a third of the load). The
user then reported the jitter returning as conversations grew.

### 2.2 How the pane works today

**Stream intake** — `frontend/app/view/agent/useAgentStream.ts`. Subscribes to
the block's output file (`getFileSubject(blockId, OutputFileName)`, `:424`),
parses NDJSON into nodes, dedups with a hook-local `nodeIdSet` that is never
pruned (`:246`), and pushes into a per-pane rAF-batched flush queue.

**Document store** — `frontend/app/store/agent-document/reducer.ts`.
`StreamFlush` (`:484`) copies `state.nodes`, `nodeIdSet` and `nodeIndexById`
on every flush that changes anything (`:497`, `:503`, `:538`). The only trim is
the session-scope clamp (`clampToSessionScope`, `:43`). Within one session the
document grows without bound, and scrolling up pulls older pages back in via
`useHistoryPagination.loadOlder` (200 lines a page).

**Layout store** — `frontend/app/store/agent-pane-layout/reducer.ts`.
Heights (measured, else per-kind estimate, else default); `positions()` is an
O(n) prefix sum materializing a `RowPosition` for every virtualized row;
`totalSize()` is another O(n) sum; `windowRangeOf()` binary-searches the
materialized array. Rows report heights through one ResizeObserver that
dispatches `RowMeasured` (`AgentDocumentVirtualList.tsx:1003`).

**Virtual list** — `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx`.
Two regions in one scroll container:

- *Virtualized head*: absolutely positioned rows from the layout store's
  window, `<Key by nodeId>` (`:1065`).
- *Streaming buffer*: trailing nodes in normal flow, all mounted,
  `<Key by id>` (`:1140`). The split is a **sticky frontier**
  (`stickyFrontierId`, `:272`) that only moves when the buffer exceeds
  `STREAMING_BUFFER_SIZE = 50` (`streaming-buffer.ts:22`), inside the
  partition memo (`:291`–`:338`). (The comment at `:326` about `<Index>` is
  stale; the buffer uses `<Key>`, per the render-site comment at `:1101`.)

**Pin-to-bottom** — three triggers all call `scrollToTrueBottom()` (`:229`):

- a `createEffect` on `nodes().length` / `totalSize` that queues a
  **microtask** (`:565`) — runs before the browser's own layout, so reading
  `scrollHeight` there is a forced synchronous layout;
- a ResizeObserver on the scroll container's client height (`:600`);
- a ResizeObserver on the virtualized region and the streaming buffer (`:649`).

`scrollToTrueBottom` reads `scrollHeight` and, for shrink attribution,
**every streaming-buffer row's `offsetHeight`** (`sampleStreamingRows`,
`:216`). The analysis doc measured 387 ms of forced layout from this in one
10 s window.

**Turn end** — `agent-view.tsx:1008` `turnJustEndedAtom`, an edge detector
already used to schedule the tools-only orphan scrub 2 s after a turn ends
(`:1070`).

**History tab** — `frontend/app/view/agent/history/AgentHistoryView.tsx`. A
separate blockStack tab, read-only, not a consumer of the document store. Reads
the transcript via `BlockfileLineCountCommand` + `BlockfileReadRangeCommand`
in 600-line pages. Loads once on mount; **does not follow new output**.

**Platform** — CEF 148 (Chromium 148) on Windows, macOS and Linux.
`scheduler.yield()` (Chromium 129+) is available; the repo already mandates it
for deferrable input-path work (`SPEC_INPUT_RESPONSIVENESS…` Rule 2).

### 2.3 Cost model

| Cost | Scales with | Bounded today by |
|---|---|---|
| DOM size, style/layout of the tail | bytes in the last 50 nodes | node count only |
| Pin-to-bottom forced layout | the tail's DOM | nothing |
| Per-row `offsetHeight` sampling | tail row count | 50 |
| Node array / Set / Map copy per flush | all nodes in the session | session boundary |
| `positions()` + `totalSize()` per layout change | virtualized rows in the session | session boundary |
| Stream hook `nodeIdSet` memory | every node ever seen by the hook | pane lifetime |
| Stream work vs keystrokes | number of visible streaming panes | nothing (no priority) |
| Virtualized head DOM | viewport + overscan | already bounded |

## 3. Industry practice (research, 2026-09-23)

- **Virtualize everything, including the streaming message.** TanStack
  Virtual's chat guide renders only visible items plus `overscan: 6`; there is
  no unvirtualized tail. The growing last item is re-measured and the size
  delta applied with `anchorTo: 'end'`. A published LLM-chat benchmark held
  200–240 fps over 5-minute streams with full windowing versus 0 fps naive.
- **Follow only when pinned** (`followOnAppend`, `scrollEndThreshold` ≈ 80 px).
  Reading history mid-stream must not snap back (opencode #29094).
- **Prepend keeps the visible item fixed**, located by stable key, never index.
- **Read layout inside a ResizeObserver callback.** Layout is already clean
  there; reading it anywhere else forces layout.
- **Break long work up and yield to input** with `scheduler.yield()`;
  continuations run ahead of new same-priority tasks, so deferred work isn't
  starved (web.dev "Optimize long tasks").
- **Batch stream updates per animation frame.** Necessary, not sufficient
  (we already batch).
- **`content-visibility: auto` + `contain-intrinsic-size: auto <px>`** skips
  off-screen layout/paint and remembers sizes (web.dev: 232 → 30 ms). **But**
  hermes-agent removed it (PR #71269): on Windows, Electron 40 / Chromium 144,
  transcript rows with it drove unbounded renderer memory growth (~5.5 GiB);
  removing it cut renderer memory 89 %. Layout reads inside a skipped subtree
  also force it to render.

Where we differ:

| Practice | AgentMux today | After this spec |
|---|---|---|
| Only visible rows mounted | last 50 always mounted | only the turn in flight always mounted |
| Layout reads after layout | microtask read + 2 RO reads per flush | one RO-driven read per frame |
| Bounded working set | grows until session boundary | bounded live window; History tab is the archive |
| Long work yields to input | flushes run to completion | frame-budgeted scheduler, input first |
| Follow only when pinned | yes (`stickToBottom`) | unchanged |
| Stable keys | yes (`<Key>`) | unchanged |

## 4. Goals, non-goals, targets

**Goals**

- Frame timing and key→paint latency are **flat in conversation length**.
- Typing stays within budget **regardless of how many panes stream**.
- Old messages move out of the live pane with **no visible jump**, whether or
  not the History tab is open.
- Renderer and main-process memory are **bounded** over arbitrarily long
  sessions.
- **No new crash class.** Every phase ships with a kill switch and stays
  revertible until it has soaked.

**Non-goals**

- Replacing the custom layout store with a library (TanStack/virtua). The
  store already does what they do; this spec changes its data structure and
  what feeds it.
- Changing the History tab's reading UX, the session-scope clamp semantics,
  or the transcript's format. (Two backend additions are in scope because
  eviction depends on them: a file generation for the output file, and a
  durable journal for nodes that never reach the transcript — §6.3.1, §6.3.2.)

**Acceptance targets** — measured with the Phase 0 bench, 4 visible panes
streaming heavy markdown + tool output, user typing continuously, at every
N in {0, 25, 100, 200, 500} turns of history, on Windows (primary), macOS and
Linux:

| Metric | Target |
|---|---|
| key→paint (Event Timing) | p50 ≤ 33 ms, p95 ≤ 50 ms, max ≤ 100 ms |
| Frame rate while streaming + typing | ≥ 55 fps |
| Time in long animation frames (> 50 ms) | ≤ 5 % |
| Slope of any metric above vs N | not distinguishable from 0 (N=25 vs N=500 within run-to-run noise) |
| Renderer + main-process memory, 8 h soak | ≤ 5 % growth after 30 min warm-up |
| Per-flush reducer cost | O(batch + log n): flat in N |
| Forced synchronous layouts from agent-pane code | 0 per flush (trace-verified) |
| Crashes / error-boundary trips in soak and fault suites | 0 |

These are the definition of done. If A–E don't meet them, Phase 8 (F) is
triggered, and further levers are specified before closing this spec.

## 5. Invariants

Each is enforced three ways: a unit/property test, a dev-build runtime
assertion that logs to the render trail and the perf HUD, and (where
applicable) a CI guardrail.

1. **Single residency.** A node id is never in the virtualized head and the
   streaming buffer in the same reactive tick. Frontier moves happen only
   inside the partition memo.
2. **No fight.** A user scrolled away from the bottom is never moved by the
   pin (`stickToBottom` gates it), and migration or eviction while they read
   preserves the first visible node and its offset (§6.3.5).
3. **No jump.** Nothing visible moves when content is added, migrated or
   evicted.
4. **No loss.** Only a node whose provenance is durable is evicted: its
   transcript line (or `out-of-band.jsonl` line) is covered by that file's
   line count, so the History tab can show it (§6.3.2).
5. **Layout consistency.** The layout store's `totalSize` equals the sum of
   effective heights of its rows; every mounted virtualized row's measured
   height equals its stored height within 1 px after the measure RO fires.
6. **Dormancy.** Hidden panes (dormancy gate) and hidden History tabs do no
   render or layout work.
7. **Input first.** No stream-driven task runs longer than the scheduler's
   slice while input is pending.
8. **Stable identity.** A node's id is a function of its source position and
   file generation only; any parser instance, from any clean boundary, gives
   the same content the same id (§6.3.1).
9. **No resurrection.** Content at or below an eviction watermark never
   re-enters the live pane, whatever replays it (§6.3.3).
10. **Bounded while reading.** Live-pane memory is bounded whether pinned or
    not (§6.3.5).

## 6. Target design

### 6.1 (A) Pin-to-bottom: one post-layout read per frame

- The `nodes().length`/`totalSize` effect (`:555`) only **arms** the pin; it
  never reads layout. The microtask at `:565` is removed.
- A single **pin controller** owns pinning. Its inputs are the two existing
  ResizeObservers (`:600`, `:649`) plus the armed flag. It performs at most one
  `scrollHeight` read and one `scrollTo` per frame, inside an RO callback
  (layout already clean, before paint).
- `sampleStreamingRows()` moves behind the existing diagnostic toggle. The
  PR #2887 attribution concern only affects that diagnostic.
- `jumpToBottom` (keystroke) stays synchronous and explicit; it is the one
  user-initiated forced read, documented as such.
- `collapseScrolledOffTools()` moves into the same controller pass.
- **Guardrail:** a CI grep (pattern of `input-handler-guardrails.yml`) fails
  on `scrollHeight|offsetHeight|getBoundingClientRect|scrollTop` reads in
  `frontend/app/view/agent/virtualization/` outside an allow-listed set of RO
  callbacks and `jumpToBottom`.
- **Test:** a counter-instrumented unit test asserts ≤ 1 `scrollHeight` read
  per simulated flush; the resize-contract suites
  (`resize-contract.test.ts`, `AgentDocumentVirtualList.resize.test.tsx`)
  stay green.

### 6.2 (B) The tail holds only the turn in flight

The streaming buffer holds **the in-flight turn**: every node from the last
`user_message` onward, plus any earlier node still in progress (thinking
markdown, `running`/`awaiting_answer`/`pending_approval` tool, open shell).
A hard ceiling (count **and** bytes, e.g. 40 nodes / 512 KB, set from the
Phase 0 curve) migrates the oldest *finished* nodes of a very long turn early.

**Migration of finished nodes into the virtualized head**

- **When:** on `turnJustEndedAtom` (after the in-flight flush drains), and
  when the ceiling is exceeded. Never per token.
- **Where:** only nodes wholly above the viewport, or any finished node while
  pinned (the move is off-screen or at the bottom where the pin re-asserts).
- **How:** one frontier advance per migration, moving all eligible nodes at
  once, inside the partition memo (invariant 1). `<Key>` disposes departing
  slots by id; remaining tail rows are untouched.
- **Height handoff:** before the frontier advances, `RowMeasured` is
  dispatched for every migrating node with its current height, read inside
  the pin controller's RO pass (free, layout clean; ÷zoom like the measure RO).
  The layout store therefore has exact heights, `totalSize` is right on the
  first frame, and the view does not move (invariant 3). A node with no
  height yet falls back to its per-kind estimate and is corrected on first
  measure.
- **Ordering:** migration is a scheduler task (§6.5) at "housekeeping"
  priority — it never runs in a frame with pending input.

**Why this is safe now when it was not in June:** the June crashes came from
the tail array **growing** past the cap in a render and from `<Index>`
position slots. The current design already moves the frontier inside the memo
and renders the tail with `<Key>`. This adds a trigger for the same move.

**Why the 50-node tail is no longer required:** the redesign kept 50 nodes
mounted to avoid measurement lag while tokens stream. Only nodes receiving
tokens need that. Finished nodes change height only on expand/collapse, which
the layout store already tracks per state (`inFlowState`).

### 6.3 (C) Bounded live document

The live pane keeps a **live window** of the most recent turns; older turns are
removed from the document store, layout store and stream-hook dedup set.

- **Budget:** `LIVE_WINDOW_TURNS` (proposed 30) and `LIVE_WINDOW_BYTES`
  (proposed 2 MB), whichever binds first, never smaller than the in-flight
  turn. Values set from the Phase 0 curve; both enforced.
- **Top of the live pane:** the `history_link` row shows whenever anything was
  evicted or clamped.
- **Find in page** covers what the live pane holds; the History tab covers the
  rest. Release-noted.

Eviction is only safe if every evicted node can be found again, and can never
come back as a duplicate. Today neither holds (Codex review of this spec,
2026-09-23): parser ids are per-instance counters, some nodes never reach the
transcript, and replay dedup relies on ids that eviction would remove. §6.3.1–
§6.3.3 make both hold. §6.3.4–§6.3.5 are the eviction rules.

#### 6.3.1 Stable node identity from transcript position

`ClaudeCodeStreamParser` ids are counters (`node_0`, `user_0`, …) that restart
for every parser instance. The live pane avoids collisions with a `skipIds`
callback over the reducer's live id set (`useAgentStream.ts:390`–`:414`); the
History tab avoids them by reparsing everything it has loaded in one parser
on every page (`AgentHistoryView.tsx:98`–`:107`). Neither works once nodes are
evicted: a skip set without the evicted ids can't stop a replayed line
reusing one, and a reparse of everything is O(history).

- Every parser-produced node gets an id derived from its **source position**:
  `G<gen>L<line>` for the first node a transcript line produces,
  `G<gen>L<line>.<k>` for the k-th after it. `line` is the 0-based line index
  in the block's output file; `gen` is the file's **generation**, which the
  backend bumps whenever it truncates or replaces that file, so line numbers
  that restart never reuse an id for different content. (Phase 5 establishes
  where truncation happens today and adds the generation to the line-count and
  range-read responses.) The same line always yields the same ids in every
  parser instance — live pane, reconnect replay, History tab, any page order.
- Every such node carries `src: { line }` (new field). Nodes produced from
  several lines (a text run) take the line that **started** them, so a node's
  id never changes as it grows.
- The parser exposes `atBoundary()`: true when no text, thinking or tool
  accumulator is open. Lines where it is true are **clean boundaries**; parsing
  may start at any clean boundary and produce the same ids and nodes as a parse
  from line 0.
- `skipIds` is removed once ids are positional: two parsers can no longer mint
  the same id for different content.
- **Migration:** node ids are keys in the layout store (heights, expansion),
  pins, held-open tools and the dispatch-match map. Phase 5 inventories every
  consumer that **persists** a node id across reloads; each gets either a
  one-time mapping from old ids (built by reparsing the loaded range with both
  schemes) or an explicit, release-noted reset of that UI state.

#### 6.3.2 Durability for nodes that never reach the transcript

Two producers create nodes outside the transcript parser:

| Producer | Node | Where it is durable today |
|---|---|---|
| `usePendingMessageAcceptance.ts:118`–`:128` | optimistic `user_message` (id = pending id) | nowhere until the provider echoes the message into the transcript |
| `useShellNodeStream.ts:180`–`:201` | `shell` (id = shell id) and its output chunks | a backend replay ring of 64 events plus a per-shell chunk ring — **bounded, so old shells are not recoverable** |

Rule: **every node kind declares its provenance**, and only a node with a
**durable** provenance is evictable.

- `transcript` (parser-produced): durable once the block's line count covers
  `src.line`.
- `journal`: the backend appends every out-of-band node event (shell create,
  chunk, exit; optimistic user message accepted) to a new per-block durable
  journal file, `out-of-band.jsonl`, with its own line numbers. A journal node
  carries `src: { journal: <line> }` and is durable once the journal's line
  count covers it. The History tab reads the journal alongside the transcript
  and merges by timestamp, so shells appear in history too (today they don't).
- Optimistic `user_message`: Phase 5 first establishes, per provider, whether
  the transcript carries an echo of a sent message and how (if at all) it is
  matched to the optimistic node today — not verified while writing this. If
  there is a reliable echo, the optimistic node is re-keyed to the echo's
  positional id and `src.line` when it arrives and becomes `transcript`.
  Otherwise the accepted message is written to `out-of-band.jsonl` at
  acceptance and is `journal`.
- Anything else is `ephemeral` (working indicators, synthetic rows such as
  `history_link` and `day_divider`): never evicted as content; regenerated
  from state.

A node whose provenance is not yet durable is **pinned in the live window**
and counts against the budget. If more than a hard cap (proposed 64 nodes) of
non-durable nodes accumulate, that is a bug surfaced in the dev HUD and the
render trail, not something eviction papers over.

#### 6.3.3 No resurrection after eviction

Codex review: the live pane seeds its dedup from the reducer's id set, so
dropping evicted ids lets a reconnect replay re-add them — duplicated, and in
the wrong place.

- **Transcript watermark.** The pane keeps `evictedThroughLine`: every
  transcript line at or below it belongs to an evicted turn. It only moves
  forward, and survives eviction (it is pane state, not a node). On replay the
  stream consumer drops lines at or below it **before parsing**. Because ids are
  positional (§6.3.1), lines above it that are still in the window dedup by id
  exactly.
- **Journal watermark.** The same for `out-of-band.jsonl`
  (`evictedThroughJournalLine`).
- **Ring replays.** A `shell_node_create` replayed from the backend's 64-event
  ring for an evicted shell carries its journal position (added to the event),
  so the journal watermark drops it. Until that field ships, a bounded
  tombstone set of the last 256 evicted out-of-band ids (4× the ring) drops
  them.
- Both watermarks are part of the pane's persisted state, so a pane remount
  doesn't reset them.

#### 6.3.4 Eviction while pinned

- **When:** at turn end (housekeeping priority, §6.5) while pinned.
- **What:** whole turns from the front, oldest first, whose nodes are all
  durable (§6.3.2), until within budget.
- **How:** reducer command `EvictThrough { line, journalLine }` removes the
  turns in one pass (the same shape as `clampToSessionScope`) and advances both
  watermarks in the same reduction, so there is no window where a node is gone
  but its replay is still accepted. The layout store prunes ids no longer
  present (`agent-pane-layout/reducer.ts:118`). Evicted rows are virtualized
  and above the viewport; only `totalSize` changes and the pin holds the
  bottom.

#### 6.3.5 Reading far from the bottom: a detached window

Codex review: suspending eviction while the user is scrolled up lets the live
pane grow without bound, one turn per completed turn, for as long as they read.
Instead, a scrolled-up reader keeps a bounded **reading window** and the pane
stops holding everything between it and the present — the "jump to present"
pattern chat apps use.

- While unpinned, the pane holds at most two ranges: the **reading window**
  (the turns around the viewport, budgeted like the live window) and the
  **present** (the in-flight turn plus the newest turns, up to a small budget,
  default 5 turns).
- Turns completed while reading are added to the present; when the present
  exceeds its budget, its oldest turns are dropped from memory (durable, so
  nothing is lost). If the present and the reading window no longer touch, a
  **gap row** between them says "N newer turns — jump to latest" and reports
  the count from the watermarks.
- Scrolling down into the gap loads the next turns by range read (clean
  boundaries, positional ids), and drops turns from the top of the reading
  window to stay within budget — anchor-preserving: the first visible node and
  its offset are kept, as the existing `headAnchor` does for prepends.
- "Jump to latest" (or the keystroke `jumpToBottom`) discards the reading
  window and re-pins on the present. Nothing needs reloading: the present is
  already there.
- Scrolling up past the reading window's top loads older turns by range read
  the same way. Paging and eviction share the reading window's budget, so they
  can't fight.
- Memory bound while unpinned: reading-window budget + present budget + the
  in-flight turn, independent of how long the user reads.

### 6.4 (C) History tab follows the live pane

"Moving" messages to history is not a copy — they're already in the
transcript file. What's missing is the History tab showing **new** output.

- **Doorbell, not polling:** the History tab subscribes to the source block's
  output file subject (the same `getFileSubject(sourceBlockId, …)` the live
  pane uses) and to `out-of-band.jsonl` purely as change notifications. It
  never parses the live stream events.
- **An incremental parser, not a reparse** (Codex review: a range parsed on its
  own collides on ids and splits runs that span the boundary; reparsing
  everything is O(history)). The tab keeps one **tail parser** positioned at
  the end of what it has read. New lines go through that same instance, so
  open text/thinking runs continue and ids are positional (§6.3.1) — appending
  costs O(new lines).
- **Visible tab:** on a doorbell, coalesced to at most once per second and
  deferred to housekeeping priority, read the new line range
  (`BlockfileReadRangeCommand` from the last known count), feed it to the tail
  parser, and upsert the resulting nodes by id (a node still growing at the
  boundary is updated in place, not duplicated). If the reader is at its
  bottom, follow; otherwise show "new messages below".
- **Hidden or dormant tab:** record only that the doorbell rang. On reveal, one
  range read from the last known count to the current one, through the same
  tail parser.
- **Tail parser lost** (tab remounted, parser error): restart it at the most
  recent clean boundary (§6.3.1) at or before the last line read — at most one
  turn of reparse — and upsert. Positional ids make the overlap idempotent.
- **Older pages** use their own parser started at a clean boundary; with
  positional ids, pages loaded in any order no longer collide, which retires
  the "reparse all loaded lines per page" workaround.
- **Closed tab:** nothing; it loads fresh on open.
- The History tab gets the same A/B/D treatment as the live pane (it reuses
  `AgentDocumentView`), so a very long history read is equally cheap.

### 6.5 (E) Cross-pane stream scheduler with input priority

Today each pane's rAF queue flushes independently and to completion. Replace
with one app-wide scheduler that owns *when* stream work runs:

- **Per-frame budget** for stream work across all panes (proposed 8 ms of a
  16.7 ms frame; tune from Phase 0). Panes are served round-robin, visible
  before hidden-but-undormant, the focused pane's own stream first.
- **Yield to input:** between panes and between chunks within a pane's flush,
  `await scheduler.yield()`; when the budget is spent, the remainder carries
  to the next frame.
- **Coalescing, not dropping:** a pane that misses a frame merges more tokens
  into its next flush. No data is lost; only paint cadence of the stream
  changes under load. The pinned view still ends at the latest content.
- **Priorities:** input handlers > stream flush of focused pane > other
  visible panes > housekeeping (migration, eviction, History-tab catch-up).
  Housekeeping never runs in a frame with pending input.
- **Starvation guard:** every visible pane gets a flush at least every
  100 ms, even under continuous typing.
- **Instrumentation:** per-pane flush cost, deferred-token backlog, and
  budget overruns in the perf HUD.

This implements `SPEC_INPUT_RESPONSIVENESS…` Rule 2 for the streaming path and
is the only change that addresses "keystrokes queue behind flushes" directly,
independent of history length.

### 6.6 (D) O(batch + log n) stores

The live window (§6.3) bounds n, but per-flush cost must not depend on n at
all, so a longer budget or a scrolled-up user never reintroduces the problem.

**Document store**

- `nodes` becomes a **chunked immutable sequence** (fixed-size chunks, e.g.
  128 nodes). Appending copies only the last chunk; updating a node copies
  its chunk; eviction drops whole chunks from the front plus one partial.
- `nodeIndexById` becomes **id → (sequence number)** with a base offset, so
  eviction doesn't renumber; stored in a persistent map (HAMT) so a flush
  copies O(log n), not O(n). `nodeIdSet` is subsumed by it.
- The public reducer interface (`DocumentState` accessors used by renderers)
  is preserved by an adapter so consumers change in one place.

**Layout store**

- Heights live in a **Fenwick tree** (binary indexed tree) keyed by row
  index: `RowMeasured` is O(log n) update, `totalSize` is O(1) (maintained),
  prefix start of any row is O(log n), and the window's first/last rows are
  found by O(log n) descent on the tree.
- `positions()` no longer materializes every row; only the window's rows are
  materialized.
- Eviction from the front uses a base offset (and periodic rebuild), not a
  shift.

**Correctness strategy:** the current O(n) implementations are kept verbatim
as **test oracles**. Property-based tests (add `fast-check` as a dev
dependency) generate random command sequences — flushes, updates, collisions,
session boundaries, evictions, history loads, measures, expansion toggles,
scrolls — and assert the new stores produce states observably identical to
the oracles. Existing reducer tests and `reducer.bench.ts` stay; the bench
gains N = 1k/10k/100k cases with a flatness assertion.

### 6.7 (F) Off-main-thread markdown — decision gate

Streaming markdown parse (unified/remark) and syntax highlighting run on the
main thread. #3521/#3559 made them incremental. Moving them to a Worker
removes them from the main thread, at the cost of serializing the tree across
the boundary and a new ordering/failure surface.

- **Decision criterion (Phase 8):** after Phases 1–7, if parse + highlight is
  > 15 % of main-thread time during the 4-pane streaming bench, or any §4
  target is still missed with parse/highlight on the critical path, build it.
- **Design if built:** one worker per app (not per pane); requests carry
  (pane, node, revision); the main thread applies only the newest revision
  per node; the worker returns hast for the tail block only (the frozen
  prefix is already cached); highlight is async and never blocks first
  paint of plain text; worker crash → automatic restart and main-thread
  fallback for in-flight nodes, with a render-trail entry.
- It is gated on data because a worker that doesn't help still adds a
  failure surface — that would be a robustness regression.

### 6.8 `content-visibility` — gated experiment

Behind a flag, apply `content-visibility: auto; contain-intrinsic-size:
auto <estimate>px` to finished mounted rows. Adopt only if the bench shows a
frame-time win **and** the 8 h soak shows renderer memory flat on Windows,
macOS and Linux (CEF 148; the reported leak was Chromium 144, unverified
whether fixed). Before enabling, assert no pin/measure path reads layout
inside a skipped row (Chromium logs a console warning; the soak fails on it).
After B, few finished rows stay mounted, so expect a small win.

## 7. Migration plan

Every phase:

- is its own PR with a changeset and updates this spec's Status;
- has a **runtime kill switch** (setting, not just a dev flag) that restores
  the previous path without a restart where feasible; the old path is deleted
  only after the phase has soaked for 7 days of daily use by the user with no
  regressions;
- lands its tests, dev-build invariant assertions (§5) and HUD counters in
  the same PR;
- is verified on Windows, macOS and Linux builds;
- re-runs the full Phase 0 bench and the fault suite (§8) and records the
  numbers in the analysis doc.

| Phase | Content | Exit criteria |
|---|---|---|
| **0 — Measurement** | Commit `full-conversation-bench.mjs`. Add: renderer + main-process memory (CDP `Performance.getMetrics`, `SystemInfo.getProcessInfo`, OS working set), DOM node count, frame gaps, LoAF with script attribution, Event Timing key→paint, per-flush `dispatchDoc` cost, forced-layout count from a trace. N = 0/25/100/200/500. 8 h soak harness. Fault-suite harness (§8). Record `main` baseline on all three OSes. | Baseline curves and soak numbers on record |
| **1 — Pin (A)** | §6.1 | 0 forced layouts from the pin path in a trace; guardrail live; resize/scroll-follow suites green; manual matrix: panels appearing, zoom 50–200 %, hidden→visible, pane resize, split/unsplit |
| **2 — Scheduler (E)** | §6.5 | key→paint targets met at N=0 with 4 panes streaming; starvation guard verified; no stream content lost (byte-for-byte transcript vs rendered comparison) |
| **3 — Turn-scoped tail (B)** | §6.2 | Tail DOM independent of N; replaceChild crash repro suite and streaming-buffer tests green; zero invariant-1/3 assertions in soak; frame-by-frame screen recording shows no movement at turn end |
| **4 — O(log n) stores (D)** | §6.6 | Property tests: 100k random sequences per run in CI, 10M locally, 0 divergences; reducer bench flat from 1k to 100k nodes |
| **5 — Node identity and durability (C prerequisites)** | §6.3.1–§6.3.3: positional ids with file generation, `src`, `atBoundary()`, id-consumer migration, provenance per node kind, `out-of-band.jsonl` (backend + History-tab merge), both watermarks, ring-replay handling | Property test: parsing any line range from any clean boundary, in any page order, yields the same ids and nodes as a parse from line 0 (100k random splits per CI run); replay-after-eviction suite produces zero duplicates; every node kind has a declared provenance (a test enumerates the `DocumentNode` union); shells appear in the History tab |
| **6 — Bounded live document (C)** | §6.3.4–§6.3.5 | Memory and per-flush cost flat in N, pinned **and** while reading far from the bottom for 1 h of streaming; invariant 4 verified by killing the backend mid-eviction; no non-durable node ever evicted (runtime assertion, soak) |
| **7 — History tab follows (C)** | §6.4 | Visible tab shows new turns ≤ 1 s after turn end; appends cost O(new lines) (profile); hidden tab does zero work and catches up on reveal; tail-parser loss recovers with no duplicates; long-history read meets the same targets |
| **8 — Worker decision (F)** | §6.7 criterion evaluated; build if triggered | §4 targets met on all three OSes |
| **9 — Default on** | Remove flags once each phase has soaked; update `SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md` Status to point here | 8 h soak and fault suite green on all three OSes; user sign-off after daily use |
| **10 — `content-visibility` (§6.8)** | flagged experiment | memory flat over 8 h on all three OSes *and* measurable frame win; otherwise documented as rejected |

Order rationale: A and E are independent of history and fix the most for
the least risk, so they land first and make every later measurement cleaner.
B precedes D and C because it removes the dominant DOM cost; D precedes C so
eviction lands on the final data structures rather than being written twice.
Phase 5 precedes eviction because eviction without stable identity and durable
provenance either loses data or resurrects it.

## 8. Verification

**Fault and edge suite** (automated where possible; every item run each phase):

- Single 5 MB tool result; single 1 MB markdown message; 1,000 tool calls in
  one turn; 200 turns in 10 minutes.
- Pane hidden, zoomed, resized, split, or moved to another window **during** a
  migration or eviction.
- Session boundary (`fresh` outcome), `/clear`, fork, history restore, stream
  reconnect with replayed (duplicate) nodes — each mid-migration.
- User scrolls up during streaming for 10 minutes, then returns: no fight,
  eviction catches up, no jump.
- User reads far from the bottom for 1 hour while 4 panes stream: memory stays
  within the reading-window + present budgets; the gap row's count is right;
  "jump to latest" re-pins with no reload.
- Stream reconnect and pane remount **after** eviction, replaying the whole
  transcript and the 64-event shell ring: zero duplicates, nothing evicted
  reappears (invariant 9).
- Output file truncated or replaced mid-session (generation bump): no id
  reuse across generations (invariant 8).
- Shells and optimistic messages: never evicted before their journal (or echo)
  line is durable; visible in the History tab afterwards.
- History tab's tail parser discarded mid-run (remount, injected error):
  recovery from the last clean boundary, no duplicates, no split runs.
- Backend killed mid-eviction, mid-journal-write and mid-History-tab read
  (invariant 4).
- 8 panes streaming at once; 4 panes streaming while the History tab of each
  is open and visible.
- Expand/collapse of a tool in the tail, then migration, then scroll back to
  it: stored height matches.
- Composer IME composition during heavy streaming (the scheduler must not
  delay composition events).

**Soak:** 8 h, 4 panes streaming continuously, synthetic typing bursts every
minute, memory sampled every minute; zero crashes, zero invariant
assertions, memory target met.

**Regression protection:** the Phase 0 bench's deterministic counters (forced
layouts per flush, mounted tail rows, per-flush reducer operations) become
unit-level assertions in CI; timing-based numbers run nightly on the dev
machine and are compared against the recorded baseline with an alert on
regression.

## 9. Decisions (formerly open questions)

1. **Budgets** are set from the Phase 0 curve, but both a turn count and a
   byte limit are always enforced; never one without the other.
2. **Live-pane paging** pages within a budget in both directions: the live
   window while pinned, the reading window while reading (§6.3.5). Paging and
   eviction share that budget, so they can't fight. The History tab remains the
   place for reading the whole conversation.
3. **History-tab updates** use the output-file and journal subscriptions as a
   doorbell, not polling, and an incremental tail parser, not a reparse
   (§6.4).
4. **The user's last message** is the first node of the in-flight turn and
   migrates with the rest at turn end. No special-casing.
5. **Eviction requires durable provenance.** Content that exists only in memory
   or in a bounded replay ring is never evicted; out-of-band nodes get a
   durable journal first (§6.3.2).

## 10. Risks

| Risk | Mitigation |
|---|---|
| replaceChild crash returns (node in both lists) | frontier moves only in the partition memo; batch moves; invariant-1 runtime assertion; June crash repros every phase |
| View jumps at migration or eviction | height handoff; only while pinned or off-screen; invariant-3 assertion; frame-by-frame recording |
| Pin misses a growth source after removing the microtask read | the content RO covers content growth; `jumpToBottom` retained; resize-contract suites; guardrail |
| Scheduler starves a pane or delays IME | starvation guard; IME in the fault suite; HUD backlog counter |
| New store structures diverge from current behaviour | old implementations kept as oracles; property tests at scale |
| Eviction before the transcript has the data | line-count precondition (invariant 4); kill-backend test |
| Evicted content replayed back in as duplicates | positional ids + eviction watermarks + ring tombstones (invariants 8–9); replay-after-eviction suite |
| Positional ids break UI state keyed by today's ids | Phase 5 inventory of every persisted id consumer; mapping or explicit reset per consumer |
| Line numbers restart after a truncate/replace | file generation in every id (§6.3.1); truncate test |
| Out-of-band node lost (shells, optimistic messages) | provenance rule; `out-of-band.jsonl` before eviction; non-durable nodes pinned and capped (§6.3.2) |
| Live pane grows while the user reads | detached reading window + bounded present (§6.3.5, invariant 10) |
| History tab append corrupts runs or ids | one tail parser, clean-boundary restart, upsert by positional id (§6.4) |
| `content-visibility` memory growth | experiment only, 8 h soak on all OSes |
| Worker adds a failure surface | built only on the §6.7 criterion; restart + fallback |
| Find-in-page loses old messages | History tab; release note |

## 11. Sources

- TanStack blog, "Chat UIs Are Lists Until They Aren't" — https://tanstack.com/blog/tanstack-virtual-chat
- TanStack Virtual chat docs — https://tanstack.com/virtual/latest/docs/chat
- web.dev, content-visibility — https://web.dev/articles/content-visibility
- web.dev, Optimize long tasks — https://web.dev/articles/optimize-long-tasks
- Chrome for Developers, `scheduler.yield()` — https://developer.chrome.com/blog/use-scheduler-yield
- hermes-agent PR #71269, content-visibility memory growth — https://github.com/NousResearch/hermes-agent/pull/71269
- Tiger Oakes, "ResizeObserver is a safe place to read scrollWidth/clientWidth" — https://tigeroakes.com/posts/resize-observer-avoid-forced-sync-layout/
- "Chasing 240 FPS on LLM chats" — https://dev.to/gokhan_koc_88338a026508b3/chasing-240-fps-on-llm-chats-4gde
- opencode #29094, viewport re-snaps during streaming — https://github.com/anomalyco/opencode/issues/29094
- virtua — https://github.com/inokawa/virtua
