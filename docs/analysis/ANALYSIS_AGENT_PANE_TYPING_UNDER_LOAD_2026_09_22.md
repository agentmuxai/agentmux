# ANALYSIS — Agent-pane typing goes slow under heavy pane activity

**Date:** 2026-09-22
**Author:** Manoz
**Status:** analysis — **§2 measured and confirmed** (see §6, `tools/tests/bench-markdown-parse.mjs`). §4's pane multiplier is still code-reading only and needs the in-app check in §6.4.
**Symptom (user-reported):** "when there is a lot of activity in the agent pane, typing becomes extremely slow."
**Umbrella:** Discussion #1161 · `TRACKING_TYPING_AND_TERMINAL_RESPONSIVENESS_2026_09_21.md`

---

## 1. Scope — this is not the terminal problem, and none of the terminal work applies

Almost every "typing" doc under the #1161 umbrella is about the **terminal**, where
typing latency is an *echo round-trip*: keystroke → WS → sidecar → PTY → echo → WS
→ paint. Predictive echo, ACK flow control, and the PTY coalescing window all live
on that path.

**None of it applies here.** The agent-pane composer is an **uncontrolled
`<textarea>`** — the DOM owns the value (`AgentFooter.tsx:42`, `:459`). Keystrokes
typed into it never leave the renderer: no WS, no sidecar, no PTY, no echo. The
browser paints the character natively.

So there is no round-trip to shorten, and a slow keystroke can mean only one thing:

> **the main thread was busy, and the frame in which the character should have
> painted was spent doing something else.**

Everything below is about what that something else is. The fact that the composer
is uncontrolled is *why* the original April fix (v0.33.91) worked, and it is also
why this problem is a different shape: we already removed the per-keystroke work.
What is left is per-*output* work landing on the same thread.

## 2. Root cause 1 — each streaming block re-parses its whole message, forever

`MarkdownBlock.tsx` renders a streaming message by writing the full text into a
throttled signal, then handing it to `<Markdown text={...}>`:

- `MarkdownBlock.tsx:62-78` — a `createEffect` on `props.node.content` commits a
  new view at most once per `STREAM_RENDER_MS` (90ms), skipping syntax
  highlighting mid-stream and doing one full highlighted render once output
  settles.
- `markdown.tsx:138` — `transformBlocks(resolvedText())`, which does
  `content.split("\n")` and walks **every line** (`markdown-util.ts:43-50`).
- `markdown.tsx:278-285` — `unified().use(remarkParse).use(remarkGfm)
  .use(remarkRehype).parse(txt)`: a **complete markdown parse of the entire
  message**, from the first character, every commit.
- Then Solid reconciles the resulting tree into the DOM.

The block's own comment (`MarkdownBlock.tsx:22-24`) states the problem exactly:

> *"During streaming the message content grows ~60x/s. Re-parsing the whole
> document (including syntax highlighting) on every frame is O(n^2) and starves
> keystrokes."*

**The shipped fix (#1213) throttled that loop to 90ms. That changes the constant,
not the complexity.** Each commit is still O(n) in the message length, and there
are still O(n) commits over the message's life, so the total remains **O(n²)** —
now running at ~11Hz instead of ~60Hz. The per-commit cost still grows without
bound as the message streams.

That is the crux: **a 90ms throttle is a rate limit on an operation whose cost is
increasing.** Early in a message the parse is trivial and nothing is felt. Late in
a long message — a big code-heavy answer, a long thinking block — a single parse
can exceed the frame budget, and from then on the main thread is blocked for that
long, ~11 times a second. A keystroke that arrives during one of those blocks
cannot paint until it finishes.

This also explains the *shape* users report: typing is fine at first and degrades
as the response grows, which is what O(n²) feels like from the outside and is not
what a fixed-cost regression would feel like.

## 3. Root cause 2 — nothing is off the main thread; there are zero workers

`grep -rn "new Worker" frontend/app` returns **nothing**. The frontend has no web
workers at all.

`PLAN_INPUT_RESPONSIVENESS_EXECUTION_2026_05_29.md` Phase 1 was *"offload the
proven hotspot: Shiki off-thread (frontend's first Worker); incremental
main-thread markdown."* **Neither half was ever built** — this analysis is the
first thing to check, and both are still open.

Shiki is lazy-loaded and `await`ed (`HighlightedCode.tsx:28-33`, `:93-95`), which
makes it *asynchronous* but **not off-thread**: the work still executes in a task
on the main thread. Highlighting is skipped during streaming (good — that is what
#1213 bought) but runs in full on the settle commit, which is therefore the single
most expensive frame of a message's life, landing exactly when the user is most
likely to start typing a reply.

## 4. Root cause 3 — no visibility gate, and keep-alive just multiplied it

`useAgentStream.ts` routes every document write through one `StreamFlushQueue`
(one shared RAF, one shared `batch()`), which is good hygiene. The only wrapper on
it is `createHidingStreamFlushQueue` — and that gates on **hidden memory
reinjection after compaction**, not on pane visibility
(`hiding-stream-flush-queue.ts` header). **There is no visibility gate anywhere on
this path**, matching the finding already recorded in
`TRACKING_TYPING_AND_TERMINAL_RESPONSIVENESS_2026_09_21.md` §3 and cited by
`SPEC_TAB_SWITCH_PER_PANE_PROGRESSIVE_REVEAL_2026_09_21.md`.

Four days before this report, **PR #3391 (2026-09-18) added `"agent"` to
`KEEP_ALIVE_TYPES`** (`pane-leaf-chrome.tsx:101`,
`SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md`). Keep-alive switches tabs by
**visibility, not existence** — the file says so directly (`:340-343`): a
backgrounded member stays **mounted and in the layout tree** under
`visibility: hidden`, deliberately chosen over `display: none` so layout does not
collapse.

Combined, these mean a backgrounded, still-streaming agent tab **keeps parsing
markdown at full cost on the same main thread as the pane you are typing into**.
Total cost scales with *(number of streaming panes × their message sizes)*,
independent of how many are actually on screen.

This is the most likely reason the symptom is reported as recent and severe:
§2 has been true since streaming existed, but until 2026-09-18 a backgrounded
agent tab stopped rendering when it stopped existing.

## 5. Why the existing measurements never caught this

- `term-echo-render` and `term-keypress` are **terminal** marks. They do not
  instrument the composer at all.
- `bench-agent-keystroke.mjs` (#1150) drives synthetic keystrokes, but the
  documented runs were on an **idle** pane. This bug is by definition invisible at
  idle: at n≈0 the parse is free.
- The one instrument that would show it — a long-task/keystroke-to-paint measure
  taken *while a large message streams* — does not exist for the agent pane.

So "p95 0.4ms, zero long tasks" has been repeatedly quoted about a pane that was
not doing the thing that makes it slow.

## 6. Measurements

`tools/tests/bench-markdown-parse.mjs` replays the **real** pipeline from
`markdown.tsx:230-286` — same plugins, same order, including the two app-local
remark plugins — against synthetic agent output (25% code blocks). Run it with
`npx tsx tools/tests/bench-markdown-parse.mjs`.

Every number below is a **lower bound**: the bench stops after `runSync`, so it
excludes `toJsxRuntime` and Solid's DOM reconcile, which run after it and cost
more on top. Node 22 / same V8 as the renderer, but no competing app load.

### 6.1 Per-commit cost vs message length — §2 CONFIRMED

| message | mid-stream commit (no highlight) | settle commit (highlighted) |
|---|---|---|
| 4KB | 7.6ms | 14.4ms |
| 8KB | 8.1ms | 17.9ms |
| **16KB** | **17.3ms** — over one frame | 44.0ms |
| **32KB** | **66.4ms** | 127.7ms |
| **64KB** | **114.8ms** | 154.6ms |
| 128KB | 162.9ms | 223.6ms |

Fitted scaling exponent (cost ∝ nᵇ): **b = 0.84 mid-stream, 0.76 settle.**
Cost is **not flat** — it rises with message length, roughly linearly per
commit, which compounds to ≈O(n^1.8) over a full message. §2's mechanism is
real and the magnitudes are severe well inside normal message sizes: a
**16KB** response — a medium answer with a couple of code blocks — already
blows the frame budget on every mid-stream commit.

### 6.2 One 32KB message, streamed end to end

```
total main-thread time in parse:   1679ms
final settle (highlighted):         151ms   <- single longest block
worst mid-stream commit:             92.5ms
commits over one frame:              39/40
stream wall-clock:                  ~3.6s
main thread occupied by parse:       46.6%
```

**Rendering one ordinary 32KB response burns 1.7 seconds of main thread and
keeps it ~47% occupied for the whole stream**, in blocks of up to 92ms (151ms
at settle). Roughly every other keystroke typed during that response lands
inside a blocking parse and cannot paint until it finishes. That is the
reported symptom, quantitatively.

### 6.3 What the fix buys — measured, not projected

`--incremental` models the §7.1 fix: freeze everything before the last blank
line, re-parse only the trailing open block per commit.

| | 32KB message | 64KB message |
|---|---|---|
| total parse — current | 951ms | 3728ms |
| total parse — incremental | **14ms** | **27ms** |
| worst commit — current | 52.9ms | 188.2ms |
| worst commit — incremental | **1.0ms** | **1.6ms** |
| occupancy — current | 26.4% | 51.1% |
| occupancy — incremental | **0.4%** | **0.4%** |

**67x / 139x faster**, and the important part is the *shape*: incremental stays
flat as the message grows (1.0ms → 1.6ms worst, 0.4% occupancy at both sizes)
while the current path doubles. That is the O(n)→O(1) per-commit change showing
up exactly where the theory says it should, and it puts the worst commit an
order of magnitude **under** the 16.7ms frame budget instead of 11x over it.

Caveat: absolute figures vary run to run (an earlier 32KB run measured 1679ms /
92.5ms worst vs 951ms / 52.9ms here — machine noise). The *ratio* and the
flat-vs-rising shape are stable and are what the conclusion rests on.

This measures the parse-cost ceiling of the fix, not its correctness — see the
bench's own note on reference definitions, loose lists, and setext headings,
which constrain what is safe to freeze without changing the cost shape.

### 6.4 Still unverified — §4's pane multiplier

The bench covers one pane. §4's claim that N backgrounded-but-mounted panes
multiply this is still code-reading only. Confirm in-app: stream in one pane
vs. three backgrounded streaming panes and compare. The app already emits a
`markdown-render` mark with `len=` (`markdown.tsx:226`, `:302`), so this needs
no new instrumentation — just read the marks.

### 6.5 Incidental finding — the processor is rebuilt every commit

`unified()...use(...)` is constructed **inside** the render memo
(`markdown.tsx:278`), so the whole plugin chain is rebuilt on every commit.
That shows up as the ~4ms floor at 1-2KB in 6.1, where it dominates. Hoisting
it is a small, safe, independent win — ~4ms × ~11 commits/sec per streaming
pane — and does not depend on the larger fix landing.

## 7. Fix directions, in the order the evidence supports

0. **Hoist the processor out of the render memo (§6.5).** Independent, tiny,
   safe, and worth doing regardless of what else lands.
1. **Incremental markdown — the real fix (§2, confirmed §6.1/§6.2).** Stop re-parsing the whole message.
   Markdown is block-structured: everything before the last blank line is
   structurally final and can be parsed once and frozen, with only the trailing
   incomplete block re-parsed per commit. That converts the per-commit cost from
   O(n) to O(size of the last block) and the total from O(n²) to O(n). This is the
   "incremental main-thread markdown" half of Phase 1 that was never built, and it
   is the only change here that fixes the complexity rather than the constant.
2. **Gate rendering — not data — on pane visibility (§4).** A backgrounded pane
   should keep *receiving* stream events (so nothing is lost and switching to it is
   instant) but should not re-parse or re-render until shown. The `StreamFlushQueue`
   is already the single mandatory choke point, so the gate has one correct home,
   and `createHidingStreamFlushQueue` is a working precedent for wrapping it.
   This is the smallest change with the largest immediate win for the reported
   multi-pane case.
3. **Move Shiki to a worker (§3).** The other half of Phase 1. Biggest effect on
   the settle frame specifically. Larger change; do it after 1 and 2, and only if
   the profile still shows highlighting on the critical path.
4. **Do not raise `STREAM_RENDER_MS`.** It is the obvious lever and it is a trap:
   it lowers the duty cycle while making each blocking task *longer* and the
   stream visibly chunkier. It treats the symptom and worsens the cause.

## 8. Explicitly not the cause

- **The PTY coalescing window** (removed in PR #3515, 2026-09-22). That was a real
  20ms latency bug on the *terminal echo* path, and agent panes share the flusher
  so it did delay agent **output** — but composer keystrokes never traverse it. It
  is not this.
- **Predictive local echo.** Terminal-only (`termwrap.ts`); the composer has no
  echo to predict.
- **The uncontrolled-textarea regression (v0.33.91).** Verified still in place
  (`AgentFooter.tsx:42`, `:459`); it has not come back.
