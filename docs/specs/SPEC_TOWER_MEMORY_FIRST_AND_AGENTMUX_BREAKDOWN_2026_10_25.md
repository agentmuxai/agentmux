# SPEC: Tower puts memory first, and shows what AgentMux's memory is made of

**Status:** draft — awaiting the operator's approval in conversation (decisions marked ◆ in §10)
**Date:** 2026-10-25
**Author:** Loap@narko
**Requested by:** the operator, 2026-10-25, through AgentY (Kanban #2858):
> "we want the tower to focus first on memory, it seems sort of hard to get the memory to appear, even on small panes, we also want to understand AgentMux's actual memory both total and the components. We want a breakdown of AgentMux's memory since it also appears to contain claude instances. So that would all get built into the tower"

**Builds on:**
- `SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md` (the sampler, the metric definitions in §4) and `SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md` (the Agents and Processes views, the rail)
- `SPEC_TOWER_PANES_VIEW_2026_10_21.md` (Korp, #5298: per-pane numbers, measured / shared / estimated)
- `SPEC_TOWER_THEME_SURFACE_AND_COLOR_2026_10_17.md` (process kinds, the `--process-kind-*` colours, load levels)
- `SPEC_INSTANCE_MEMORY_ATTRIBUTION_2026_10_18.md` (`own_instance_pids`: what counts as this instance's own tree)

**Related, not changed:** `SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md` and `crates/cef/src/recycle_governor` (what the host does about memory), `PLAN_MEMORY_BUDGET_CLOSE_THE_GAP_2026_10_18.md` (the 500 MB budget), `SPEC_GPU_PROCESS_MEMORY_2026_10_13.md` (the GPU process).

---

## 0. The ask, and the short answer

Four things:

1. **Memory first.** Tower opens on memory, and memory is on screen at every pane width.
2. **One number for AgentMux's real memory**, and how it relates to what the OS shows.
3. **A breakdown by component**: the core (launcher, srv, the CEF host and its GPU and utility processes), the panes, and each agent's CLI tree (Claude's node processes, Codex, MCP servers, tools).
4. **A metric that doesn't double-count shared memory**, said in the UI.

What this spec recommends:

- A new first view, **Memory**, that opens by default. It is a headline total, one stacked bar split into **Core / Panes / Agents / Not placed**, and a table under it that opens each part down to its processes (§4).
- In the other views, and in narrow panes, **memory becomes the first number column and the last one to go**; CPU yields to it (§3).
- The total is **private memory of every process in this AgentMux's own tree**, in the metric Tower already uses (private working set on Windows, anonymous RSS + swap on Linux, physical footprint on macOS). It does not double-count shared pages, and the UI says what it leaves out (§6).
- The reason the AgentMux row seems to hold Claude instances is real and cheap to fix (§2.2): today the AgentMux group is "everything under the launcher that no pane claimed", so an agent's CLI that no pane tracker claimed lands in it. They get their own part, **Not placed**, and stay out of **Core**.
- Trends: srv keeps one hour of per-component totals at 10-second steps, even when no Tower is open, and flags a component that keeps growing (§7).

Measured on this machine today (narko, 0.59.41, Windows), so the numbers in this spec are not guesses:

| | Processes | Private working set | Working set |
|---|---|---|---|
| Everything under `agentmux.exe` (the app) | 175 | **6.1 GB** | 12.2 GB |
| of which AgentMux's own binaries (`agentmux-*.exe`: CEF host, renderers, GPU, srv, bashwrap, mcp) | ~60 | ~2.7 GB | ~4.3 GB |
| of which `claude.exe` (16 processes) | 16 | 2.2 GB | 3.5 GB |
| of which compilers, build tools, emulators, shells | ~100 | ~1.2 GB | ~4.4 GB |

Two facts to read off it: the OS's own tree under AgentMux is more than twice AgentMux's core, and "working set" overstates by 2× (one `qemu` process alone is 122 MB private and 2,276 MB working set). That is why the metric matters (§6).

---

## 1. What is wrong now

### 1.1 Memory is hard to see

Everything below is in `frontend/app/view/tower/`; line numbers are from main at `fcc3af1a5`, and `git diff` to `55950215a` (today's main) shows no change to any file this spec cites.

1. **Tower opens on Agents, sorted by CPU.** `tower-model.ts` returns "agents" for any unknown `tower:view`; the default sort is `{ key: "cpu", desc: true }`. The busiest process is first, not the biggest.
2. **CPU comes before Memory** in every table (`tower-agents.tsx`: Process, CPU, Memory, then More columns, then PID).
3. **At narrow widths memory is dropped before CPU.** In `tower-view.scss`:
   - below 480 px the rail moves above the table and is capped at 30% of the pane's height;
   - below 680 px the rail row loses its memory line and its sparkline (`.tower-rail-mem`, `.tower-spark` → `display: none`), leaving a name and a CPU number;
   - the detail table at 460 px or less hides every column after the third, so Peak and PID go, and with them "More columns";
   - below 640 px the number columns shrink to 76 px and 64 px;
   - the history chart grid is one column below 560 px, so memory is a chart you scroll to.
4. **No total.** The per-entry total ("CPU … · Memory …") is in the detail header only. A user who wants "how much memory is AgentMux using" has to add up rows.
5. **Memory history is not kept.** The frontend keeps 300 points per rail entry for about ten minutes, in the pane, unsaved (`tower-model.ts`). A pane opened later has no history; a closed one loses it.

### 1.2 AgentMux's own memory is not one thing

- The **AgentMux task** (`TowerTaskKind::Agentmux`) is built in `tower_sampler::group()` step 4: climb from srv's own pid while the parent's name starts with "agentmux", then take every descendant of that top process that no pane task claimed. The comment in `tower_agentmux.rs` says so itself: the group "holds every unclaimed descendant of the launcher, not only AgentMux's binaries". Its rows get a `detail` label (Launcher, Server, GPU, Renderer, Network service, Crash reporter, …) only when they are AgentMux's own binaries; any other process in it is plain.
- A pane is claimed by its tracker (a Job Object on Windows, a cgroup on Linux, a best-effort descendant scan on macOS) or by its shell's process tree. An agent's CLI that no pane claims is in the AgentMux group. Cases found by reading the code (to be measured in step 0 of §9, not assumed): an agent with no open pane (background, Swarm or Supervisor agents), a process that starts before its pane's tracker has its pids, a process that was reparented to the launcher, and a second AgentMux started from an agent's shell.
- So "AgentMux" in Tower today can include `claude.exe`, `rustc.exe` and `qemu`, and the operator reads that as AgentMux's own memory. It is not, and there is no way to tell the two apart in the row.
- The **Panes view** (#5298) answers "what does each pane cost"; it does not say what AgentMux as a whole costs, or how it splits.

---

## 2. The model: where each process goes

### 2.1 Four parts

Every process in this AgentMux's own tree is in exactly one part. The tree is the one `own_instance_pids` already defines (§5.2).

| Part | What is in it | How a process gets there |
|---|---|---|
| **Core** | The launcher, srv (and its crash monitor and document parser), the CEF host's main process, GPU, network, storage, audio and other utility processes, the crash reporter | An AgentMux binary with a `detail` label, other than a renderer |
| **Panes** | The renderers (one per window, plus a browser pane's own renderer when Chromium gave it one), and the shells and programs of terminal panes | A renderer, by the CEF host's renderer map (`renderer_serves`); a terminal task's processes |
| **Agents** | One entry per agent: its CLI (`Main`), what runs for it (`Support`: its MCP servers, `agentmux-bashwrap`, conhost), and what it started (`Started`: build tools, test runs, a dev AgentMux) | An agent task's processes, as today |
| **Not placed** | Everything else under the launcher: a process no pane claimed. Shown by what it is (kind and name), never inside Core | The AgentMux group's plain processes, i.e. today's source of "Claude inside AgentMux" |

The bar and the headline use these four. **Panes** is two things (the app's drawing and the user's shells); the table splits it (§4.3).

### 2.2 Why "Not placed" and not "assign it to an agent"

The obvious fix is to find the agent for each stray `claude.exe`. That is worth doing where it is cheap (§9 step 0 measures how many strays there are and why), but it can't be the whole answer: a process can be genuinely unplaceable (reparented, the pane closed, a tool that daemonized). The honest label for those is "AgentMux started this, nothing claims it". They are counted in the AgentMux total, because AgentMux's tree owns them, and never inside Core.

Placing more strays is a separate improvement, and each one moves memory from **Not placed** to **Agents** without changing the total:

- Unclaimed processes whose name or command line says they are an agent CLI (`tower_kind` already knows every provider's CLI and npm package) go to a "no pane" agent entry for that provider, not into Core.
- Where the process's environment carries `AGENTMUX_AGENT_ID` or `AGENTMUX_BLOCKID` and it can be read (same user), use it. Windows needs a read of the other process's environment block; the sampler already reads command lines once per process and caches them. Left as an option (◆ D3) because it adds a read per new process.

---

## 3. Memory first, at every width

### 3.1 The rules

1. **Memory is the first number column**, ahead of CPU, in every Tower table (Agents detail, Processes, Panes, the Memory view).
2. **Default sort is memory, descending**, in every table that has a sort. CPU stays one click away, and a sort the user chose is kept as today (`tower:sort`).
3. **Memory is the last column to go.** When a table is too narrow it drops, in this order: PID, Peak, CPU time, CPU, and only then (below 240 px) the process names' second line. Never Memory.
4. **A summary line is always visible**, sticky above the list at every width: `AgentMux 6.1 GB` and, on the Agents and Processes views, the selected entry's memory. It does not scroll away and it is one line down to 140 px wide.
5. **The rail shows memory** at every width. The rail's memory line and sparkline are hidden below 680 px today; below 680 px the rail shows memory *instead of* CPU, with CPU in the row's hover text.

### 3.2 The width ladder

Container queries on the pane (the pane's own width, not the window's), the same way the current rules are written.

| Pane width | What shows |
|---|---|
| ≥ 900 px | Headline, bar, component table with trend sparklines; the rail beside the detail; chart grid two columns |
| 600–899 px | As above without the sparklines in the table; chart grid one column |
| 380–599 px | Headline, bar, component table with memory and share; CPU and Peak behind a row's expand; the rail above the table, uncapped on the Memory view |
| 240–379 px | Headline, bar, one line per part: swatch, name, memory. Tap a part to open its processes in place |
| < 240 px | Headline and bar, with the four parts as a legend under the bar (swatch, name, memory), one per line; nothing opens |

The tests in §8 assert the ladder with fixed container widths, as `tower-view.test.tsx` already does for the rail.

### 3.3 Narrow sizes of numbers

- Memory is formatted with at most 3 significant digits and a unit (`6.1 GB`, `312 MB`, `48 MB`), as now (`tower-util.ts`), and never wraps. A cell wide enough for `259.6 MB` is 70 px (the existing comment); the Memory column keeps 76 px at every width and is the one that gives its room last.
- The pane never scrolls horizontally to find memory. If the process name is the only thing that can give, the name truncates.

---

## 4. The Memory view

A new first tab, **Memory**, ahead of Agents, Processes and Panes. It opens by default (`tower:view` absent or "memory"); the three existing views stay as they are, with §3's ordering. It is this computer only, like Panes: a remote machine has no tasks to place (§5.5), so the tab is hidden there and Tower opens on Processes, as Panes already does.

### 4.1 Headline

One line, large: **AgentMux** and its total, e.g. `6.1 GB`. Under it, in secondary text: `175 processes · private memory in RAM`, and a small info button.

The info text (one place, §6.2): "Memory that only AgentMux's processes use. Shared libraries and shared memory are not counted twice. This is the figure Task Manager shows as a process's Memory, added up over everything AgentMux started."

Beside the number, a trend arrow with the change over the last hour (`+310 MB in 1 h`), from the ledger (§7). It is hidden until an hour of history exists, and says "since 14:20" until then.

### 4.2 The bar

One horizontal stacked bar, full width, four segments in this order: **Core, Panes, Agents, Not placed**. The segment widths are shares of the total. Colours are the existing theme tokens (§4.5). The bar is the same element at every width; below 240 px it is the only graphic.

Hover or focus on a segment shows its name, memory and share. Clicking one opens that part in the table below and scrolls to it. At narrow widths the bar's legend is the list of parts (§3.2).

### 4.3 The table

Rows, top level, in this order:

```
Core                         2.7 GB   44%   ▁▂▂▃▃▃
  Server (srv)                 340 MB
  Browser host (main process)  180 MB
  GPU process                  394 MB
  Network, storage, audio …    110 MB
  Launcher, crash reporter      28 MB
Panes                        1.4 GB   23%
  Window renderers (4)         1.1 GB   ← one row per window
  Browser panes (3)            250 MB   ← one row per pane, "shared" where Chromium shared a renderer
  Terminals (5)                 60 MB   ← one row per terminal
Agents                       1.8 GB   30%
  Loap                         620 MB   ← one row per agent, as the Agents view
    claude (node), 3 processes   510 MB
    MCP servers, bashwrap         28 MB
    tools it started             82 MB
  …
Not placed                   180 MB    3%
  claude.exe ×2               140 MB
  qemu-system ×1               40 MB
```

- The first level is always the four parts, always in that order, with a zero part kept as `0 MB` so the layout doesn't jump. Opening one is a disclosure triangle; the busiest part opens on first view, as the Type grouping already opens its busiest two.
- Rows under **Core** are the AgentMux processes grouped by their `detail` (Server, GPU, Network service, Renderer, …), so "the CEF host" is visible as its processes and not hidden in a total.
- Rows under **Panes**: the window renderers come from the CEF host's map (`renderer_serves`); each browser pane shows the number the Panes view already shows (measured, shared once, or estimated, and labelled the same way); each terminal is its task's total.
- Rows under **Agents**: one row per agent, with the pane's name and colour as in the rail; its own tree is the existing Agents detail, one click down. The agent's three roles (CLI, support, tools) are shown as sub-rows so "Claude's node processes", "MCP servers" and "tools" read directly.
- **Not placed** is expanded by kind and name, never hidden. Its row has a one-line hint: "AgentMux started these and no pane claims them."
- Columns: name, **Memory**, share of AgentMux's total, a sparkline of the last hour, then CPU (hidden by §3.2 first). Sorting is by memory within a level.
- Clicking a process row opens it in the Processes view (the existing "reveal" behaviour).

### 4.4 Reuse

The Memory view is one component (`tower-memory.tsx`) over a new `TowerMemory` payload (§5.3). It reuses `KindIcon`, `LoadCells`, `ProcessName`, the history chart and the sort header. It adds no new chart library.

### 4.5 Colours

The four parts use theme tokens, not literal colours (`SPEC_COLOR_THEME_TOKEN_HARDENING`). Proposed, from the tokens the Tower already measures: **Core** `--process-kind-agentmux`, **Panes** `--process-kind-shells`, **Agents** `--process-kind-agents`, **Not placed** the secondary text colour at 46%, as the System kind is drawn. The contrast and distance test in `process-kind-colours.test.ts` is extended to the four part colours in all 13 themes (◆ D5 asks for confirmation; it is the same machinery as #5072).

---

## 5. Data and backend

### 5.1 What exists

- `agentmux_procstats::snapshot` reads every process once per sample: `mem_private` (the Tower number), `mem_resident` and `mem_commit` (Windows), the parent, the start time.
- `tower_sampler::group()` places processes into tasks; `tower_agentmux.rs` labels AgentMux's own; `tower_kind` classifies by name; `tower_panes.rs` and `sampler/panes.rs` do the per-pane numbers with renderer sharing.
- The sampler runs while a Tower pane is visible and asking, every 2 s (`INTERVAL`), reusing a sample under 900 ms old.

### 5.2 The part of each process

New module `crates/srv/src/backend/tower_memory.rs`, pure functions over what a sample already has:

```rust
pub enum MemoryPart { Core, Panes, Agents, NotPlaced }

/// Where one process of this instance's own tree goes (§2.1). First match wins:
/// 1. in an agent task                                  → Agents
/// 2. in a terminal task                                → Panes (terminals)
/// 3. an AgentMux renderer                              → Panes (window renderers)
/// 4. an AgentMux binary with a detail label            → Core
/// 5. anything else in the AgentMux group               → NotPlaced
pub fn part_of(task_kind: Option<TowerTaskKind>, detail: Option<&str>) -> MemoryPart;
```

- **The instance's tree** is `own_instance::own_instance_pids` (§SPEC_INSTANCE_MEMORY_ATTRIBUTION): walk up from srv while the parent is `agentmux*`, then everything below. A process of another AgentMux (a stable beside a dev build, or one an agent started with `task dev`) is outside it; if it is below an agent's shell it is that agent's tool (it is in its tree), and if it is a sibling it is listed under **Other instances** (§5.5).
- The existing grouping is not rewritten. `part_of` reads its result: task kind and role for rules 1–2, `detail` for 3–4, the rest is the AgentMux group's plain processes.

### 5.3 The wire

`TowerSnapshot` gains an optional `memory: TowerMemory` (absent when the request didn't ask; optional so older peers and remote frames keep working, as `TowerPanes` did):

```rust
pub struct TowerMemory {
    pub total: u64,                    // private bytes, every process of this instance
    pub processes: u32,
    pub unreadable: u32,               // processes whose memory the OS wouldn't give
    pub parts: Vec<TowerMemoryPart>,   // always four, in order
    pub other_instances: Option<u64>,  // other AgentMux trees on this computer, not in total
    pub metric: String,                // same string as TowerSnapshot::memory_metric
    pub history: Option<TowerMemoryHistory>,   // §7
}
pub struct TowerMemoryPart {
    pub part: MemoryPart,
    pub mem: u64,
    pub processes: u32,
    pub rows: Vec<TowerMemoryRow>,     // second level: by detail, window, pane, agent, kind+name
}
pub struct TowerMemoryRow {
    pub key: String,                   // stable: "core:gpu", "pane:<block>", "agent:<id>", "np:claude"
    pub label: String,
    pub mem: u64,
    pub processes: u32,
    pub shared: bool,                  // a renderer counted once for several panes
    pub task: Option<String>,          // TowerTask::id, to open the existing detail
}
```

Rows carry totals and ids, not processes: the process lists are in the snapshot already (`tasks[].processes`, `host.processes`) and the view joins by `task` and `id`. That keeps the payload small and gives one source for every number. `TowerSampleReq` gains `memory: bool`; the existing views are unchanged.

TypeScript bindings come from the same `ts_rs` export as the rest (`frontend/types/rpc/`). `rpc-api/tower.ts` exports them.

### 5.4 Cost

- No new process enumeration: the part and the totals are one pass over the rows the sampler has (O(processes), ~700 on a busy machine).
- The ledger (§7) needs a sample when no Tower is open. It uses the same `snapshot` call at 10 s: one `NtQuerySystemInformation` on Windows, a `/proc` walk on Linux, a `proc_listpids` pass on macOS. The build measures the cost on all three (step 0) and records it in the PR. The target is under 0.1% of one core on average; if the measurement says otherwise, the ledger steps to 30 s.
- No command-line or environment reads are added for the default design (§2.2's environment read is optional, ◆ D3).

### 5.5 Edge cases

| Case | Behaviour |
|---|---|
| **A process exits** | The sampler already keeps exited rows for 60 s (`EXITED_LINGER`). The ledger stores per-part totals, so a part's history doesn't lose a process that ended; the table drops it after the linger. |
| **PID reuse, stale parent** | `ProcKey` (`pid`, `start_key`) and `is_child_of`, unchanged. |
| **Unreadable processes** | `mem` is `None`, never 0. They are not in the total; `unreadable` says how many, and the info text says "N processes could not be read". |
| **Shared renderer** | One renderer serving several panes is counted once in Panes, as `shared_mem` does in the Panes view; each sharing pane shows "shared". |
| **Other AgentMux instances on this computer** (a dev build beside the stable install, or another channel) | Not in this total, as in #5083. Listed as one line, **Other AgentMux instances 1.0 GB**, under the table with a hint, because it explains the OS's higher number. |
| **A dev AgentMux started by an agent** | It is in that agent's tree: the agent's **tools**, counted once. Not shown as an "other instance". |
| **Containers, WSL, VMs** | The container engine's host process is a tool or **Not placed** by name; what runs inside a VM is not visible to the host's process table, and the VM's guest RAM is not private working set of the host process (`qemu` above: 122 MB private against 2.3 GB working set). The row's hover says "memory inside the VM isn't counted". |
| **SSH and remote panes** | The local process is the `ssh` client (small). The remote shell's memory is on the other machine and is not counted; the pane row says "remote". A paired computer's Tower is its own snapshot, shown in its own Tower. |
| **A remote or older peer without `memory`** | The Memory tab is hidden for a remote machine (§4), and the other views still work. |
| **Tracking is best-effort** (macOS, Linux without cgroup v2) | The Agents part is as exact as the tracker; the rows say "approximate" as the Agents view does. Anything missed is in **Not placed**, so the total is right either way. |
| **Windows, no Job Object** | Same. |

---

## 6. The metric

### 6.1 One number per OS, already in use

| OS | Tower's per-process number | What it is | Shared memory |
|---|---|---|---|
| Windows | `WorkingSetPrivateSize` ("private working set") | RAM pages only this process uses | Not counted |
| Linux | `RssAnon + VmSwap` from `/proc/<pid>/status` | Anonymous RAM and swap | File-backed pages and shared memory (`Shmem`) are not counted |
| macOS | `ri_phys_footprint` | The memory the process is charged for (Activity Monitor's "Memory") | Not counted |

These come from `crates/procstats` and are the numbers each OS's own tool shows per process, so a total built from them does not double-count a shared library or a Chromium renderer's shared mappings. The spec **keeps them** (◆ D4). Working set (resident) is not summable (the table in §0: 12.2 GB against 6.1 GB private), and stays a per-process detail and a hover.

### 6.2 What the UI says

- Headline info (§4.1): the sentence above.
- Column header hover: "Private memory in RAM: what only this process uses."
- Per OS, the metric name from `memory_metric` is shown in the info panel ("Private working set"), as `TowerSnapshot::memory_metric` already carries it.
- The total's relationship to the OS, one line in the info panel: "Task Manager's AgentMux row counts only AgentMux's own programs. This total also counts the agents and tools AgentMux started, split into the parts below." On macOS: "Activity Monitor lists each process; this adds up everything AgentMux started." Each statement is checked on the reference machine in step 0 (§9) before it ships, since what the OS groups under one name differs by version.

### 6.3 Known gaps, said once

- **GPU memory held in video RAM** is not in the host's private working set; the GPU process's own number is system RAM only (`SPEC_GPU_PROCESS_MEMORY`). The info panel says "video memory isn't counted".
- **Copy-on-write after fork** on Linux counts a page in each process until it is written; for Chromium's zygote this is small. If it ever matters, PSS is the answer (§6.4).
- **Memory compression** (macOS) and the Windows memory compression store are counted as the OS reports them; the total can be lower than the sum of resident sets.

### 6.4 PSS on Linux (not in the first build)

`/proc/<pid>/smaps_rollup` gives the exact proportional share, but costs a read per process (the procstats header rules it out for the 2 s sample). Option for a later phase (◆ D4): read it for the processes of the AgentMux tree only, every 10 s, as the ledger's Linux total. The list stays on `RssAnon + VmSwap`.

---

## 7. Trends and warnings

### 7.1 The ledger

srv keeps a ring of **360 points at 10-second steps (one hour)** of, per part and per second-level row key: private bytes. About 30 keys × 360 points × 8 bytes is under 100 KB. It lives in memory only; it starts empty on a restart and the UI says "since 14:20".

- It is fed by the Tower sampler when one is running (every 2 s, thinned to the 10 s steps) and by its own 10 s timer when none is, so a Tower opened later shows the past hour, which the 300-point in-pane history can't do (§1.1 item 5).
- Rows come and go (an agent closes): a key that has not been seen for an hour is dropped; one that disappears keeps its history until then, so the line ends where the process ended.
- It is for this computer's tree only; it does not leave the machine.

### 7.2 What the UI shows

- A sparkline per part and per row in the table, and the full chart for the selected row below it, reusing the history chart with a time axis of 10 minutes / 1 hour (a toggle).
- The headline's hour change.

### 7.3 "Grows unusually"

A row is flagged when **all** of these hold, over its last 30 minutes:

1. it is at least **150 MB**;
2. it grew by at least **50%** over its minimum in the window and by at least **100 MB**;
3. it has not fallen by more than 10% from its maximum since (it is not already coming back).

A flagged row shows an amber badge and its text: "grew 310 MB in 20 min". The part above it carries the badge too, so it is visible with the table collapsed, and at narrow widths the headline carries a small amber dot with the same text in its hover.

The thresholds are first guesses to tune with the memory lab's noise data (`REPORT_MEMORY_SOAK_GATE_2026_10_18.md`), not constants to defend. They are one object in `tower_memory.rs` with a test that pins the cases below.

The warning is **only in Tower**: no notification, no status-bar item, no message to an agent. It does not act (no recycling): the memory supervisor (`recycle_governor`, `mem_supervisor`) is unchanged. If the operator wants the signal to reach the supervisor or an agent, that is a separate spec on this ledger (◆ D6).

---

## 8. Tests

**srv (`tower_memory.rs`, `tower_sampler.rs`):**
- `part_of`: an agent task's `Main`, `Support` and `Started` processes are Agents; a terminal's shell and a command in it are Panes; a renderer is Panes; the srv, GPU and network processes are Core; a plain `claude.exe` in the AgentMux group is Not placed and **never** Core (the regression behind §1.2).
- The four parts add up to `total`; `total` equals the sum of the tree's `mem_private` over processes whose memory was readable, and `unreadable` counts the rest.
- Another instance's processes (two trees side by side, as `own_instance.rs` tests do) are in `other_instances` and not in `total`.
- A renderer serving two panes is counted once.
- An exited process keeps its part's history and leaves the table after the linger.
- Rows' keys are stable across samples.
- Ledger: thinning from 2 s to 10 s, the ring's size, a key dropped after an hour, a restart starts empty.
- Growth flag, pinned cases: steady 800 MB: no; 200 → 400 MB in 20 min: yes; 900 → 300 → 350 MB: no; 100 → 160 MB (+60%, +60 MB): no, growth under 100 MB; 120 → 300 MB: yes.
- A `TowerSampleReq` without `memory` returns no `memory` (existing views unchanged).

**Frontend (`tower-view.test.tsx` and a new `tower-memory.test.tsx`):**
- Tower opens on Memory with no `tower:view`; Agents, Processes and Panes still open from their meta.
- Memory is the first number column in every table; the default sort is memory descending; a stored `tower:sort` still wins.
- The width ladder (§3.2): at 900, 600, 400, 300 and 200 px the headline is present and the elements the table lists are, and no horizontal scrollbar; Memory is present at every width, CPU is the first to go.
- The bar's segments add to 100%; a zero part is kept as `0 MB`; clicking a segment opens its part.
- A part opens to its rows; "Not placed" shows the processes by name and carries its hint.
- The info text and the metric name are in the DOM (per `memory_metric`).
- The hour change and "since HH:MM" before an hour of history exists.
- The amber badge on a flagged row and on its part.
- `process-kind-colours.test.ts` extended: the four part colours in all 13 themes meet the same distance and contrast bars.
- `tsc`, eslint, `check-ui-primitives` (no raw radius, font size or undefined variable).

**By eye, in an isolated dev build:** the four parts on a machine with agents, a browser pane and a build running; Tower at 1,200, 600, 320 and 200 px wide; two themes; the number against Task Manager (private working set) for the AgentMux processes.

---

## 9. Phases

Each is one PR; none changes how the existing views compute their numbers.

**Step 0 (measure, before any code; part of phase 1's PR notes).** On narko, run the existing Tower with the Agentmux group's rows listed, and record: which processes sit in the AgentMux group that are not AgentMux's binaries, and why each is not claimed. Check the OS comparison sentences of §6.2 on Windows 11 Task Manager. This decides whether §2.2's easy re-placements (by name; by environment) are worth building in phase 2. It also records the sampler's cost for §5.4 on this machine; the build then asks the macOS and Linux platform testers to repeat both measurements on their machines.

1. **Memory first (frontend only).** §3: column order, default sort, the width ladder in the three existing views, the sticky summary line, the rail at every width. No srv change; the summary line's AgentMux total is the sum of the host rows' `mem` over the AgentMux task until phase 2 replaces it.
2. **The total and the breakdown (srv and view).** §2, §4, §5: `tower_memory.rs`, `TowerMemory`, `TowerSampleReq.memory`, the Memory view as the default tab, "Not placed", other instances, the colours. Includes the §2.2 re-placement by name if step 0 shows it pays.
3. **Trends and warning.** §7: the ledger, sparklines, the hour change, the growth flag.
4. **Later, only if asked:** Linux PSS (§6.4); the environment-based placement (◆ D3); a signal to the supervisor or an agent (◆ D6).

Phase 1 can go first and alone: it answers "hard to get the memory to appear", the part the operator can feel today.

### Files the build touches

- `crates/srv/src/backend/tower_memory.rs` (new), `tower_sampler.rs`, `rpc_types/tower.rs`, `own_instance.rs` (a reader for Tower; the function already exists), `server/app_api/tower_pane.rs` (the request flag)
- `frontend/types/rpc/` (regenerated: `TowerMemory.ts`, `TowerMemoryPart.ts`, `TowerMemoryRow.ts`, `TowerSampleReq.ts`)
- `frontend/app/view/tower/`: `tower-memory.tsx` (new), `tower-model.ts`, `tower-view.tsx`, `tower-agents.tsx`, `tower-panes.tsx` (column order), `tower-history.tsx`, `tower-view.scss`, `tower-util.ts`, the tests
- `frontend/app/theme.scss` (the four part tokens, if they are not aliases of existing ones)
- `docs/specs/SPEC_TOWER_*`: status lines

---

## 10. Decisions for the operator

Recommended answers first. Each is cheap to change before the build.

- ◆ **D1. A new Memory view, opened by default**, rather than only reordering the Agents view. Recommended: new view. The Agents view is about one agent at a time; "what is AgentMux's memory" is a different question and wants its own first screen. Cost: one more tab (four).
- ◆ **D2. The four parts and their names:** Core, Panes, Agents, Not placed. Alternative: fold "Not placed" into "Agents" (it is mostly agents and their tools). Recommended: keep it separate, because it is the only honest place for what nothing claims, and it is where today's confusion comes from.
- ◆ **D3. Placing strays by environment variable** (read `AGENTMUX_AGENT_ID` / `AGENTMUX_BLOCKID` from a process's environment), in addition to by name. Recommended: **not in the first build.** Decide after step 0 says how many strays name alone leaves.
- ◆ **D4. Metric:** keep private working set / anonymous RSS + swap / footprint; no PSS in the list; optional PSS total on Linux later. Recommended: keep.
- ◆ **D5. Colours** for the four parts (§4.5) and extending the 13-theme contrast test. Recommended: yes.
- ◆ **D6. Warnings stay inside Tower.** No notification, no agent message, no action on the memory supervisor. Recommended: yes for now; the ledger is the foundation if you want more.
- ◆ **D7. The ledger always runs** (10 s, one hour, in memory), including with no Tower open. Alternative: only while a Tower is open, so a new Tower starts empty. Recommended: always, with the 30 s fallback if the measured cost is above 0.1% of a core.
- ◆ **D8. Other AgentMux instances** are listed on one line outside the total. Alternative: not shown. Recommended: shown; it explains why the OS can show a bigger number.
- ◆ **D9. Phase order:** phase 1 (memory first, frontend only) ships alone before the srv work. Recommended: yes.

---

## 11. What this does not do

- It does not reduce AgentMux's memory. The budget work (`PLAN_MEMORY_BUDGET_CLOSE_THE_GAP`, the renderer fixes, GPU process) does that; this makes the number and its parts visible.
- It does not change the supervisor, recycling, or any limit.
- It does not measure other computers' AgentMux: a paired or SSH machine has its own Tower.
- It does not add video memory, and says so.
- It does not rewrite how processes are grouped into tasks; it reads the result and adds the "Not placed" reading of what the grouping already leaves in the AgentMux group.
