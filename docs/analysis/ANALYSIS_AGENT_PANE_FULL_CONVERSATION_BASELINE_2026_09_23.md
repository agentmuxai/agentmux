# ANALYSIS: Agent pane cost vs conversation length — `main` baseline (Phase 0)

**Date:** 2026-09-23
**Status:** analysis — Phase 0 baseline of `SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md`, measured on Windows. macOS and Linux baselines not yet recorded.
**Author:** Manoz
**Related:** `docs/specs/SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md` (§4 targets, §7 Phase 0),
`docs/analysis/ANALYSIS_AGENT_PANE_FLUSH_REMOUNT_CHURN_2026_09_23.md` (§6 predicted these costs).

---

## 1. Summary

With 3 agent panes streaming and one composer receiving key repeat, `main`
(8b69500) goes from 44 fps at an empty conversation to ~25 fps once each pane
holds 25 turns, and stays there through 500 turns. Up to 3.4 s of every 10 s
is forced synchronous layout. Renderer memory keeps climbing the whole way,
from 1.2 GB to 3.2 GB.

This matches the spec's model:

- **Drawing cost plateaus** because the always-mounted tail is capped at 50
  nodes: 25 turns × 3 nodes already fills it, and the DOM stays at ~99k
  elements from there on (§6.2 of the spec).
- **Forced layout is large from the start**: ~2.8 s of layout per 10 s even
  at N = 0, and 0.54–3.4 s of it forced from script (§6.1, pin-to-bottom).
- **Memory has no ceiling**: it grows with every turn of history even though
  the DOM does not (§6.3, bounded live document). Most of the growth is
  outside the JS heap (heap 63 → 353 MB, renderer resident 1.2 → 3.2 GB),
  which is worth attributing before Phase 6.
- **Key → paint** is not yet the worst symptom at this load (p95 64–88 ms),
  but the frame rate is: typing lands, the page around it stutters.

## 2. Method

`scripts/ui-screenshots/full-conversation-bench.mjs` (this PR) against a dev
build of `main@8b69500` (`task dev`, CEF 148, Vite), Windows 10, window
630 × 1077 CSS px, 3 visible agent panes (Mopeo, Poal, Oozp; the spec calls
for 4 — a fourth split could not be opened programmatically, recorded as a
deviation). Their resumed history was cleared from the view first
(`--clear-existing`; transcripts untouched).

```
node scripts/ui-screenshots/full-conversation-bench.mjs --target <main page id> \
     --history 0,25,100,200,500 --secs 10 --clear-existing
```

At each history size N (cumulative), every pane holds N synthetic turns
(user message, a Bash tool call with 40 lines of output, a ~20 KB markdown
reply); then, for 10 s, every pane streams a new ~20 KB reply at ~11 commits/s
while the first pane's composer receives 20 keys/s. Every window was valid:
page visible throughout, no stray keys, draft restored on the captured
composer.

## 3. Results

| N (turns per pane) | fps | long frames (share of time) | forced layout in long frames | rAF gap p50 / p95 / max | key → paint p50 / p95 / max | dispatch p95 | DOM elements | layouts / layout time | style time | JS heap | renderer resident (5 procs) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 43.9 | 6.7 % | 542 ms | 16.7 / 50 / 67 ms | 48 / 64 / 72 ms | 31.3 ms | 5,984 | 597 / 2,759 ms | 3,046 ms | 63 MB | 1,212 MB |
| 25 | 24.6 | 38.7 % | 3,431 ms | 33.3 / 83 / 217 ms | 56 / 72 / 104 ms | 45.4 ms | 99,223 | 559 / 3,275 ms | 3,579 ms | 194 MB | 2,280 MB |
| 100 | 26.6 | 31.6 % | 2,286 ms | 33.3 / 67 / 133 ms | 56 / 88 / 128 ms | 35.7 ms | 99,260 | 567 / 2,871 ms | 3,319 ms | 194 MB | 2,643 MB |
| 200 | 24.7 | 36.6 % | 3,165 ms | 33.3 / 83 / 317 ms | 56 / 80 / 96 ms | 45.5 ms | 99,260 | 569 / 3,217 ms | 3,697 ms | 198 MB | 2,791 MB |
| 500 | 26.4 | 28.2 % | 2,248 ms | 33.4 / 67 / 117 ms | 56 / 80 / 96 ms | 37.4 ms | 99,223 | 564 / 2,769 ms | 3,202 ms | 353 MB | 3,153 MB |

Per window of 10 s. "Layouts / layout time" and "style time" are CDP
`Performance.getMetrics` deltas; "forced layout" is the
`forcedStyleAndLayoutDuration` of scripts inside long animation frames.
"Renderer resident" sums every renderer process of the instance (the main
window's plus the pooled windows), so it is a trend, not a per-window figure.
The browser process stayed at ~158 MB throughout.

Against the spec's §4 targets (key→paint p95 ≤ 50 ms, ≥ 55 fps, long frames
≤ 5 %, flat in N, memory flat), `main` misses every one from N = 25 on, and
misses the fps and key→paint targets even at N = 0.

## 4. Observations for the next phases

1. **Phase 1 (pin):** ~560–600 layouts per 10 s at every N — about one per
   frame per pane — and 0.5–3.4 s of forced layout. The pin microtask and the
   pre-layout scroll handler are the expected sources; Phase 1 should remove
   most of the forced share, and the bench's "forced layout" column is its
   exit metric.
2. **Phase 3 (turn-scoped tail):** the 16× DOM jump from N = 0 to N = 25 with
   no further growth is the 50-node tail filling. Phase 3's exit metric is
   that the DOM stays near the N = 0 figure at every N.
3. **Phase 6 (bounded document):** memory grows ~2 GB from N = 0 to N = 500
   with a constant DOM. The JS heap accounts for ~300 MB of it. Before Phase 6,
   attribute the rest (layout objects for virtualized rows, retained markdown
   trees, image/texture memory, pooled renderers) with a heap snapshot and
   `memory-infra` trace, so the bound targets the right thing.

## 5. Caveats

- Windows only; macOS and Linux baselines are still to be recorded, per §7
  Phase 0 of the spec.
- 3 panes, not 4. Load per pane is the same as the spec's; total load is ¾.
- One run per N. The next runs should repeat each N (at least 3) to report
  spread, especially for the frame-time columns.
- Synthetic content is uniform; real conversations mix tool-heavy and
  prose-heavy turns. The fault suite (a later Phase 0 PR) covers the extremes.
