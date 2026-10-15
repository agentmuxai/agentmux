# Spec: srv's memory attribution log measures its own instance

**Date:** 2026-10-13
**Status:** proposed
**Plan item:** #4 of `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md`.

## 1. The problem

srv logs a "commit attribution" line every 30 s, or sooner when commit runs low (`crates/srv/src/backend/sysinfo.rs`, `log_memory_attribution` at L348, classifier `classify_commit_attribution` at L176-203). It's what engineers read during memory incidents (`muxlog srv grep mem_attribution`). See:
- `docs/incident/INCIDENT_2026_09_20_APP_CLOSED.md:109-155`;
- `docs/status/STATUS_SRV_SECTION_HANDLE_LEAK_2026_08_08.md:188-197`;
- `docs/status/STATUS_PF_COMMIT_GROWTH_INVESTIGATION_2026_07_24.md:704,755`.

Its numbers are wrong whenever more than one AgentMux runs, and agents land in the wrong bucket:

| Field | What it should be | What it is |
|---|---|---|
| `agentmux_mb` | this instance's own processes | every process on the machine whose name starts with `agentmux`, from **every instance**. The operator's machine, running 4 instances, reads about 7 GB for one instance's ~3.6 GB. |
| `panes_mb` | this instance's terminals and agents | descendants of `pidregistry` roots. Only the PTY shell spawn registers there (`shell/lifecycle.rs:1014`), so **agent CLIs are missing**. `panes_mb="0"` was observed for long stretches (`docs/retro/RETRO_MAIN_WINDOW_SHOW_SILENT_NOOP_2026_08_13.md:17`). |
| `other_mb` | everything else on the machine | also includes this instance's agent CLIs (`claude.exe`, `node.exe`). |

The name check runs first, so `agentmux-mcp.exe` started by this instance's agents counts as AgentMux, not panes.

The 2026-09-20 incident already cited `agentmux_mb` of 560 MB–1.7 GB taken while two instances ran, so a mixed figure has already fed an incident analysis.

## 2. The change

**Attribute with the same grouping Tower uses**, which is scoped to this instance and already tested (`crates/srv/src/backend/tower_sampler.rs`, `group()`, L121-267). It:
1. claims each block's processes from its Job Object (the agent process tracker, `members_by_block()`, `process_tracker/registry.rs:359-376`, which every agent spawn path joins);
2. claims terminal process trees from `pidregistry`;
3. walks up from srv's own pid while the parent is an `agentmux*` process, and takes everything under that top process that no task claimed.

**Implementation:**
- Make `group()` (or a small wrapper over it) `pub(crate)`.
- `log_memory_attribution` takes one `agentmux_procstats::snapshot()`, builds the same `Inputs` as `tower_pane.rs:112-118` (tracker members, `pidregistry`, own pid), calls the grouping with an empty sticky map, and sums `mem_commit` (Windows: private commit, `procstats/src/windows.rs:154`) per group.

**The log line's fields become:**

| Field | Meaning |
|---|---|
| `agentmux_mb` | this instance's own processes: the host and its CEF processes, srv, the launcher, and their untracked children |
| `agents_mb` | agent blocks' Job Objects: the CLI, its MCP servers, its tool processes |
| `terminals_mb` | terminal process trees |
| `other_mb`, `other_top` | everything else on the machine, as today |
| `other_instances_mb` | other AgentMux instances' processes (name `agentmux*`, not in this instance's tree), so the machine-wide view isn't lost |

`panes_mb` stays as `agents_mb + terminals_mb` for one release, then goes. The incident docs quote it.

**The descendant limit:** Tower's walk stops at 2048 processes per task (`MAX_TREE`), and the log used 4096. The log keeps 4096 and still warns on truncation, as today.

**The handle-anomaly check** (`classify_handle_anomalies`, `sysinfo.rs:262`) gives `agentmux*` processes a lower handle threshold and an "AgentMux process" warning. It uses the same instance tree, so another instance's process isn't reported as this one's leak.

## 3. Tests

- **`classify_commit_attribution`** (now over groups) with a synthetic snapshot of two instances:
  - this instance's host, srv and renderers go to `agentmux_mb`;
  - the other instance's go to `other_instances_mb`;
  - an agent's `claude.exe` and `agentmux-mcp.exe` in a tracked job go to `agents_mb`;
  - a terminal's shell goes to `terminals_mb`.
- **The handle check:** an `agentmux*` process outside this instance's tree isn't flagged as this instance's.
- **Lab:** two instances running. Each one's `agentmux_mb` matches Tower's AgentMux row for that instance, within 5%.
