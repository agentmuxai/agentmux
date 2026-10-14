# Report: the Sysinfo pane, why it breaks under load, and what a modern one looks like

**Date:** 2026-10-13
**Author:** AgentX@narko
**Status:** active. Phase 0 (§6) built 2026-10-13: gaps draw as breaks, the area is no longer stacked, and the x axis has real second and minute ticks. Phase 1 built 2026-10-13, in a narrower form than §5.1 described (see §6). Phases 2 to 5 not started; the open questions in §7 are unanswered.
**Origin:** investigation to decide between fixing and rewriting, and to scope a redesign.
**Inputs:**
- the owner's two screenshots of the CPU chart during heavy load (2026-10-13, 08:52 to 08:54);
- a read of the code at main `5d9af9260`;
- about 40 primary sources on sampling and monitor design (listed in §8).

**Builds on:**
- `REPORT_SYSINFO_COMBINED_CHART_RESEARCH_2026_08_17.md`: small multiples over a combined chart, still right;
- `SPEC_TERMINAL_INPUT_PRIORITY_OVER_SYSINFO_2026_06_16.md`: why sysinfo yields to terminal output;
- `docs/specs/archive/sysinfo-tick-irregularity.md`.

---

## 1. Summary

The gaps are not the machine failing to measure. srv takes its samples, then throws most of them away on the way to the window, and the chart draws what is left wrongly.

1. **Missing samples come from the transport.**
   - Sysinfo rides the WebSocket's background lane, which is served only when terminal and agent output are idle. Heavy CPU is exactly when they aren't.
   - When the lane is finally served, it keeps only the newest sample.
   - The samples in between still exist in srv's event ring, but the pane never asks for them again.
2. **The chart turns a gap into a ramp from zero.**
   - The area mark is stacked, and Observable Plot's stack maps a missing value to the baseline.
   - So every gap is drawn as a drop to 0 followed by a diagonal climb back up. That is a reading that never happened.
3. **The x axis is a number axis, not a time axis.** With minute labels on a two-minute window, every tick says "8:52 AM".
4. **Short gaps are painted over.** A 1 to 3 tick gap is filled by repeating the last value, which shows readings that were never taken.

**Recommendation:** rewrite the frontend (model, chart, layout) and the transport for sysinfo; extract and harden the sampler rather than rewrite it. The sampling code itself is under 200 lines and sound. The frontend's problems are structural (SVG rebuilt per redraw, stacked area, linear x, fabricated points), and the redesign in §5 changes the layout anyway.

Sysinfo and Tower should also share one data layer, with one process snapshot, one way of grouping processes into tasks and one ring buffer. A scheduler should run each source only while someone is looking at it (§5.5): today the two pipelines scan the same machine twice and can show different numbers for the same pane. The panes themselves stay separate. Plan in §6.

## 2. What the screenshots show

| Symptom | Cause | Where |
|---|---|---|
| 10 to 30 s with no data, then one point | the background lane is starved, then coalesced to the newest sample | `server/websocket.rs:180` (biased select), `:276–289`, `coalesce_background` `:391–408`; lane choice `backend/eventbus.rs:327` |
| After a gap, a straight ramp up from 0 | `Plot.areaY` stacks implicitly; stack maps NaN to the baseline (`@observablehq/plot/src/transforms/stack.js:109`); the reducer puts one NaN sentinel only on the gap's left edge | `sysinfo-plot.tsx:152`, `sysinfo-model.ts:182–184` |
| Every x tick reads the same minute | `ts` is a plain number (linear scale); ticks formatted `h:mm A` | `sysinfo-plot.tsx:257–260` |
| Large view: no y-axis labels | axis titles are switched off on purpose (`label: null`, `sysinfo-util.ts:108`); tick values should still render. Not reproduced, so to be checked | `sysinfo-util.ts:108`, `sysinfo-plot.tsx` |
| (not visible) values never sampled | 1 to 3 missed ticks are filled by repeating the last value | `sysinfo-model.ts:174–181` |

The gap is never repaired: history is read only when the pane opens or the connection changes (`sysinfo-model.ts:327–345`). A gap-triggered reload was removed on purpose (`sysinfo-view.tsx:48–51`), and live events are dropped while that load runs (`sysinfo-view.tsx:55`).

The sampler can still miss single ticks under load. On every tick, after publishing, it refreshes all processes, plus up to 64 PIDs per pane with `ProcessRefreshKind::everything()` (`sysinfo.rs:1209`, `:1242`). Every 30 s on Windows it also opens a handle to every process (`:348–499`). These cost milliseconds to hundreds of milliseconds, so they explain an occasional skipped tick (`MissedTickBehavior::Skip`, `:1098`), not 30 s holes.

## 3. The current code, briefly

**Backend (`backend/sysinfo.rs`, 1,452 lines):**
- **Loop:** one tokio task (`run_sysinfo_loop`, spawned in `bootstrap/background.rs:125`) ticks every `telemetry:interval` (default 1 s, 0.2 to 2 s). OS reads run in `block_in_place`.
- **Source:** CPU comes from the `sysinfo` crate 0.35.2 (PDH on Windows). Total CPU is the mean of the per-core values.
- **Metrics:** CPU total and per core, memory (total/used/available/free, plus Windows commit), network and disk MB/s, per-volume disk. No GPU, temperature or pressure.
- **Timestamp:** wall clock (`now_ms`), not monotonic.
- **The rest of the file:** about 60% is Windows memory-commit and handle-leak diagnostics, and per-pane block stats. They share the loop but are a different job.

**Transport:**
- `publish_event::<SysInfo>` with a 1,024-entry persisted ring, scope `local`.
- Payload `{ts, values: {name: f64}, uptime_secs}`, typed in #4791.
- Six frontend subscribers: status bar, popovers and the pane.

**Frontend (`frontend/app/view/sysinfo/`, about 1,100 lines plus 540 lines of tests):**
- **Chart:** Observable Plot 0.6.17, with a full SVG rebuilt per redraw, sometimes twice to refit the left margin (`sysinfo-plot.tsx:249`, `:276–281`). Redraws are throttled to one every 2 s.
- **Model:** an immutable array copied about three times per sample, so the model, view and chart concerns are mixed.
- **Plot types:** nine. Per-core metadata stops at 32 cores (`sysinfo-types.ts`), so "All CPU" on a larger machine shows "?".
- **Dead code:** `getDefaultData` is never called, and `plotMetaAtom` is a constant.
- **Tests:** they cover the reducer and the layout maths. Nothing tests what is actually drawn (gaps, baseline, ticks) or transport drops.
- **History:** 48 commits since March, from the Wave Terminal fork through a March modularisation to resize and typing work. Much of it is patching around the structure above.

## 4. What the best monitors do (research digest)

### 4.1 Sampling that survives a saturated machine

- **Own OS thread.** bottom runs collection on its own thread and hands results to the UI over a channel; btop does the same (secondary sources). Not a shared async runtime.
- **Thread priority.** Netdata's samplers ran at `idle` priority until v1.34 and had chart gaps on busy machines; it moved to `batch`. On Windows, a modest `THREAD_PRIORITY_ABOVE_NORMAL` is the safe step. Microsoft warns against going far above it, and against background mode, which also lowers I/O and memory priority.
- **Stamp and divide by the real time.** Stamp each sample with a monotonic clock (`Instant`) and compute rates as change in counter over measured elapsed time; htop does exactly this with `/proc/stat`.
  - Use wrapping subtraction: macOS per-core tick counters are 32-bit, and Stats crashed when they wrapped after about 17 days.
  - Treat a suspend (wall clock jumps while monotonic time doesn't, or the reverse) as a gap.
- **Interval.** 1 s is the common default (bottom; btop recommends 2 s). The `sysinfo` crate needs at least `MINIMUM_CPU_UPDATE_INTERVAL` between CPU refreshes. Poll expensive sources (per-process data, sensors) less often.
- **Windows specifics.**
  - *Utility vs Time:* Task Manager shows **% Processor Utility**, which is frequency-scaled and can exceed 100% with Turbo. `% Processor Time` is busy share. Pick one and label it; our number will differ from Task Manager's otherwise.
  - *Large machines:* on more than 64 logical CPUs, `GetSystemTimes` sees only one processor group. Use the `Processor Information` counter set (`group,cpu` instances).
- **Gaps stay gaps.** Netdata leaves a missed slot empty and the chart shows a gap. A backend ring buffer lets a client ask for "everything since t" after a stall.

### 4.2 Charts

- **Gaps:** `null` breaks the line in uPlot (with `spanGaps: false`), Chart.js and Observable Plot (if not stacked). Never emit 0 for "no sample".
- **Streaming:** use canvas. uPlot (Canvas2D, about 50 KB) streams 3,600 points at 60 fps for about 10% of a core, by keeping one chart and calling `setData()`.
- **Labels and sync:** uPlot's axis label table switches between seconds, minutes and hours by visible span, and `cursor.sync` gives one crosshair across several charts.
- **Long windows:** decimate with min/max, which keeps CPU spikes; LTTB keeps the overall shape.
- **High DPI:** scale the canvas by `devicePixelRatio`, and re-check it when the window moves between monitors.

### 4.3 Information design

| Pattern | Seen in | Why |
|---|---|---|
| Header tiles: current value, min/max, sparkline | Grafana stat panels, Stats, iStat Menus | the answer at a glance, the chart for context |
| CPU split by mode (user / system; iowait, steal on Linux) | Grafana Node Exporter Full, htop | "busy doing what" |
| Per-core heatmap (cores × time), one sequential hue | btop, bottom (as bars) | 32+ cores don't fit as lines |
| Memory as composition + a pressure signal | macOS Activity Monitor (pressure graph), Linux PSI | "used" alone misleads; pressure is what matters |
| Network and disk as mirrored up/down on one axis | Grafana "negative Y" transform | in and out together, one scale |
| Process list grouped by app, with totals | Windows Task Manager, btop tree | many processes, one owner |
| Synced crosshair; 1m / 5m / 15m / 1h selector; pause | Grafana, Netdata | compare charts at one moment |
| Colour never alone; Okabe-Ito categories; viridis for heatmaps | WCAG 1.4.1 | colour-blind readers |

### 4.4 Attribution to the app's own work

The strongest idea in the research is one AgentMux can already support. Windows Job Objects (with nested jobs since Windows 8) and Linux cgroup v2 account CPU time for a whole process tree, including children that already exited, such as a compiler that ran for 3 s between samples. AgentMux already:
- puts every pane's processes in a Job Object (`process_tracker/windows.rs`), reading `JOBOBJECT_BASIC_ACCOUNTING_INFORMATION` (`:219`);
- gives each agent a cgroup v2 on Linux (`process_tracker/cgroup_linux.rs`).

So "how much of the machine are my agents and their builds using, versus everything else" is mostly a matter of sampling data we already hold. No other monitor can show it. For memory, use proportional measures (PSS on Linux, `phys_footprint` on macOS) rather than summed RSS, which double-counts shared pages.

## 5. Proposed design

### 5.1 Pipeline

```
sampler thread (own OS thread, ABOVE_NORMAL, monotonic clock)
  └─ fast sources every 1 s: CPU per core and by mode, memory, net, disk, per-agent job/cgroup CPU
  └─ slow sources every 5–30 s: per-process top list, temperatures, GPU (where available)
  → ring buffer in srv: 15 min at 1 s, seq-numbered; a missed interval is stored as an explicit gap
  → transport: batch of samples since the client's last seq (not latest-only), still on the background lane
  → client after any stall: "since seq N" fills the hole; a true gap stays a gap
```

- **Coalescing:** the background lane may keep coalescing, but it coalesces into a batch instead of discarding. Terminal input priority (the 06-16 spec) is unchanged: sysinfo still yields, it just doesn't lose data when it does.
- **The Windows diagnostics** (commit and handle attribution) move out of the sysinfo loop into their own slow task. They aren't charted, and they cost the most per tick.

### 5.2 Chart engine

- **Engine:** uPlot, one instance per chart, updated with `setData`. The alternative is to keep Observable Plot and fix it (unstacked area, sentinels on both sides of a gap, a time scale); that's cheaper but keeps the per-redraw SVG rebuild.
- **Gaps:** `null` for a missing sample, drawn as a break, with no hold-last-value. A missing stretch is optionally shaded faintly and labelled on hover ("no samples 08:52:31 to 08:52:58").
- **Axes:** the x axis is a real time axis, labelled by visible span (`:30s` steps for a short window, `HH:mm` for longer ones). The y axis always shows tick values, with the unit in the tile header (%, GB, MB/s).

### 5.3 Layout (small multiples, one shared time axis)

1. **Header tiles:** CPU, Memory, Network, Disk, and *Agents*. Each shows its current value, the min and max in the window, and a sparkline. Clicking a tile focuses that chart.
2. **CPU:**
   - an area split into *AgentMux agents* and *rest of the system*, both from job/cgroup accounting;
   - a per-core heatmap below, replacing "All CPU" lines and working for any core count.
3. **Memory:** composition (used, cache or standby, commit on Windows) plus a pressure strip (PSI on Linux, memory pressure on macOS, commit against the limit on Windows).
4. **Network and disk:** received or read above the axis, sent or written below it, on one shared axis with automatic units.
5. **Top consumers:** grouped by agent or pane first, then other processes, each with a CPU sparkline. Clicking an agent focuses its pane.
6. **Annotations on the time axis:** an agent turn started, a build started, AgentMux restarted, a sampling gap.
7. **Controls in one row:** 1m / 5m / 15m window, pause, and a synced crosshair whose tooltip lists every chart's value at that moment.

**Colour and accessibility:**
- One sequential hue for the heatmap; a fixed categorical order for agents versus the rest, validated for colour blindness against the dark surface.
- The status colours (warning, critical) stay reserved for thresholds, with an icon and a label.
- Every series has a legend or a direct label.
- A table view of the current window, for screen readers and for copying.

### 5.4 What it should say plainly

- Which CPU measure it shows ("% busy time, all cores"). A tooltip explains why Task Manager's Utility number can differ.
- When it has no data, and why ("AgentMux was busy; samples resumed at 08:53:02"). A gap is never drawn as a value.

### 5.5 One data layer for Sysinfo, Tower and the pane headers, run on demand

Sysinfo is not the only thing that samples this machine. Today two pipelines measure the same processes in different ways:

| | Sysinfo, status bar, pane header stats (`blockstats`) | Tower |
|---|---|---|
| Process scan | `sysinfo` crate, about every second: all processes, then a full read of up to 64 PIDs per pane (`sysinfo.rs:1203–1240`); every 30 s on Windows, again for the memory-commit log | `agentmux_procstats::snapshot` every 2 s while Tower is open (`tower_sampler.rs:39`, `:407`) |
| CPU rate | the `sysinfo` crate's | Tower's own `RateMeter` |
| A pane's processes | the shell's descendants by parent PID, at most 64 (`process_tree::collect_descendants`) | the pane's Job Object or cgroup first, the tree only as a fallback |
| Machine CPU / memory | per-core PDH mean / used memory | sum of per-process rates / sum of private bytes (`tower_sampler.rs:666`) |
| History chart | Observable Plot | its own SVG with crosshair (`tower-history.tsx`) |

What this costs:
1. **The numbers disagree.** A pane's header CPU and its Tower row are measured by different libraries over different sets of processes. A process that left the parent tree (`setsid`, reparenting) is counted by Tower's tracker and missed by the header. "Machine CPU" means two different things in the two panes.
2. **The work is paid twice.** With Tower open, the machine is scanned about 1.5 times a second by two libraries, plus once every 30 s, and that happens exactly when the machine is busy.
3. **Fixes don't carry over.** Tower's grouping is the right one (it is what §4.4 builds on), and Sysinfo would have to rebuild it.

The panes themselves should stay separate. They answer different questions: "how is the machine doing over time" versus "which task is using it right now". What they share is the data underneath.

#### Shared layer

```
scheduler thread (monotonic deadlines, ABOVE_NORMAL)
  ├─ sources: each knows its cost, minimum interval and what it needs
  │    machine counters ─┐
  │    process snapshot ─┼─► task grouping (tracker first) ─► per-task CPU/memory
  │    volumes, labels, diagnostics, sensors (slow / on demand)
  ├─ leases: who wants what, how often, until when
  └─ ring buffers + "since seq N" reads ─► Sysinfo, Tower, pane headers, status bar, remote frames
```

- **One process snapshot per tick**, shared by Tower, the pane headers and the top-consumers list.
- **One grouping rule** (tracker, then tree), **one rate calculation** and **one memory definition**, so every view shows the same number for the same pane.
- **One chart component** for Sysinfo and Tower, not a third.
- **Library:** standardise on `agentmux_procstats`, which Tower and the remote helper already use, so local and remote measure the same way. First check it covers what the `sysinfo` crate gives Sysinfo today (per-core CPU, disk and network throughput, volumes); add what is missing to `procstats` rather than keep two libraries.

#### The scheduler: run only what someone is looking at

Each consumer holds a **lease** naming the sources it wants, how often, and an expiry it renews while it is still looking.

| Consumer | Wants | Interval | Lease comes from |
|---|---|---|---|
| Status bar | machine totals: CPU, memory, commit, disk, net (`SystemStats.tsx`) | 1 s | always on (the baseline) |
| Cores popover, or a per-core chart | per-core CPU | 1 s | while open |
| Volumes popover / watch volume | free space per volume | 30 s | while open; watch volume always, slow |
| Sysinfo pane | its charts' sources, plus history | 1 s | while the pane exists |
| Pane header stats | that pane's task | 2 s | per pane while its header is visible (`useBlockStats`) |
| Tower | process snapshot, grouping, labels for new processes | 2 s | while open |
| Remote frame / paired viewer | what that view asks for | per request | per request |
| Windows commit diagnostics | all-process memory, handle counts | 30 s, faster when commit is low | its own trigger, never on the fast path |

Leases can come from two places:
- **Implicitly, from broker subscriptions.** `mps.rs` already tracks who subscribes to which event and scope, so subscribing to `sysinfo` or `blockstats:<block>` is the lease.
- **Explicitly, from a small RPC** (`sampler.want` / `sampler.release`) for on-demand readers such as Tower and remote frames.

A lease that is not renewed expires, so a crashed window doesn't keep sources running.

**Rules:**
- **Interval:** each source runs at the shortest interval among its live leases, never below its own minimum (for example the `sysinfo` crate's `MINIMUM_CPU_UPDATE_INTERVAL`). No lease, no run.
- **Ticks:** sources due within the same ~100 ms share a tick, so one process snapshot serves all of them. Dependencies run first (snapshot, then grouping, then per-task rates).
- **No catch-up:** a late tick doesn't burst-run what it missed; it records a gap.
- **Budget:** every source has a time budget. One that keeps overrunning is stretched (2 s, then 4 s), and the stretch is visible, so the cheap machine totals keep their cadence when the process scan gets slow.
- **History:** sources with history (machine totals at least) keep their ring buffer even with no viewer, because Sysinfo shows the last minutes as soon as it opens. On-demand sources keep history only from when someone asked.
- **Visible:** `sampler.status` reports which sources are running, why (which leases), their cadence and their last run time. A debug view and tests both read it.
- **Pure core:** the scheduling decision is a function of (leases, sources, clock), tested with a fake clock, the same way `wan_presence/machine.rs` is.

**Payoff:**
- **Idle cost:** with only the status bar showing, it is one cheap read per second; no process scans.
- **Load:** opening Tower doesn't add a second pipeline; it adds a lease.
- **New sources:** GPU or temperatures are one more source with a lease, sampled only while a chart shows them.

## 6. Plan

| Phase | Scope | Fixes |
|---|---|---|
| 0. Hotfix (**built**) | unstacked area (`y1: 0`), NaN markers on both gap edges, no hold-last-value (a gap up to the threshold is joined by the line between two real samples), x ticks on round second or minute boundaries sized to the window; not done: the y axis, whose missing labels in the large view were not reproduced | the ramp and the duplicate labels, without the redesign |
| 1. Transport (**built, as a pane-side backfill**) | when a live sample arrives after a gap, the pane reads srv's history ring (`readMuxEventHistory`) and merges the samples it missed, without clearing the chart or pausing live events; one fetch at a time, at least 5 s apart. The live lane is unchanged: still latest-only, so the status bar and popovers stay cheap and terminal input keeps its priority. Not built: batching on the lane, a `since seq N` read, and gap markers from the sampler | the 10 to 30 s holes |
| 2. Data layer and scheduler (**begun**: `backend/sampler/schedule.rs`, a pure core with leases, per-source minimum intervals, budgets that stretch a slow source, and no catch-up bursts, tested on a fake clock; the sysinfo loop now runs on it, with the machine's counters as the baseline and the panes' processes scanned at the telemetry rate only while something subscribes to `blockstats`, otherwise every 30 s to evict dead panes. The pane stats now come from Tower's sample (`sampler/panes.rs`): one process snapshot, shared with an open Tower pane, and Tower's grouping (Job Object or cgroup, else the shell's tree), so the badge and Tower count the same processes; the baseline machine source is never stretched, so it keeps its cadence under load. Not yet: `procstats` coverage, `sampler.want` / `sampler.status` RPCs, Tower on it) | §5.5: one scheduler thread with sources, leases and budgets; one process snapshot, grouping and rate calculation; `procstats` extended to cover Sysinfo's metrics; diagnostics as a slow source; per-core metadata for any core count | duplicate scans, numbers that disagree between panes, skipped ticks, 32+ cores |
| 3. Move the consumers | the status bar, popovers, pane headers (`blockstats`) and Tower read from the data layer; the old sysinfo loop and Tower's own sampling are removed. `GET /agentmux/viewer/procs` stays byte-for-byte as it is: the phone app reads it | one source of truth |
| 4. New pane (**begun**: the charts are uPlot on a canvas, built once and updated with `setData`; a gap is `null` and draws as a break; the pane's charts share one crosshair; they follow the 1 Hz samples) | uPlot charts (one component, also for Tower's history), tiles, heatmap, mirrored I/O, crosshair, window selector, table view | the redesign in §5.3 |
| 5. Attribution | per-agent CPU and memory from the shared grouping; top-consumers list; agent and build annotations | "what are my agents costing me" |

Phase 0 is worth shipping on its own: the pane stops showing readings that never happened. Phases 1 to 3 make every view truthful under load and agree with each other. Phases 4 and 5 are the redesign.

**Tests to add:**
- rendered-output tests: a gap draws a break, not a ramp; tick labels differ across a 2-minute window;
- a transport test: 30 queued samples behind a busy priority lane all reach the client;
- a sampler test: a stall becomes a gap record, not a skipped interval silently merged.

## 7. Open questions

1. **New dependency:** is uPlot acceptable? The cheaper path keeps Observable Plot with the phase 0 fixes; §5.2 compares them.
2. **CPU measure:** busy time (what we show now) or Task Manager's Utility on Windows? Showing both is possible but busier.
3. **Retention:** is 15 minutes at 1 s enough in srv's ring, or should a coarse 1-hour series (5 s buckets, min/max) sit behind it?
4. **GPU and temperatures:** in scope? There's no single cross-vendor API on Windows; this would be per-vendor work.
5. **Remote hosts:** the pane can show a remote host's sysinfo over a connection. Should phases 1 to 3 cover remote helpers in the same pass, or follow?
6. **Baseline:** which sources always run, keeping history with no viewer? The proposal is machine totals only (what the status bar shows). Per-core history would then start when someone opens a per-core view.
7. **Machine CPU definition:** once there is one data layer, "machine CPU" needs one meaning: counter-based busy time (today's Sysinfo) or the sum of process rates (today's Tower). The first is cheaper and doesn't need a process scan. The second adds up exactly to the per-task rows.

## 8. Sources

**Sampling**
- bottom collection thread: https://raw.githubusercontent.com/ClementTsang/bottom/main/src/lib.rs
- bottom intervals: https://manpages.debian.org/testing/btm/btm.1.en.html
- btop: https://github.com/aristocratos/btop
- htop CPU accounting: https://github.com/htop-dev/htop/blob/main/linux/LinuxMachine.c
- Netdata scheduling and gaps:
  - https://github.com/netdata/netdata/issues/12254
  - https://github.com/netdata/netdata/blob/master/src/daemon/README.md
  - https://learn.netdata.cloud/docs/developer-and-contributor-corner/rest-api/queries
- Rust `Instant`: https://doc.rust-lang.org/std/time/struct.Instant.html
- tokio missed ticks: https://docs.rs/tokio/latest/tokio/time/enum.MissedTickBehavior.html
- `sysinfo` crate:
  - https://docs.rs/sysinfo/latest/sysinfo/struct.System.html
  - https://docs.rs/sysinfo/latest/sysinfo/constant.MINIMUM_CPU_UPDATE_INTERVAL.html
- macOS counter wrap: https://github.com/exelban/stats/issues/3014
- Apple on the Mach CPU APIs: https://developer.apple.com/forums/thread/748948

**Windows**
- Thread priority: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setthreadpriority
- `GetSystemTimes`: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getsystemtimes
- Processor groups: https://devblogs.microsoft.com/oldnewthing/20200824-00/?p=104116
- Utility vs Time:
  - https://learn.microsoft.com/en-us/troubleshoot/windows-client/performance/cpu-usage-exceeds-100
  - https://aaron-margosis.medium.com/task-managers-cpu-numbers-are-all-but-meaningless-2d165b421e43
- PDH: https://learn.microsoft.com/en-us/windows/win32/perfctrs/collecting-performance-data
- Job Objects:
  - https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects
  - https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_accounting_information

**Linux**
- cgroup v2: https://docs.kernel.org/admin-guide/cgroup-v2.html
- PSI: https://docs.kernel.org/accounting/psi.html
- PSS (`smaps_rollup`): https://docs.kernel.org/filesystems/proc.html

**macOS**
- Memory in Activity Monitor: https://support.apple.com/guide/activity-monitor/view-memory-usage-actmntr1004/mac
- `phys_footprint`: https://www.bazhenov.me/posts/activity-monitor-anatomy/

**Charts**
- uPlot:
  - https://github.com/leeoniya/uplot
  - https://github.com/leeoniya/uPlot/blob/master/docs/README.md
  - synced cursor demo: https://leeoniya.github.io/uPlot/demos/sync-cursor.html
- Chart.js gaps: https://www.chartjs.org/docs/latest/charts/line.html
- Chart.js decimation: https://www.chartjs.org/docs/latest/configuration/decimation.html
- Observable Plot line gaps: https://github.com/observablehq/plot/blob/main/docs/marks/line.md
- LTTB: https://github.com/sveinn-steinarsson/flot-downsample
- MinMaxLTTB: https://ar5iv.labs.arxiv.org/html/2307.05389
- `devicePixelRatio`: https://developer.mozilla.org/en-US/docs/Web/API/Window/devicePixelRatio

**Design**
- Grafana Node Exporter Full: https://grafana.com/grafana/dashboards/1860
- Grafana time series: https://grafana.com/docs/grafana-cloud/learn-and-build/visualizations/panels-visualizations/visualizations/time-series/
- Beszel: https://github.com/henrygd/beszel
- Stats: https://github.com/exelban/stats
- WCAG 1.4.1: https://www.w3.org/WAI/WCAG22/Understanding/use-of-color.html
- Colour palettes: https://thenode.biologists.com/data-visualization-with-flying-colors/research/
