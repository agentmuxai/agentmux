# Showing a whole window tab at once: drawing never-shown agent panes while the window is idle

**Status:** implemented behind `window:prewarmagenttabs`, off by default (task #3286).
**Date:** 2026-10-25
**Author:** Camper@narko

## 1. The problem

Switching to a window tab for the first time showed its panes one by one: the terminals, Swarm and browser panes were there at once, and the agent transcripts filled in after the reveal. The operator asked for the whole tab to appear at once.

## 2. Cause

An agent pane in a hidden window tab is kept mounted but does as little as it can (`agent-dormancy.tsx`):

- it draws no markdown and builds no tool bodies;
- it keeps only the live feed (the turn in flight and the last few turns), and the fill of older history stays paused (`fillPaused`).

A hidden pane held 2 rows where the same pane shown held 24. On the first visit all of the rest is drawn in the frames after the reveal. Measured on a six-tab layout with two agent panes in hidden tabs, nodes added to the agent panes after the reveal: 43 and 34. A longer wait at the reveal would only move that work; the work itself has to happen before the visit.

## 3. What was built

A lease (`store/agent-prewarm.ts`): one agent pane at a time is let to draw as if shown while its tab stays hidden, then goes dormant again. What it drew stays. A scheduler (`workspace/agent-prewarm-driver.ts`) grants it:

- only with the setting on, the window visible, and no tab switch, resize or input in the last 2 s (checked again after the memory read, so typing in the composer is never raced);
- only to panes of tabs never shown in this window; a tab's panes before the next tab's;
- only while the host's memory pressure is normal;
- with a lease cap of 4 s; a pane that closes mid-lease gives it back.

`useRenderDormant(hidden, blockId)` reads the lease, so the one flag that already gates markdown, tool bodies and the history fill gives way for the leased pane (a hidden window stays dormant whatever the lease says).

## 4. Measurements

Dev build on current main, six window tabs (two with an agent pane, one pane each), window idle, setting off and on, page reloaded before each run:

| | prewarm off | prewarm on |
|---|---|---|
| Rows in a hidden agent pane | 2 | 24 |
| Agent nodes added after the reveal (tab with pane A / pane B) | 43 / 34 | 0 / 0 |
| JS heap after a forced collection | 104 MB | 114 MB (+9.7 MB for two panes) |
| DOM nodes in the page | 16,133 | 16,463 (+330) |
| Main-thread work per pane | none | about 200 ms, while the window is idle |

The per-pane line in the console (`[prewarm] pane … nodes, heap grew … (before collection)`) shows the nodes a pane added and the heap growth before collection, which includes garbage and reads high (14 to 36 MB here); the figures in the table are after a collection.

The panes in this layout have short histories. A pane holds at most the live feed's turns and a bounded history fill, so its cost is bounded, but the real cost per pane on a long-running agent has to be read from the log lines on the operator's own layout before a default is chosen.

## 5. Decision for the default

Off. Ask the operator to turn it on for a week and send the `[prewarm]` lines (they are in the window's log), with the memory figures of a typical layout; then decide. The scheduler is cheap to turn on by default later: it already waits for idle and backs off under memory pressure.

## 6. Not done

- No Settings pane control: the key is in `settings.json` and the template only.
- Panes in the same window tab but behind another pane tab (a kept-alive tab in a pane's tab strip) are not prewarmed; this is about whole window tabs.
- Terminals, Swarm and browser panes need nothing: they already show at once.
