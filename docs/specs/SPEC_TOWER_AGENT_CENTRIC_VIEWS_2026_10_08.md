# SPEC: Tower, agent first: an Agents rail and a Processes list grouped by agent

**Status:** active — Phase 1 (the Agents view) shipped in PR #4597; Phase 2 (Processes grouped by agent) in PR #4611, except carrying the rail's selection into Processes as a filter; Phases 3-4 ("started by" labels and exited processes, history) not started. Verified 2026-10-10.
**Date:** 2026-10-08
**Builds on:** `SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md` (Tower as shipped in
#4498, #4499, #4500).
**Related:** `SPEC_FILE_BROWSER_PANE_2026_10_01.md` (Hangar, the layout this
copies), `SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md` and
`SPEC_BACKGROUND_TASK_STRUCTURED_FEED_AND_SWARM_OWNERSHIP_2026_09_27.md` (where
"which tool call started this" comes from),
`SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md` (controls).

## 0. The ask

From the operator, after using Tower:

> the tower's specialty is showing what agents are running and the resources
> they are using. Lets model it a bit like the hangar, where the list of agents
> by color is listed on the left, and when selecting it, you see the tasks the
> agent is running. … often we want to know which instance of rustc.exe is
> being driven by who, and how much CPU and memory is the agent taking overall.
> … Explain the difference between "Tasks" and "hosts" .. I am even thinking
> having a single process list, but grouped by agent. it could be 2 views.

## 1. "Tasks" and "Host" today, and why they don't answer the question

| | **Tasks** (shipped) | **Host** (shipped) |
|---|---|---|
| A row is | a pane (agent or terminal) and every process it started, plus one "AgentMux" row | one process on the machine |
| Found by | the pane's Job Object / cgroup / process tree | the OS's process table |
| Covers | only what AgentMux started | everything, whoever started it |
| Answers | "what does each pane cost?" | "what is using this machine?" |

Both are right, but neither is shaped around the question the operator
actually asks:

1. **Agents are not first-class.** A task row is a pane labelled with the
   agent's name; there is no agent color, no status, no "this agent overall"
   view, and terminals and AgentMux sit in the same list.
2. **"Which `rustc.exe` belongs to whom?"** needs two views today: the Host
   list shows `rustc.exe` with an agent badge only when the pane's tracker
   owns it, and the Tasks view shows the agent's processes flat, so the
   `bash → cargo → rustc` chain that explains *why* rustc is running is lost.
3. **Short bursts vanish.** A `cargo build` starts dozens of `rustc` that live a
   few seconds; a 2 s sampler shows some of them for one tick, or none.
4. **No history.** A row says what is happening now, not whether this agent has
   been pegging the machine for ten minutes.

## 2. What the best tools do (research summary)

Sources in §11. The ones that solve "owner → its processes" best:

- **Kubernetes Lens / k9s, Docker Desktop:** an owner list (namespace, compose
  project) with summed CPU and memory; select one to see its units with
  charts over time. The closest analogue to agent → processes.
- **Windows Task Manager, Users tab:** one row per owner with a total and a
  process count `(n)`, expanding to that owner's processes. The Processes tab
  groups by app (Apps / Background / Windows), with a toggle for a flat list.
  Its long-standing complaints: no tree, so same-named processes can't be told
  apart (users fall back to PowerShell parent-PID lookups: the rustc problem);
  rows jump while sorted (the hidden Ctrl-to-freeze since 1994).
- **macOS Activity Monitor ("All processes, hierarchically"), Process
  Explorer / System Informer:** a process tree is what explains a process.
  System Informer sorts parents by their subtree totals; Process Explorer keeps
  exited processes visible (red) for a while, which is how short-lived
  processes are seen at all.
- **atop / glances:** "per program" pivots (`rustc ×7`), and accounting of
  exited processes so a compile isn't invisible to sampling.
- **Chrome / Firefox task managers, VS Code Process Explorer:** the warning.
  When attribution stops one level too high ("Renderer", "extension host"
  repeated with no owner) users can't act on it.
- **Agent-specific tools** (agent-ps, RAM Sleeper, ccdeck): per-agent status
  and tokens are common; real per-agent CPU and memory over the whole process
  tree is mostly unoccupied ground. agent-ps colors each agent and marks a
  guessed owner with `?`.

## 3. Design: two views, agent first

The pane header keeps the machine picker (this computer, SSH, WSL, paired
AgentMux computers) and the CPU convention. The two tabs become:

| New | Replaces | Question it answers |
|---|---|---|
| **Agents** (default) | Tasks | "What is each agent doing, and what is it costing?" |
| **Processes** | Host | "What is using this machine, and whose is it?" |

### 3.1 Agents view: a rail and a detail, like Hangar

```
┌──────────────────────┬──────────────────────────────────────────────────┐
│ ● AgentX      38% ▁▃█ │ ● AgentX · claude · working        38% · 4.1 GB  │
│ ● AgentY       2% ▁▁▁ │ ▁▁▂▃▅▇█▇▅▃ (CPU, last 10 min)        12 processes │
│ ● Clamk        0% ▁▁▁ │ ─────────────────────────────────────────────── │
│ ─────────────────────  │ ▾ claude.exe              1%    310 MB          │
│ ▫ Terminals    3% ▁▂▁ │   ▸ agentmux-mcp.exe        0%     40 MB  support│
│ ▫ AgentMux     1% ▁▁▁ │   ▾ bash  ⟵ "Run the srv tests"  37%  3.6 GB    │
│ ▫ Everything else     │     ▾ cargo.exe           0%     90 MB          │
│                       │       ▸ rustc.exe ×6      36%    3.4 GB          │
│                       │       ⋯ rustc.exe (exited 4 s ago)              │
└──────────────────────┴──────────────────────────────────────────────────┘
```

**The rail** (left, the Files pane's `.files-places` pattern; hidden below
~420 px wide with a toolbar toggle, like Hangar):

- One row per **agent** running here: its **color swatch and name** (never
  color alone), provider icon, a status dot (working / waiting / idle, words
  in the tooltip, reusing Swarm's `AgentStatusChip` states), **CPU and memory
  totals**, and a 60 s CPU sparkline in the agent's color. A process count in
  the tooltip.
- Below a divider, fixed entries so everything adds up to the machine:
  **Terminals** (panes with no agent), **AgentMux** (the app itself), and
  **Everything else** (processes no pane started; selecting it shows the
  Processes view's "Other" group).
- **Order:** stable by default (pane order), with "Sort by CPU / Memory / Name"
  in the rail's menu. A live sort uses a smoothed value (5-10 s average),
  re-orders at most every few seconds, and freezes while the pointer is over
  the rail, so rows don't jump under the cursor (Task Manager's hidden-Ctrl
  problem, solved without a hidden key).
- Arrow keys move the selection (the `Tabs` vertical roving pattern);
  selection is kept in block meta (`tower:agent`).
- Clicking an agent's name reveals its pane (as Swarm does); clicking the row
  selects it here.

**The detail** (right) for the selected agent:

- **Header:** swatch, name, provider, status, total CPU and memory, process
  count, and a **10-minute CPU and memory chart** (history kept by the pane,
  not persisted).
- **A process tree,** not a flat table: `claude → bash → cargo → rustc`, so
  the reason for a process is on screen. Siblings sort by the chosen column;
  a parent sorts by its subtree total (System Informer). Expand all / Collapse
  all. Several siblings with the same name collapse into one line
  (`rustc.exe ×6`, with their totals), expandable.
- **"Started by" labels:** the tool call that started a subtree, from data
  AgentMux already holds (§5.3): the Bash tool's own description ("Run the
  srv tests"), the same text the agent's transcript already shows. No command
  line is shown (Tower still never shows command lines).
- **Roles** as shipped: the agent's own process, its support processes (MCP
  servers, conhost; shown dimmer), what it started.
- **Exited processes linger** dimmed for 60 s, with the CPU time they used
  ("rustc.exe, exited 4 s ago, 2.1 s CPU"), so a build's burst is visible.
- **Columns:** CPU (now), CPU time (cumulative, survives exit), Memory
  (private), Peak memory, PID, Started. CPU time and peak are opt-in columns.

### 3.2 Processes view: one list, grouped by agent

The answer to "which rustc is whose" in one glance, and the place for the rest
of the machine.

- One **group per agent**, in the agent's color (a stripe on the left of the
  group header and its rows), with the agent's totals and count in the header.
  Headers stay pinned while their group scrolls.
- Then **Terminals**, **AgentMux**, and **Other processes**, the last grouped
  by app as Task Manager does (`chrome.exe (46)`; the held
  `qtest/tower-host-groups` branch).
- Groups start collapsed except the busiest one or two; the expansion is
  remembered.
- **Search keeps rows under their owner:** typing `rustc` shows every
  `rustc.exe` under its agent's header, so ownership is never lost.
- **Group by: Agent | App.** By App pivots the other way: `rustc.exe ×9`,
  opening to its instances split by agent (glances / atop's per-program view).
- Selecting an agent in the Agents rail and switching to Processes keeps it
  as a filter (Lens's namespace filter); "All" clears it.

### 3.3 "Which rustc.exe is driven by whom?", end to end

1. Agents view: AgentX is at the top at 38%. Selecting it shows
   `bash ⟵ "Run the srv tests" → cargo → rustc.exe ×6`. That is the answer:
   AgentX's test run.
2. Processes view, search `rustc`: two groups, AgentX (6) and AgentY (1), each
   in its color. AgentY's one is under `bash ⟵ "Check the build"`.
3. A `rustc.exe` under **Other processes** was started outside AgentMux (your
   own terminal outside AgentMux, an IDE).

### 3.4 Other machines

- **SSH and WSL** have no agents: the Processes view only, grouped by app.
- **A paired AgentMux computer** has agents: both views, with that computer's
  agents and colors.

## 4. Visual and interaction rules

- Color is always paired with the agent's name (WCAG 1.4.1). Colors come from
  the agent's pane (`blockRoleColor`: the user's pane color, else the agent's
  color), so the rail matches the panes.
- CPU stays a share of the whole machine by default (agents add up to the
  machine total, Task Manager's 24H2 formula); "% of a core" stays as the
  toggle. Memory stays private memory, named in the header tooltip.
- Refresh: every 2 s while visible, as now. Values update in place; rows never
  rebuild on refresh (the keyed store from #4498).
- Narrow panes: the rail collapses to swatch + CPU, then hides behind the
  toolbar toggle.

## 5. Data and backend changes

### 5.1 Agent identity on a task

`TowerTask` gains what the rail needs, so the frontend needs no extra lookups
per refresh: `agent_id` (definition), `provider`, `status` (working / waiting
/ idle). Color is resolved in the frontend from the block's meta (it already
has it, and it follows theme changes). Tasks of one agent in several panes
(rare) are summed under one rail row with the panes listed in the detail.

### 5.2 A tree, not a list

Processes already carry `ppid` and their start; the detail builds the tree in
the frontend, trusting a parent only if it started first (as the sampler
does). Sticky membership (shipped) keeps a reparented process under its last
parent, marked "detached".

### 5.3 "Started by"

- **Backgrounded Bash calls:** `db_background_tasks` already maps
  (block, tool call, root PID) to the call's description, filled by the CLI's
  task feed and `agentmux-bashwrap`'s PID report
  (`COMMAND_BACKGROUND_TASK_PID`). The sampler joins a subtree's root PID to
  it.
- **Foreground Bash calls** are not in that registry today. Phase 3 has
  `agentmux-bashwrap` report the root PID of every command it wraps (it already
  reports declared-background ones), keyed by tool call.
- **MCP `Shell()` and `!cmd`** are spawned by srv itself (`spawn_tracked` with
  `Join::Adopted`); srv records (PID, start) → the request at spawn.
- Unknown stays unlabeled; a guessed owner is marked as guessed (agent-ps's
  `?`), never presented as certain.

### 5.4 Exited processes and cumulative CPU

The sampler keeps the last sample of each process for 60 s after it disappears
(with its final CPU time), and per task the Job/cgroup account (shipped)
already includes exited members, so the task total stays right. Peak memory:
the max of private memory seen per (pid, start) while sampled.

### 5.5 History

The pane keeps a ring buffer per rail entry (10 minutes at 2 s = 300 points of
CPU and memory, a few KB per agent). Not persisted; reopening starts empty.

## 6. What stays as shipped

The samplers, attribution (Job Object / cgroup / tree / sticky), metrics,
no-elevation rules, remote hosts and LAN pairing are unchanged. This is a
re-shaping of the pane around agents plus four additions: agent fields on a
task, the tree, "started by", and lingering exited processes.

## 7. Phases

1. **Agents view.** Rail (swatches, totals, sparklines, fixed entries, stable
   sort) and detail with the process tree and `×n` collapsing. Rename tabs to
   Agents / Processes. `TowerTask` agent fields.
2. **Processes view.** One list grouped by agent, Other grouped by app (fold in
   the held group-by-app branch), search keeps owners, Group by Agent | App,
   the rail selection as a filter.
3. **Attribution detail.** "Started by" labels (background registry, then
   bashwrap for every command, srv spawns), lingering exited processes, peak
   memory and CPU time columns.
4. **History.** The 10-minute chart in the detail header.

## 8. Testing

- Rail: one row per agent with its pane's color and name; totals equal the sum
  of its processes (or its Job account); fixed entries add up to the machine;
  stable order under changing CPU; selection survives refreshes and is
  restored from meta.
- Tree: `bash → cargo → rustc` nesting from a fixture; a reused parent PID
  isn't trusted; `×n` collapsing and its totals; parents sort by subtree.
- Processes view: search `rustc` keeps rows under their agents; By App pivot
  splits instances by agent; Other groups by app.
- "Started by": a backgrounded task's description labels its subtree; nothing
  labels a process whose root isn't known.
- Exited: a process that disappears stays dimmed for 60 s with its CPU time,
  then goes.
- Manual, in an isolated `task dev`: two agents running `cargo build` at once;
  each `rustc.exe` is under the right agent in both views.

## 9. Open questions

1. Agents with no running process (idle, no pane open): list them in the rail
   (greyed) or only agents with panes here? Proposed: only agents with a pane
   on this machine.
2. Terminals: one "Terminals" rail entry, or one per terminal pane? Proposed:
   one entry, with the panes as top-level nodes in its detail.
3. A process two agents use (a shared language server or MCP server): one
   owner (whoever spawned it) with a "shared" badge naming the others, rather
   than splitting its cost. Agree?
4. Should "Everything else" be in the Agents rail at all, or only in the
   Processes view? Proposed: in the rail, last, so the rail adds up to the
   machine.

## 10. Names

"Tasks" and "Host" were AgentMux-internal words. "Agents" and "Processes" say
what each view lists. The pane stays **Tower**.

## 11. Sources

- Windows Task Manager: https://www.bleepingcomputer.com/news/microsoft/closer-look-at-windows-11s-new-task-manager/,
  https://www.elevenforum.com/t/enable-or-disable-group-by-type-view-on-processes-page-in-task-manager-in-windows-11.30305/,
  https://www.digitalcitizen.life/manage-users-task-manager-windows/,
  https://learn.microsoft.com/en-us/answers/questions/3719769/cant-find-process-identifier-in-task-manager,
  https://www.bleepingcomputer.com/news/microsoft/windows-task-manager-refresh-can-be-paused-using-ctrl-key/,
  https://www.windowslatest.com/2025/05/26/windows-11-24h2s-task-manager-new-cpu-usage-formula-rolls-out-to-everyone/
- Activity Monitor: https://support.apple.com/guide/activity-monitor/view-information-about-processes-actmntr1001/mac,
  https://eclecticlight.co/2024/11/08/why-cpu-in-activity-monitor-isnt-what-you-think/
- Process Explorer / System Informer: https://documentation.help/procexp/The_Process_View.htm,
  https://documentation.help/Process-Explorer/Options.htm,
  https://deepwiki.com/winsiderss/systeminformer/6.6-additional-plugins
- htop / btop / glances / atop: https://man7.org/linux/man-pages/man1/htop.1.html,
  https://github.com/aristocratos/btop/issues/673,
  https://glances.readthedocs.io/en/latest/aoa/ps.html,
  https://manpages.debian.org/unstable/atop/atop.1.en.html
- Browsers and editors: https://chromestory.com/2025/01/chrome-task-manager-new/,
  https://support.mozilla.org/en-US/kb/task-manager-tabs-or-extensions-are-slowing-firefox,
  https://github.com/microsoft/vscode/wiki/performance-issues
- Containers and Kubernetes: https://docs.docker.com/desktop/use-desktop/container/,
  https://k9scli.io/topics/columns/,
  https://docs.k8slens.dev/k8slens/using-lens/workloads/pods/
- Agent tools: https://github.com/mkhuda/agent-ps, https://kejid.github.io/ramsleeper/,
  https://ccdeck.dev/
- Accessibility: https://accessibility.build/wcag/1-4-1
