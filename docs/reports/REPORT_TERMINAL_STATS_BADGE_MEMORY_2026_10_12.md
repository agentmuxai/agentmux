# The terminal's CPU/memory badge: what it measures, why a PowerShell pane's number is large and keeps rising, and a hover tooltip for it

**Status:** analysis, with a proposal (§5). Nothing implemented yet.
**Date:** 2026-10-12 | **Version:** main at `ed7d038a3` (0.59.20). Measured on Windows 11 with PowerShell 7.6.6 and PSReadLine 2.4.5.
**Requested by:** repo owner: "I opened a bunch of terminals on Tab 5 and I kept typing `ll` <enter> in one of them... every time, the memory indicator on the top right grows indefinitely by 2mb.. why is it so large? what is the indicator? we want to add a quick pop tooltip that explains it when hovering over it in the terminal with the mouse."
**Author:** Loap.

---

## 1. Summary

- The indicator is the **pane stats badge** (`BlockStatsBadge`): CPU and memory of the terminal's **shell and every process it started**, summed, refreshed every second.
- Its memory figure is the processes' **working set** on Windows (resident set on Linux and macOS): the RAM the OS holds for them right now, **including pages shared with other processes**.
- The terminals here run **PowerShell 7** (`pwsh`). An idle `pwsh` with this machine's profile holds about **101 MB** of working set, about 37 MB of it private. Most of the rest is the .NET runtime and modules, mapped by every `pwsh` at once. That is why the number is large, and why the badges of several panes overlap rather than add up.
- `ll` is an alias the user's PowerShell profile defines for `Get-ChildItem`. Each run allocates short-lived .NET objects (about 0.9 MB in a script, around 2 MB at the interactive console). The .NET garbage collector frees them, but **keeps the freed memory for reuse instead of returning it to Windows**, so the working set climbs and then **levels off**. In 400 runs it rose from 94 MB to 141 MB in the first 50, then slowed and settled near 155 MB. That is **not a leak** in AgentMux or in the shell, as far as this measures (§4).
- **The badge also goes down.** About half an hour after the `ll` runs, the busy terminal's working set had dropped from 166.7 MB to 90.3 MB with nothing typed: Windows trims an idle process's working set. Its **committed private memory**, what the process has allocated for itself whether in RAM or not, stayed at 92.8 MB. That is the steadier, heap-like number (§5).
- The badge has `pointer-events: none`, so hovering it shows nothing today. A tooltip needs the badge to take the mouse (§5).

## 2. What the badge is

| | |
|---|---|
| Component | `frontend/app/element/blockstats.tsx` (`BlockStatsBadge`), placed by `frontend/app/block/blockframe.tsx` at the pane's top-right corner, over the content |
| Shown on | terminal panes (`term.tsx`: `statsBadgeSetting: "term:showstatsbadge"`); on unless that setting is `false` |
| Data | the `blockstats` event, scoped `block:<id>`, published by `crates/srv/src/backend/sysinfo.rs` on every telemetry tick (`telemetry:interval`, default 1 s) |
| Processes | the pane's shell PID (`pidregistry`) and all its descendants (`process_tree::collect_descendants`, capped at `MAX_PIDS_PER_BLOCK`) |
| CPU | the sum of `sysinfo`'s `cpu_usage()` over those processes: percent of **one** core, so it can pass 100% on several cores. 50%+ is drawn in the warning colour, 90%+ in the error colour |
| Memory | the sum of `sysinfo`'s `memory()`. In `sysinfo` 0.35.2 on Windows that is `WorkingSetSize` (`src/windows/process.rs`); on Linux and macOS the resident set size. Shown as `K`, `M` or `G` |
| Not shown | the event also carries `pids`, the number of live processes in the tree |

It is **not** AgentMux's own memory: the server, the window and its renderers are separate processes and are not in a pane's tree. Tower shows those.

## 3. Why a PowerShell pane's number is large

Measured read-only on this machine, against the 24 `pwsh` terminals under the running AgentMux 0.59.20 server:

| Terminal | Working set | Private |
|---|---|---|
| The one `ll` was typed into | 166.7 MB | 92.7 MB |
| Typical idle sibling (6 of them) | 101.3–101.5 MB | 36.7–37.1 MB |

- **The baseline (~101 MB) is PowerShell itself:** the .NET runtime, the PowerShell engine, PSReadLine and whatever the profile loads. Only about 37 MB of it is private to that one process. The rest is shared DLL pages, which Windows maps once and counts in every process's working set.
- **So the badges overlap.** Each of 24 terminals reports about 101 MB, but they don't use 24 × 101 MB of RAM between them. The badge is a good "what is this pane's shell holding" number, not a share of total RAM.
- A `bash` or `zsh` terminal starts far smaller (a few MB), because those shells aren't .NET programs.

## 4. Why it rises with each `ll`, and why it stops

The user's profile (`Documents/PowerShell/Microsoft.PowerShell_profile.ps1`) defines `ll` as an alias for `Get-ChildItem`. AgentMux starts `pwsh` with the profile loaded (`-NoExit -File wavepwsh.ps1`, `crates/srv/src/backend/shellintegration.rs`), and its own integration script (`shellintegration/pwsh.ps1`) defines no `ll` and does little per prompt (a directory OSC and an agent-id OSC).

Each `ll` builds a file object per entry, formats them into a table, and writes the text. All of it is garbage a moment later. In a separate `pwsh` with the same profile, running `ll <a 40-entry folder> | Out-String | Out-Null` repeatedly (script in §7):

| After | Working set | Private | .NET heap |
|---|---|---|---|
| start | 93.9 MB | 33.0 MB | 8.2 MB |
| 50 `ll` | 141.4 MB | 78.4 MB | 8.6 MB |
| 100 | 147.0 MB | 83.2 MB | 4.3 MB |
| 150 | 151.2 MB | 87.2 MB | 47.9 MB |
| 200 | 153.2 MB | 88.6 MB | 43.7 MB |
| 300 | 154.7 MB | 89.5 MB | 35.2 MB |
| 400 | 155.3 MB | 89.7 MB | 26.6 MB |
| then a full GC | 152.3 MB | 86.6 MB | 4.5 MB |

What it shows:
- **The objects are garbage, not kept:** the managed heap grows about 0.9 MB per `ll` and the collector brings it back down by itself (43.9 MB to 8.8 MB in an earlier run; 4.5 MB after a forced collection here).
- **The memory isn't handed back:** after a collection, .NET keeps the freed segments committed for the next allocations, and Windows keeps those pages in the process's working set. So the badge, which reads the working set, only goes up.
- **It levels off:** about 1 MB per command at first, then slower as the collector settles on its heap size: +6 MB over runs 50–100, +2 MB over 150–200, +0.6 MB over 300–400.

At the interactive console each command does more (PSReadLine, the prompt, writing the table to the terminal), which fits the ~2 MB per `ll` the owner saw and the 166.7 MB of the real terminal. It should level off the same way, a bit higher. **What would show a real leak:** the working set still rising at a steady rate after a few hundred commands. Or, more reliably, the managed heap after `[GC]::GetTotalMemory($true)` growing with every batch. Neither happened here.

**Then Windows trims it.** Read again about half an hour later, with nothing typed in either terminal:

| Terminal | Working set | Private working set | Committed private | Peak working set |
|---|---|---|---|---|
| The busy one | 166.7 → **90.3 MB** | 9.6 MB | 92.7 → **92.8 MB** | 184.2 MB |
| An idle sibling | 101.3 → 79.4 MB | 8.7 MB | 39.5 MB | 102.9 MB |

The working set is "what is in RAM right now", and Windows moves an idle process's pages out of it. Committed private memory is what the process has allocated for itself, wherever it currently is. It barely moved, and it is the better measure of how much a shell has grown. The private working set is small (under 10 MB) for both, because most of their private pages were the ones trimmed.

Not measured: a few hundred `ll` at a real interactive terminal, reading the badge itself.

## 5. Proposal: a hover tooltip on the badge

### 5.1 The numbers it can show

What each costs to add. "procstats" is `crates/procstats`, which Tower already uses for every process (`ProcInfo`). The badge's own sampler (`sysinfo.rs`) reads only `sysinfo`'s working set and CPU today.

| Number | Meaning | Windows | macOS | Linux | Cost |
|---|---|---|---|---|---|
| **Processes** | the shell and everything it started, with names (`pwsh`, `node`, …) | yes | yes | yes | free: the tree is already walked; the count is in the event (`pids`) |
| **CPU now** | share of one core, summed | yes | yes | yes | shown today |
| **CPU time** | CPU used so far by the processes alive now | yes | yes | yes | procstats `cpu_ns` |
| **Memory (badge)** | working set / resident set: in RAM now, shared pages included; trimmed when idle | yes | yes | yes | shown today |
| **Private** | memory only this pane's processes use: private working set (Windows), memory footprint (macOS), anonymous resident + swap (Linux) | yes | yes | yes | procstats `mem_private` |
| **Committed (heap-like)** | private memory the processes have allocated, in RAM or not: their heaps, .NET's included. The steadiest measure of growth | yes | — | — | procstats `mem_commit`, Windows only |
| **Peak** | the most memory the processes have held | yes (`PeakWorkingSetSize`) | partly (lifetime max footprint) | yes (`VmHWM`) | new code in procstats per platform. Or the highest value this AgentMux has seen, as Tower's Peak column does: cheap, but resets on restart and misses short peaks |
| **Running since** | when the shell started | yes | yes | yes | procstats `started_at_ms` |
| .NET managed heap | the "heap" PowerShell itself reports with `[GC]::GetTotalMemory` | — | — | — | **not readable from outside the process**; only `pwsh` can report it about itself |

Recommended for the tooltip: processes (with names, up to three), CPU now, memory (the badge's number), private, committed on Windows, peak (Tower's way: the highest value seen), and running since. CPU time adds little next to CPU now.

### 5.2 Text

Example, the busy terminal above:

> **This pane's processes:** pwsh (1 process), running since 13:20.
> **CPU 0.3%** of one core (can pass 100% when several cores are busy).
> **Memory 90 MB:** in RAM right now, including memory shared with other processes (every PowerShell terminal maps the same runtime), so panes overlap. Windows trims it when the shell is idle.
> **Private 10 MB:** in RAM and used only by this pane.
> **Committed 93 MB:** what these processes have allocated for themselves, in RAM or not. The best measure of how much the shell has grown.
> **Peak 184 MB.**
> Updated every second. Hide it with `term:showstatsbadge`.

On macOS and Linux the committed line is left out, and "Windows trims it" becomes "the system can page it out". The PowerShell example only makes sense for a `pwsh` or `powershell` pane, so it is dropped for other shells.

### 5.3 Implementation (built 2026-10-12)

One procstats snapshot takes about 20 ms here (626 processes, release build), too much to take for every terminal every second. So the tooltip's extra figures are read only while it is open, and the 1 Hz event gains only the peak, which costs nothing.

1. `crates/srv/src/backend/blockstats_detail.rs`: `detail()` walks a snapshot from the pane's shell, as many processes as the badge counts (64), and returns the process count, names (shell first, then the most numerous, without `.exe`), the shell's start time, and summed private and committed memory. `Peaks` keeps each pane's highest badge memory, reset when the pane gets a new shell.
2. `sysinfo.rs`: the `blockstats` event carries `mem:peak` next to `cpu`, `mem` and `pids`.
3. `blockstats.detail { block_id }` (`server/app_api/blockstats.rs`): one snapshot, off the async workers. The window's only: a connection registered as an agent is refused, as for Tower.
4. `blockstats.tsx`: the badge is wrapped in the UI `Tooltip`, placed below it. The tooltip's content is mounted only while it is open; it asks for the detail then, and again every 2 s. If the request fails, it still shows CPU, memory and peak.
5. `blockstats.scss`: the badge takes the pointer (`pointer-events: none` removed), with `opacity: 0.9` on hover.
6. Tests: the tree walk, sums, names, cap and peaks (Rust), the RPC end to end and its refusal of agents, and the tooltip's figures, platform text and fallback (vitest).

**The trade-off:** today the badge lets clicks and text selection through to the terminal under it. With a tooltip, its small area (about 70 × 16 px at the top-right) takes the pointer instead. A long line's last characters under it can't be clicked or selected without scrolling or resizing. Options:
- **(a)** Accept that. The badge is small and semi-transparent. This is my recommendation.
- **(b)** Move the badge into the pane header, where there is no terminal content. It would need room in the header, and the hoisted-header case (`noHeader`) handles it separately.
- **(c)** Keep it click-through and add the explanation to a pane menu instead. Discoverable only by people who already know to look.

## 6. Out of scope, worth knowing

- **Which number the badge itself should show.** It shows the working set, which overlaps between panes and drops when Windows trims an idle shell. The private figure would make the panes add up, and the committed figure would show growth steadily. Both differ per platform (§5.1), so this report leaves the badge as it is and puts them in the tooltip.
- **Reducing PowerShell's footprint** is the user's environment: a lighter profile, or `bash` (Git Bash) as the terminal shell, starts far smaller.

## 7. How to reproduce

From a `pwsh` with the same profile:

```powershell
param([int]$Runs = 400, [string]$Dir = $HOME)
$p = Get-Process -Id $PID
function m([string]$label) {
    $p.Refresh()
    '{0,-22} ws {1,7:N1} MB   priv {2,7:N1} MB   heap {3,6:N1} MB' -f $label,
        ($p.WorkingSet64 / 1MB), ($p.PrivateMemorySize64 / 1MB), ([GC]::GetTotalMemory($false) / 1MB)
}
m 'start'
for ($i = 1; $i -le $Runs; $i++) {
    ll $Dir | Out-String | Out-Null
    if ($i % 50 -eq 0) { m "after $i ll" }
}
[GC]::Collect(); [GC]::WaitForPendingFinalizers(); [GC]::Collect()
m 'after a full GC'
```

In the terminal itself: `(Get-Process -Id $PID).WorkingSet64 / 1MB` is what the badge shows (for a pane with no child processes running), and `[GC]::GetTotalMemory($true) / 1MB` is the live managed heap after a collection.
