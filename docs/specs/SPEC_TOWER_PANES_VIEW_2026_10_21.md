# Spec: Tower's Panes view: CPU and memory for every pane, measured where it can be and labeled where it can't

**Date:** 2026-10-21
**Status:** draft. Nothing is built.
**Author:** korp
**Trigger:** Operator, 2026-10-21: *"how can I see memory use reliably inside of tower? we need to know the CPU/memory use of each pane inside of tower."*
**Builds on:**
- `docs/specs/SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md`: Tower, its sampler, its metric definitions (§4).
- `docs/specs/SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md`: today's Agents and Processes views.
- `docs/specs/SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md`: sleeping browser panes, which this view shows.
- `crates/cef/src/renderer_map.rs`: which renderer process serves which browser.

---

## 1. Summary

Tower answers "what does each **agent** cost" (the Agents view) and "what is using this machine" (the Processes view). It doesn't answer "what does each **pane** cost", and it can't say it today for most panes, because most panes have no process of their own: every pane except a browser pane runs inside its window's one renderer process.

This spec adds a third view, **Panes**: every window, its tabs and its panes, each pane with its CPU and memory. Each number says what kind of number it is:

| Kind | Shown as | Where it comes from |
|---|---|---|
| **Measured** | `310 MB` | The operating system, for processes the pane owns alone. As exact as Task Manager. |
| **Shared** | `310 MB shared` | A process the pane shares with other panes (two browser panes Chromium put in one renderer). Shown under each, counted once in totals. |
| **Estimated** | `≈ 40 MB` (italic, "est.") | The pane's share of its window's renderer, from what it holds (§5). Not a measurement. |
| **Not attributable** | `—` | Costs no pane can be given, like the GPU process; shown on the window or AgentMux row instead. |

Totals always add up: each window's rows sum to its renderer's measured memory, with an **Other** row for what the estimates don't cover.

## 2. What can be measured, per kind of pane

| Pane | Its own processes (measured) | Its part of the window's renderer | Today in Tower |
|---|---|---|---|
| **Agent** | the CLI and everything it started (the tracker: a Job Object or cgroup) | the transcript, its DOM, its stores: **estimate** | yes, in Agents |
| **Terminal** | its shell and everything under it | xterm's buffer, its WebGL canvas: **estimate** | yes, under Terminals |
| **Browser** | its page's renderer, **and the renderer of each cross-site frame on the page** | its address bar: negligible | only the main renderer, as an unnamed-pane row under AgentMux (§3) |
| Editor, Files, Sysinfo, Swarm, Tower, Settings, widgets | none | everything: **estimate** | no |
| **The window itself** | its renderer (the frontend) | — | yes, as a row under AgentMux |
| **GPU process** | one per instance, shared by every window and browser pane | not attributable | yes, under AgentMux |

A sleeping (hibernated) browser pane has no process: it shows **asleep** and what it last used before it slept (§6.3).

## 3. What's wrong with today's numbers

- **Memory is reliable per process.** Tower's Memory column is the private working set on Windows (Task Manager's "Memory (active private working set)"), the physical footprint on macOS, and anonymous RSS plus swap on Linux (Tower spec §4.1). For any one process it can be trusted.
- **A browser pane's cross-site frames are lost.** Site isolation runs each cross-site iframe in its own renderer (ads, embeds, sign-in widgets). A renderer reports its PID only when a browser is created in it (`renderer_map.rs` `on_browser_created`), keyed by browser: one PID per pane. The frame processes are either missing or overwrite the main one. Camper's browser-pane lab counted 11 renderer processes for 5 pages: the map names at most 5 of them.
- **Two panes can share one renderer.** Chromium can put pages of one site in the same process when it is near its process limit, or reuse a spare. The map can't say a process serves two panes.
- **Most panes have no row.** Editor, Files, Sysinfo, Swarm and widget panes cost memory only inside the window's renderer, which Tower shows as one number.

## 4. The Panes view

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Agents   Processes   Panes                         % of machine ▾         │
├──────────────────────────────────────────────────────────────────────────┤
│ ▾ Window "My Bizzy"                            9%    3.9 GB              │
│   ▾ Tab 1 (shown)                                                        │
│     ● Korp            agent     shown       2%   310 MB  + ≈ 120 MB est. │
│     ◍ GitHub · PR…    browser   hidden      0%   185 MB  (3 processes)   │
│     ◍ GitHub · Issue… browser   asleep      —    —   freed ≈ 110 MB      │
│     ▣ Swarm           swarm     shown       —    ≈ 30 MB est.            │
│   ▸ Tab 2 (hidden)                              1%    1.1 GB              │
│   Window renderer (measured)                   3%    2.4 GB              │
│     Other (not attributed to a pane)                  ≈ 1.1 GB           │
│ ▸ Pool windows (idle)                          0%    260 MB              │
│ AgentMux: GPU, network, srv, host, launcher    2%    410 MB              │
└──────────────────────────────────────────────────────────────────────────┘
```

- **Grouped as the person sees them:** window, then tab, then pane, in layout order; pane-tab members under their pane. A tab's row shows whether it is the shown one.
- **Each pane row:** its kind's icon and its title, the kind, its state (**shown**, **hidden**, **dormant** for an unselected pane tab, **asleep** for a hibernated browser pane), CPU, Memory. Measured memory plain; the estimate after it, italic, with "est.".
- **Expanding a pane:** its processes (as in the Processes view, with their own CPU and memory), and for an estimate, what it was estimated from (§5): "2,400 transcript rows, 18,000 DOM nodes", "scrollback 10,000 lines × 120 columns".
- **The window renderer row** is measured and shows the sum of its panes' estimates and the **Other** remainder, so nothing is hidden or double counted.
- **AgentMux's shared processes** (GPU, network, storage, srv, host, launcher) are one row at the end, as in the Processes view.
- **Sort:** layout order by default; "Sort by Memory / CPU" in the view's menu sorts panes within their tab.
- **Selecting a pane** focuses it in its window (Tower's existing "go to" action), so a heavy pane is one click from being closed or put to sleep.
- **Sleep from Tower:** a browser pane row's menu has **Put to sleep** and **Keep awake** (the hibernation spec's commands for one pane).

## 5. Estimating a pane's share of its window

There is no per-pane memory number inside one renderer: Chromium measures per process, and Blink's heap doesn't record which part of the page an object belongs to. What the window can measure cheaply is **what each pane holds**, and the lab can measure **what each of those things costs**.

### 5.1 What each pane reports

Each window's frontend, while a Tower is sampling, reports per pane every 2 s (on an idle callback, so it never delays input):

| Pane | Weights |
|---|---|
| any | DOM nodes under the pane's root (`[data-blockid]`), images decoded, canvases and their pixel size |
| agent | transcript rows in the store, their text bytes, mounted rows |
| terminal | scrollback lines × columns, whether it has a WebGL context and its atlas size |
| editor | open documents and their text bytes |
| sysinfo, Tower | series held × points |
| widget | its iframe: Chromium can measure a frame on its own (§5.3) |

### 5.2 From weights to megabytes

- **Coefficients from the memory lab:** for each weight, megabytes per unit, fitted from lab runs that add one kind of pane at a time to an empty window (the memory lab already builds those layouts: `scripts/perf/memory-lab`). For example: bytes per DOM node, per transcript byte, per terminal cell (the budget report's 12.5 bytes per xterm cell as a start).
- **Scaled to the measured total:** the estimates are scaled so they never sum past the window renderer's measured memory. What they don't cover is **Other**: Blink's freed-but-kept pages (the recycle spec measured 89–92 % of a long-running renderer's heap empty), the app's shared stores, fragmentation.
- **Shown as estimates:** italic, "est.", rounded to two significant figures, and the tooltip names the weights. Never summed into a measured column.

### 5.3 Where Chromium can measure a part of the page

A **widget** runs in a sandboxed iframe with its own document. `performance.measureUserAgentSpecificMemory()` would attribute memory to it, but it needs a cross-origin-isolated page, and the app's page isn't one (making it one breaks the app's own resources). Not used. If Chromium gives sandboxed frames their own process (Loap-3's GPU spec asks the same question), a widget becomes a measured row like a browser pane.

### 5.4 CPU for panes without a process

Not estimated. A DOM pane's CPU is the window renderer's, and the browser's long-task and long-animation-frame APIs attribute time to scripts, not to parts of the page. Those panes show `—` for CPU with a tooltip; the window renderer row has the measured CPU. (The agent pane's streaming work is the usual cause, and its own row shows it is streaming.)

## 6. Data and backend changes

### 6.1 Every frame's renderer, not just the page's

`renderer_map.rs` changes from one PID per browser to **the set of PIDs serving each browser**:
- The renderer side reports on `OnContextCreated` (called for every frame, in whichever process hosts it), not only `OnBrowserCreated`: `(browser id, frame id, pid)`.
- The host keeps, per browser, frame id → PID, drops a frame on its context's release, and the whole browser on close.
- `snapshot` returns, per PID, every browser (and so every pane) it serves. A PID in two panes' sets is **shared** (§1).
- A renderer serving no frame (Chromium's spare) is labeled "spare renderer" under AgentMux.

### 6.2 The sample

`tower.sample` (srv, `server/app_api/tower_pane.rs`) gains a **panes** section built from what it already has plus two inputs:
- **Layout:** windows, tabs, blocks and their meta from the store (as `Pane` `layout` does, `service/introspect.rs`), including `browser:sleep`.
- **Processes per pane:** the tracker and process-tree groups it already computes (agents, terminals), and the renderer map's frame PIDs (browser panes).
- **Weights:** the windows' latest reports (§5.1), sent with a new `tower.paneweights` RPC from each window while srv has said Tower is sampling (a `tower:sampling` event, on and off). Weights older than 10 s are dropped.

Estimation (§5.2) runs in srv so every window and remote viewer gets the same numbers.

### 6.3 Sleeping panes

srv remembers each browser pane's last measured memory. When the pane's `browser:sleep` appears, its row shows **asleep** and "freed ≈ N MB" from that value. The hibernation card can use the same number later instead of its generic "usually 100 to 200 MB".

### 6.4 Remote and paired machines

The Panes view is for this machine's AgentMux. Over a paired AgentMux computer (Tower's LAN peers), it works the same if that instance is new enough to send panes; otherwise the tab is hidden for it. SSH and WSL hosts have no AgentMux panes and don't show the tab.

## 7. Phases

| Phase | What | Done when |
|---|---|---|
| **1** | The Panes view with **measured numbers only**: windows, tabs, panes; agent and terminal processes; browser panes with every frame's renderer (§6.1) and shared processes; asleep panes; the window renderer and AgentMux rows | A news page with cross-site frames shows all its renderer processes under its pane, matching Task Manager's process list for that page; totals equal the instance's measured total |
| **2** | Estimates (§5): weights from each window, coefficients from the lab, the Other remainder | Lab: each pane kind's estimate within ±30 % of its measured marginal cost (the window renderer's growth when that pane is added to an empty window) |
| **3** | Sleep and keep-awake from Tower; "freed ≈ N MB" on the hibernation card | — |

## 8. Tests

- **Host:** a page with a cross-site iframe reports two PIDs for its browser; closing the frame drops one; two browsers sharing a PID are both listed for it.
- **srv:** the panes section groups processes under the right pane, marks shared processes, counts each once in totals, shows asleep panes with their last value, drops stale weights; estimates never exceed the window renderer's measured memory.
- **Frontend:** the view's grouping and states; measured and estimated values are visually distinct and never summed together; the tooltip names the weights.
- **Lab:** the Phase 1 and 2 "done when" checks, on Windows first, then macOS and Linux.

## 9. Decisions for the operator

1. **A third tab, "Panes"**, or panes folded into the Agents rail (each agent's panes under it, others under Terminals/AgentMux)? Recommended: a third tab; the Agents view answers a different question.
2. **Estimates (Phase 2):** show them, labeled, or show measured numbers only and leave DOM panes as `—`? Recommended: show them labeled; "where does my window's 3.8 GB go" is the question that matters most here, and it's all DOM panes.
3. **Order:** Phase 1 before the hibernation PR merges, or after? Recommended: after; they don't depend on each other, and #5290 is approved.

Agent: korp@narko
