# TRACKING — Agent pane bounded live window (phase progress)

**Date:** 2026-09-23
**Type:** Tracking doc for `SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md` — where each phase stands, what was measured, and what the measurements changed. Not a design spec.
**Status:** living — updated with every phase PR.
**Parent tracker:** `TRACKING_TYPING_AND_TERMINAL_RESPONSIVENESS_2026_09_21.md` §2.1a (the whole typing/terminal responsiveness family).

---

## 1. Phase status

| Phase | What | PR | State |
|---|---|---|---|
| — | Spec | #3588 | merged |
| — | Specs-index generator ported to Node (unblocked fast doc work on Windows) | #3590 | merged |
| — | CDP measurement scripts | #3569 | merged |
| 0 | Full-conversation bench + `main` baseline | #3593 | merged |
| 0b | Bench streams through the real pipeline (`--stream-mode pipeline`, default) | #3598 | merged |
| 1 | Pin-to-bottom without forced layout | #3599 (together with Phase 2 — see §3.1) | merged |
| 2 | Cross-pane stream scheduler, input first | #3599 | merged |
| 2b | A mid-stream pause no longer re-parses the whole message (§3.4) | #3604 | merged |
| 2c | Tool logs measure height only when their branch changes (§3.4, §2.2a) | #3607 | merged |
| 3a | Migration into the head keeps the node's exact position: row gap + height handoff (§2.2b) | #3610 | merged |
| 3b | The tail holds only the turn in flight (§2.2c, §3.7) | #3611 | merged |
| 3 | Tail holds only the turn in flight | 3a + 3b | **done**; kill switch `agent:turnscopedtail` |
| 4 | O(batch + log n) stores | — | **deferred** by the §6.9 revision (live feed keeps n small; stores ≤ 8 % at 200 turns, §3.8) |
| 5 | Node identity and durability — re-planned as 5a–5e (spec §6.3.6) | #3619 (plan), #3620 (5d echo), #3624 (5a design), #3628–#3648 (5a-1…5a-4), #3663, #3701 | **5a done**; 5b, 5c, 5e deferred by §6.9. 5d: user messages reach the transcript for persistent Claude (stdin line, 5a-3c) and the Gemini family (CLI echo, #3620), and — **added by #3701** — for panes on the per-turn subprocess controller: Kimi, muxcode/container Claude, and Codex *not* run by the app-server controller. **Not covered:** app-server Codex, ACP (spec §6.9 PR 4c); also open: the shell journal (PR 4b) |
| 6 | Bounded live document — **revised as the live feed with roll-off (spec §6.9)** | #3700, #3701 | **merged**, on by default; kill switch `agent:livefeed`, K = `agent:livefeedturns` (§2.2d). Every pane but ACP and app-server Codex; turns holding an in-pane shell stay until the journal |
| 7 | History tab follows the transcript (spec §6.9) | #3695 | merged |
| 8 | Off-main-thread markdown (decision) | — | not started |
| 9 | Remove the kill switches once each phase has soaked (the flags themselves are already on by default) | — | not started |
| 10 | `content-visibility` experiment | — | not started |

## 2. Measurements

All on Windows 10, dev build, 3 visible agent panes streaming a synthetic
~20 KB reply each while one composer receives 20 keys/s,
`scripts/ui-screenshots/full-conversation-bench.mjs`. macOS and Linux not yet
measured.

**What a "run" is.** The replies are paced for 10 s (`--secs 10`, a chunk
every 90 ms), but the measurement window lasts until every pane has received
its whole reply — so a slower build takes longer to deliver the same content
(14–42 s observed). Times below are totals over that run, not per 10 s
(earlier revisions of this doc mislabelled them "per 10 s"). "Time to deliver"
is reported as its own row where it differs.

### 2.1 Phase 1 + 2 vs `main`, real pipeline, medians of 3 clean runs

| | `main` (03dfcd6) | Phase 1 + 2 |
|---|---|---|
| key → paint p95, N = 0 / 25 turns | 200 / 216 ms | **136 / 144 ms** (−32 %, −33 %) |
| key → paint p50 | 72 / 88 ms | **48 / 56 ms** |
| fps | 14.2 / 8.8 | **15.4 / 12.3** |
| time in long frames blocking input, per run | 9.3 / 13.0 s | **8.2 / 9.6 s** |
| forced layout in long frames | 4.0–5.3 s | **0.3–0.7 s** |

Per-run values (run 1 of each was discarded: a test suite was running on the
same machine): `main` N = 25 fps 6.3 / 9.9 / 8.8 with one run's typing p95 at
1,792 ms; Phase 1 + 2 N = 25 fps 11.5 / 12.3 / 17.2, typing p95 136–160 ms in
every run. Phase 1 + 2 is also the steadier of the two.

Reproduce: serve each variant from the dev worktree and run
`node scripts/ui-screenshots/full-conversation-bench.mjs --target <main page id> --history 0,25 --secs 10 --clear-existing`
(`--stream-mode pipeline` is the default since #3598).

### 2.2 Phase 2b (markdown stays incremental through a settle), medians of 3

Same bench, same panes, A = Phase 1 + 2, B = A + this fix (only
`markdown.tsx` / `MarkdownBlock.tsx` differ), interleaved A/B/A/B/A/B, each
run after a page reload, a 30 s settle and a discarded 3 s warm-up.

| N = 0 / 25 turns | Phase 1 + 2 | + 2b |
|---|---|---|
| key → paint p95 | 152 / 168 ms | **96 / 112 ms** (−37 %, −33 %) |
| key → paint p50 | 64 / 80 ms | **56 / 64 ms** |
| fps | 15.8 / 11.7 | **35.9 / 24.0** (2.3×, 2.1×) |
| share of time in long frames | 70 / 72 % | **27 / 41 %** |
| time in long frames blocking input, per run | 8.2 / 9.1 s | **1.8 / 3.3 s** |
| script time, per run | 11.7 / 12.5 s | **2.8 / 3.5 s** |
| layout time, per run | 3.1 / 3.3 s | 2.6 / 2.9 s |
| time to deliver the 10 s stream | 23.8 / 28.2 s | **14.0 / 16.8 s** |

Every B run beat every A run on fps and typing p95 (A fps 13.4–17.7 at
N = 0, B 35.0–36.0). Script time — not layout — was the cost: it fell ~4×,
while layout moved only as much as the shorter run. Raw:
`md-{A,M}-r{1,2,3}.json` from `md-ab.sh` (dev worktree harness, not
committed).

### 2.2a Phase 2c (tool logs measure only on a branch change)

A = Phase 1 + 2 + 2b, B = A + 2c; both on the bench with #3606's settle
(§3.6), interleaved ×3.

**Mounting history** — `inject(25 turns)` into 3 panes, timed in the page,
3 cycles per round (`probe-inject.mjs`):

| | A | + 2c |
|---|---|---|
| synchronous mount task (9 cycles each) | 8.3–9.4 s (median 8.8 s) | **5.7–6.4 s (median 6.1 s), −31 %** |
| until the page is quiet again | 10.2–12.2 s (median 11.2 s) | 10.3–11.5 s (median 10.7 s) |

Every B mount task was shorter than every A one. This is the longest single
main-thread block in the bench — the same shape as opening or restoring a
pane with history. The follow-on work until quiet (trailing markdown renders
and the like) barely moved.

**Streaming window** — no measurable change: N = 25 fps 22.0 vs 21.4, typing
p95 112 vs 112 ms, forced layout 253 vs 253 ms. In the bench the history tools
are collapsed, so each `isMeasurable` walk stopped at the hidden panel after
a couple of ancestors; a real conversation with open tool panels walks the
whole chain. N = 0 mounts no tool log at all, so its A/B difference (fps 32
vs 27.4 median) is noise — a useful floor: consecutive runs of identical code
paths differ by up to ~15 % fps.

### 2.2b Phase 3a (a migrating node keeps its exact position)

Live probe on one pane (`probe-jump.mjs`): 60 markdown nodes of varied
height, scrolled so a streaming-buffer row sits mid-viewport (not pinned),
then 8 appends — each forces one node out of the full 50-node buffer into the
head. Recorded: how far that on-screen row moves over the next 4 frames. Two
interleaved rounds each.

| | on-screen movement per migration |
|---|---|
| `main` (+ 2b, 2c) | **−18 to −141 px, every append, and it stays** |
| + 3a | **0 px in all 16 appends** |

The row that migrates itself (watching buffer row 0 instead of row 10):
**−4 px on every migration** with the first version of 3a — `.agent-document`'s
own flex gap made the head/buffer seam two gaps (Codex P2 on #3610) — and
**0 px** once the virtualizer cancels it (`margin-bottom: -4px`, tied to
`ROW_GAP_PX` by a test).

On `main` the shift never corrects: the migrated node is far above the
viewport, so it is never mounted in the head, never measured, and its
estimate error (plus the 4 px gap the head did not have) stays in the layout.
Anyone reading the buffer while an agent streams saw the text jump on every
migration. With 3a the store places head rows `ROW_GAP_PX` apart (the buffer's
flex gap, tied by a test) and a migrating node enters with the height the
buffer last laid it out at (ResizeObserver, no forced layout), in the same
batch that adds it.

### 2.2c Phase 3b (the tail holds only the turn in flight)

A = `main` + 3a (tail = last 50 nodes), B = A + 3b; bench with #3606's
settle, interleaved ×3.

**Streaming + typing, medians of 3:**

| N = 0 / 25 turns | A | + 3b |
|---|---|---|
| DOM elements | 6,025 / 99,267 | 6,028 / **14,820** (−85 % at N = 25) |
| fps | 33.5 / 23.4 | 35.7 / **34.3** |
| key → paint p95 | 112 / 120 ms | 96 / **96 ms** |
| share of time in long frames | 34 / 44 % | 27 / **28 %** |
| time in long frames blocking input, per run | 2.5 / 3.5 s | 1.8 / **1.9 s** |
| JS heap | 113 / 249 MB | 109 / **132 MB** |

Every B run beat every A run at N = 25 (fps 34.2–35.1 vs 17.6–25.0; p95
96–104 vs 120–144 ms). **N = 25 is now indistinguishable from N = 0** — the
first measurement in this series where the cost of streaming does not grow
with the history above it (spec §4, "flat in conversation length").

**Mounting history** (`probe-inject.mjs`, 25 turns × 3 panes, 6 cycles each,
first version of 3b): synchronous mount 5.5–7.1 s → **2.75–3.14 s**
(−53 %); until quiet 9.9–12.5 s → **5.7–7.2 s**.

**Per pane after a history load** (`probe-dom.mjs`): pinned panes mount the
last turn (3 nodes) and no head rows; a pane scrolled to the top mounts the
last turn plus ~10 head rows in view — 5.5k elements, where the count policy
mounted 33k and the first version of 3b 44k (§3.7).

### 2.2d The live feed: finished turns roll off (spec §6.9, PR 3)

Same dev build, `agent:livefeed` off (every turn stays, paging on) vs on
(K = 3), 3 panes, bench history injected at N turns (20 KB each).

**After the load, forced GC (`HeapProfiler.collectGarbage` ×3), N = 200, two
runs each:**

| | off | on |
|---|---|---|
| document nodes per pane | 602 | **14–20** (4–6 turns: K finished + the one in flight + any on screen) |
| JS heap | 68.1 / 67.3 MB | **56.1 / 55.9 MB** (−17 %) |
| live DOM nodes | 26.8k | 26.7k — unchanged: Phase 3b already mounts only what is on screen |

The heap saving is the rolled-off turns' content (~200 × 20 KB per pane
here); it grows with what real turns carry (tool output, long replies).

**Streaming + typing, interleaved ×3, medians** (N = 0 / 25 / 200): fps
59.2–59.8 and key → paint p95 64 ms in every cell, on and off — this machine
was quiet (§3.9); script time per run 2,137 / 2,013 / 2,088 ms off vs
2,095 / 1,862 / 1,917 ms on. Heap and live-DOM readings inside those windows
are GC-timing noise (e.g. off read 127 MB at N = 25 and 87 MB at N = 200), so
the table above is the memory result.

### 2.3 `main` baseline, direct mode (Phase 0)

`docs/analysis/ANALYSIS_AGENT_PANE_FULL_CONVERSATION_BASELINE_2026_09_23.md`.
Direct mode injects straight into the document store; see §3.2 for why those
absolute numbers understate real cost.

## 3. Findings that changed the plan

### 3.1 Phase 1 alone makes typing worse — it ships with Phase 2

Removing the forced layouts (Phase 1) took forced layout from 0.5–3.4 s per
run to ~0 and made each stream dispatch 10–30× cheaper, **but total layout
time did not change** (~3 s per run either way) and key → paint p95 got worse
(80 → 128 ms at N = 25 in direct mode). The layout had moved, not gone: on
`main` each pane's forced layout ran inside that pane's own small task, and
input could be handled between tasks; with Phase 1 all panes' layout lands in
one rendering step per frame, which input waits behind. Blocking time in long
frames went up 10–20×.

That is exactly what Phase 2's scheduler addresses — while the user is typing,
one pane flushes per frame (two once the oldest has waited 100 ms, never more)
— and together they are the §2.1 result.
The spec's per-frame *script* budget was replaced by a *panes-per-frame* cap
for the same reason: the cost is layout in the rendering step, which a script
budget cannot see.

### 3.2 The bench's first stream mode bypassed the real pipeline

Phase 0's bench dispatched synthetic content straight into the document store.
That skipped the translator, parser and flush queue — so it could not measure
Phase 2 at all (Phase 2 read as "no effect"), and it understated real streaming
cost about 3× (N = 0: direct 37–44 fps vs pipeline 13–15 fps on the same
panes). The bench now pushes Claude stream-json NDJSON into each pane's live
output subject by default.

### 3.3 Layout cost is ~constant; what grows with history is script

Total layout time is ~2.8 s per run at N = 0 (6k DOM elements) and ~3.1 s at
N = 25 (99k): it is a cost of streaming three panes, not of history. Script
time grows with history (+1.9 s per run from N = 0 to N = 25): the O(n)
reducer copy (Phase 4), Solid work over the 50-node tail (Phase 3), and
`MarkdownBlock`'s trailing-render timer.

### 3.4 The largest remaining cost was markdown re-parsing on a settle guess

In the real pipeline, `flushPendingNodes` averaged ~40 ms per flush and
`MarkdownBlock`'s trailing-render timer added 1.4–1.8 s per 4 s window at
N = 0. A CPU profile of Phase 1 + 2 in pipeline mode (8.1 s window, 3 panes
streaming, typing) named both:

| Where | Share |
|---|---|
| Inside `flushPendingNodes` (1,043 ms): `markdown.tsx` segment parse + render | **64 %** |
| Markdown render outside the flush (the trailing timer) | ~1,000 ms of the window |
| `resize-contract.ts` `isMeasurable` (`getComputedStyle` up every ancestor), from `ToolOverlayLog`'s per-chunk `beginHeightContinuity` | 4.8 % self |
| `useHistoryPagination` initial restore | 11.8 % — one-off pane mount inside the window, not a streaming cost |

Cause of the markdown cost: `MarkdownBlock` guesses "settled" from a 90 ms
quiet window and renders highlighted; the next chunk renders un-highlighted.
Each flip changed the processor the frozen (already-parsed) prefix had been
rendered with, invalidating it, so every pause longer than 90 ms re-parsed the
**whole** message twice — and every message ended with one full highlighted
re-parse. Real streams pause that long constantly; so does the bench under
Phase 2's scheduler. Deterministic test: a stream with a 200 ms pause every
4th chunk parsed 11.4× the message.

Fix (Phase 2b): `highlight` keeps meaning the document's final quality; a new
`streaming` prop affects only the trailing open block. Closed blocks are
rendered once, at final quality, so a streaming ↔ settled flip re-renders the
tail and nothing else — now < 2.5× in the same test, and the settled DOM
matches a from-scratch render. Measured effect: §2.2.

The other named cost, `isMeasurable`, was the tool-log height FLIP
re-baselining on every node update: every flush hands each mounted tool log a
new node object, and each baseline was `getComputedStyle` up the ancestor
chain plus `scrollHeight`/`offsetHeight`. Phase 2c measures only when the
rendered branch changes, capturing the "from" height in a `createComputed`
(pure computations run before Solid's render effects, so the DOM still shows
the old branch). Measured effect: §2.2a — mostly on mounting history, where
it had been 3.1 s of a 10 s injection.

### 3.5 Rejected: the streaming buffer's flex layout

Hypothesis: the buffer is a flex column, so one child growing re-lays out all
siblings. Tested by overriding it to `display: block` on the live page:
forced layout 2,117 vs 2,129 ms per run — no difference (rows already carry
`contain: layout style`). Left as is.

### 3.6 History setup leaked into N > 0 windows

The bench injected history, slept a fixed 1.5 s, and opened the window.
Injecting 25 turns × 3 panes is 7–10 s of synchronous script with more queued
behind it (trailing markdown renders, resize observers, the injection frame's
own rendering), so a timing-dependent share of it was measured as part of the
window. Found while A/B-testing the tool-log fix: every run of one build
counted a single 7.5–8 s long-frame entry at N = 25 and no run of the other
did, which doubled "share of time in long frames" and read as a regression.
A CPU profile of the same runs showed the opposite — the fix removed 3.1 s of
`isMeasurable` from the injection itself.

Fix: every window now starts once the page has gone 1 s without a frame gap
over 50 ms (capped at 30 s), recorded as `settle: { quiet, ms }`
(`waitQuiet` in `scripts/ui-screenshots/lib/bench-page.js`). The soak loop,
which had no settle at all, gets the same. N = 0 windows were at most lightly
affected (only a clear precedes them, no injection); N > 0 figures taken
before this carry some setup cost and are best compared only within the same
A/B.

### 3.7 Deferring a move to protect visible rows must stay small

3b moves the frontier only across rows wholly above the viewport, so nothing
the user is looking at is remounted. The first version made the bench's N = 25
DOM **larger** (143k vs 99k): a history load into a pane scrolled to the top
kept all 76 of its rows mounted, because the head/buffer split is contiguous
and one visible row at the start of the buffer held back the whole batch
after it (the 2×-ceiling hard cap, 80 nodes, never triggered). Deferral now
covers at most 12 nodes / 256 KB; beyond that the move happens and the
visible rows are remounted — jump-free since 3a. Regression test in
`AgentDocumentVirtualList.turn-tail.test.tsx`.

**Where 3b differs from spec §6.2's text:** migration is not scheduled on
`turnJustEndedAtom` as a housekeeping task. The frontier is recomputed inside
the partition memo on every document change and moves as soon as rows are
off-screen (or the deferral cap is hit). That needs no turn-end signal,
keeps invariant 1 trivially (one memo), and each move is cheap and jump-free
after 3a. The spec's per-node in-row windowing for single huge nodes (§6.2)
is not part of 3b.

### 3.8 After Phase 3: what the cost is now, and what it is not

Measured on `main` + 3b (Windows dev build, 3 panes streaming, typing):

- **History length up to 200 turns (~600 nodes per pane) does not move
  streaming cost.** One N = 0/25/100/200 run read 28.3/32.0/35.7/25.5 fps;
  three more runs of 0/100/200 showed the cost falling *through each run and
  across runs* (N = 0 went 27 → 18 fps from the first run to the third) —
  an order effect, not a history effect. Rendered DOM stays 14.8k at every
  N ≥ 25. A CPU profile of the streaming work at N = 200 puts the stores and
  the per-flush list bookkeeping at ≲ 8 %; rendering the growing last
  message dominates (markdown ~35 %, `replaceChild` of the open block ~23 %).
  Phase 4 (O(log n) stores) is therefore not the next bottleneck at these
  sizes.
- **The run-to-run drift is environmental and peak-driven, not a leak.** Six
  identical N = 0 runs back to back right after a reload did not degrade
  (31 → 35 fps; JS heap flat at 68–94 MB; renderer RSS +250 MB after the
  first run, then ~15 MB per run). The machine is shared (another app's
  audio engine, ~20 CEF processes from other dev instances, 25–45 % CPU load
  between runs). Five load-200-turns/clear cycles with forced GC: JS heap
  after clear 37.9 → 42.9 MB — identical with and without 3b, so it
  predates this work; a heap snapshot after three cycles holds only 749
  detached DOM objects, mostly pane-header chrome retained by block-frame
  closures (not the agent pane). What does grow is the renderer's resident
  high-water mark after large loads — which 3b already lowers a lot (loaded:
  heap 180 → ~65 MB, DOM 223k → ~35k nodes for 200 turns) and Phase 6
  bounds.

### 3.9 Rejected: updating the open block's text in place

§3.8 attributed ~23 % of streaming work to `replaceChild` of the growing last
block. Tried: when a commit only lengthens the open block's last text run
(same element shape, same other text), set that DOM text node's `data`
instead of rebuilding the block. Built behind a setting and A/B-tested on one
dev build (`main` 8b69b29 + the change), interleaved ×3, 3 panes, N = 0 / 25:

| N = 0 / 25 turns, medians of 3 | rebuild (as `main`) | in place |
|---|---|---|
| fps | 59.6 / 59.5 | 59.7 / 59.6 |
| key → paint p95 | 64 / 64 ms | 64 / 64 ms |
| script time per run | 1,968 / 1,817 ms | 1,907 / 1,692 ms |

Three CPU-profile pairs (9 s each, 3 panes streaming, typing): `replaceChild`
0–4 ms per window either way, markdown (inclusive) 483 / 252 / 255 ms in
place vs 656 / 245 / 270 ms rebuilding — noise. Two reasons: the cost §3.8
named is not there on current `main` at these sizes (not re-measured at
N = 200), and the bench's reply (~185 characters per commit, dense inline
markup, short blocks) changes the open block's shape on most commits, so
only 41 of 284 commits could be patched at all. Not shipped: a setting and a
text-node patch path for no measured gain. The machine was quiet (every run
60 fps), so this bounds the gain rather than proving it zero on a loaded one.

**Found on the way — a real bug.** Frozen segments (#3559) are rendered under
their own `createRoot` so their components outlive later commits, but
`solid-js/h` returns elements as lazy thunks that Solid only calls from its
insert effect — so the components were created under that effect instead and
disposed on the next commit, while their DOM stayed. Visible effect: a table
in a long streamed reply never showed "✓ copied" (and never would, since
frozen segments are kept after the stream ends); an image there would never
have resolved had a caller passed `resolveOpts` (none does today). Fixed by
resolving the thunks inside the segment's root; regression tests in
`markdown-frozen-owner.test.tsx` fail on `main` and pass with the fix.

## 4. Open follow-ups

- **Bench pipeline mode** (`--stream-mode pipeline`, default) — #3598; §2.1's
  numbers were taken with it.
- **Mounting history: ~1.2 s per load** for 25 turns × 3 panes with 3b's
  review fixes (down from ~8.8 s before 2c/3b). Profiled: 82 % is parsing the
  markdown of rows that are actually on screen (the visible 20 KB last message
  and visible head rows) — no longer hidden rows. What lowers it further is
  in-row windowing (below) or off-main-thread parsing (Phase 8).
- **Per-flush work still proportional to the head** (Phase 4): the slice-
  feeding effect re-maps every head id and `JSON.stringify`s each head row's
  expansion on every partition change, and the reducer copies the node array.
  3b made the frontier search itself independent of history (`from`, Codex P2
  on #3611); these are the remaining O(history) costs per stream flush — under
  ~8 % of streaming work at 200 turns (§3.8), so Phase 4 waits. The reducer's
  copy alone, benched in isolation: 0.056 / 1.14 / 19.3 ms per append at
  1k / 10k / 100k nodes (200 turns ≈ 600 nodes) — it matters only far beyond
  the sizes Phase 6 will allow.
- **Renderer memory after large loads** (§3.8): a resident high-water mark
  from peak loads, not an agent-pane leak. Phase 6 (bounded live document)
  bounds the peak; the 8 h soak verifies it.
- **Phases 5b, 5c, 5e are deferred** (spec §6.9): the live feed shipped
  (#3700, #3701) on 5a alone. They come back only if scroll-back in the feed
  or History anchored to a turn is wanted. What remains is listed under
  "Live feed follow-ups" below.
- ~~**The growing last message's DOM is replaced on every commit**~~ —
  looked at (§3.9): an in-place update was built and measured, no gain on
  current `main`; not shipped. The look found a real bug instead, fixed
  alongside (§3.9).
- **Pane-header chrome retained by block-frame closures** (§3.8): ~13 copies
  of header buttons/icons survive a clear. Small; outside the agent pane.
- **In-row windowing for one huge node** (spec §6.2) — not in 3b.
- **macOS and Linux baselines** (spec §7 Phase 0).
- **Fault-suite runner** (spec §8) — a later Phase 0 PR.
- **Live feed follow-ups (§6.9):** the shell journal (`out-of-band.jsonl`)
  so turns holding an in-pane shell can roll off and History shows shells;
  rebuilding AskUserQuestion's styled answer on replay (the answer is already
  in the tool result); ACP and the Codex app-server controller writing the
  user's message; opening History at the first kept turn (needs 5b). Not yet
  verified with a real signed-in agent on a dev build — the dev instance's
  agents had no credentials; the load-time roll-off was checked on real
  transcripts (a long pane opened at 4 turns).
- **Send-and-record ordering across stdin writers** (found in review of
  #3703): every persistent-controller path writes a stdin line and then
  records it in the transcript as two steps — `send_message`'s
  `DeliverDirect` (`deliver_direct`, then `persist_message_to_blockfile`),
  the queue drain (`tx.send(..).await`, then persist), the muxbus path
  (`append_delivered_message`) and the dead-air fallbacks. Two writers
  racing within that window can be recorded in the opposite order to the one
  the CLI received. Rare (it needs two sends within milliseconds), but History
  and conversation recovery would replay it that way. Fix: one per-controller
  send-and-record ordering lock taken by every path, the async drain
  included — its own PR, since it touches the hottest send path.
- **Residual nodes after a clear:** a cleared pane can refill with a few
  transcript nodes; the bench records them (`residualNodes`).
