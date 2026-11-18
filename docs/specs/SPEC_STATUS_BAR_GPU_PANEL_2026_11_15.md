# Spec: the status bar's "GFX HW" item opens a GPU panel (use and memory, total and per process)

**Date:** 2026-11-15
**Status:** active. Approved; phase 1 (Windows) is built in PR #PR_NUMBER, phases 2 and 3 (Linux, macOS) are not. Decisions in section 9.
**Task:** #47726 (build: #47727). Written against `origin/main` 0b31efac3.
**Neighbours:** Masty's compact summary (starpower #44) replaces only `agentmux:agent-counts` and `agentmux:vault-requests`; GFX HW stays its own item (`gpu`, `GpuStatus.tsx`, `DROP_AT` gpu: 2). No collision. The panel builds on `AnchoredPopover` and the `popover-frame` readout mixins, as `CpuCoresPopover` and `DiskVolumesPopover` do.

## 1. Why

A rough-feeling UI with every meter cool raises the question "is it the GPU, and is it us?", and the status bar cannot answer it:
it shows whether graphics are hardware accelerated, not how busy the GPU is or who uses it. The panel makes that a click. It also has
room for the one meter that answers the other common cause, the UI thread (section 7).

## 2. What exists

- `GpuStatus.tsx` shows GFX HW / SW / off from a WebGL probe (`util/gpuutil.ts`). No utilisation.
- `SystemStatsParts.tsx` already has a `GpuReadout` ("GPU n%") fed by a `gpu` key of the `sysinfo` event that **no backend sets**; the slot is wired and empty.
- `sysinfo.rs` `run_sysinfo_loop` emits the `sysinfo` event every 1 s (0.2 to 2 s by setting); `tower/sampler.rs` samples processes every 2 s while Tower is visible, labels AgentMux's own (kind, role) and has no GPU column.
- No PDH, DXGI, NVML or IOKit use anywhere; `windows-sys` has no performance-counter feature on. The memory lab only has the GPU process's CPU seconds.

## 3. Data, per platform

| | Adapter use % | Adapter memory | Per process |
|---|---|---|---|
| **Windows** (all vendors) | PDH `\GPU Engine(*)\Utilization Percentage`, summed per adapter over the busiest engine type (3D, compute, copy, video) | PDH `\GPU Adapter Memory(*)\Dedicated Usage` and `Shared Usage`; limit from DXGI `DedicatedVideoMemory` | PDH instance names carry `pid_<n>_luid_..._eng_<k>_engtype_<t>`: utilisation per pid by summing its engines; `\GPU Process Memory(*)\Dedicated Usage` per pid. DXGI `QueryVideoMemoryInfo` reports only the calling process, so it is no help for others |
| **macOS** | IOKit `IOAccelerator` `PerformanceStatistics` ("Device Utilization %") without root | "In use system memory" (unified memory on Apple silicon: dedicated vs shared is not a split there) | **Not available** without private APIs or root. Show the adapter figure and list AgentMux's processes without a GPU column; say so in the panel |
| **Linux** | AMD `gpu_busy_percent` (sysfs); Intel: no cheap figure without a privileged perf counter (show memory only); NVIDIA: NVML (`libnvidia-ml` loaded at run time) `nvmlDeviceGetUtilizationRates` | AMD `mem_info_vram_used` / `_total`; NVIDIA NVML memory; Intel shares system memory | `/proc/<pid>/fdinfo` `drm-engine-*` and `drm-memory-*` (kernel 5.14 and later, amdgpu, i915, xe); NVML process lists on NVIDIA. Per-process needs the process to be ours or readable |

Several adapters: one row per adapter, with the integrated and the discrete one named; per-process figures are for the adapter the counter names. No GPU, or software rendering (GFX SW): the panel opens and says "software rendering: no GPU in use" with the renderer string the item already has, and the adapter table is empty rather than missing. Nothing is installed, and no dependency is added where the OS has the counters: `windows` crate features for PDH and DXGI on Windows, libloading for NVML (a missing library is "NVIDIA tools not found", not an error).

## 4. Cost, and who samples

- PDH wildcard counters enumerate hundreds of instances: the call is the expensive part, and its cost is not known here. **It is measured before the build merges**: one sample's time and the page's frames while sampling, on a Windows CI runner and on charlie for Linux (not on narko's screen).
- **Sampling is off unless something needs it.** The compact number beside GFX HW is adapter-level (cheap on Windows: two counter reads, no per-process enumeration) and is sampled by the existing `sysinfo` loop every 5th tick (5 s). Per-process data is sampled only while the panel is open, every 2 s, on srv's own thread (never the page's), by the Tower sampler's tick when Tower is open too (**one sampler for each number, not two**).
- The panel reads its numbers from a new `GpuSample` RPC; the compact number rides the existing `sysinfo` event as the `gpu`, `gpu:mem` keys the readout already expects.
- A sample that takes longer than a budget (proposal: 50 ms) widens the interval and says so in the panel's footer; it never runs back to back.

## 5. Which processes, and how they are named

AgentMux's own processes reuse Tower's labels (kind and role: the GPU process, each window's renderer, each browser pane, srv) so the same process has one name in both views. Other processes are named by **image name only**: no window title, no path, no command line. The biggest other users are listed (top 5 by GPU use, plus "others"), so the answer "who else is using the GPU" is a name, not a total.

## 6. The panel

- Opens on a click of "GFX HW" in the status bar, inline, no modal, like the CPU cores panel. Escape and outside click close it.
- **Top:** the adapter(s): name, use %, memory used of total (dedicated, then shared), and the renderer string.
- **Rows:** process, kind, use %, memory; sorted by use, then memory; AgentMux's rows first inside each sort tie. A click on an AgentMux row opens Tower at that process.
- **Compact form:** a "GPU 35%" meter in the system strip beside CPU and Mem (the `GpuReadout` slot the strip already had), not a suffix on "GFX HW"; its label turns the warning colour at 90% use or 90% memory. A click on the meter or on "GFX HW" opens the same panel. When no GPU is readable the meter is not shown (never 0% or an error). It is dropped at the same width tier as the "GFX HW" item.
- **Narrow windows and zoom:** the panel is width-aware like the CPU one (rows collapse to name and one number below the narrow breakpoint); pane zoom follows the shell's zoom as `AnchoredPopover` already handles. Light and dark themes.

## 7. The "UI thread busy" figure (approved)

The meter whose absence hid a recent UI stall. The page's share of each second spent in tasks of 16 ms or more, from the Long Tasks observer the app already runs (`perf/observers.ts`), shown beside CPU as "UI 12%" and in the panel's footer; it turns the warning colour above 50%. Cost: summing durations the observer already delivers, nothing new in srv. **Decision (operator, relayed by AgentY 2026-11-18): build it with the panel.** Built as "UI n%" beside CPU and in the panel's footer. What the browser reports as a long task is a task of 50 ms or more (not 16 ms), so the figure is the share of the second spent in tasks of 50 ms or more, and the tooltip says so. It covers the window it is shown in, not agents, srv or other windows, and the tooltip says that too. A window in the background shows no figure (its timers and tasks are throttled, so a reading would be a false 0%), and the first second after it returns is not counted.

## 8. Tests and checks

- Rust: parsers for each platform's counter names and sysfs / fdinfo text from recorded fixtures (a Windows instance list, an amdgpu `fdinfo`, an NVML reply), the "no GPU" and "several adapters" cases, the budget widening.
- Frontend: panel rows and sort, the three states of the item, narrow and zoom, with vitest.
- Screenshots at three widths, light and dark; macOS look check by Clare, Linux by Maricon (the visual-change rule); a machine with an integrated GPU only: Clare and Maricon say which of their machines is one.
- The cost measurement in section 4, in the PR.

## 9. Phases and decisions

1. **Phase 1 (Windows):** adapter figures, compact number, panel with per-process rows; PDH. Recommendation: yes, the operator's machine and where the question came from.
2. **Phase 2 (Linux, NVIDIA and AMD):** NVML, sysfs, fdinfo. Recommendation: yes after phase 1; Intel shows memory only.
3. **Phase 3 (macOS):** adapter figures only. Recommendation: yes, with the panel saying per-process figures are not available there.

Decisions (operator, via AgentY): (a) phases Windows, then Linux, then macOS; (b) the UI-thread figure: yes (section 7); (c) the 90% warning threshold: yes; (d) the compact number: on by default.

## 10. Built in phase 1

- One sampler: `backend::gpu::GpuSampler` holds the one open set of PDH counters. The telemetry thread reads it every 5 s for the meter; the open panel reads it every 2 s through `gpu.sample`. A read newer than the asker's window is reused, so the two never read back to back. The panel's process names come from Tower's own sample (`tower.sample` with the host list), the one place that names a process.
- Reading the counters enumerates every process's engines, so one read is the cost, not the number of counters. A read over 50 ms doubles the interval to the next one (up to 60 s for the meter, 10 s for the panel) and the panel's footer says so; a read under 25 ms halves it back. The cost test (`gpu::cost_tests`, ignored by default) prints a run's median and maximum; it runs in CI and on a host of its own, never on a person's screen.
- The adapter's use is its busiest engine's (as Task Manager's is); a process's is its engines added up, within 100%. Memory share for the warning is dedicated memory for a card with its own, shared memory for an integrated GPU (dedicated under 1 GiB).
- A click on an AgentMux row opens Tower's Processes view filtered to AgentMux's own processes; Tower has no filter for one process id yet.
