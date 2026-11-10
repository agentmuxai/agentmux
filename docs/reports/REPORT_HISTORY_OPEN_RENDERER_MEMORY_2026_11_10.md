# Report: why opening an agent pane with history costs the main renderer about 1 GB

**Date:** 2026-11-10
**Status:** implemented in PR #6921 (the two fixes in §4); the before/after run is a lab and has not been run.
**Author:** Loap-3@narko (task #46547)
**Build measured:** 0.60.22 `main` (own isolated dev instance; not a lab run, no scripted soak)

## 1. Summary

- Opening a pane whose last session left tool rows "held open" mounts every one of those rows' previews at once. With 40 held rows that was about 22 000 elements and 94 000 DOM nodes in the page for a few seconds, and the main renderer's private memory went from about 400 MB to about 790 MB.
- The page does not give that memory back. The previews are removed from the screen after the rows collapse, but their nodes stay detached until a garbage collection runs, and the recycle governor only collects after the window has been unused for 10 minutes. The renderer's floor after each collection kept rising across opens (Blink keeps the pages it reserved).
- Fixes in this change:
  1. A restored pane opens every row closed (the held ids are no longer saved or restored).
  2. The governor collects garbage (no reload) in a window that has been quiet for a minute and is over the recycle floor, at most once in 10 minutes.
- Not changed, and why: windowing `PreviewLines` (mount only the visible lines of a long preview). It would help a long live preview, not the open; it also changes find-in-page and select-all inside a preview. Left as a follow-up until these two fixes are measured.

## 2. What was measured (own dev instance)

| Moment | Main renderer private | DOM nodes | Elements in the pane |
|---|---|---|---|
| Empty pane, after GC | 194 MB | 3.7k | 90 |
| Live, 12 scripted turns | 861 MB | 300k | 13.8k |
| Live, after a GC | 528 MB | 49k | 12.4k |
| Reopen with 40 rows restored held, +3 s | 789 MB | 94.5k | 22.5k |
| Same, +15 s (rows released) | 409 MB | 8.5k | 2.3k |
| Next reopen (nothing held any more), +3 s | 514 MB | 8.5k | 2.3k |

The elements on a live pane are 95% inside tool panels (preview lines and their highlight spans). On a reopen the same is true only while the restored held rows are open; after they are released the pane is 2.3k elements.

The operator's instance logs a renderer line every five minutes and the governor logs `before_mb` / `after_gc_mb`; in those logs a collection took 2621 to 1722 MB, 3026 to 1905 MB and 3369 to 2242 MB, and the floor after collection rose from 1.3 GB to 2.2 GB over about two and a half hours.

## 3. Cause

`rowDisclosure` opens a tool row that is pinned, running, or held (`heldOpenNodes`). Since PR #5059 / the E12 follow-up the snapshot saved up to 40 held ids and restored them as held, so that a restart or upgrade did not collapse what the reader had open. Each open row mounts its whole preview (up to `MAX_TOOL_OUTPUT_LINES` = 1000 lines, each line a flex row of 2 to 3 elements plus one span per highlight token).

## 4. Changes

- `useHistoryPagination.ts`: a restored pane starts with no held rows; `restoredHeldNodes` and `MAX_SAVED_HELD` are gone. `useSnapshotPersistence.ts` no longer writes `heldOpenNodeIds` or the hidden/page-away write that existed only for them; `held-release.ts` loses the effect that carried restored rows.
- `recycle_governor.rs`: `pick_tidy` and `tidy`: a window quiet for 60 s and at least `min_mb` (600 MB) gets `HeapProfiler.collectGarbage`, at most once per 10 min, logged as `[recycle] <label> quiet: collected garbage` with `before_mb` and `after_gc_mb`. The same `window:autorecycle` setting turns it off.

## 5. Behaviour change to know about

A pane brought back after a restart or an upgrade no longer reopens the tool rows that were open. Rows that finish while the pane is shown are still held until they scroll off, and a pane that is only hidden keeps its held rows (the carry on show is unchanged).

## 6. How to check it (no lab needed)

- Unit tests: a snapshot's held ids are ignored; the snapshot no longer carries them; `pick_tidy` picks a quiet, big window once per 10 minutes and nothing else.
- In the operator's instance: after this ships, the five-minute renderer log should show `quiet: collected garbage` lines with a lower `after_gc_mb`, and a reopen should not show the 2x spike.
- A before/after of the same 12-turn script on a task dev is a lab; it waits for the operator's yes.
