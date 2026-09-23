# ANALYSIS — Agent-pane typing lag under multi-pane streaming: every flush rebuilt every finished tool result

**Date:** 2026-09-23
**Author:** Manoz
**Status:** analysis — measured, fixed (this PR), verified live — §5. Two smaller residual
costs are named in §6 and left open with numbers attached.
**Symptom (user-reported):** with four agents streaming in four visible panes,
typing in a composer is "quite slow" and the agents' output "appears laggy".
**Builds on:** `ANALYSIS_AGENT_PANE_TYPING_UNDER_LOAD_2026_09_22.md` (whose fixes
#3521 / #3536 shipped and hold — this is the next layer down, not a regression
of them).
**Umbrella:** Discussion #1161 · `TRACKING_TYPING_AND_TERMINAL_RESPONSIVENESS_2026_09_21.md`

---

## 0. The one-paragraph version

The lag was not caused by typing. Under four streaming panes the renderer ran at
**2.3 fps whether or not anyone typed**; keystrokes merely queued behind the
frames. 97% of every frame was `flushPendingNodes` (`stream-flush-queue.ts`),
and inside it two things dominated: **constructing `OverlayScrollbars` instances
(46%)** and **first-run markdown parses in freshly created `<Markdown>` components
(38%)**. Both are the signature of components being *created*, not updated. The
trigger: `dispatchMatches` — a memo over the whole document array — yields a new
`Map` identity on every flush, and it was read through an inline prop getter
(`DocumentRow.tsx:216`), which subscribed every `ToolOverlayResult`'s `<Show>`
children memo to that identity. So `renderToolResultBody` re-ran and rebuilt the
entire result subtree for **every finished tool in the streaming buffer, every
flush** — including `.md` Read previews that re-parse a 13 KB file and construct
a scrollbar with its forced layouts each time. One equality-gated memo fixes it.

## 1. What was measured, and how

All numbers come from the running dev build (`main @ 7519f71`, CEF/Chrome 152)
over CDP, with the four scripts this PR adds under `scripts/ui-screenshots/`:

| Script | Instrument |
|---|---|
| `cpu-profile.mjs` | V8 sampling profile of the page's main thread, self-time by function/file |
| `trace-capture.mjs` | all-process Chromium trace (browser / renderer / GPU / compositor) |
| `cdp-eval.mjs` | run in-page instrumentation: Event Timing API, Long Animation Frames, rAF gaps, MutationObserver |
| `synthetic-typing-loaf.mjs` | inject OS-style key-repeat via `Input.dispatchKeyEvent` while recording LoAF — no human needed |
| `typing-under-load-experiment.mjs` | the A/B: send a prompt to every composer, ramp, then 15 s LoAF *without* typing and 15 s *with* synthetic typing on identical load |

Three sanity results that shaped the rest:

- **Typing alone (agents idle):** 200 synthetic keystrokes, 648 frames in 10.8 s,
  rAF gap p95 16.8 ms, zero long frames. The keystroke path is fine.
- **The "streaming alone is fine" reading in an early capture was wrong** — that
  capture had been taken after the streams ended. Under live load, streaming alone
  is *exactly* as bad as streaming + typing (§2 table).
- **The GPU/compositor is not it:** Quadro K1200 at 23%, GPU compositing on,
  browser main thread idle, DWM 10%. (`--enable-unsafe-swiftshader` in the dev
  command line is a fallback flag, not an active software renderer.)

Two traps worth recording so the next person doesn't lose an hour to them:
`document.visibilityState` reads `hidden` and rAF stops when the *window is
minimized* — every measurement must check it first (the scripts do); and the CEF
dev build binds CDP on 9223 (9222 is the production instance next to it).

## 2. Root cause — finished tool results were rebuilt on every flush

### 2.1 The numbers

A/B on identical load (4 panes streaming a 30-section PostgreSQL reference;
Maksi's pane also held 62 finished tool rows from an earlier tool-heavy prompt):

| 15 s window | Frames | rAF gap p50 / p95 | Long frames | Stalled | Forced layout in scripts |
|---|---|---|---|---|---|
| A — streaming, no typing | 35 (2.3 fps) | 433 / 667 ms | 34 | **15.26 s of 15.3 s** | 5,961 ms |
| B — streaming + 20 keys/s | 71 | 83 / 550 ms | 40 | 14.5 s | 4,947 ms |

Long Animation Frames attribution in both: **`FrameRequestCallback → flushPendingNodes`**
= 13.7 s across 95 calls (**~144 ms per flush**, 43% of it forced layout). Typing
contributed ~30 ms in total. Keystroke → paint under this load measured p50 88 ms,
p95 472 ms, max 552 ms (Event Timing API, 372 real keydowns).

A 15 s CPU profile *during* streaming, restricted to the `flushPendingNodes`
subtree (11.5 s of 15.7 s sampled):

| Inside `flushPendingNodes` | Time | Meaning |
|---|---|---|
| `OverlayScrollbars` constructor subtree | **5,284 ms (46%)** | `getPropertyValue` 2,790 ms, `set scrollLeft` 1,835 ms, `getCSSVal` 416 ms — the constructor's environment/RTL probes, each a forced layout of the whole pane |
| `runSegment` (remark parse) via the render memo's **first run** in a new `<Markdown>` | **4,394 ms (38%)** | only 455 ms ran through the in-place (incremental) path; the rest was fresh component instances |
| `createComponent` | 4,785 ms inclusive | components created inside the flush |
| garbage collector | ~760 ms | churn from the above |

Every `OverlayScrollbars` construction traced to `Markdown`'s `onMount`
(`markdown.tsx:444`), i.e. a Markdown component with `scrollable` at its default
`true`. `MarkdownBlock` passes `scrollable={false}`, so the streaming message was
not the source. The only `<Markdown>` in the agent pane that leaves the default
is `ToolOverlayLog`'s `.md` Read/Write preview (`:507`, `:564`).

Per-pane DOM census over 10 s (MutationObserver on each `.agent-document`):

| Pane | Markdown roots added | Scrollbar hosts added | Rows added | Elements added / removed |
|---|---|---|---|---|
| Maksi (62 tool rows, 3 `.md` previews of 13 KB / 5 KB / 13 KB) | **102** | **51** | 0 | 3,438 / 3,776 |
| Mzks (no tool rows) | 0 | 0 | 0 | 2,506 / 2,498 |

Maksi rebuilt `agent-tool-read` ×136 and `agent-bash` ×340 in those 10 s. Net DOM
growth was +93 nodes: the same subtrees were being destroyed and recreated.

### 2.2 The mechanism

```
AgentDocumentView.tsx:180   dispatchMatches = createMemo(() =>
                              correlateDispatchesForBlock(blockId, documentNodes(), …))   // new Map every flush
DocumentRow.tsx:216         <ToolBlock dispatchMatch={props.dispatchMatches?.().get(id)} … />  // inline getter
ToolBlock → ToolBlockOverlay → ToolOverlayLog → ToolOverlayResult
ToolOverlayLog.tsx:439      <Show when={status !== "running"}>
                              {renderToolResultBody(props.node, { dispatchMatch: props.dispatchMatch })}
```

`<Show>`'s children live in a memo. Reading `props.dispatchMatch` inside it walks
the getter chain up to `dispatchMatches()`, so the memo depends on the **Map's
identity**, not on the `.get()` result. The result is `undefined` for anything
but Agent/Task/Workflow tools, and unchanged for those too — but the Map is new
each flush, so the memo re-ran, `renderToolResultBody` re-executed, and the
whole result subtree was rebuilt. Profile chain, verbatim:
`renderRead ← renderToolResultBody ← get children@ToolOverlayLog ← Show.createMemo ← completeUpdates ← flushPendingNodes`
(4,404 ms for `renderRead` alone).

Two amplifiers: the tool panel is always mounted (`inert` when collapsed,
`ToolBlock.tsx:511`), so collapsed results pay too; and the streaming buffer holds
the last 50 nodes, so a tool-heavy turn keeps dozens of results inside the churn.

### 2.3 Why the 09-22 analysis didn't see it

Its benchmark (`bench-markdown-parse.mjs`) and fixes targeted the *streaming
message's* parse (#3521) and *backgrounded* panes (#3536). Both held here: the
in-place incremental path cost 455 ms of 11.5 s. The remaining cost lived in
sibling rows that were not streaming at all, and needed a tool-heavy transcript
in a visible pane to show up.

## 3. Fix (this PR)

1. **`DocumentRow.tsx`** — resolve the row's match once through an equality-gated
   memo: `const dispatchMatch = createMemo(() => props.dispatchMatches?.().get(node.id))`,
   pass `dispatchMatch()`. A memo compares the resolved value with `===`, so Map
   identity churn stops at the row.
2. **`ToolOverlayLog.tsx`** — `scrollable={false}` on both `.md` preview
   `<Markdown>`s, for the same reason `MarkdownBlock` already does it: the
   virtualized document owns the scroll, and a per-block `OverlayScrollbars`
   costs forced layouts on every mount.

Regression tests (`DocumentRow.test.tsx`): a finished Read tool row must keep
the *same* `.agent-tool-read` element across two new-identity `dispatchMatches`
Maps (fails on `main` with "serializes to the same string" — the exact remount);
and a `.md` preview must render `.content.non-scrollable` with no
`[data-overlayscrollbars-initialize]` host.

## 4. Not changed, deliberately

- `correlateDispatchesForBlock` still allocates a new Map per flush. That is cheap
  (it filters tool nodes); the bug was who subscribed to it, not that it exists.
- `renderToolResultBody` still re-runs when `props.node` identity changes
  (running → success, chunk appends). That is the intended path.

## 5. Verification — same load, same script, after HMR delivered the fix

| 15 s window | Frames | rAF gap p50 / p95 | Long frames | Stalled | Forced layout in scripts | Worst frame |
|---|---|---|---|---|---|---|
| A — streaming, no typing | **828 (55 fps)** | 16.7 / 32.7 ms | 12 | 1.04 s (7%) | **48 ms** | 208 ms |
| B — streaming + 20 keys/s | **769 (51 fps)** | 16.7 / 33.4 ms | 34 | 2.6 s (17%) | 387 ms | 142 ms |

`flushPendingNodes`: 13,710 ms / 95 calls → **1,364 ms / 43 calls (~32 ms per
flush)** with typing; 242 ms / 6 calls without.

That first post-fix run was lighter than the baseline (two of the four agents
declined to regenerate the same document — 7.6k vs 12.7k mutations in A). A
second run on a fresh topic with all four streaming, taken ~50 s into 20k-word
generations (so with larger streaming messages than the baseline):

| 15 s window | Frames | rAF gap p50 / p95 | Long frames | Stalled | Forced layout in scripts | Worst frame |
|---|---|---|---|---|---|---|
| A — streaming, no typing | **565 (38 fps)** | 16.7 / 100 ms | 39 | 5.7 s (38%) | 950 ms | 291 ms |
| B — streaming + 20 keys/s | **572 (38 fps)** | 16.7 / 117 ms | 42 | 5.9 s (39%) | 1,152 ms | 295 ms |

Against the baseline's 2.3 fps / 100% stalled / 5,961 ms forced layout, the fix
holds under the heavier condition too. What remains — `flushPendingNodes` at
~45 ms per flush plus `MarkdownBlock`'s trailing render timer at ~1.1 s per
window — is §6 items 1 and 2, whose cost grows with the streaming message's
length, which is exactly why this run reads worse than the first.

## 6. What remains, with numbers

1. **Markdown rebuilds the whole message's DOM per commit** (`markdown.tsx:415`,
   `toJsxRuntime` over frozen+tail hast, then the element tree is swapped).
   #3521 made the *parse* incremental; the DOM is not. Mzks (no tools) re-created
   952 paragraphs, 314 headings, 306 code blocks and 298 tables in 10 s with zero
   rows added. This is most of the remaining 32 ms/flush and of `MarkdownBlock`'s
   `setTimeout` trailing render (585 ms in B above). Fix direction: reuse the
   frozen-prefix DOM and only re-render the tail block.
2. **Pin-to-bottom forces layout up to 3× per flush** — `scrollToTrueBottom`
   from the microtask pin (`AgentDocumentVirtualList.tsx:570`) and two
   ResizeObservers (`:605`, `:654`), each reading `scrollHeight` plus every
   streaming row's `offsetHeight`. 387 ms of forced layout in B is this. Fix
   direction: one layout read per frame, ROs only re-arm the pin.
3. Two dev-launch oddities seen on the way, unrelated to this bug: `task dev`
   spawns **two** `agentmux-srv` processes (launcher-spawned and host-spawned,
   same data dir, different ports), and the srv logs a 100-line `fs_watch`
   retry storm in its first minute.

## 7. How to re-measure

```
# find the dev page target, then (window must be restored, not minimized):
node scripts/ui-screenshots/typing-under-load-experiment.mjs 9223 <targetId> \
  --send "<long unique-heading markdown prompt>" --ramp 40 --secs 15 --kps 20 --cycles 2 --typeInto <paneIndex>
node scripts/ui-screenshots/cpu-profile.mjs 9223 <targetId> 15 out.cpuprofile
```

Every claim in §2 and §5 was produced by these two commands plus `cdp-eval.mjs`
one-liners; nothing here is inferred from reading code alone.
