# SPEC: The agent pane unloads collapsed tool results from memory

**Status:** proposed — nothing here is built.
**Date:** 2026-10-01
**Verified against:** `agentmux` `main` @ `fd95207ea`, plus #4121 (the live
feed bounded by size). Paths are relative to `frontend/app/view/agent/`
unless noted.
**Related:**
- the live-feed report `REPORT_LIVE_FEED_ROLL_OFF_TOO_AGGRESSIVE_2026_09_30`
  (#4115): the 15 MB live feed this makes room in;
- `SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md` (roll-off,
  phase 5b node identity);
- `SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md` (the live log).

## 0. The ask

From the owner, 2026-10-01:
> "When the tool call is collapsed, can the preview be unloaded from
> memory? … So currently, the app keeps the tool preview in memory even
> when the tool is collapsed? … Let's get a spec written to file regarding
> the tool results, then implement."

## 1. What happens today

- **The DOM is already lean.**
  - A collapsed row builds its body only once it's first opened
    (`components/ToolBlock.tsx:219`, `bodyMounted`).
  - Off-screen rows are unmounted by virtualization.
  - One gap: a row opened and then collapsed keeps its built body while it's
    on screen (`bodyEverShown` latches).
- **The data isn't.** Every tool node keeps its full `result` in the
  document store whether it's collapsed, open or off-screen. Finished tools
  also keep their live log: `log.chunks` stays after the `tool_result`
  lands (`store/agent-document/reducer.ts:635,941`).
- **Finished logs are never shown again.** `ToolBlock` renders the log only
  while it's open (`ToolBlock.tsx:435`, `log?.open === true`), so a finished
  tool's chunks serve nothing.
- **The live feed's budget counts it all** (`nodeBytes`,
  `virtualization/streaming-buffer.ts:222`). A log over 4,000 chunks is
  charged the full 2 MB cap.

**How much is at stake.** Tool results as a share of displayed bytes,
measured on the local transcript store on 2026-10-01:

| Agent | Tool results | Displayed total | Share |
|---|---|---|---|
| AgentA | 69.3 MB | 160.5 MB | 43% |
| Marks | 1.8 MB | 2.8 MB | 66% |

## 2. What a collapsed row needs

The collapsed row and the code around it read only these:
- **whether there is a result:** for the pill and the dispatch fallback
  (`ToolBlock.tsx:248-253`);
- **the pill** (`toolPill`, `tool-meta/tool-descriptors.ts:292`), e.g.
  "3 matches" or "+12 −4";
- **the size,** for the hover estimate (`ToolBlock.tsx:304`, which uses the
  `params` + `result` text);
- **background-launch detection** (`activity/tool-adapter.ts:121-124`):
  `backgroundTaskId` or a short "accepted" text. These results are tiny.

Everything else that reads `result` is a body renderer
(`components/tool-renderers/*`, `DiffViewer`, `ToolOverlayLog`). Those run
only when the body is built, which is when the row is open.

## 3. Design

### 3.1 Step 1: free a finished tool's live log

When a tool's `tool_result` lands (`log.open` → false), the reducer keeps
`log: { open: false, chunks: [] }` and drops the chunks.
- Nothing renders them after that.
- `nodeBytes` stops charging them. This also closes the "one long build is
  charged 2 MB" hole in the report (§3.3).

**Before dropping:** confirm that no reader uses a finished log.
`ActivityRow.tsx` and `PersistentShellBlock.tsx` read logs; the shell
block's log is a separate ring and is out of scope. Test that each reader
handles a finished tool with no chunks.

### 3.2 Step 2: record where each result came from

A tool node gets `resultSource?: { stream: string; gen: string; line: number }`:
the transcript line its `tool_result` record was read from.
- **Replay** (`parseHistoryLines.ts`): the read offset plus the line's index
  in the batch, with the stream and gen from the `BlockfileReadRange`
  response.
- **Live** (`useAgentStream.ts`): the transcript cursor already numbers
  every delivered line (`transcript-cursor.ts`, `next`). It passes the line
  number with each line it delivers.
- **Unknown:** a cursor that isn't pinned (an uncounted file), or a provider
  whose result isn't a single record. The field stays unset, and **that node
  is never unloaded.**

### 3.3 Step 3: unload collapsed results

A pass (`UnloadToolResults`, one reducer command, run off the input path at
the same times as roll-off) replaces `result` with a stub on every tool node
that:
- is finished (not `running`, `pending_approval` or `awaiting_answer`);
- is collapsed, i.e. not pinned, not in `expandedTools`, and not held open;
- isn't in the turn in flight;
- has a `resultSource`;
- has a result of at least **8 KB**. Small results stay, which keeps
  background-launch detection and every small read working unchanged;
- isn't a content-first tool (`isContentFirstTool`, e.g. WebSearch), whose
  body is shown collapsed.

The stub replaces `result`:

```ts
interface UnloadedResult {
  unloaded: true;
  pill: string | null;   // toolPill() of the real result, computed before unloading
  bytes: number;         // the result's size, for the hover estimate and display
  tokens: number;        // estimated
}
```

- `nodeBytes` charges a stub about 100 bytes, so the 15 MB budget holds
  much more conversation.
- The pill and the hover estimate read from the stub.
- `isAcceptedBackgroundLaunch` never sees a stub, because those results are
  under the threshold.

### 3.4 Step 4: load it back when the row opens

When a row with a stub expands, `ToolBlock` asks for the result:
1. A read of one line, `BlockfileReadRange { offset: line, limit: 1 }`, with
   the stream and gen checked against `resultSource`.
2. That line is parsed with the same parser (`parseHistoryLines` on the one
   line) to take the node's `result`.
3. A `ResultLoaded` reducer command puts it back.

While it loads, the body shows the pill and "Loading result…". If the line
can't be read (the stream was replaced or truncated), the body says
"Result no longer available in this pane — open History", and the stub
stays.

**Unloading again:** collapsing a reloaded row makes it eligible on the next
pass. Its built body is released at the same time, which closes the
`bodyEverShown` gap.

### 3.5 What doesn't change

- **History** (the tab) reads ranges from the transcript itself; it isn't
  touched.
- **Copy, find and export from the pane** read the DOM or the open body
  today. Opening a row restores its result. Any future reader of collapsed
  results has to handle the stub.
- **What the model receives:** this is display-side memory only.

## 4. Tests

- **Step 1:** a finished tool's chunks are dropped; a running tool's are
  kept; replaying the result after its log doesn't bring the chunks back;
  `nodeBytes` falls accordingly.
- **Step 2:**
  - replay sets `resultSource.line` to `readStart` plus the index;
  - live sets the cursor's line;
  - an unpinned cursor leaves it unset.
- **Step 3:**
  - a collapsed, finished, large result with a source becomes a stub, with
    the pill and bytes preserved;
  - each exclusion keeps its result: running, pinned, held, in the turn in
    flight, no source, under 8 KB, content-first;
  - `nodeBytes` of a stub is small.
- **Step 4:**
  - expanding a stub reads exactly that line and restores an identical
    result (parity with the original parse);
  - a missing line shows the fallback text.
- **Measured, with a dev build:** a long pane's store size and JS heap
  before and after; the number of turns that fit in 15 MB before and after.

## 5. Phases

| Phase | What ships | Size |
|---|---|---|
| **U1** | Step 1: free finished logs | small |
| **U2** | Steps 2–4: result source, unloading, reloading on open | medium |

## 6. Open questions

1. **The threshold** (8 KB) and whether to unload only once the pane's
   budget is under pressure, or always. Recommended: always; it's cheap, and
   reloading takes one line read.
2. **Large tool inputs** (`Write` content, `Edit` old and new strings): the
   same treatment, later, once U2 is proven.
