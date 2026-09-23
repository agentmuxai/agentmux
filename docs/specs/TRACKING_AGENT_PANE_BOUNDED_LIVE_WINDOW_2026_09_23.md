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
| 0b | Bench streams through the real pipeline (`--stream-mode pipeline`, default) | this PR's companion (see §4) | open |
| 1 | Pin-to-bottom without forced layout | this PR (together with Phase 2 — see §3.1) | open |
| 2 | Cross-pane stream scheduler, input first | this PR | open |
| 3 | Tail holds only the turn in flight | — | not started |
| 4 | O(batch + log n) stores | — | not started |
| 5 | Node identity and durability | — | not started |
| 6 | Bounded live document | — | not started |
| 7 | History tab follows | — | not started |
| 8 | Off-main-thread markdown (decision) | — | not started |
| 9 | Default on | — | not started |
| 10 | `content-visibility` experiment | — | not started |

## 2. Measurements

All on Windows 10, dev build, 3 visible agent panes streaming a synthetic
~20 KB reply each while one composer receives 20 keys/s, 10 s windows,
`scripts/ui-screenshots/full-conversation-bench.mjs`. macOS and Linux not yet
measured.

### 2.1 Phase 1 + 2 vs `main`, real pipeline, medians of 3 clean runs

| | `main` (03dfcd6) | Phase 1 + 2 |
|---|---|---|
| key → paint p95, N = 0 / 25 turns | 200 / 216 ms | **136 / 144 ms** (−32 %, −33 %) |
| key → paint p50 | 72 / 88 ms | **48 / 56 ms** |
| fps | 14.2 / 8.8 | **15.4 / 12.3** |
| time in long frames blocking input, per 10 s | 9.3 / 13.0 s | **8.2 / 9.6 s** |
| forced layout in long frames | 4.0–5.3 s | **0.3–0.7 s** |

Per-run values (run 1 of each was discarded: a test suite was running on the
same machine): `main` N = 25 fps 6.3 / 9.9 / 8.8 with one run's typing p95 at
1,792 ms; Phase 1 + 2 N = 25 fps 11.5 / 12.3 / 17.2, typing p95 136–160 ms in
every run. Phase 1 + 2 is also the steadier of the two.

Reproduce: serve each variant from the dev worktree and run
`node scripts/ui-screenshots/full-conversation-bench.mjs --target <main page id> --history 0,25 --secs 10 --clear-existing`
(needs `--stream-mode pipeline`, the default once §4 lands).

### 2.2 `main` baseline, direct mode (Phase 0)

`docs/analysis/ANALYSIS_AGENT_PANE_FULL_CONVERSATION_BASELINE_2026_09_23.md`.
Direct mode injects straight into the document store; see §3.2 for why those
absolute numbers understate real cost.

## 3. Findings that changed the plan

### 3.1 Phase 1 alone makes typing worse — it ships with Phase 2

Removing the forced layouts (Phase 1) took forced layout from 0.5–3.4 s per
10 s to ~0 and made each stream dispatch 10–30× cheaper, **but total layout
time did not change** (~3 s per 10 s either way) and key → paint p95 got worse
(80 → 128 ms at N = 25 in direct mode). The layout had moved, not gone: on
`main` each pane's forced layout ran inside that pane's own small task, and
input could be handled between tasks; with Phase 1 all panes' layout lands in
one rendering step per frame, which input waits behind. Blocking time in long
frames went up 10–20×.

That is exactly what Phase 2's scheduler addresses — while the user is typing,
at most one pane flushes per frame — and together they are the §2.1 result.
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

Total layout time is ~2.8 s per 10 s at N = 0 (6k DOM elements) and ~3.1 s at
N = 25 (99k): it is a cost of streaming three panes, not of history. Script
time grows with history (+1.9 s per 10 s from N = 0 to N = 25): the O(n)
reducer copy (Phase 4), Solid work over the 50-node tail (Phase 3), and
`MarkdownBlock`'s trailing-render timer.

### 3.4 The largest remaining cost is not yet a named phase

In the real pipeline, `flushPendingNodes` averages ~40 ms per flush and
`MarkdownBlock`'s trailing-render timer adds 1.4–1.8 s per 4 s window at N = 0.
Neither is directly targeted by Phases 3–8. Next step: profile a pipeline-mode
window (CPU profile + LoAF attribution, both in `scripts/ui-screenshots/`) and
either fold the fix into Phase 3 or add a phase.

### 3.5 Rejected: the streaming buffer's flex layout

Hypothesis: the buffer is a flex column, so one child growing re-lays out all
siblings. Tested by overriding it to `display: block` on the live page:
forced layout 2,117 vs 2,129 ms per 10 s — no difference (rows already carry
`contain: layout style`). Left as is.

## 4. Open follow-ups

- **Bench pipeline mode** (`--stream-mode pipeline`, default) — companion PR to
  this one; §2.1's numbers were taken with it.
- **Profile the ~40 ms flush** (§3.4).
- **macOS and Linux baselines** (spec §7 Phase 0).
- **Fault-suite runner** (spec §8) — a later Phase 0 PR.
- **Residual nodes after a clear:** a cleared pane can refill with a few
  transcript nodes; the bench records them (`residualNodes`).
