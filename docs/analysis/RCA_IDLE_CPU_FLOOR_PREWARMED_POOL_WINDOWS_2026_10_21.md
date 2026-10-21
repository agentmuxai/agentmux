# RCA: a CPU floor of about 30% that never goes away, from AgentMux's hidden pre-warmed windows

**Status:** analysis. Written 2026-10-21 against `main` at afd0387db, measured on the operator's machine (Area54: Windows 10, 8 logical cores) in a running 0.59.34 portable. The main cause is fixed in #5145 (released in v0.59.35). §6 lists what is still open.
**Author:** Manoz@Area54, at the operator's request
**Trigger (operator, 2026-10-21):** "why does it appear like there is a bottom floor around 30% cpu that is always active? what is using that?" Then: "lets rca this to file. also during research, record any time consuming blockers so that future research agents work faster."
**Related:** #5145 (the fix: the splash pulse is paused in a waiting pool window), #4767 (`SPEC_WINDOW_OPEN_LATENCY_AND_WARM_POOLS_2026_10_13.md`: pools are no longer shed at Warn memory pressure), `SPEC_TEAR_OFF_POOL_PATH_2026_05_06.md` (what a pool window is). The research notes in §7 are also in [`docs/debug/RUNBOOK_ATTRIBUTE_AGENTMUX_CPU_ON_WINDOWS.md`](../debug/RUNBOOK_ATTRIBUTE_AGENTMUX_CPU_ON_WINDOWS.md).

## 1. Summary

The floor was about 30% of all cores. Not all of it is AgentMux, and not all of AgentMux's share is waste:

| Share of all 8 cores | What | Verdict |
|---:|---|---|
| ~12% | `rustc`: another agent's `task package`, holding the build slot | Transient. Not a bug. |
| **~12%** | **Two hidden pre-warmed "new window" pool windows (about 47% of one core each) plus their share of the GPU process** | **The bug.** Fixed in v0.59.35 (#5145). |
| ~4% | The main window's renderer: 2 tabs, 4 agent panes, 2 sysinfo panes, 2 swarm panes, Tower, 2 editors | Real work, while agents stream. Not examined further. |
| ~2% | The rest of the GPU process | Expected. |
| ~3–4% | Other applications (Traktor, Voicemeeter, Parsec, dwm, svchost) | Not AgentMux. |

AgentMux keeps three hidden windows loaded so that "New Window", tear-off and floating panes open instantly: two for new windows and one for floating panes. Each waits on the startup splash, whose logo pulses with an infinite CSS animation (`index.html`, `@keyframes startup-pulse`). The windows are hidden and parked at -32000,-32000, but Chromium still treats their pages as visible, because native occlusion tracking is off (CEF #3638). So the pulse keeps producing frames for a window nobody can see, for as long as the window waits: here, since launch, 12.5 hours.

Since #4767 (v0.59.24) the pools stay populated under Warn memory pressure, which this machine is in most of the day. Before it, the pools were usually evicted, which hid the cost. #5145 (v0.59.35) pauses the pulse while `<html data-pool-window>` is set. The running instance is 0.59.34, one release before the fix.

## 2. Evidence

### 2.1 Which processes

A 20-second delta of `Get-Process` CPU for the running instance (host pid 10756), as % of one core:

| pid | renderer-client-id | Role (see §2.3) | % of one core | CPU since launch |
|---|---|---|---:|---:|
| 9424 | 8 | pre-warmed window | ~38–47% | 20,934 s |
| 7132 | 6 | pre-warmed window | ~38–47% | 20,985 s |
| 15404 | 5 | main window | ~30–36% | 11,848 s on its main thread |
| 7816 | 7 | pre-warmed pane window | ~2% | 3,132 s on one thread |
| 25592 | — | GPU process | ~19% | — |

The two pre-warmed window renderers each used about 5.8 CPU-hours over 12.5 hours of uptime: a steady 46–47% of a core from launch on, not a cost that grew later. Each idle pre-warmed window renderer used more CPU than the main window, which draws everything the user sees.

### 2.2 What kind of work

- **One thread per renderer is hot. It is not the JavaScript thread.** The hot thread (tid 15824 in 9424, 17096 in 7132) starts in `libcef.dll` at the generic Chromium thread entry. Each renderer's main thread starts at the exe entry and has used 14 s since launch.
- **It is a frame loop, not a spin.** Windows' `Thread` performance counters show each hot thread at 47% of a core with about 250 context switches a second, a few wake-ups per 60 Hz frame. A busy-wait would show either very few switches or a storm of them.
- **This fits a compositor-driven animation.** The splash pulse animates `transform` and `opacity` with `will-change` set, so Chromium runs it on the compositor thread, without the main thread.

### 2.3 Which window is which

PID order and process start times can't tell which page a renderer hosts (§7.1). In an isolated test instance started with DevTools on (`AGENTMUX_CDP_PORT=9333`, the 0.59.35+ portable from `Manoz/desktop-driving`), each page's `Performance.getMetrics` `ProcessTime` matched one renderer's OS CPU time:

| renderer-client-id | Page |
|---|---|
| 5 | Window 1 (the main window) |
| 6 | AgentMux (pre-warmed window) |
| 7 | AgentMux (pre-warmed pane window) |
| 8 | AgentMux (pre-warmed window) |
| 9 | none (not a page target) |

Both instances start the same windows in the same order (`[pool] starting the window and pane pools` at launch, same sizes and positions, §2.5), so the hot renderers in the running instance, clients 6 and 8, are its two pre-warmed new-window pages. That is an inference: the running instance has no DevTools server to confirm it directly (§7.2).

### 2.4 The cause, reproduced

In the test instance, which has the #5145 rule:

| | GPU process | pane-pool renderer | window-pool renderers |
|---|---:|---:|---:|
| pulse paused (as shipped in v0.59.35) | 3.9% | 0.0% | 0.0% |
| pulse running (attribute removed: 0.59.34 behaviour) | 12.7% | 3.1% | 0.1% |
| paused again | 3.7% | 0.0% | 0.0–0.2% |

(% of one core, 20 s windows.) Running the pulse costs about 9% of a core in the GPU process and 3% in the renderer whose page ticks. Pausing it returns both to zero at once, which matches #5145's own measurements (GPU process 15.8% → 1.0% on Area54).

### 2.5 Ruled out

- **Window geometry:** in both instances the pool windows are hidden (`IsWindowVisible` 0), not minimised, at -32000,-32000, sized 920×800 (windows) and 900×600 (pane window).
- **Pool churn:** the running instance's host log has no promote, demote, eviction or refill since `[pool] pool window renderer ready` at 02:17:34Z. The windows have waited untouched since launch.
- **GPU loss:** `cef-debug.log` has no GPU, context-lost or software-fallback lines since launch, and the GPU process (25592) has run since 19:17 local without restarting.
- **A JavaScript loop:** the pool pages' main threads are idle (14 s since launch). Before promotion a pool page only runs `markPoolWaiting` and waits (`frontend/app-init.ts`).

## 3. Why the cost was so much larger here

The test instance reproduces the mechanism but not the size. There, a ticking pool page costs about 3% of a core, and only the pane pool page ticks. The two new-window pages cost nothing even with the pulse running, which suggests Chromium throttles them there. In the running instance, both new-window pages tick, and each costs about 47% of a core: 15 times more per window.

Not settled. The running instance was in this state from launch on (its CPU totals give a steady rate), so whatever differs was decided when the windows were created, not by uptime. §5 checks whether time alone grows the cost. Candidates, unverified:

1. **Software compositing in those renderers.** If a renderer's GPU channel isn't established at creation (the instance runs with `EstablishGpuChannelAsync` and launched while other agents were compiling), its compositor rasterises every frame on the CPU, which is about this expensive. The test instance launched on a quieter machine.
2. **Visibility decided at creation.** In one instance Chromium throttles the hidden new-window pages; in the other it doesn't. A display or session change during launch (Parsec is running on this machine) could flip that.
3. **Different launch path.** The running instance started after an in-app upgrade from 0.59.33 restored a two-tab session into a maximised portrait window. The test instance was a fresh launch.

None of these change the fix: with the pulse paused, a hidden pool page has nothing to draw, so it costs nothing whichever path its frames would take.

## 4. Impact

- **About 12% of total CPU, all day, on this machine** (two renderers at ~47% of a core plus ~9% of a core of GPU process, over 8 cores), from launch until the user opens a new window or tears off a pane.
- **Every install on 0.59.24 to 0.59.34 has the mechanism.** How much it costs depends on §3.
- Power and fan noise on laptops. On Area54 it also adds to the load other agents' builds compete with.

## 5. Does the cost grow with time?

Experiment: in the test instance, the pulse was unpaused in all three pool pages for 20 minutes, sampling the renderers and the GPU process every 2 minutes (the runbook's `measure.ps1`, in a loop).

**No.** Over ten rounds, from 07:46 to 08:05:

| % of one core | min | max |
|---|---:|---:|
| GPU process | 9.5% | 14.4% |
| pane-pool renderer (ticking) | 1.8% | 3.4% |
| the two window-pool renderers | 0.0% | 0.3% |

The cost is flat, and the new-window pages stayed throttled the whole time. So the running instance's 47% per window was not built up by uptime. It was decided when its windows were created (§3).

## 6. What to do

1. **Upgrade the running instance to v0.59.35 or later** (v0.59.36 is out). It needs a restart, which also restarts the agents. Then check (§7.6): the two pre-warmed window renderers should drop to about 0% and the GPU process to a few percent.
2. **Guard against the next animation in a pool page.** #5145 pauses one selector (`#startup-loading svg`). Any other infinite animation added to the splash, or to whatever a pool page shows while it waits, brings the cost back. Proposed: one test that loads a waiting pool page and asserts `document.getAnimations().filter(a => a.playState === "running")` is empty, rather than checking one element's `animation-play-state`.
3. **Make the hidden windows really hidden to Chromium.** Pausing one animation treats the symptom: any timer-driven repaint in a pool page would cost the same. Worth a spike: tell Chromium the pool windows are hidden while they wait, for example by keeping their HWND hidden in a way Chromium's visibility tracking honours, or a CEF patch, while keeping `CalculateNativeWinOcclusion` off for CEF #3638.
4. **Answer §3** before deciding item 3's priority. If item 1 of §3 is right, other hidden or covered windows pay the same 15× cost for any animation they run.
5. **A diagnostic that works on a release build** (§7.2): `muxspect`, or the Instance panel, listing each browser's page title, `document.visibilityState`, running animations and process CPU, through the in-process CDP the browser API already uses (`crates/cef/src/browser_api/cdp.rs`). With it, this RCA would have taken minutes, and it would have worked on the running instance without a restart.

## 7. Research notes: what cost time, and the faster path

The point of this section: the next agent asked "what is using the CPU" can answer it in about 15 minutes. The same notes, as steps, are in [`docs/debug/RUNBOOK_ATTRIBUTE_AGENTMUX_CPU_ON_WINDOWS.md`](../debug/RUNBOOK_ATTRIBUTE_AGENTMUX_CPU_ON_WINDOWS.md).

1. **Check for a fix before measuring anything (cost: about 25 minutes).** The fix had been merged two days earlier. `git log v<running version>..origin/main -- <files>` on the pool and splash files (`index.html`, `frontend/app/init/`, `crates/cef/src/commands/window_pool.rs`) found #5145 at once; I ran it only after profiling. First step for any "the installed app does X" report: compare the running version (`AGENTMUX_VERSION`) with `main`.
2. **The running release build has no DevTools (cost: about 15 minutes).** Release builds don't start the CDP server (#3681). The port the host listens on (`127.0.0.1:<port>`, `IPC HTTP server started on …` in the host log) is the frontend asset server, not DevTools. Turning DevTools on needs `AGENTMUX_CDP_PORT` at launch, which means restarting the instance and losing the state you want to look at. So: reproduce in a separate instance started with `AGENTMUX_CDP_PORT=<port>`, and keep the live one for OS-level measurements.
3. **A renderer's PID doesn't say which page it hosts (cost: about 15 minutes, and a wrong inference).** `--renderer-client-id` order and process start times don't map to windows. Chromium starts a spare renderer before it is needed, so a pool window's renderer can start before the host logs spawning the window. With DevTools on, read `ProcessTime` from each page's `Performance.getMetrics` and match it to `Get-Process` CPU (`page-pids.mjs`). Without DevTools there is no reliable mapping.
4. **Thread names aren't readable from outside a sandboxed renderer.** `GetThreadDescription` returned nothing. What works: per-thread CPU deltas from `(Get-Process -Id N).Threads`, the thread's start address (`NtQueryInformationThread` class 9; the main thread starts in the exe, worker threads in `libcef.dll`), and per-thread context switches from the `Thread` performance counters.
5. **`Get-Counter '\Thread(*)\…'` is awkward.** It reports invalid samples (pass `-ErrorAction SilentlyContinue`), and instance names repeat across processes. Key each sample on its counter path minus the counter name, and filter with `\Thread(agentmux*)`.
6. **Finding the logs.** `~/.agentmux/channels/<channel>/logs/current-host-v<ver>.path` holds the path of the real log, under `versions/<ver>/logs/`, which also has `cef-debug.log` (Chromium's own log, including the frontend console). The channel is `AGENTMUX_INSTANCE_CHANNEL`. It is not the channel in the path of your Claude identity folder, which can belong to another channel.
7. **A fresh instance didn't reproduce it.** Its new-window pool pages were throttled. Don't conclude "not a bug" from a clean repro: state decided at launch can differ (§3). Force the condition instead, here by removing the attribute the fix keys on.
8. **CDP from Node.** Node 24 has a global `WebSocket`, so a CDP client needs no packages. Calling `process.exit` right after `ws.close()` trips a libuv assertion on Windows (exit code 9). It's harmless, but it looks like a failure.

The runbook has the scripts this used.
