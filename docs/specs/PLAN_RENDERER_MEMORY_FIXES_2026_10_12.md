# Plan: renderer memory fixes, in ROI order

**Date:** 2026-10-12
**Status:** Active. Items 1–3 are in review (#4751, #4756, #4767).
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
| 1 | **`<Key>` slots retain old rows.** `keyArray`'s slot closure keeps the reconcile result array alive. | DOM nodes 497k → 249k; renderer −90 MB with 4 panes. Also helps the tile layout. | S | Low | `SPEC_KEYED_SLOT_RETENTION_FIX_2026_10_12.md` (#4751) |
| 2 | **Tool bodies are never unmounted.** A body mounts on first open, and every live tool call opens. | Together with #1: renderer −53% (4 panes) and −56% (12 panes); DOM −94% to −97%. | M | Medium (collapse transition, measured heights, cost of reopening) | `SPEC_TOOL_BODY_UNMOUNT_ON_COLLAPSE_2026_10_12.md` (#4756) |
| 3 | **Opening a widget in a new window or floating pane is slow** (the operator's report; it replaced "shrink the spare-window pool"). The warm pools were shed at Warn, which this machine is at most of the day; the pane pool never refilled; cold new windows had no cache. | Floating panes from a pool: 0 → 23 of 42 opens; cold new windows load 8 of 8 JS files from cache (was 0 of 7). The pool stays the same size. | S–M | Medium (shared request context for secondary windows) | `SPEC_WINDOW_OPEN_LATENCY_AND_WARM_POOLS_2026_10_13.md` (#4767) |
| 4 | **The commit attribution log mixes instances.** `agentmux_mb` adds up every `agentmux*` process on the machine. | Nothing directly; makes field memory data trustworthy. | S | Low | [SPEC_INSTANCE_MEMORY_ATTRIBUTION_2026_10_18](SPEC_INSTANCE_MEMORY_ATTRIBUTION_2026_10_18.md) |
| 5 | **Recycle the renderer when idle and over budget.** Blink's heap is 89–92% empty after a long session. | Reload: 1,009 → 338 MB (12 panes), 407 → 198 MB (4 panes). | M–L | Medium–high (must be invisible) | to write |
| 6 | **Hibernate agent panes in hidden tabs.** They build and keep their full DOM today. | Hidden panes cost about 0; cost stays flat as tabs grow. | L | Medium–high | to write |
| 7 | **Unload tool output from finished rows.** The stores keep the text. | About 97 MB of strings in the 12-pane run. | M | Low–medium | to write |
| 8 | **Window churn leaks.** About 60 open/close cycles took the lab from 189 MB to 1.47 GB: recycled pool windows grow to about 120 MB with 13 documents each, and more of them stay parked than the pool's cap. | Up to about 1.2 GB after heavy window use. | M | Medium | to write |
| 9 | **Floating panes lose their parent.** Half of the floating opens log "could not resolve parent main HWND", so they don't follow the main window on minimize or close. | Correctness, not memory. | S | Low | to write |
| 10 | **Toolchain is slow to settle,** about 3.6 s even in a warm window. | Latency of one widget. | S | Low | to write |
| 11 | **The GPU process.** 394 MB in the field against 81–98 MB in the lab. | Unknown: needs measuring first (browser and media panes, Sysinfo charts, a large display). | S–M | Low | to write |

**Why this order**
- #1 is a two-line fix with a heap-snapshot proof, so it goes first.
- #2 is the biggest saving, and #1 makes it fully effective.
- #3 became the operator's window-open report: same pools, different finding.
- #4 is small and independent.
- #5 and #6 are the larger changes that make the 500 MB budget hold at any pane count. #1 and #2 make both cheaper: lower peaks mean less fragmentation, and less DOM means faster restores.
