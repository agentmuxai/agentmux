# Report: the nightly UI memory rise of November 2026, read from the recorded nights

**Date:** 2026-11-20
**Status:** analysis. Read-only, from the artifacts of nightly runs that had already happened; no lab was run.
**Task:** #47680 (nightly drift issue #7340).

## Summary

- The rise the drift check reported (app memory about 815 MB before, about 1000 to 1100 MB at its top) is gone. It ended in one step between 0.60.28 and 0.60.30, the nights have been level since, and CI closed the drift issue on 2026-11-19.
- Both the rise and the fall sit in one place: the main window renderer's Blink garbage-collected heap (`blink_gc`). It went from about 405 MB (0.60.20, 0.60.21) to about 620 MB (0.60.28) and back to about 425 MB (0.60.30 and later).
- The page did not hold more: DOM nodes grew by about a tenth over the same span (42,000 to 47,000) and did not fall when the memory did; the JavaScript heap moved by 8 MB.
- The lab records each allocator's committed size, not the size of the objects alive in it. So the recorded nights cannot say whether the extra 200 MB was live objects or heap kept committed after heavy allocation. The fall came with a quarter less renderer CPU, which fits the second reading, but this is a hypothesis.
- No merge is proven as a cause or as the fix. The recorded nights narrow each step to one night's merges, no further. Naming a merge needs builds between two nightly builds.
- This report used no lab run: everything is read from the artifacts of the nightly runs that already happened.

## 1. What was read

Each nightly run of `perf-regression.yml` uploads a small artifact with every run's `result.json`. It holds, per process, Chromium's memory dump reduced to allocator roots (`v8`, `blink_gc`, `malloc`, `partition_alloc`, ...), and per page the JavaScript heap, DOM node, layout object and listener counts. Thirteen nights were read, 2026-11-07 to 2026-11-19. Two small scripts printed the tables below (they are not committed).

The workload is the nightly standard: 12 agents in 4 tabs, 10 rounds, a forced collection before the final sample, four runs a side, A and B interleaved on one runner.

## 2. The numbers

Medians per side. "Renderer" is the main window's renderer process (private bytes). A and B of one night ran on the same runner with the same lab scripts; different nights did not.

| night | A | B | app MB, A to B | renderer MB, A to B | blink_gc MB, A to B | DOM nodes, A to B |
|---|---|---|---|---|---|---|
| 11-07 | 0.60.17 | 0.60.19 | 829 to 821 | 628 to 652 | 390 to 435 | 41,356 to 42,373 |
| 11-08 | 0.60.19 | 0.60.20 | 870 to 856 | 660 to 658 | 430 to 405 | 42,004 to 42,877 |
| 11-09 | 0.60.20 | 0.60.21 | 792 to 877 | 620 to 673 | 403 to 410 | 42,015 to 43,955 |
| 11-10 | (no runs) | 0.60.22 | none | none to 709 | none to 466 | none to 46,587 |
| 11-11 | 0.60.22 | 0.60.24 | 948 to 996 | 734 to 737 | 473 to 502 | 44,630 to 46,316 |
| 11-12 | 0.60.24 | 0.60.25 | 1024 to 1051 | 787 to 825 | 508 to 557 | 46,099 to 47,601 |
| 11-13 | 0.60.25 | 0.60.26 | 1069 to 1043 | 869 to 841 | 561 to 569 | 46,239 to 47,903 |
| 11-14 | 0.60.27 | 0.60.27 | 1035 to 1061 | 831 to 854 | 546 to 559 | 46,309 to 46,128 |
| 11-15 | 0.60.27 | 0.60.28 | 1126 to 1106 | 923 to 902 | 643 to 627 | 46,830 to 46,937 |
| 11-16 | 0.60.28 | 0.60.30 | 1167 to 912 | 918 to 710 | 617 to 426 | 46,605 to 46,177 |
| 11-17 | 0.60.28 | 0.60.30 | 1035 to 912 | 836 to 710 | 562 to 433 | 47,369 to 47,065 |
| 11-18 | 0.60.30 | 0.60.33 | 950 to 913 | 721 to 687 | 436 to 424 | 46,817 to 46,592 |
| 11-19 | 0.60.33 | 0.60.35 | 902 to 932 | 658 to 692 | 401 to 410 | 47,731 to 46,602 |

How far to trust them:

- One build measured on different nights differs by up to about 80 MB in `blink_gc` (0.60.27: 546, 559 and 643; 0.60.28: 627, 617 and 562). A step of 30 to 50 MB between two nights is inside that spread; only a same-night pair, or a level held over several nights, counts.
- The level is what is solid: about 400 to 435 MB up to 0.60.21, about 470 at 0.60.22, about 505 at 0.60.24, about 560 from 0.60.25, up to 640 at 0.60.27 and 0.60.28, and about 400 to 435 again from 0.60.30. The two same-night pairs inside the rise agree in direction (+29 MB on 11-11, +49 MB on 11-12).
- The fall is the one large step, and it was measured twice on the same pair of builds: 191 MB and 129 MB of `blink_gc`, with renderer CPU down from about 232 s to about 169 s both times.

Other allocators of the same renderer: `blink_objects`, `malloc` and `partition_alloc` stepped up together on 11-12 (0.60.24 to 0.60.25: +15, +22 and +15 MB) and gave back about half at 0.60.30. Against 0.60.20 they are now about 14, 12 and 11 MB higher. `v8` rose 5 MB over the whole span. The 11-09 night's +84 MB of app memory is mostly outside the main renderer's allocators: its private bytes rose 53 MB but its allocator roots only 11 MB (`blink_gc` +7 MB), and the rest is the pre-warmed windows' renderers (+28 MB), srv and the host. The next nights did not keep it as a step of its own.

## 3. What the recorded nights cannot say

**Live or committed.** The lab keeps the `size` attribute of each allocator in the dump: the memory the allocator holds from the system. The dump also carries `allocated_objects_size`, the objects alive in it, and the lab drops it. Blink's garbage-collected heap frees a page only when the whole page is empty, so after a workload that allocates a great deal between collections the heap can stay large with little alive in it. Two readings fit the nights:

1. More was alive: something other than DOM nodes held about 200 MB on that heap (styles, layout results, animation or observer objects), and a merge in 0.60.30 stopped holding it.
2. The same amount was alive, and more was allocated and dropped per update, so the heap grew further before each collection and stayed committed. A merge in 0.60.30 cut the allocation rate.

The fall arriving with 27% less renderer CPU, and the node count not moving, favour the second. The nights of the rise do not show CPU rising with memory within a night, which does not favour either. It stays a hypothesis until one night records both sizes.

**Which merge.** Each step is bracketed by two nightly builds:

| step | range | merges | touching the page's code |
|---|---|---|---|
| 0.60.21 to 0.60.22 (about +60 MB, no same-night pair) | `a8da0848e..b4fa1c400` | 41 | 16 |
| 0.60.22 to 0.60.24 (+29 MB) | `b4fa1c400..63848b092` | 82 | 47 |
| 0.60.24 to 0.60.25 (+49 MB, and the three other allocators) | `63848b092..e7d4b7864` | 64 | 27 |
| 0.60.28 to 0.60.30 (-129 to -191 MB) | `358df1147..bcc6d83a2` | 137 | 89 |

Merges in the fall's range whose subject is how often or how much the page redraws, the ones to try first: the change that limits how many panes one frame flushes and how long it may take (#7468); a hidden window draining its streaming panes (#7503); the shared clock for working marks (#7514, #7511); the status bar's last-sound item redrawing at most once a second (#7494); a very large history opening from a bounded recent part (#7368). None is shown to be the cause.

## 4. Why the 11-10 night failed

Its four B runs (0.60.22) finished normally; they are the 0.60.22 row above. All four A runs (0.60.21) were stopped by the lab's own check that the load was the one asked for: no agent played its turn (no tool rows in any transcript), twice per run. The lab installed its stand-in agent CLI in one place, and the two builds' srv looked for it in different places. The next day's change to the lab (#6996) installs it where each build looks and names a side that cannot run. It was a lab fault, not an app fault, and it is fixed.

## 5. Recommendations

1. **Record live size beside committed size.** Done in the change that adds this report: the lab keeps `allocated_objects_size` next to `size` for each allocator root (`scripts/perf/memory-lab/lib/allocators.mjs`), each run's summary has the main window renderer's line, and each night's comparison has a table of both sides by allocator. The next rise is then read as "more alive" or "more churn", and in which allocator, from its first night. The first night after the change shows which allocators' dumps carry the live figure.
2. **Have the drift issue name the allocator.** Not done. The drift issue links the night's run, whose comparison now has the table; putting the table into the issue text itself is a follow-up if the link proves too far.
3. **Do not bisect now.** The level is back, so no fix is owed. Naming the merge that brought it back is a bisect of the fall's 137 merges: about seven steps, each a Windows build of a chosen commit plus one 45-minute comparison on a CI runner (two or three if one of the five merges of section 3 is it). Decided: not now; only if the nightly level rises again.
4. **No fix tasks are filed.** No merge is shown to be at fault, and the memory it cost is no longer there.
