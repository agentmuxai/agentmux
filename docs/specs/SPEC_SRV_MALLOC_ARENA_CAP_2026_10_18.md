# Spec: srv caps glibc's malloc arenas at two (Linux only)

**Date:** 2026-10-18
**Author:** Maricon@charlie
**Type:** Spec (an allocator setting; no behaviour change)
**Status:** proposed
**Part of:** track E (srv's memory) of the 500 MB budget (`docs/reports/REPORT_MEMORY_FOOTPRINT_500MB_BUDGET_2026_10_12.md`). Asked by Camper@narko, after the measurements below.
**Platform:** **Linux with glibc only.** Windows and macOS have their own allocators; this changes nothing there.

## 1. The problem

On Linux, srv's private memory under load is about three times what it holds live. With 12 agents running tool-heavy turns, srv's live Rust heap is 31–40 MB (a `dhat` profile) while its private memory is 100–130 MB. The rest is memory glibc keeps after it was allocated and freed.

glibc gives a thread its own malloc **arena**, up to eight per core, and each arena keeps the high-water mark of what its threads allocated and freed. srv has a tokio worker per core and a pool of blocking threads, so a long, busy srv ends up with freed memory spread over many arenas, none of which gives it back.

## 2. The change

At the start of `main`, before any thread exists, srv sets glibc's `M_ARENA_MAX` to 2 (`mallopt`), so its threads share two arenas.

- **Before any thread:** a thread already attached to an arena keeps it, and `#[tokio::main]` starts the runtime's workers before `main`'s body runs. `main` therefore builds the same runtime by hand (`new_multi_thread().enable_all()`, which is what `#[tokio::main]` expands to) after the call.
- **The operator wins:** if `MALLOC_ARENA_MAX` is set in the environment (an empty value counts as unset), srv does not touch it.
- **Compiled only for** `target_os = "linux"` and `target_env = "gnu"` (`crates/srv/src/malloc_arenas.rs`). musl, Windows and macOS don't build it.
- The same srv binary also runs as the document-reading child and the crash monitor; they get the cap too, which costs them nothing.

## 3. Measurement

Linux (charlie, Ubuntu 26.04, glibc), srv built from main with #5090, in the repo's scripted fake-claude lab: 12 agents in 12 tabs, 8 rounds of tool-heavy turns (about 218 KB per turn), transcript growth checked. The cap was set with `MALLOC_ARENA_MAX=2` in the process's environment for these runs (the same setting as the call). Runs were interleaved (default, capped, default, capped, default, capped), and each sample also recorded the machine's process count and load: charlie is a shared host, and the same binary settled at 126 and 167 MB in two runs, so single comparisons are not evidence.

| srv private memory, settled 15 s after the last round | default arenas | two arenas |
|---|---|---|
| run 1 | 128.1 MB | 95.9 MB |
| run 2 | 114.3 MB | 88.1 MB |
| run 3 | 103.3 MB | 82.1 MB |
| **median** (spread) | **114.3 MB** (25) | **88.1 MB** (14) |
| **paired difference** (default − capped) | 32.2 / 26.2 / 21.2 MB, **median 26.2 MB** (~23%) | |
| anonymous memory, median | 86.5 MB | 60.1 MB |
| srv CPU time over the run, median | 134.1 s | 117.4 s |
| 12 agents idle | 87.7 / 84.6 / 77.6 MB | 76.7 / 67.3 / 66.3 MB |

The capped run was lower in all three pairs, and in the two single pairs measured before (126 vs 85, 167 vs 112 MB), so in five of five. The effect is 21–55 MB. srv's CPU time was not higher with two arenas (it tracks the machine's load more than the setting), so no lock contention shows at this load.

Memory fell in both groups from one repetition to the next as the machine quieted (processes 696, 650, 628 for the default; load 4.5 to 1.8): srv's memory depends on how busy the machine is. That is a separate lead (the per-process state srv keeps) and is not part of this change.

## 4. What it does not do

- It does not touch Windows or macOS. Whether Windows' heap shows the same retention is measured there (Camper, with Loap-3's memory lab); any number belongs in `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md`.
- It does not reduce the live heap, and it does not return memory to the OS (no `malloc_trim`): it limits how many places freed memory can be stranded in.
- It does not help a machine where srv runs few threads; the gap is largest with many agents.

## 5. Tests

- `malloc_arenas::tests`: the cap applies when `MALLOC_ARENA_MAX` is unset or empty and is left alone when the operator set it, and the value is two.
- Live, with **no** `MALLOC_ARENA_MAX` in the environment (the cap comes from the call), the default build against this change, 12 agents x 8 rounds, counting arena heaps (64 MB-aligned anonymous regions in `/proc/<pid>/maps`; threads in parentheses):

| | default build | with the cap |
|---|---|---|
| arena heaps, clean idle | 29 (29 threads) | **1** (29 threads) |
| arena heaps, after the run | 43 (44 threads) | **1** (43 threads) |
| srv private, settled | 114.0 MB | **90.2 MB** |
| srv CPU time over the run | 132.7 s | 132.1 s |
| machine at the time | 668 processes, load 1.15 | 662 processes, load 1.18 |

  Without the cap, every thread srv starts gets an arena of its own; with it there is one. This pair ran under matching machine load, so its 23.8 MB difference is comparable with the interleaved table (§3).
