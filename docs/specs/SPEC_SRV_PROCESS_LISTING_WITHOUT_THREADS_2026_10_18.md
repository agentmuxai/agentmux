# Spec: srv's process listings stop reading every thread (Linux CPU)

**Date:** 2026-10-18
**Author:** Maricon@charlie
**Type:** Spec (a read-cost change; no behaviour change)
**Status:** active — the change and its tests are in PR #PRNUM; remaining: a Windows reading (Posa, with Loap-3's memory lab) to see whether the same listing costs CPU there.
**Part of:** track E (srv's memory and CPU) of the 500 MB budget (`docs/reports/REPORT_MEMORY_FOOTPRINT_500MB_BUDGET_2026_10_12.md`). Approved by Camper@narko as "option A", after the profile in §1.
**Platform:** measured on **Linux**. The code in `sysinfo.rs` and `scan.rs` is shared; the thread reads it removes are a Linux/`/proc` cost, so Windows and macOS are expected to be unchanged but are not yet measured.

## 1. The problem

srv's own CPU time under a 12-agent load was dominated by listing processes. A local-only timer build (never pushed) around the suspects found **63% of srv's CPU in `list_members`**, the call that lists the processes of each agent's cgroup for the process tracker.

The cause is a default in sysinfo 0.35: `ProcessRefreshKind::nothing()` leaves *tasks* on, so a refresh reads `/proc/<pid>/task/<tid>/stat` for every thread of every process it touches, and holds a file descriptor for each. The listing only needs process-level fields. Measured on one process with 40 threads: **10.6 ms with tasks, 0.43 ms without**. srv lists every agent's process group repeatedly, and the agents' own CLIs are multi-threaded.

Two smaller costs of the same kind:

- The machine-wide snapshot used by the process scan (`scan.rs`) also read every thread.
- `run_sysinfo_loop` started from `System::new_all()`, which takes a full-detail snapshot of every process on the machine (command lines and environments included) and keeps it for the life of srv, though the loop only reads CPU and memory counters.

## 2. The change

- `describe_pids` (used by `list_members`) and the scan snapshot refresh with `.without_tasks()`. Command line, parent, start time and memory are still read, and two new tests pin that (`a_process_with_many_threads_lists_once`, `the_listing_still_reports_command_parent_start_and_memory`).
- The sysinfo loop's `System` is built with only the CPU and memory it reads.

No setting, schema or message changes; what the listings return is the same.

## 3. Measurement

Linux (charlie, Ubuntu 26.04), Loap-3's memory lab (`scripts/perf/memory-lab`, workload v2) on one portable built from main at 6c9d92bf0. Only the srv executable differs: the same commit without and with the change, swapped in with `--srv`. 12 agents over 4 tabs, 10 rounds, seed 1, forced GC. Four runs of each side, interleaved (before, after, before, after, ...). Medians, with the lowest and highest in brackets.

| | before | after | change |
|---|---:|---:|---:|
| **srv CPU, seconds over the workload** | 104.1 (101.7–105.7) | 85.8 (85.5–86.1) | **−17.5%** |
| app total CPU, seconds | 323.6 (320.2–324.7) | 306.9 (306.1–315.2) | −5.2% |
| srv private memory, MB | 55.9 (53.8–58.4) | 47.1 (36.4–47.5) | −15.7% |
| app total private memory, MB | 1189.1 (1162.6–1198.8) | 1174.9 (1166.7–1184.8) | −1.2% |
| round time, ms (median of rounds) | 11435 (11324–11488) | 11682 (11270–12247) | +2.2% |

The srv CPU ranges do not overlap. The other process groups (renderer, host, utility, other) moved by at most 1% in either direction, which is the noise between runs. Round time moved +2.2% with overlapping ranges, so nothing is shown either way; it is not a speed-up and the change does not claim one. The srv memory figure is a side effect (less transient listing state) and its ranges nearly touch: read it as "not higher", not as a promise of 9 MB.

## 4. What this does not do

- It does not change what the process tracker reports.
- It is not the memory lever: srv's memory is mostly allocator behaviour (`SPEC_SRV_MALLOC_ARENA_CAP_2026_10_18`) and the transcript store.
- It does not touch the sampler's CPU use of `sysinfo`'s per-process counters, which does need them.

## 5. Tests and caveats

- The two new tests run on Linux with the rest of `process_tracker`. The cgroup tests are skipped in CI (no delegated cgroup), so CI proves the build and the unit tests, not the cgroup listing; the lab above exercised it.
- Two cgroup tests already flake on main without this change (`assign_after_spawn_and_kill_pid_only_members`, about 3 in 40 solo runs; `every_descendant_is_tracked…`, about 2 in 400). On the branch, 1 in 400 solo runs failed (main: 2 in 400); in a smaller batch of 60 each, the branch failed 1 and main 0. These are too few to tell the two apart, and the tests that flake are the ones that spawn and kill processes, not the listing.
