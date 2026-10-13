# Plan: renderer memory fixes, in ROI order

**Date:** 2026-10-12
**Status:** Active. Item 1 is in progress.
**Evidence:**
- `docs/reports/REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md`: what was measured and how.
- `docs/reports/REPORT_MEMORY_FOOTPRINT_500MB_BUDGET_2026_10_12.md`: the 500 MB budget.

**How it works:**
- One item at a time.
- Each item gets its own spec, written before its code. The spec ships in the same PR as the code.
- Each PR is measured with the lab workload from the root-cause report: 4 or 12 scripted agents, a forced-GC memory-infra dump, and a heap snapshot.

## Order

ROI is the measured (or expected) saving divided by effort and risk. The 12-agent figures are from the lab's live-like run: 12 scripted agents over 4 tabs, 40 rounds, after a forced GC.

| # | Item | Measured saving | Effort | Risk | Spec |
|---|---|---|---|---|---|
| 1 | **`<Key>` slots retain old rows.** `keyArray`'s slot closure keeps the reconcile result array alive. | DOM nodes 497k → 249k; renderer −90 MB with 4 panes. Also helps the tile layout. | S | Low | `SPEC_KEYED_SLOT_RETENTION_FIX_2026_10_12.md` |
| 2 | **Tool bodies are never unmounted.** A body mounts on first open, and every live tool call opens. | Together with #1: renderer −53% (4 panes) and −56% (12 panes); DOM −94% to −97%. | M | Medium (collapse transition, measured heights, cost of reopening) | to write |
| 3 | **The spare-window pool.** Up to 3 warm renderers when idle. | About 110 MB if the pool is cut to one window (3 × 52–58 MB measured). | S | Low–medium (window-open latency) | to write |
| 4 | **The commit attribution log mixes instances.** `agentmux_mb` adds up every `agentmux*` process on the machine. | Nothing directly; makes field memory data trustworthy. | S | Low | to write |
| 5 | **Recycle the renderer when idle and over budget.** Blink's heap is 89–92% empty after a long session. | Reload: 1,009 → 338 MB (12 panes), 407 → 198 MB (4 panes). | M–L | Medium–high (must be invisible) | to write |
| 6 | **Hibernate agent panes in hidden tabs.** They build and keep their full DOM today. | Hidden panes cost about 0; cost stays flat as tabs grow. | L | Medium–high | to write |
| 7 | **Unload tool output from finished rows.** The stores keep the text. | About 97 MB of strings in the 12-pane run. | M | Low–medium | to write |
| 8 | **The GPU process.** 394 MB in the field against 81–98 MB in the lab. | Unknown: needs measuring first (browser and media panes, Sysinfo charts, a large display). | S–M | Low | to write |

**Why this order**
- #1 is a two-line fix with a heap-snapshot proof, so it goes first.
- #2 is the biggest saving, and #1 makes it fully effective.
- #3 and #4 are small and independent.
- #5 and #6 are the larger changes that make the 500 MB budget hold at any pane count. #1 and #2 make both cheaper: lower peaks mean less fragmentation, and less DOM means faster restores.
