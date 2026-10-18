# Spec: The commit attribution log counts only this instance's own processes

**Date:** 2026-10-18
**Author:** Maricon@charlie
**Type:** Spec (a diagnostic's accounting; no behaviour change)
**Status:** implemented — shipped in PR #5083 (the module, the Windows attribution change, its tests). Verified on a real Windows machine running two instances (0.59.30 beside 0.59.31): the walk from 0.59.30's srv reaches its launcher and stops at explorer.exe, and 0.59.31 has its own tree (Camper@narko, 2026-10-18).
**Part of:** `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md` item 4, under track E (srv's memory) of the 500 MB budget (`docs/reports/REPORT_MEMORY_FOOTPRINT_500MB_BUDGET_2026_10_12.md`).
**Asked by:** Camper@narko, coordinating the budget: "Count only this instance's own process tree. Small, and it makes field numbers trustworthy."

## 1. The problem

srv logs a memory attribution line on Windows (`sysinfo::log_memory_attribution`, every sample tick and sooner under commit pressure) that splits the machine's process commit into three buckets: `agentmux_mb`, `panes_mb` (the agents' own process trees) and `other_mb` (everything else, with the top few by name). Its `agentmux_mb` bucket was every process whose name starts with `agentmux`, on the whole machine.

With two AgentMux instances running, a stable install beside a `task dev` build, or two channels, `agentmux_mb` was both instances' processes added together, and neither instance's own number could be read from it. The field data the budget work relies on (§1 of the budget report: the 2 GB figure, the per-process breakdown) is only as trustworthy as that bucket.

## 2. The behaviour

`agentmux_mb` is the commit of the **processes of this instance**: the instance's launcher and everything under it.

- **An instance is a process tree.** The launcher starts srv and the CEF host; the host starts the GPU, utility and renderer processes. srv is somewhere in that tree, so it finds the tree from its own pid.
- **Finding it:** from srv's own process, walk up through parents while the parent is also named `agentmux*` (the launcher); stop at the first parent that is not (the terminal, `task`, or AppImageLauncher that started the instance is not part of it; with a by-hand srv, srv is its own root). The instance is that root and every descendant of it.
- **A nested instance is its own:** an agent running `task dev` in a pane's shell starts a second AgentMux below this one. An `agentmux*` process whose parent is not `agentmux*` is a launcher, so it and everything under it are left out of this instance's tree (its processes count under `other_mb`, and from its own srv the nested tree is the instance).
- **Counting:** a process counts in `agentmux_mb` when it is named `agentmux*` **and** is in this instance's tree. An `agentmux*` process of another instance falls into `other_mb` under its own name (for example `agentmux-cef.exe`), so it is still visible, and the top-contributors list can show it. `panes_mb` is unchanged.
- **Not changed:** the handle-count leak detector (`classify_handle_anomalies`) still treats any `agentmux*` process as ours: a leaking process is ours to act on whichever instance it is in.

## 3. Design

- **`crates/srv/src/backend/own_instance.rs`** (not platform-specific): `own_instance_pids(nodes, self_pid)` is a pure function over a process table (`pid`, `parent`, `is_agentmux`). The upward walk is bounded by a seen set, because a parent link can be stale (a parent that exited and whose pid was reused) and loop. Returns an empty set when `self_pid` isn't in the table.
- **`sysinfo.rs` (Windows):** `log_memory_attribution` builds the table from the process list it already has (names and `parent()`, both always available), calls the function with `std::process::id()`, and sets a new `own` flag on each `ProcSample`; `classify_commit_attribution` counts `own && name starts with "agentmux"` in `agentmux_mb`.
- Nothing else reads `agentmux_mb` (it is a log field).

## 4. Tests

- **`own_instance.rs` (every platform):** two instances side by side, a stable install and a dev build, each with srv, a CEF host, a renderer, and in one a shell (the case that mixed): from either srv, only that instance's pids; the same tree from any member; the walk stops at a non-`agentmux*` parent (an AppImageLauncher-style parent); an AgentMux started from a pane's shell inside another is left out of the outer one's tree, and is its own from its own srv; a by-hand srv is its own root; an unknown pid and a missing parent; a cycle in stale links ends.
- **`sysinfo.rs` (Windows):** `another_instances_agentmux_processes_are_other_not_ours`: this instance's processes are 390 MB of `agentmux_mb`, the other instance's 1,020 MB land in `other_mb`.
- The existing classification tests still hold (their samples are this instance's).

## 5. What this does not do

- It changes no memory. It makes the number mean "this instance".
- The Windows glue (`sysinfo.rs`) is compiled only on Windows; the logic is in the module above so Linux and macOS test it.
