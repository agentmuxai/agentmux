# Runbook: what in AgentMux is using the CPU (Windows)

**Status:** living. Written 2026-10-21 from [`RCA_IDLE_CPU_FLOOR_PREWARMED_POOL_WINDOWS_2026_10_21.md`](../analysis/RCA_IDLE_CPU_FLOOR_PREWARMED_POOL_WINDOWS_2026_10_21.md) §7. Add to it when a step here costs you time.

For a question like "why is there always 30% CPU in use?". The steps are in the order that saves the most time. The slow part is usually step 4: linking a busy renderer to the window or page it draws.

## 1. Rule out a fix that's already merged (2 minutes)

The installed app is often a release or more behind `main`.

```bash
echo "$AGENTMUX_VERSION"                       # the version the user is running
git fetch -q origin
git log --oneline "v$AGENTMUX_VERSION"..origin/main -i -E --grep='cpu|idle|perf|animation|pool|spin'
git log --oneline "v$AGENTMUX_VERSION"..origin/main -- <files you suspect>
```

The 2026-10-21 RCA spent about 25 minutes measuring before finding the fix (#5145) this way.

## 2. Which processes (1 minute)

Task Manager shows % of all cores. The numbers below are % of one core, so divide by the core count to compare.

```powershell
# 20 s CPU delta per process, top 15
$a = Get-Process | Select-Object Id, ProcessName, CPU; Start-Sleep 20
Get-Process | ForEach-Object { $o = $a | Where-Object Id -eq $_.Id
  if ($o -and $_.CPU) { [pscustomobject]@{ Id=$_.Id; Name=$_.ProcessName; PctOfCore=[math]::Round(($_.CPU-$o.CPU)/20*100,1) } } } |
  Sort-Object PctOfCore -Descending | Select-Object -First 15
```

- `rustc`, `cargo`, `node` or `vite` belong to someone's build. `bash ~/.agentmux/shared/locks/build-slot.sh status` says whose.
- An AgentMux process's role is in its command line: `(Get-CimInstance Win32_Process -Filter "ProcessId=<pid>").CommandLine`, then `--type=renderer`, `gpu-process` or `utility`. The host (browser process) has no `--type`.

Per renderer and GPU process of one AgentMux host:

```powershell
# measure.ps1 -HostPid <host pid> [-Secs 20]
param([int]$HostPid, [int]$Secs = 20)
$kids = Get-CimInstance Win32_Process -Filter "ParentProcessId=$HostPid" | Where-Object { $_.CommandLine -match '--type=(renderer|gpu-process)' }
$a = @{}; foreach ($k in $kids) { $a[$k.ProcessId] = (Get-Process -Id $k.ProcessId).CPU }
Start-Sleep -Seconds $Secs
foreach ($k in $kids) {
    $c = (Get-Process -Id $k.ProcessId).CPU
    $type = [regex]::Match($k.CommandLine, '--type=([a-z-]+)').Groups[1].Value
    $cid = [regex]::Match($k.CommandLine, 'renderer-client-id=(\d+)').Groups[1].Value
    "{0,6} {1,-12} client={2,-3} {3,6:N1}% of one core" -f $k.ProcessId, $type, $cid, (($c - $a[$k.ProcessId]) / $Secs * 100)
}
```

## 3. What kind of work: which thread, how often it wakes (2 minutes)

```powershell
# Per-thread CPU and context switches for every AgentMux thread
$c = Get-Counter -Counter '\Thread(agentmux*)\ID Thread','\Thread(agentmux*)\ID Process','\Thread(agentmux*)\% Processor Time','\Thread(agentmux*)\Context Switches/sec' -SampleInterval 4 -MaxSamples 2 -ErrorAction SilentlyContinue
$by = @{}; foreach ($x in $c[-1].CounterSamples) { $i = $x.Path.LastIndexOf('\'); $k = $x.Path.Substring(0, $i)
  if (-not $by.ContainsKey($k)) { $by[$k] = @{} }; $by[$k][$x.Path.Substring($i + 1)] = $x.CookedValue }
$by.Values | Where-Object { $_['% processor time'] -gt 5 } | Sort-Object { $_['% processor time'] } -Descending | Select-Object -First 8 |
  ForEach-Object { "pid {0,6} tid {1,6}: {2,4:N0}% of core, {3,6:N0} switches/s" -f $_['id process'], $_['id thread'], $_['% processor time'], $_['context switches/sec'] }
```

- Instance names repeat across processes, so the script keys on the counter path. `-ErrorAction SilentlyContinue` is needed: the counter set always reports some invalid samples.
- **A few hundred switches a second at steady CPU** means a frame loop: an animation or a repaint at 60 Hz. **Very few** means a busy spin. **Thousands** means a polling or yield loop.
- **Is it the JavaScript thread?** A renderer's main thread starts at the exe's entry point; Chromium's worker threads (compositor, IO, thread pool) start in `libcef.dll`. Read the start address with `NtQueryInformationThread` (class 9, `ThreadQuerySetWin32StartAddress`) and match it to `(Get-Process -Id <pid>).Modules`. A busy worker thread with an idle main thread points to a compositor animation, not script.
- `GetThreadDescription` returns nothing for a sandboxed renderer's threads, so thread names aren't available from outside.

## 4. Which page a renderer draws

**PIDs, `--renderer-client-id` order and start times don't tell you.** Chromium starts a spare renderer before it is needed, so a window's renderer can start before the host logs creating the window.

The reliable way needs DevTools (CDP), which **release builds don't start** (#3681). The port the host listens on (`IPC HTTP server started on 127.0.0.1:<port>` in its log) is the frontend asset server, not DevTools. To get DevTools, start a separate instance with `AGENTMUX_CDP_PORT` set. Don't restart the user's instance for it: you would lose the state you want to look at.

```powershell
$env:AGENTMUX_CDP_PORT = '9333'    # any free port from 1024 to 65535
Start-Process "<portable folder>\agentmux.exe"     # a portable runs in its own channel
(Invoke-WebRequest -UseBasicParsing http://127.0.0.1:9333/json/list).Content    # one entry per page
```

Pre-warmed pool windows have the titles `AgentMux (pre-warmed window)` and `AgentMux (pre-warmed pane window)` (`markPoolWaiting`, `frontend/app/init/pool.ts`).

Page to process: each page's `Performance.getMetrics` includes `ProcessTime`, the CPU seconds of the process hosting it. Match it to `(Get-Process -Id <pid>).CPU`.

```js
// page-pids.mjs [port]: Node 24 has a global WebSocket, so no packages are needed
const port = process.argv[2] ?? "9333";
const pages = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter((p) => p.type === "page");
for (const p of pages) {
    const ws = new WebSocket(p.webSocketDebuggerUrl);
    await new Promise((r) => (ws.onopen = r));
    const call = (id, method) => new Promise((resolve) => {
        ws.onmessage = (ev) => { const m = JSON.parse(ev.data); if (m.id === id) resolve(m.result); };
        ws.send(JSON.stringify({ id, method }));
    });
    await call(1, "Performance.enable");
    const { metrics } = await call(2, "Performance.getMetrics");
    const m = Object.fromEntries(metrics.map((x) => [x.name, x.value]));
    console.log(`${m.ProcessTime.toFixed(1).padStart(8)}s  ${p.title}`);
}
process.exit(0);   // exiting right after ws.close() trips a libuv assertion on Windows (exit code 9): harmless
```

## 5. What a page is doing

Through the page's `webSocketDebuggerUrl`:

- **Running animations:** `Runtime.evaluate` of `document.getAnimations().filter(a => a.playState === "running").map(a => a.animationName ?? a.constructor.name)`.
- **Visibility as Chromium sees it:** `document.visibilityState`. A hidden pool window can still report `visible`.
- **Main-thread work:** `Performance.getMetrics` twice, some seconds apart. Compare `TaskDuration`, `ScriptDuration`, `LayoutDuration` and `RecalcStyleDuration` (seconds of work per second).
- **Script hot spots:** `Profiler.start`, then `Profiler.stop`, and sum self time per call frame.
- **Prove a cause by toggling it, then measure again with `measure.ps1`:** for example, remove the attribute a CSS fix keys on (`delete document.documentElement.dataset.poolWindow`), and put it back afterwards.

## 6. Logs

- **Host log path:** `~/.agentmux/channels/<channel>/logs/current-host-v<version>.path` contains the real path, under `versions/<version>/logs/`. The channel is `$AGENTMUX_INSTANCE_CHANNEL`. The channel in your Claude identity folder's path can be a different one.
- **The same folder has:**
  - `cef-debug.log`: Chromium's own log, including the frontend's console as `INFO:CONSOLE` lines;
  - the srv log.
- **Pool windows log under the targets `dnd:tearoff:pool` and `pool:pane`:** spawned, ready, promoted and evicted.
- **Repeated lines show a loop quickly:**
  ```bash
  grep -v INFO:CONSOLE cef-debug.log | grep -E ':(ERROR|WARNING)' | sed -E 's/^\[[^]]*\]//' | sort | uniq -c | sort -rn | head
  ```

## 7. Traps

- **A fresh instance may not reproduce it.** State decided at launch can differ: in the 2026-10-21 RCA, a fresh instance throttled the pool pages that the long-running one drew at full cost. Force the condition instead of concluding "not a bug".
- **Don't kill or restart the user's AgentMux to measure.** It restarts every agent in it.
- **Heavy builds on the same host skew every number.** Check `build-slot.sh status` and note who is building in your measurements.
