# Spec: streaming must not stall while the user types, then flood

**Date:** 2026-10-13
**Status:** active. Fixes R1–R4 (§3) implemented in #4845; §5 lists what is still open.
**Plan item:** new, found while measuring typing for item #7 of `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md`.
**Related:** `stream-scheduler.ts` (input-first flushing, PR #3599), `agent-dormancy.tsx` (rendering pauses for hidden panes), `SPEC_TOOL_BODY_UNMOUNT_ON_COLLAPSE_2026_10_12.md` (tool bodies dropped after collapse), `SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md` (height FLIP).

## 1. The problem

With several agents streaming, typing into a composer makes the panes stop moving; when typing stops, the held-back output arrives at once and the window freezes for several seconds. The operator sees: panes streaming, typing starts, panes go quiet, typing ends, panes flood.

Measured on an isolated production portable (the #7 build), 12 scripted Claude agents over tabs (6 visible), turns paced like the real CLI (35 ms per chunk, tools taking 0.2–2.5 s), every agent's queue kept full, 20 keys/s for 20 s into a streaming pane's composer. Per second, from CDP (`Network.webSocketFrameReceived`, a `MutationObserver` on each visible pane, a rAF counter):

| Phase | Stream data reaching the page | Frames | |
|---|---|---|---|
| Before typing | ~270 KB/s, ~300 messages/s | 24 fps | |
| Typing, first 8 s | ~270 KB/s | 38–41 fps | |
| Typing, last 8 s | 115–170 KB/s, 55–140 messages/s | 36–40 fps | backlog building |
| First 10 s after typing | 1.4 MB/s peak, 1,700 messages/s | **3–11 fps** | the flood |

The server keeps producing at the same rate throughout; what changes is how fast the page reads its socket.

## 2. Why

**The renderer's main thread has no idle time** (CPU profile, 500 µs sampling: zero `(idle)` samples before, during or after typing). Whichever kind of task Chromium runs first wins and the other falls behind:

- With no input, the socket's message tasks run and frames suffer (24 fps).
- While keys arrive, Blink prioritises input and the frames after it. The networking task queue gets what is left, the WebSocket data pipe fills, the network service stops reading, and the backlog waits upstream of the page.
- When input stops, the backlog is delivered and processed at once, and frames drop to 3–5 fps.

The stream scheduler's input-first policy (one or two panes flushed per frame while interacting) is not the stall: panes keep flushing during typing. What stops is the data feeding them. So the fix is main-thread headroom, not a different priority rule.

### 2.1 Where the time goes

The lab (isolated production portables of `main`): 12 scripted agents over 4 tabs (4/2/4/2, tab 1 shown), every agent's queue kept full, agents offset rather than in lockstep. A Chrome trace with invalidation tracking over 4 s of streaming: about 250 layouts (over 90% forced from script) and 330–430 style recalculations, 0.5 s of layout and **1.0 s of style recalculation**. Style is the largest single cost, and it follows how many elements are created or restyled.

What created them (a wrapper on the page's DOM removal and insertion paths that records each call's stack): in 5 s, **110,000 layout invalidations**, two thirds of them nodes removed from and added to layout, almost all of them the lines of tool output previews (`agent-preview-line`, `agent-preview-text`, their text nodes). 16,000–23,000 preview lines were built in 5 s. Three causes follow.

### 2.2 A finished tool's output was built up to three times

`ToolOverlayLog` drew a tool's output through a four-branch `<Switch>`:

1. the streamed chunks while the stream is open;
2. the structured result;
3. the streamed chunks after the stream closed, when no result came yet;
4. the placeholder.

Branches 1 and 3 render the same `ChunkList`, but as different branches the stream closing rebuilt it, every line, and the height FLIP measured a transition between two equal heights. Then the result arrived: branch 2 built the result view, every line again. And a finished tool's panel closes as its result arrives and its body is dropped a second later (`body-mount.ts`), so that third build was for a panel nobody saw. Measured: each line still on screen had been built 3 times (p50 and p90), up to 6.

### 2.3 Panes on hidden tabs built tool output nobody could see

Inactive window tabs stay laid out (`window:keepinactivetabslaidout`, the default) and hide with `visibility: hidden`. The agent pane's dormancy signal (`agent-dormancy.tsx`, true while the pane is hidden) was routed only to `MarkdownBlock`. Tool bodies kept building: 70% of the preview lines torn down in the lab were in hidden panes (8 of the 12).

Laying out hidden tabs is not the cost: with `window:keepinactivetabslaidout = false` (`content-visibility: hidden`) layouts drop by a third but layout time doesn't, style time drops 25–35%, and typing latency doesn't change. Building elements is the cost.

### 2.4 A `:has()` rule made every row an anchor

`.agent-document-row:has(.agent-user-message--expanded.agent-user-message--startup:not(.agent-user-message--pinned))` (`_document.scss`) can only match a user-message row, but as written every row was a `:has()` anchor: each node inserted anywhere in any row, each streamed span of a tool's output, made Blink check the row's style again. 3,000 of 8,900 style invalidations in 3 s.

### 2.5 Smaller, measured, not fixed here

- The Swarm pane recreates each agent's row group about 12 times a second: `<For each={section.agents}>` keys by object, and the model hands out new objects on every update (5–8% of invalidations).
- `Element.animate` on a pseudo-element (the activity flash, `activity-flash.ts`) and the scroll follower's `scrollTop` writes each force a layout when the DOM is dirty. Moving both into one per-frame phase after the stream flushes was built and measured (§4): no gain, because together they force only about 10% of the layouts. Not shipped.
- `ToolOverlayLog`'s `onScroll` reads `scrollTop`/`scrollHeight` on every scroll event, including its own pins (about 1%).
- `ws.ts` `sendMessage` builds a `Blob` per outgoing message to measure its size, and the page sends about 30 `docknodestatus` messages a second while streaming (about 1%).

## 3. Fixes

**R1. One chunk branch.** `ToolOverlayLog` renders the chunks in one branch whether or not the stream is still open, and the placeholder and the result in one branch (`ToolOverlayResult` swaps them itself). The stream closing no longer rebuilds the list or measures a FLIP.

**R2. Nothing new is built while the output can't be seen.** `ToolOverlayLog` takes `open`: whether its panel is open and its pane isn't dormant (`ToolBlock`'s `bodyShowing`). While it is false the log keeps showing what it showed, branch and node both held (the reducer frees a tool's chunks when its result arrives, and a closing panel must not empty out mid-collapse). The body stays mounted, so the row keeps its height and nothing moves when a hidden pane is shown again; it catches up then. This covers both the closing panel of §2.2 and the hidden panes of §2.3.

**R3. A new node object is not a change.** `ToolOverlayResult` passes a new node on only when one of its own fields changed (`sameToolNode`; the reducer replaces the nested objects it changes), so an update that only replaces the object doesn't rebuild the result.

**R4. Scope the `:has()` rule** to `[data-node-type="user_message"]` rows.

## 4. Results

Same lab, `main` against `main` with R1–R4, alternated twice. Each run: 20 s of typing at 20 keys/s into a streaming pane with stream data, pane mutations and frames per second recorded, then three 12 s typing windows, each started only once at least 75% of the visible panes were moving, with key-to-paint from Event Timing.

| | main, run 1 | main, run 2 | R1–R4, run 1 | R1–R4, run 2 |
|---|---|---|---|---|
| Preview lines built in 5 s | 16,000–23,000 | | 3,732 | |
| Frames per second during the flood | 8 | 8 | 15 | 19 |
| Frames per second after typing | 13 | 7 | 19 | 16 |
| Stream messages reaching the page while typing (before typing: ~375/s) | 303/s | 214/s | 353/s | 354/s |
| Long animation frames, share of time | 21.7% | 19.4% | 13.2% | 9.1% |
| Keys over 100 ms to paint | 10% | 8% | 6% | 4% |
| Key-to-paint p95 / worst | 120 / 352 ms | 112 / 192 ms | 112 / 240 ms | 104 / 176 ms |

The scroll-write phase and deferred flash (§2.5), measured on their own: key-to-paint p95 128 vs 120 ms, forced layouts within run-to-run noise.

Against the acceptance criteria:

1. Stream data while typing within 20% of the rate before: met with R1–R4 (main falls to 57%).
2. No burst after typing: not met; the after-typing rate is still about 1.2 times the before rate.
3. 30 fps throughout: not met (15–19 during the flood).
4. Key-to-paint p95 at most 50 ms: not met (104–112 ms).
5. Idle time in every phase: not yet measured with R1–R4.
6. One-way-flow soak (`one-way-soak.mjs`, `task dev`, 4 visible panes, 3 min per run, alternating code in the same instance): no worse than `main`. The first run after a reload fails on both (main 111 violations; R1–R4 126 and 104, the same causes, content shrink and tool rows growing), the next passes on both (main 1 violation, R1–R4 none). That first-run failure predates this change. Showing a tab whose two agents streamed three turns while it was hidden: the panes show the finished turns, with nothing left behind from while they were hidden.

## 5. Still open

- Style recalculation is still the largest cost: about 26,000 elements restyled per 3 s with R1–R4. Next: which elements, and why (the trace's per-recalc element counts and invalidation sets).
- The Swarm pane's row groups (§2.5): key them by `blockId`.
- The lab measures noticeably more style time right after the 20 s typing probe than in a fresh run; the cause is not known yet.

## 6. How it is checked

On isolated portables, never the operator's instance: a feeder keeps every agent's queue full for the whole run; a gate starts each typing window only once most visible panes are moving; the typing probe records key-to-paint and, per second, how many visible panes moved; a flow probe records stream data, pane mutations and frames before, during and after typing; CPU profiles and Chrome traces (with invalidation tracking) are symbolicated through the bundle's source map to attribute layouts, style recalculations and DOM churn to source.
