# SPEC: Agent pane bounded live window — migration plan

**Date:** 2026-09-23
**Status:** active — Phases 0–3 shipped (#3593, #3598, #3599, #3604, #3607, #3610, #3611); Phase 5a shipped (#3628–#3648, design §6.3.7). **Plan revised 2026-09-24 (§6.9): the bounded live document ships as the "live feed" with turns rolling off into History** — replacing Phases 6–7 as written in §6.3.4–§6.3.5 and §6.4, and deferring Phases 4, 5b, 5c and 5e. Progress: `TRACKING_AGENT_PANE_BOUNDED_LIVE_WINDOW_2026_09_23.md`.
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

## 0. Terms (agreed with the user, 2026-09-24)

"Conversation" meant two different things in this work. From here on:

| Term | Meaning |
|---|---|
| **Transcript** | The agent's complete record on disk — the source of truth. |
| **Live feed** | What the agent pane shows: the turn in flight plus the last few finished turns. Bounded. |
| **History** | The History tab: a static reader over the whole transcript. |
| **Roll off** | A finished turn leaves the live feed. Nothing is copied — it is already in the transcript, and History shows it. |

"Archive" is avoided: the code already uses it for session archiving
(`session:archive`). Older sections say "live pane", "live window" and
"evict"; they mean live feed and roll off.

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
4. **No loss.** Only a finished content node whose provenance is durable is
   evicted (`ephemeral` rows — §6.3.2, §6.9 — are not content):
   its **last** contributing transcript line (`src.endLine`) or journal line
   (`journalEnd`) is covered by that file's line count, so the History tab can
   show all of it (§6.3.1–§6.3.2).
5. **Layout consistency.** The layout store's `totalSize` equals the sum of
   effective heights of its rows; every mounted virtualized row's measured
   height equals its stored height within 1 px after the measure RO fires.
6. **Dormancy.** Hidden panes (dormancy gate) and hidden History tabs do no
   render or layout work.
7. **Input first.** No stream-driven task runs longer than the scheduler's
   slice while input is pending.
8. **Stable identity.** A node's id is a function of its source position and
   file generation only; any parser instance, restored from any checkpoint,
   gives the same content the same id and turn ordinal (§6.3.1).
9. **No resurrection.** A source line outside the pane's accepted ranges and at
   or below its high-water mark never re-enters the live pane, whatever replays
   it; ranges are per generation (§6.3.3).
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

**One node bigger than the ceiling** (Codex review, second round): the ceiling
only moves finished nodes, so a single growing markdown message or tool output
can exceed it on its own and stay fully mounted. Such a node is windowed
**inside its row**:

- **Markdown.** Past a threshold (proposed 64 KB), a growing markdown node's
  frozen prefix — the completed blocks #3559 already keeps as stable DOM — is
  rendered as block-level sub-rows in a nested virtual region: only the
  sub-rows in or near the viewport plus the live tail block are mounted.
  Sub-row heights feed the row's total height the same way rows feed the list
  (measured, else estimated), so the outer layout and the pin see one row of the
  right height.
- **Tool output.** `output-cap.ts` already bounds a body to 1,000 lines / 1 MB
  of characters. While running, the body is a virtualized line list: only the
  visible lines plus the latest lines are mounted, however many the cap allows.
- The row-internal window uses the same pin controller (§6.1) and height
  handoff (below) as the list, and the fault suite's 1 MB markdown and 5 MB tool
  output cases are its acceptance tests.

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
  priority — it does not run in a frame with pending input **until its
  deadline passes**; after that it runs in bounded slices even with input
  pending (§6.5, "Housekeeping deadline").

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
- Every such node carries `src: { line, endLine }` (new fields). `line` is
  the record that **started** it, so a node's id never changes as it grows;
  `endLine` is the **last** record that contributed to it, and moves forward
  as a text run, thinking run or tool result accumulates. Durability (§6.3.2)
  is judged on `endLine`, never on `line` (Codex review, third round): a
  completed node whose later records were received live but not yet written
  is not durable.
- Every such node also carries `turn`, the ordinal of the turn it belongs to
  (count of turn-starting `user_message` records before it in this
  generation), assigned by the parser from source position, so it is the same
  in every parser instance.
- **Parser checkpoints, not "clean boundaries"** (Codex review, fourth round).
  A boundary guessed from a few accumulators is not safe: the parser also holds
  pending tool calls and their timestamps, the current agent id, the replay
  flag and `hidingUntilNextUserMessage` (set after a hidden memory-reinjection
  message, `stream-parser.ts:224`), and the per-provider translator in front
  of it (`createTranslator(outputFormat)`) can hold state too. A parser started
  fresh mid-transcript loses all of that, and cannot know the turn ordinal.
  So the whole line-to-node pipeline (translator + parser) gets
  `checkpoint()` → a versioned, serializable snapshot of **every** field that
  affects output, including the turn ordinal, and `restore(checkpoint)`.
  - The live pane records a checkpoint at every turn start
    (`{ gen, line, turn, state }`) as it consumes the transcript, and persists
    them per block (`parser-checkpoints.jsonl`, written through a new
    append-only block-file RPC).
  - Any range parse — an older page, the History tab's tail recovery, a range
    read into the detached window — restores the nearest checkpoint at or
    before its first line and parses forward from there: identical ids, nodes
    and turn ordinals to a parse from line 0, at a cost of at most one turn of
    extra parsing.
  - A transcript with no checkpoints (written before this ships, or the file
    lost) gets its index rebuilt once by a full parse in housekeeping slices
    (§6.5), then cached; until it exists, range reads fall back to today's
    reparse-from-start behaviour, correct but slower.
  - Completeness is enforced by tests, not by care: the parser keeps all
    output-affecting state in one object whose keys a test compares with the
    checkpoint schema (adding a field without serializing it fails CI), and a
    property test parses real and synthetic transcripts (including hidden
    reinjection, tools spanning a turn edge and every provider format) from
    line 0 and from every checkpoint, requiring identical output.
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

- `transcript` (parser-produced): durable once the node is finished **and**
  the block's line count covers `src.endLine`.
- `journal`: the backend appends every out-of-band node event (shell create,
  chunk, exit; optimistic user message accepted) to a new per-block durable
  journal file, `out-of-band.jsonl`, with its own line numbers. A journal node
  carries `src: { journal, journalEnd }` — its first and last journal records
  (a shell's create, then its last chunk or exit) — and is durable once it is
  finished **and** the journal's line count covers `journalEnd`. The History tab reads the journal alongside the transcript
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

A single "evicted through line N" watermark is not enough (Codex review, second
round): the detached window (§6.3.5) evicts from the **middle** — between the
reading window and the present — and a truncate starts a new generation at
line 0. So the pane records what it holds as **source ranges keyed by
generation**:

- **Accepted ranges.** Per source (transcript, `out-of-band.jsonl`), the pane
  keeps `{ gen, ranges, high }`: `ranges` is the sorted, merged set of line
  intervals whose nodes the pane currently holds, each with the first and last
  turn ordinal it contains (`{ from, to, firstTurn, lastTurn }`, for the gap
  row, §6.3.5), and `high` is the highest line it has ever consumed in that
  generation. At most three intervals exist
  at once (reading window, present, and the in-flight tail when it is not
  contiguous with the present), so the state stays small.
- **Replay filter.** Before parsing, a line is kept if it is in `ranges` or
  above `high` (new content). Anything else — the evicted prefix, the evicted
  middle gap — is dropped. Because ids are positional (§6.3.1), kept lines
  that were already present dedup by id exactly.
- **Eviction and loading update the ranges in the same reduction** that
  removes or adds the nodes (`EvictRange`, `LoadRange`), so there is no instant
  where nodes and ranges disagree.
- **Generation change.** A line or read response carrying a newer generation
  atomically resets that source's state to `{ gen: new, ranges: [], high: -1 }`
  in the same reduction that handles the event, so no new-generation line is
  filtered by an old-generation range. What the pane does with the old
  generation's nodes follows the existing truncate/session-scope rules. A line
  from an **older** generation than the current one is dropped.
- **History tab cursor.** The tab's tail cursor is also `(gen, line)`; a
  generation change restarts its tail parser at line 0 of the new generation.
- **Ring replays.** A `shell_node_create` replayed from the backend's 64-event
  ring carries its journal position (added to the event), so the journal
  filter drops it if it is outside `ranges`. Until that field ships, a bounded
  tombstone set of the last 256 evicted out-of-band ids (4× the ring) drops
  them.
- The ranges are part of the pane's persisted state, so a remount doesn't
  reset them.

#### 6.3.6 Phase 5 groundwork: what exists today (investigated 2026-09-24)

§6.3.1–§6.3.3 assumed a transcript line number is already a stable address.
A read-only investigation of `main` (four parallel code surveys) found it is
not, and found durability gaps that exist today independent of eviction.
Phase 5 is re-planned on these facts.

**The transcript has no stable line identity.**

- Reads (`BlockfileLineCountCommand` / `BlockfileReadRangeCommand`) are served
  from the agent's **global zone** (`agent:<defId>:current`) whenever it is
  non-empty, else from the block's own `output` (`global_output_source`,
  `server/app_api/mod.rs`). The source can flip (zone cleared, `agentId` meta
  change, archive), and the two files number lines differently.
- Some records are appended **without an MPS event**
  (`persist_to_blockfile_silent` — the Claude persistent controller's own
  stdin user lines), so the live pane never sees them. Every block and channel
  of the same agent appends to the shared global zone, and concurrent mirrors
  can interleave.
- There is no generation or epoch. `output` is replaced or deleted by
  `session:archive` / `session:restore`, `agent:session:archive` and the
  one-time transcript backfill. There is no ring window: `total` is the
  all-time count (the "ring buffer window" comments in
  `useHistoryPagination.ts` are stale).
- Live events (`WSFileEventData`) carry a **byte** offset of the block's own
  file — not a line index, and not of the file reads come from — and the
  frontend drops it. History and live are joined only by node-id dedup.
- No generic per-block append RPC exists (`blockfile:write_state` replaces a
  whole file; `fileappend` has no backend handler).

**Node ids already differ between live and replay.** The live parser's
`skipIds` consumes counter values replay does not; replay does not flush text
accumulators at `session_end` (live does); several ids and timestamps come
from `Date.now()`. Positional ids are therefore also a correctness fix, not
only an eviction prerequisite.

**Tool node ids must not change.** `ToolNode.id` is the provider's
`tool_use_id`, and several joins depend on it: tool output chunks, the dock,
AskUserQuestion, replay merge, and the durable `db_background_tasks` table (a
mismatch leaves rows "running" forever, silently). Positional ids apply only
to the counter-minted kinds (text, thinking, agent and user messages, jekt,
errors). Nothing parses the id format; persisted collapse/pin sets fail
silently (a documented reset is enough); snapshot v1 embedded `nodes[]`
should be treated as a schema reset.

**Durability gaps today (independent of eviction):**

| What | Lost when |
|---|---|
| User messages, Gemini-family providers | Always on reload: the CLI's echo is in `output`, the translator drops it |
| User messages, Codex / Kimi / ACP | Always on reload: never persisted (ACP never even creates the node) |
| Shell blocks and their output | srv restart, or more than 64 shells in a block (memory-only MPS rings) |
| AskUserQuestion answer text | Always on reload (optimistic update only) |
| Heuristic compaction marker, `compaction_started`, stderr rows, system notifications, "Interrupted" | Always on reload (live-only) |

**Revised Phase 5 plan** (each a separate PR, each shippable alone):

| Step | What | Where |
|---|---|---|
| 5a | Fix the transcript address at the source: one authoritative stream per pane, a **generation** that changes whenever it is replaced, deleted or re-sourced, and the **absolute line index** (of that stream) on every live event, including records written today without an event. Line-count and range-read responses carry the generation. | Backend (+ event type) |
| 5b | Positional ids `G<gen>L<line>[.k]` for counter-minted kinds only; `src` / `endLine` / `turn`; replay flushes like live; `skipIds` removed; persisted collapse/pin reset; snapshot v1 treated as a schema reset. | Frontend parser |
| 5c | Parser and translator `checkpoint()` / `restore()` with the completeness test; checkpoints persisted per block (needs an append RPC, added in 5a or here). | Frontend (+ RPC) |
| 5d | Durability for out-of-band nodes: render the Gemini-family user echo (a bug fix that stands alone); journal (`out-of-band.jsonl`) for shells, AskUserQuestion answers and user messages of providers without an echo. *Revised by §6.9:* user messages go into the transcript itself (#3701 for the subprocess controller; ACP and the Codex app-server controller still to do), AskUserQuestion answers need no journal (already in the tool result; the dead-air re-delivery is written to the transcript since #3703) — the journal is for in-pane shells only. | Frontend + backend |
| 5e | Accepted ranges and the replay filter (§6.3.3). | Frontend |

Order: 5d's Gemini echo fix can land any time (shipped in #3620); 5a before
5b and 5c; 5e last, just before Phase 6. 5a's design is §6.3.7.

#### 6.3.7 Phase 5a design: one addressable transcript stream (2026-09-24)

**Goal.** Every record a pane parses has an address `(stream, gen, line)`
that is the same live, on reconnect replay and in the History tab, and that
never names different content twice. The pane's parser consumes each
stream's lines **exactly once, in order, with no gaps**. Live events, replays
and range reads all deliver into that one contract. Nothing in 5a changes
node ids (5b), and nothing is evicted.

**What the write path does today** (read-only survey of `main`, in addition
to §6.3.6; paths under `agentmux-srv/src/`):

- **No transactions.** `FileStore` has one `Mutex<Connection>` per store
  (`storage/filestore/core.rs:36`) and no SQLite transaction anywhere.
  `append_data_at` (`core.rs:484`) reads the size, writes the 64 KB parts and
  updates the size as separate autocommit statements. The comments that say
  "single-tx" (`helpers.rs:24`, `blockfile.rs:449`, `v1_templates.rs:593`) are
  wrong.
- **Several processes append to one global zone.** The global store
  (`<shared>/agents/transcripts/filestore.db`, `bootstrap.rs:738`) is opened
  by every srv instance on the machine; the in-process mutex doesn't cover
  them. Two instances appending to one agent's zone can both read size `S`
  and overwrite each other's bytes, and a crash between statements leaves a
  torn append. This is a data-loss risk today, independent of this spec.
- **`stat()` answers from a per-process cache** (`core.rs:299`) that is never
  flushed, so one process can see another's append or delete late.
- **Several writers per block, even in one process.** The controller's stdout
  reader, the reactive jekt echo (`server/reactive.rs:151`) and error frames
  all append to the same block. Error frames skip the global zone
  (`agent_handlers/input.rs:764,983`, `app_api/agent_io.rs:234`,
  `host_spawn.rs:737`, `container_spawn.rs:326`), and so does the app-server
  controller (`app_server_controller.rs:181`) whose reads still switch to the
  global zone. The block file and the global zone therefore hold different
  line sequences.
- **The event goes out before the write** (`shell/file_ops.rs:201`) with an
  offset from a stat taken before it (`:163`). The global mirror publishes
  nothing, on any scope, so a pane never sees another block's lines live.
- **Line numbering is the reader's rule:** non-blank lines under
  `trim().is_empty()`, CRLF-aware (`shell/indexing.rs:14`). `output.idx` is a
  lazily rebuilt reader cache with no lock around scan-then-write. Live callers
  append exactly one non-blank, `\n`-terminated line; the RPC
  `append_session_output` (`agent_session/session_io.rs:137`) takes any text.
- **Eight paths delete or replace `output`** with no common choke point:
  `archive_session` and `clear_global_current_zone`
  (`agent_session/archive.rs:31,275`), `archive_session_output` and
  `restore_session_output` (`backend/session_archive.rs:99,189`), the
  transcript backfill (`backend/transcript_backfill.rs:143`), the m0003 zone
  move (`agent_session/migrations/v1_templates.rs:419`), `blockfile:write_state`
  (`app_api/blockfile.rs:437`), and the dead `handle_truncate_block_file`.
  Several leave `output.idx` / `output.tsidx` behind, which then describe
  content that no longer exists.

**Design.**

1. **Atomic `FileStore` mutations.** Every mutation (`append_data_at`,
   `make_file`, `write_file`, `delete_file`, `delete_zone`, `write_meta`) runs
   in one `BEGIN IMMEDIATE` transaction on the store's connection. The existing
   `busy_timeout=5000` makes a second process wait instead of failing. This
   alone fixes the cross-process overwrite and the torn append, and ships
   first.
2. **Generation = a nonce minted by the store** (as built in 5a-2a,
   `filestore/counter.rs`). `db_wave_file.gen` holds 64 random bits. It
   isn't a counter, because a counter kept on the row is lost with the row
   and would restart, and a nonce needs no coordination between processes.
   Different files have different nonces, so a pane whose reads move from the
   global zone to the block file sees a different `(stream, gen)` and treats
   it as a new generation.
3. **Line counter on the row, as one epoch with `gen`.** The columns `gen`,
   `lines` (the count under the reader's rule, one shared predicate with the
   indexer), `lines_size`, `lines_tail` (where the last, possibly
   unterminated, line starts) and `lines_modts` are one **epoch**: all
   valid, or all NULL.
   - **Starting an epoch.** An epoch starts, with a fresh `gen`, only when the
     count can be vouched for from byte 0: `make_file` (empty), `write_file`
     (counts the data), and `init_line_counter` for a row that has none.
     `init_line_counter` scans the file one 1 MiB window at a time, holding
     the connection lock per window, so appends continue. It then counts
     what they added in one transaction. It gives up if the scanned bytes may
     have changed underneath it:
     - the row was re-created: its `incarnation`, 64 random bits set by an
       insert trigger for every writer, differs even within one millisecond;
     - any writer, older builds included, rewrote bytes (`rev`, bumped by
       the database's triggers for every write that isn't an append);
     - as a second line of defence, the file shrank or its scanned tail
       changed.
   - **Appends** advance the epoch in the same transaction as the write,
     re-reading only the unterminated last line (normally empty).
   - **`append_lines`** normalizes to complete, non-blank, `\n`-terminated
     lines. It first closes a torn tail with `\n`: the torn line keeps its
     index, whereas today the next record fuses with it and both become
     unparseable. It returns `AppendPos { offset, counted: { gen, first_line,
     lines } }`.
   - **Mixed versions.** Older builds share the global store and write
     without maintaining the epoch. There is no `FILESTORE_SCHEMA_VERSION`
     bump: a bump would make them refuse to open the store. The columns are
     added with `ALTER TABLE ADD COLUMN`, so an older build's rows are
     NULL, i.e. not counted.
     - **Detecting their writes.** Older builds can't avoid the database's
       own triggers, which fire for every connection. Any write that changes
       bytes already in a file bumps `rev`: a part deleted, overwritten with
       anything but a pure extension, or inserted inside the size the file
       already claims (an older build's `write_file` raises the size before
       inserting parts). Appends extend the last part, or add parts at or
       past the size, so they don't bump it.
     - **Bytes must be stored.** The counter never counts a byte that the
       size claims but no part holds yet. A missing part makes init give up,
       and makes an append drop the epoch.
     - **Validity.** An epoch records the `rev` and size it was counted at,
       so an append by someone else (size moved) or a rewrite (`rev` moved)
       invalidates it.
     - **Timestamps alone were not enough.** Two writes can share a
       millisecond, so an older build's same-size replace could go unnoticed
       (Codex on #3631).
     - **Recovery.** The epoch is dropped, never patched, and the next one
       gets a new `gen`, so no line index is reused for different content.
       The cost is a resync while builds are mixed.
     - **Amended after 5a-4: an epoch that is only behind is caught up.**
       - **Measured on a live dev instance:** a 1M-line agent zone that the
         production (older) build appends to all the time. Every one of those
         appends made the next reader:
         - re-count the whole file: 1.4 s;
         - mint a new `gen`: the pane's poll never matched it, and every
           `expectGen` read failed;
         - rebuild `output.idx` in full, since the index is labelled with the
           generation. A 200-line `read_range` took 2.6–3.3 s.
       - **The triggers already prove the prefix unchanged.** `rev` moves on
         any rewrite of existing bytes and never on an append. So when `rev`
         matches and the size only grew, the counted bytes are unchanged.
       - **Catch-up.** Such an epoch is counted from the start of its last
         open line and keeps its `gen`:
         - inline in an append, up to 1 MiB;
         - in `init_line_counter`, windowed like the full scan;
         - via `catch_up_line_counter` on the read paths, which never starts
           a full count.
       - **A rewrite still drops the epoch.**
       - **`read_range` extends `output.idx`** from its last line instead of
         rebuilding it. That's what `line_count` did before it answered from
         the counter in 5a-3b.
   - **Legacy rows.** 5a-3 calls `init_line_counter` from the line-count path
     (off the runtime) the first time a pane opens a legacy row. That costs
     one read of the file per epoch. Until then, events for that stream carry
     no position.
4. **Replace and delete are atomic with the sidecars; the index is
   labelled** (as built in 5a-2b, `filestore/replace.rs`).
   - **Atomic replace and delete.** `replace_file(zone, name, data, drop)`
     and `delete_files(zone, names)` each run in one transaction. The
     transcript paths use them, so a crash can't leave `output.idx` /
     `output.tsidx` describing an `output` that is gone:
     - archive and clear delete `output` together with both sidecars;
     - restore and the transcript backfill replace `output` and drop both
       sidecars in one step. A reader never sees `output` missing or
       half-restored, and the replaced content gets a new `gen` (item 3).
   - **The index names its generation.** `output.idx` records the
     generation it was built for (`for_gen` in its file metadata, written in
     the same transaction as the index). The label is taken before the scan,
     so a replace during the scan leaves a mismatching label.
     - **When the index is trusted.** The `read_range` fast path, the
       global line count and `extend_output_idx` trust an index only when its
       label matches `output`'s valid generation. Any mismatch just triggers
       a rebuild, which is always safe.
     - **This covers every path, listed or not.** Today, `output` replaced
       by content that happens to reach the old covered size is read at the
       old file's offsets. A test reproduces that.
   - **Sizes and scans come from the database.** Those readers and the
     index builder take `output`'s size from the database (`line_state`),
     and read bytes with `read_bytes_db`, instead of this process's `stat`
     cache. Another srv instance's appends leave that cache stale, and the
     builder could otherwise record coverage it never scanned.
   - **`blockfile:write_state` refuses the transcript names.** It refuses
     `output` and both sidecars; it is for pane state files.
   - **The zone move needs no change.** Copied files get a new generation,
     and a copied index has no matching label.
   - **Replace/delete stream events** (`fileop: "replace" | "delete"`, new
     `gen`, so an open pane learns at once) move to 5a-3, with the other
     events.
5. **Events carry positions and go out after the write.** As built in
   5a-3a, in `shell/file_ops.rs::append_transcript`:
   - **Transcripts only.** Terminal (`term`) data still publishes before its
     write: publishing after would add a database write to every keystroke
     echo.
   - **Whole lines.** Both writes use `append_lines`, so a torn tail is
     closed rather than continued. The k-th non-blank line of the event's
     `data64` is record `line + k`.
   - **Ordering and offset.** The per-block lock is striped (64 mutexes):
     bounded memory, nothing to clean up. The event's `offset` is now the
     exact landing offset, not a pre-append stat.
   - **Latency (measured, release build, Windows, 3 × 3,000 lines of ~1 KB,
     `transcript_event_latency`).** Three changes were needed to meet the
     gate:
     - **One commit per stream.** Each line and its `output.tsidx` stamp are
       written in one transaction (`append_lines_stamped`), so two commits
       per line instead of four.
     - **`synchronous=NORMAL`** for `FileStore`, as the object store and saga
       log already use. The user chose this over full fsync durability on
       2026-09-24: it can't corrupt the database, but an OS crash or power
       cut can lose the last commits.
     - **Background checkpoints.** Every append rewrites a whole 64 KB part,
       so SQLite's automatic checkpoint ran about every 60 lines, inside a
       write. `FileStore` connections now run `wal_autocheckpoint=0`, and a
       thread with its own connection runs PASSIVE checkpoints past 4 MB.

     | per line | p50 | p99 |
     |---|---|---|
     | before 5a-3a: four commits (the event went out first; the stdout reader waited this long before its next line), `synchronous=FULL` | 7.1–8.1 ms | 21–41 ms |
     | 5a-3a, event waits: block file only | 0.69–0.74 ms | **1.6–1.7 ms** |
     | 5a-3a, event waits: block file + global zone | 1.25–1.30 ms | 3.0 ms |

     Gate met: p99 added latency ≤ 2 ms on the local store. Rare single
     outliers of 1–2 s occur with and without these changes.

   `WSFileEventData`
   gains `pos: [{ stream, gen, line, lines }]`, where `stream` is
   `b:<blockId>` or `g:<zone>`. It carries one entry for the block file and one
   for the global zone when the mirror succeeded. `offset` stays for terminal
   consumers. A per-stream in-process mutex is held across append and publish,
   so one process publishes a stream's events in line order. The stdout reader
   already calls both appends inline before it reads the next line
   (`subprocess/host_spawn.rs:382`), so throughput is unchanged. Each event is
   delayed by the append time, measured before and after (gate: p99 added
   latency ≤ 2 ms on the local store; the global store is reported
   separately). Those inline appends are blocking SQLite calls on a tokio
   worker, and another process holding the global database can already stall
   that worker for up to the 5 s busy timeout. If the measurement shows it,
   each stream gets one writer task fed by a queue (`spawn_blocking`). The
   queue keeps publish order without the mutex and takes SQLite off the
   runtime.
   - **Silent persists publish.** As built in 5a-3c:
     - `persist_to_blockfile_silent` becomes `persist_user_line`: a normal
       transcript append (positions, `append_lines`) whose event carries
       `echo: "stdin"`.
     - The pane (`useAgentStream`) skips echo events, so it adds no second
       node, but the stream has no gap where the record sits.
     - The event carries no message id. A queued message is persisted at the
       moment the drain delivers it, where only its JSON is held, and
       threading the id through the delivery queue would change retry and
       drain code that isn't this spec's.
     - In 5b the pane matches the echo to its optimistic node by content and
       gives that node the line's positional id, so it becomes `transcript`
       provenance (§6.3.2).
   - **Every record a pane shows live is in its stream.** As built in 5a-3c:
     - Error frames at all seven sites (`agent_handlers/input.rs` ×2,
       `app_api/agent_io.rs`, `subprocess/host_spawn.rs`,
       `subprocess/container_spawn.rs` ×2) and the app-server controller now
       resolve and mirror to the global zone like every other writer.
     - This was a bug before: a pane with an `agentId` reloads from the global
       zone, so frames written only to the block file vanished on reload,
       despite comments saying they must persist.
     - If the mirror itself fails, the event has no entry for the pane's
       stream. The pane then renders the record as non-durable (pinned, never
       evicted) and counts it in the dev HUD (5a-4).
6. **Reads name what they served.** `BlockfileLineCountCommand` and
   `BlockfileReadRangeCommand` responses gain `{ stream, gen }`, read from
   the database inside a read transaction rather than the per-process
   cache. `read_range` accepts `expectGen` and answers `genMismatch` rather
   than returning another generation's lines, so a replace landing between a
   count and a read can't mix two files. As built in 5a-3b:
   - **Stream names.** The stream is `g:<zone>` when the read is served from
     the agent's global zone, `b:<blockId>` otherwise.
   - **Line count.** It answers from the counter (O(1)). A legacy row gets
     `init_line_counter` first, on the blocking pool; only a file that can't
     be counted falls back to the index. The local-block path now counts the
     file too, instead of the `session:line_count` meta, so it matches what
     `read_range` serves.
   - **Proving the generation.** `read_range` reads the generation before
     and after its lines; only an unchanged value proves the lines belong to
     it, and only then does the response name it. With `expect_gen`, a
     change is `gen_mismatch` (`lines: []`).
   - **Replace and delete events.** `session:restore` publishes
     `fileop: "replace"` with the new generation and line count;
     `session:archive` publishes `fileop: "delete"`.
     `agent:session:archive` is keyed by agent definition, not block, so its
     event waits for the consumer design in 5a-4. Until then a pane detects
     the change by the generation mismatch on its next read. (5a-4: needs no
     event. The cursor joins the recreated file at its first record; see
     item 7.)
7. **Consumer contract (frontend part of 5a).** Per pinned `(stream, gen)`
   the stream hook keeps `next`, the next line it expects:
   - `line < next`: duplicate, dropped.
   - `line == next`: parsed.
   - `line > next`: gap. The hook reads `[next, line)` from the same stream
     with `expectGen`, parses it, then continues. Events arriving during the
     fetch queue behind it.
   - A new `gen` or `stream`, or a `replace`/`delete` event, goes through
     today's truncate / session rules (and resets the accepted ranges in 5e).
   - The existing line-count poll also fills `lines > next`. That is how
     lines appended by another block, channel or srv instance reach a live
     pane. Today they only appear after a reload.

   The parser still assigns today's ids in 5a. Its input just becomes
   `{ stream, gen, line, text }`, ready for 5b.

   As built in 5a-4 (`frontend/app/view/agent/transcript-cursor.ts`, wired in
   `useAgentStream`):
   - **Which stream.** The cursor pins the stream the history load was
     served from (`read_range`'s `{ stream, gen }`), with `next` at the end of
     the lines it showed. The v2 restore and the NDJSON load settle it through
     a latch (`createTranscriptSettleLatch`), created by `agent-view.tsx` and
     passed to both hooks. Before a pin, an event is placed in its `g:` entry
     if it has one (reads are served from the global zone whenever it has
     content), else its `b:` entry.
   - **Held until history settles.** Live events queue until the load says
     where it ended. Parsing them first would put live records above the
     history they follow. The pane is covered by its loading overlay until
     then. A load that never reports releases them after 15 s, placed from the
     first event on.
   - **Outcomes the load can report:**
     - a pin;
     - `"empty"`: nothing to show, so a first event past line 0 is a gap from
       0;
     - `null`: the load failed, a v1 snapshot, or an uncounted file. The
       cursor starts at the first event it sees.
   - **Gaps.**
     - Read with `expectGen` in chunks of 1,000.
     - Any mismatch (`genMismatch`, another stream or generation served)
       skips the gap instead of mixing files.
     - A gap over 5,000 lines is skipped too: those lines stay on disk for
       the next load.
     - Skipped lines are counted.
   - **Echoes.** Echo events aren't parsed. An `EchoLedger` pairs them by text
     with the user-message nodes this pane shows. A gap line that is the
     record of such a node (its echo event was lost in a socket drop) is
     dropped rather than becoming a second bubble. 5b replaces the text match
     with positional ids.
   - **Generation change without an event.** The cursor joins the new
     generation at the event's own line and never fills from the new file at
     the old one's line. A poll count in another generation is ignored until
     an event arrives.
     - **Why (amended in #3663):** as first built, a larger new generation
       kept `next` on the theory that it was a re-count. Codex showed that
       generation and position can't tell a re-count from a replacement, so
       that rule could splice another history into the pane.
     - **What's left:** older builds' appends no longer change the generation
       (the counter is caught up, item 3), so a new generation now means a
       rewrite or a new file.
     - **The agent-level archive:** `agent:session:archive` deletes a shared
       zone and has no block to announce it on. It reaches an open pane this
       way too: the next session's first record joins it.
   - **Resets.**
     - `truncate` keeps today's reducer-gated `StreamTruncate`.
     - `replace` (restore) and `delete` (archive) only move the cursor, and
       the pane keeps what it shows. Neither published anything before 5a-3b.
     - After a `replace` the restored content counts as history: the cursor
       joins after it.
   - **Unpositioned events** (no counter, a failed mirror, a pinned stream
     missing from the event) are parsed as before and counted.
   - **Migration.** A pane pinned to its block file follows the global zone
     once events carry it, at a contiguous point. That happens when the
     agent's first write to the zone comes after the pane loaded.
   - **The poll is new, not existing.** No line-count poll existed.
     - A pane pinned to a `g:` stream checks the count every 5 s while the
       document is visible.
     - It fills up to the count seen one tick earlier, so lines its own
       events are still bringing arrive by event first.
     - A `b:` stream has one writer (its block), so it needs no poll.
   - **Dev HUD.** Each pane's counters appear under "Transcript cursors" in
     the diag panel's agent-pane section: delivered, duplicates, echoes, gaps
     filled, lines skipped, unpositioned, generation changes, own echoes
     dropped.

**Pull requests** (each shippable alone, in order):

| PR | Scope | Proof |
|---|---|---|
| 5a-1 | `BEGIN IMMEDIATE` for every `FileStore` mutation; fix the "single-tx" comments | Two `FileStore` instances on one database file, many threads each appending numbered lines: every line present exactly once, intact. Append latency before/after. |
| 5a-2a | `gen` + counter epoch columns (no schema bump), `append_lines`, torn-tail repair, `init_line_counter`, the shared line rule | Property test: the counter equals the `output.idx` indexer on random bytes (blank, CRLF, VT, broken UTF-8, torn tails). Concurrent `append_lines` from two stores hand out distinct indices that address their records. An older build's write drops the epoch; a re-count gets a new `gen`. A database created by an older build opens concurrently, gains the columns and counts. |
| 5a-2b | `replace_file` / `delete_files` (one transaction) on the archive, clear, restore and backfill paths; `output.idx` labelled with its generation and trusted only on a match; index sizes and scans read from the database; `write_state` refuses transcript names | A same-size replace is no longer served the old index (reproduced first). A failed replace or delete changes nothing. The indexer labels what it read. Full `agentmux-srv` suite. |
| 5a-3a | Transcript appends written first (`append_lines`, block file and global zone) and published after under a striped per-block lock, with `pos` per stream and the exact offset; terminal data unchanged | Each event carries both streams' positions, numbered independently; the record is on disk when its event arrives; concurrent writers to one block publish in line order and each `line` addresses its record; a torn tail is closed and keeps its index. |
| 5a-3b | Read responses name `{ stream, gen }`; `read_range` takes `expectGen`; the line count answers from the counter; a legacy row gets `init_line_counter` off the runtime; `replace` / `delete` events | Count from the counter equals the indexer's; a replace between count and read returns `genMismatch`. |
| 5a-3c | The stdin user-line persist publishes as an `echo: "stdin"` transcript event with positions (the pane skips it); error frames (seven sites) and the app-server controller mirror to the global zone | The persisted user line fills its line (no gap) and is marked as an echo; the full suite; `tsc`. |
| 5a-4 | Frontend consumer contract: `TranscriptCursor` pinned by the history load, gap reads with `expectGen`, echo ledger, the shared-zone poll, HUD counters | Out-of-order, duplicate, gap, `gen` change mid-fetch, recreated file, cross-process append via the poll (unit); `full-conversation-bench` streaming cost unchanged. |

**Known, not changed here.** Two live sessions of the same agent (for
example, one in each of two srv instances) interleave records in one global
zone, and a single parser then sees two sessions' records mixed. Replay does
this today. 5a makes live do the same instead of hiding it. How often it
happens in practice isn't measured. 5a-3 counts, per zone, appends whose
writer (block or process) differs from the previous append's writer less than
60 s earlier.
Separating them (a per-writer tag on each record) is a later decision.

**Coordination.** 5a-3 changes the persistent controller's user-line persist
(`persistent/queue.rs:598`) and its caller in `persistent/input.rs`, the area
`maricon/no-midturn-delivery-impl` is reworking. 5a-3 goes after that branch
lands, or rebases onto it. The identity work (M4) doesn't touch these files.

#### 6.3.4 Eviction while pinned

- **When:** at turn end (housekeeping priority, §6.5) while pinned.
- **What:** whole turns from the front, oldest first, whose nodes are all
  durable (§6.3.2), until within budget.
- **How:** reducer command `EvictRange { transcript, journal }` removes the
  turns in one pass (the same shape as `clampToSessionScope`) and removes their
  line intervals from the accepted ranges (§6.3.3) in the same reduction, so
  there is no instant where a node is gone but its replay is still accepted.
  An eviction slice (§6.5) is one or more whole turns. The layout store prunes ids no longer
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
  **gap row** between them says "N newer turns — jump to latest". N comes from
  turn ordinals, not line counts (turns span arbitrary numbers of lines, in
  both sources): the accepted-range state (§6.3.3) records the first and last
  `turn` of each retained range, so N = the present's first turn − the reading
  window's last turn − 1, with no reparse of the gap (Codex review, third
  round).
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
- **Tail parser lost** (tab remounted, parser error): restore the most recent
  parser checkpoint (§6.3.1) at or before the last line read — at most one
  turn of reparse — and upsert. Positional ids make the overlap idempotent.
- **Older pages** use their own parser restored from the nearest checkpoint;
  with
  positional ids, pages loaded in any order no longer collide, which retires
  the "reparse all loaded lines per page" workaround.
- **Closed tab:** nothing; it loads fresh on open.
- The History tab gets the same A/B/D treatment as the live pane (it reuses
  `AgentDocumentView`), so a very long history read is equally cheap.

### 6.5 (E) Cross-pane stream scheduler with input priority

Today each pane's rAF queue flushes independently and to completion. Replace
with one app-wide scheduler that owns *when* stream work runs:

- **As built (Phase 2): a panes-per-frame cap, not a script budget.** With no
  user input in the last 150 ms, every pending pane flushes in the next frame
  (unchanged behaviour). While the user is interacting (key, pointer, wheel,
  text input, IME composition), one pane flushes per frame, oldest request
  first — two once the oldest has waited 100 ms, never more, so a long queue
  catches up without recreating a multi-pane frame. The originally proposed per-frame *script* budget (8 ms,
  `scheduler.yield()` between chunks) was dropped on measurement: the cost is
  layout in the frame's rendering step, roughly constant per pane per flush,
  which a script budget cannot see
  (`TRACKING_AGENT_PANE_BOUNDED_LIVE_WINDOW_2026_09_23.md` §3.1). Bounding how
  many panes' updates land in one frame is what bounds it.
- **Coalescing, not dropping:** a pane that misses a frame merges more tokens
  into its next flush. No data is lost; only paint cadence of the stream
  changes under load. The pinned view still ends at the latest content.
- **Priorities:** input handlers > stream flush of focused pane > other
  visible panes > housekeeping (migration, eviction, History-tab catch-up).
  Housekeeping does not run in a frame with pending input **except** under its
  deadline (next bullets): that exception is the only one, and it applies
  everywhere this rule is referenced (§6.2, §6.3.4).
- **Starvation guard:** every visible pane gets a flush at least every
  100 ms, even under continuous typing.
- **Housekeeping deadline** (Codex review, second round): "never with pending
  input" alone would let sustained typing or IME postpone migration and
  eviction forever, and the bounded-window invariant with them. Each
  housekeeping job has a deadline (proposed 500 ms after it became due) and
  runs in **bounded slices** (proposed ≤ 4 ms each, one per frame) once the
  deadline passes, even with input pending — a cost well inside the key→paint
  budget. A hard backstop runs it as soon as the live document exceeds 1.5×
  its budget. Slices are resumable: a migration or eviction is split per turn,
  and each slice leaves the stores consistent (every invariant holds between
  slices).
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

### 6.9 Revised plan: the live feed and roll-off (2026-09-24)

**Why revise.** §6.3–§6.4 keep up to 30 turns / 2 MB live and let the reader
scroll back through the live pane (paging older content in, and a detached
reading window with a gap row). That is what needs positional ids (5b),
parser checkpoints (5c) and multi-range replay filtering (5e) first. The user
asked for something more aggressive and simpler: keep the live feed small so
live appending stays fast, and do the reading in History, which is static and
therefore smooth even when large. With scroll-back out of the live feed, the
prerequisites shrink to what already shipped:

- **No resurrection.** Since 5a-4 the pane places every transcript record by
  line (`transcript-cursor.ts`): anything below the cursor's `next` is a
  duplicate and dropped, including a reconnect replay. A rolled-off turn is
  below `next`, so it cannot come back — whichever turns were removed, so
  removal need not be a prefix (a blocked turn can stay while turns around it
  go, below). No accepted-range state is needed: §6.3.3's ranges exist for
  range reads into a detached window, which this plan drops.
- **No range parses in the live feed.** It no longer pages older content in,
  so there is no second parser whose counter ids could collide (§6.3.1's
  reason for positional ids in the live pane).

**The live feed.**

- Holds the turn in flight plus the last **K finished turns**
  (`agent:livefeedturns`, default 3, minimum 1), and never less than one
  finished turn, so the reply just received always stays. A byte ceiling
  (1 MB of finished turns, a constant) rolls off earlier when turns are huge,
  still keeping at least one.
- **A turn** starts at a `user_message` node (parser-produced or optimistic);
  nodes before the first one form a leading turn. Same rule as
  `turnScopedFrontier` (`frontend/app/view/agent/virtualization/streaming-buffer.ts`,
  Phase 3b, §6.2). A turn is **finished** when it is not the
  last turn and none of its nodes is in progress (`isNodeInProgress`, same file).
- **Kill switch:** `agent:livefeed` (default true). Off restores today's
  behaviour exactly, paging included.

**When turns roll off** — never mid-render, never mid-turn:

- when a turn ends (the edge of `turnJustEndedAtom`,
  `frontend/app/view/agent/agent-view.tsx`);
- when a new `user_message` enters the feed (the next send);
- when the pane becomes dormant (hidden tab or window tab);
- once after the initial history load, so a pane opens with K turns, not the
  load window's worth.

Each point schedules one roll-off pass off the input path
(`requestIdleCallback`, 1 s timeout — the stream scheduler,
`frontend/app/view/agent/stream-scheduler.ts`, has no housekeeping lane yet).
If the user typed in the last 150 ms the pass is **re-queued**, never dropped:
it runs at the latest at the 1 s deadline, even with input pending, so a fast
last turn can't leave overdue turns resident waiting for a next trigger
(Codex review). A pass is a single reducer command, O(nodes kept).

**Which turns may go.**

- **Only rows wholly above the viewport.** While pinned this is the whole
  front of the feed except what is on screen; a short turn still visible at
  the top stays until new content scrolls it out. Removing rows above the
  viewport while pinned moves nothing on screen (the pin holds the bottom).
- **While the reader is scrolled up in the feed**, the feed keeps the turns
  intersecting the viewport, the newest K finished turns, the turn in flight
  and blocked turns; every other eligible turn rolls off at the usual points —
  those **below** the viewport (between what is being read and the newest K)
  move nothing on screen, and those **above** it keep the first visible row at
  the same offset (the anchor mechanism the virtual list already uses for
  prepends, applied to a front removal). A reader parked at the top therefore
  holds their slice plus K turns, not everything since (Codex review).
- **Only turns whose content is reproducible from the transcript.** A turn
  holding any of these is **blocked**:
  - `shell` nodes (AgentMux's in-pane shell runs, `useShellNodeStream.ts` —
    not the agent's Bash tool, which is a durable `tool` node; backend memory
    ring only, §6.3.2),
  - an optimistic `user_message` not yet paired with its echo.

  *Revised in PR 3 (ReAgent review):* an answered AskUserQuestion is **not**
  blocked. The answer reaches the transcript as the tool's result (the CLI
  writes "User has answered your questions: …" when the turn resumes), so
  History shows it; only the styled rendering (`answerText` / `questionText`)
  and the client-side auto-fill note are optimistic — and those fields are
  never cleared, so blocking on them would keep every such turn forever.
  Rebuilding the styled rendering on replay is a parser follow-up.
  The one path where the CLI writes no tool result — the dead-air fallback
  that re-sends the answer (or a decline, or a tool-permission decision) as a
  follow-up stdin line when the CLI abandoned the pending call — now writes
  that line to the transcript too, like every other stdin line (#3703, Codex
  review), so every answer path is durable.
- **A blocked turn is never rolled off — and does not stop the others.** It
  stays where it is; durable finished turns before and after it still roll
  off. Removing from the middle is as safe as removing a prefix here: the
  live cursor never re-delivers a line below its position, whatever was
  removed, and the feed does no range parses (Codex review of this revision:
  a blocker must not stop every later removal). Where kept turns are no
  longer contiguous, a small synthetic row between them reads "N turns in
  History". The feed therefore holds K finished turns plus the blocked ones —
  bounded by what blocks (the backend keeps at most 64 shells per block
  anyway) and visible in the dev HUD. The shell journal (§6.3.2, PR 4b below)
  makes shells durable and removes the blocked case. (Answered AskUserQuestion
  tools are not blocked at all — see the PR 3 revision just above.)
- **Live-only decoration rows are ephemeral, not content.** stderr rows,
  system notifications, "Interrupted", heuristic compaction markers and
  `compaction_started` join §6.3.2's `ephemeral` class (working indicators,
  synthetic rows): never in the transcript, dropped by every reload today,
  never shown by History. They roll off with their turn; invariant 4 and its
  runtime assertion apply to content, and `ephemeral` is outside it by
  definition (Codex review).
- **Providers whose user messages never reach the transcript** (Codex, Kimi,
  ACP — §6.3.6's durability table) would have every turn blocked by its first
  node. For them roll-off is **off** until the journal (PR 4) records user
  messages: they keep today's behaviour exactly, which is no regression.
  Claude and the Gemini family (whose echo shipped in #3620) are covered from
  PR 3 (Codex review) — Claude only under the **persistent** controller: the
  per-turn subprocess controller (muxcode, container agents) writes the
  prompt to the CLI's stdin without persisting it, so those panes are in the
  "never reach the transcript" group too (found while building PR 3).
  *PR 4a:* the subprocess controller now writes each message it sends as
  the same `{"type":"user",...}` record + `echo: "stdin"` event (not for
  Gemini, whose CLI echoes it), and the Codex and Kimi translators render it
  on replay — so Codex, Kimi and subprocess-Claude panes roll off too. ACP
  and the Codex app-server controller remain excluded until they write the
  same record (PR 4c below).

**How.** Reducer command `RollOff { ranges }`
(`frontend/app/store/agent-document/reducer.ts`): removes whole turns given as
index ranges — usually one prefix, more only around a blocked turn — in one
pass, rebuilding the id set and index as `clampToSessionScope` in that file
does, and emitting `turns-rolled-off { removedCount, turns }`. The layout store already prunes
ids no longer present (`NodesChanged` → `pruneMap`,
`frontend/app/store/agent-pane-layout/reducer.ts`); the document state's
collapsed / pinned / expanded id sets are pruned in the same pass. The pane
keeps a count of turns rolled off since mount.

**The top of the live feed.** Paging older content into the feed is off
while `agent:livefeed` is on (`onLoadOlder` not passed, `hasOlderHistory`
false; `frontend/app/view/agent/hooks/useHistoryPagination.ts`). The existing
`history_link` row (`frontend/app/view/agent/inject-history-link.ts`) shows whenever anything rolled off or
the load didn't start at line 0, and reads "N earlier turns · open in
History" when N is known (turns rolled off since mount, and the load started
at line 0), otherwise "Earlier turns · open in History". Clicking it opens or
focuses History as today. Opening History *at* the first turn still in the
feed needs node identity shared across parsers (5b) and is deferred; History
opens at its end, which is where the rolled-off turns are.

**History follows the transcript** (replaces §6.4's design with the same
goal):

- The History tab (`frontend/app/view/agent/history/AgentHistoryView.tsx`)
  keeps **one incremental parser** (`HistoryParser`, the body of
  `parseHistoryLines` in `frontend/app/view/agent/parseHistoryLines.ts` made
  resumable) for the range it has loaded; new
  lines go through the same instance, so open text runs continue and counter
  ids don't collide. `parseHistoryLines` becomes "a `HistoryParser` fed once",
  and a test requires feeding in arbitrary chunks to give identical nodes.
- It reuses `TranscriptCursor` (`frontend/app/view/agent/transcript-cursor.ts`)
  as-is on the source block's output file subject: settled from its own
  initial read (`historyPin`, same file), gaps filled by
  range reads (a gap it won't fill makes History reload — "No silent holes"
  below), duplicates dropped, the same 5 s line-count poll for other
  writers to the agent's shared zone while visible. Echoes are parsed (they
  are the user's messages; History has no optimistic copy).
- Parsing an append is O(new lines). Publishing is coalesced to at most once
  a second and costs O(loaded nodes) — day dividers are re-derived and the
  virtual list takes a new node array, as the live feed's stores do per flush
  (the class of cost Phase 4 removes, deferred on measurement). The History
  PR records the publish cost at 200 turns in the tracker; if it shows as a
  stall, dividers and the document become incremental there (Codex review).
- **A dormant tab does no work** (Codex review): events are not handed to the
  cursor at all, so nothing is decoded or parsed; the tab only notes that one
  arrived — and whether any of them was a truncate, replace or delete. On
  reveal: a missed truncate/replace/delete reloads; otherwise one line-count
  read decides — a different stream or generation than the cursor's pin
  reloads (`TranscriptCursor` ignores such counts, so they must not be handed
  to it), a gap within `GAP_FILL_MAX_LINES` (5,000) is filled by the cursor's
  chunked range reads, and a larger one reloads the newest page.
- **No silent holes** (Codex review): the cursor skips a gap over 5,000 lines
  (or a read that fails or names a vanished generation) by advancing past it,
  which in History would leave lines missing *below* the loaded range, where
  `loadOlder` can't reach them. History watches the cursor's `linesSkipped`
  counter; when it moves (and on the reveal-time reload cases above), History
  stops showing appends — what follows the hole would be out of place — and:
  - if the reader is following the bottom, reloads the newest page at once
    (it keeps them at the tail, where they were);
  - otherwise keeps the reader's pages and position untouched and shows
    "Newer turns — jump to latest" in the History header; the reload happens
    when they click it or scroll back to the bottom (Codex review: recovery
    must not yank an active long-history read).

  Either way what History shows is always a contiguous range of the
  transcript.
- `loadOlder` keeps today's wholesale reparse, and the reparse becomes the new
  incremental parser, so appends continue from it.
- Truncate / replace / delete of the stream: reload from scratch.
- **A parser error is not a silent skip** (Codex review): the cursor advances
  past a record before delivering it and only logs a throw, so History catches
  a `HistoryParser` failure itself and treats it like a skipped gap (the
  recovery above), rather than showing a range with the failed records
  missing.

**Deferred by this plan:** Phase 4 (n is now small;
`TRACKING_AGENT_PANE_BOUNDED_LIVE_WINDOW_2026_09_23.md` §3.8 measured the
stores at ≤ 8 % at 200 turns), 5b, 5c, 5e, the detached reading window (§6.3.5) and
opening History at a position. None is needed for a bounded live feed; each
comes back if we want scroll-back in the feed or History anchored to a turn.

**PRs.** (1) this revision; (2) History follows; (3) live feed roll-off,
kill switch and the top row; (4a) the per-turn subprocess controller writes
the user's message to the transcript (#3701 — Codex, Kimi, muxcode/container
Claude); (4b) the journal for in-pane shells (5d's second half), which lifts
the blocked case; (4c) ACP (`AcpController::send_input` and its pending-prompt
flush) and the Codex app-server controller (`AppServerController::spawn_turn`)
write the same user record after delivery, and their panes join the live feed
— until then they keep today's behaviour. AskUserQuestion answers need no journal: the answer is
already in the tool's result (PR 3 revision above); only rebuilding its
styled rendering on replay remains, a parser follow-up. Each re-runs the
full-conversation bench at N = 0 / 25 / 200 and records it in the tracker.
**Exit criteria for (3):** DOM, JS heap and per-flush cost flat from N = 25 to
N = 200 with the pane pinned, apart from blocked turns (counted in the HUD); nothing on screen moves when turns roll off
(frame-by-frame recording); kill switch restores paging; no reproducible
node is lost from History (every rolled-off turn is there).

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
| **5 — Node identity and durability (C prerequisites)** — re-planned as 5a–5e in §6.3.6 | §6.3.1–§6.3.3: positional ids with file generation, `src`/`endLine`/`turn`, full-pipeline parser checkpoints + `parser-checkpoints.jsonl` + one-time index rebuild, id-consumer migration, provenance per node kind, `out-of-band.jsonl` (backend + History-tab merge), generation-keyed accepted source ranges, ring-replay handling | Property test: parsing any line range restored from any checkpoint, in any page order, yields the same ids, nodes and turn ordinals as a parse from line 0 (100k random splits per CI run, every provider format, hidden reinjection included); the checkpoint-schema key test is in CI; replay-after-eviction suite (prefix, middle-gap and cross-generation cases) produces zero duplicates and drops no new-generation line; every node kind has a declared provenance (a test enumerates the `DocumentNode` union); shells appear in the History tab |
| **6 — Bounded live document (C)** | **Revised: §6.9 live feed + roll-off** (supersedes §6.3.4–§6.3.5) | Memory and per-flush cost flat in N, pinned **and** while reading far from the bottom for 1 h of streaming; invariant 4 verified by killing the backend mid-eviction; no non-durable node ever evicted (runtime assertion, soak) |
| **7 — History tab follows (C)** | **Revised: §6.9 "History follows the transcript"** (supersedes §6.4); ships before 6 | Visible tab shows new turns ≤ 1 s after turn end; parsing an append costs O(new lines), republishing ≤ 1 Hz (profile); hidden tab does zero work and catches up on reveal; tail-parser loss recovers with no duplicates; long-history read meets the same targets |
| **8 — Worker decision (F)** | §6.7 criterion evaluated; build if triggered | §4 targets met on all three OSes |
| **9 — Default on** | Remove flags once each phase has soaked; update `SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md` Status to point here | 8 h soak and fault suite green on all three OSes; user sign-off after daily use |
| **10 — `content-visibility` (§6.8)** | flagged experiment | memory flat over 8 h on all three OSes *and* measurable frame win; otherwise documented as rejected |

**Phases 1 and 2 ship together (measured 2026-09-23).** Phase 1 alone
removes forced layout but moves the same layout into each frame's rendering
step, where input waits behind every pane's update at once; typing got worse.
Phase 2's cap is what turns it into a gain
(`TRACKING_AGENT_PANE_BOUNDED_LIVE_WINDOW_2026_09_23.md` §2.1, §3.1).

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
  reuse across generations (invariant 8), and no new-generation line dropped by
  an old-generation range (§6.3.3).
- Reconnect replay while a middle gap exists (reading window + present):
  neither the gap nor the evicted prefix reappears; both retained ranges stay.
- Continuous typing and IME composition for 10 minutes while 4 panes complete
  turns: housekeeping still runs by its deadline and memory stays within budget
  (§6.5).
- Shells and optimistic messages: never evicted before their journal (or echo)
  line is durable; visible in the History tab afterwards.
- History tab's tail parser discarded mid-run (remount, injected error):
  recovery from the last checkpoint, no duplicates, no split runs, and no
  hidden reinjection reply shown.
- A multi-record node (long text run; shell with many chunks) completed live
  while its later records are not yet on disk: not evicted until `endLine` /
  `journalEnd` is covered; afterwards the History tab shows all of it.
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
| Evicted content replayed back in as duplicates | positional ids + generation-keyed accepted source ranges + ring tombstones (invariants 8–9); replay-after-eviction suite incl. middle gaps |
| A truncate drops new lines through a stale filter | ranges keyed by generation, reset atomically on a newer generation (§6.3.3) |
| Sustained typing blocks eviction | housekeeping deadline + bounded slices + 1.5× backstop (§6.5) |
| One huge in-progress node defeats the tail ceiling | row-internal windowing for markdown and tool output (§6.2) |
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
