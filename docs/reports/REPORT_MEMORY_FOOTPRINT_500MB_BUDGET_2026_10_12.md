# Report: AgentMux memory footprint, and whether it can stay under 500 MB at any pane count

**Date:** 2026-10-12
**Scope:** AgentMux's own processes (CEF host, GPU, utility and renderer processes, srv, launcher). Agent CLIs are covered separately in §7.
**Status:** research and measurement only. No code has changed.
**Update (same day):** Phase 0 has been done, and the root causes are measured in `REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md`. Where this report guesses at the causes (§4) and orders the work (§6), that report wins: fix tool-body unmounting and the `keyArray` leak first, then renderer recycling and hibernating hidden tabs.

## 1. Short answer

- **The 2 GB in Tower is real.** Tower's "AgentMux" row is the private working set of the launcher, every CEF process, and srv. On v0.59.20, after 26 hours with one window and about 10 agent panes, those processes held 2.05 GB in RAM and 3.56 GB of private commit.
- **Almost all of it is one Chromium renderer:** the main window's, at 1.39 GB private working set and 1.89 GB commit. The GPU process adds 0.36 GB and srv 0.12 GB. The agents' own CLIs (another 1.3 GB) are not in that figure; Tower shows them as separate rows.
- **Today, memory grows with every open pane and with uptime.** Comparing two running instances suggests roughly 100–150 MB of renderer memory per agent pane, and nothing caps it: hidden tabs and panes stay mounted and keep receiving data, and the renderer is never recycled.
- **Under 500 MB at any pane count is achievable for AgentMux's own processes, but not with flags.** It needs one architectural rule: **the renderer holds only what is visible, and srv holds everything else.** Then the renderer's cost depends on how many panes are on screen, not how many are open. Add a budget governor and periodic renderer recycling to catch what is left (§6).
- **It isn't achievable if the 500 MB includes the agent CLIs.** Each Claude Code process is 130–200 MB private on its own and AgentMux can't make it smaller. Those need their own policy (§7).
- **Pick the metric first (§2).** Under private working set, which Tower shows, 500 MB is a reasonable goal. Under private commit, the stricter and more honest metric, today's floor is already about 1.1 GB with a single pane, so the commit target has to be set separately.

## 2. Which number a "500 MB" target means

| Metric | What it counts | Use |
|---|---|---|
| Private working set (Task Manager's default "Memory" column, Tower's figure) | Private pages currently in RAM | The user-facing number. It can be gamed by trimming (see below), and it under-reports once Windows compresses or pages memory out. |
| Private commit / Private Bytes (Task Manager "Commit size") | All private committed memory, wherever it is | Chromium's own "private memory footprint" on Windows. It is the honest number, but it over-counts the GPU process by 50 MB or more. |
| macOS `phys_footprint`; Linux PSS / USS | Equivalent "this app's memory" measures | Already read by `crates/procstats`. |

**Recommendation:**
- Set the headline target on Tower's metric (private working set ≤ 500 MB), because that is what users see.
- Track private commit as a second, gating number, so that the target can't be met by trimming.
- **Rule out `EmptyWorkingSet` / `SetProcessWorkingSetSize(-1,-1)` explicitly.** They would make Tower read low while commit stays the same, and pages fault back in on next use. Mozilla removed this after swap storms, and Microsoft describes it as a testing aid.

`docs/plans/PLAN_PERFORMANCE_CLAIMS_VERIFICATION_2026_10_11.md` §4.1 already sets out the same metric policy for published numbers. This report uses it too.

## 3. What was measured

**Machine:** Windows 11 with 61.6 GB of RAM, 18.7 GB free at the time, display scale 1.25.

**Method:** a snapshot of the process tree under each AgentMux launcher, taken with `Win32_PerfRawData_PerfProc_Process.WorkingSetPrivate` (private working set, Tower's metric), `WorkingSetSize` and `PrivatePageCount` (commit).

### 3.1 v0.59.20: 26 h uptime, 1 window, about 10 agent panes

| Group | Processes | Private WS (MB) | Working set (MB) | Commit (MB) |
|---|---:|---:|---:|---:|
| CEF renderer: main window | 1 | 1389 | 1472 | 1818 |
| CEF renderers: pool windows and others | 4 | 139 | 407 | 365 |
| CEF GPU process | 1 | 394 | 477 | 1009 |
| srv (and its crash monitor) | 2 | 124 | 190 | 262 |
| CEF browser process | 1 | 34 | 156 | 69 |
| CEF utility processes | 3 | 12 | 88 | 37 |
| Launcher | 1 | 4 | 29 | 12 |
| **AgentMux total (what Tower shows)** | | **≈ 2,050** | | **≈ 3,570** |
| Agent CLIs (`claude.exe`), shown separately | 10 | 1294 | 2159 | 4165 |
| Per-agent helpers (`agentmux-mcp`, bashwrap, conhost, bash) | ~45 | ~35 | | |

- **The main renderer's working set peaked at 2.09 GB.** The GPU process's commit (1.5 GB by `Get-Process`) is far above its working set, which fits Chromium's note that commit over-counts the GPU process.
- **Two of the CEF switches are relevant to memory:**
  - `--disable-features=CalculateNativeWinOcclusion` means windows hidden behind other windows are never treated as hidden.
  - There is no `--js-flags`, `renderer-process-limit` or `process-per-site` switch.

### 3.2 v0.59.19: 1 agent pane, a similar uptime

| Group | Private WS (MB) | Commit (MB) |
|---|---:|---:|
| CEF renderer (1) | 328 | 507 |
| GPU process | 54 | 334 |
| srv (×2) | 79 | 205 |
| Browser process | 23 | 58 |
| Utility processes | 10 | 35 |
| **AgentMux total** | **≈ 500** | **≈ 1,140** |

**Estimated per-pane cost:** from 1 to about 10 agent panes, the renderer grew by about 1.06 GB and the GPU process by about 0.34 GB. That is roughly 120 MB of renderer and 35 MB of GPU memory per agent pane.

This is an estimate from two instances, not a controlled test. Uptime, transcript lengths and terminal panes all differed. Phase 0 (§6) replaces it with a measured curve.

### 3.3 The evidence for a low floor

v0.59.20's pool-window renderers have the app loaded but no panes, and they sit at 21–71 MB private working set each. That puts the cost of the app shell itself well under 100 MB per renderer: the floor is low, and pane content and churn make up the rest.

## 4. Where the memory goes: the architecture as it stands

All references are to current `main` (v0.59.21).

**Renderers**
- There is one renderer per top-level window, per hidden pool window (`POOL_TARGET_SIZE = 2`, up to 4 kept under Normal pressure) and per browser pane. Terminal and agent panes run in their window's renderer. (`crates/cef/src/app/mod.rs:1088-1097`, `crates/cef/src/commands/window_pool.rs:118-133`, `pane_pool.rs:85`.)
- Each secondary window gets its own in-memory request context, which forces a separate renderer. (`crates/cef/src/commands/mod.rs:57-95`.)

**Nothing is ever unloaded**
- Every tab stays mounted; by default inactive tabs use `visibility:hidden`, so they stay laid out (`frontend/app/workspace/workspace.tsx:196-205`).
- Term, agent, browser, editor and files panes are `keepAlive` (`frontend/app/block/pane-tab-registry.ts:229-231`).
- Agent "dormancy" only defers rendering: hidden agent panes still receive and store their stream (`frontend/app/view/agent/agent-dormancy.tsx`).
- There is no hibernate, suspend or unload, and `window:maxtabcachesize` is a dead setting.

**Agent transcripts are bounded per pane, but the bounds are generous**
- Rendering is virtualized.
- `LIVE_FEED_MAX_FINISHED_BYTES = 1 MB`, `LIVE_FEED_MAX_ROWS = 20,000`, and `FULL_NODE_BYTES_CAP = 32 MB` per pane. Collapsed tool results of 8 KB or more are already unloaded.
- 1 MB of raw transcript turns into much more as parsed markdown, highlighted code, SolidJS reactive state and DOM, so about 100 MB per pane is plausible. (Measured since: see `REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md`. The main cost turned out to be tool bodies that are never unmounted.)

**Terminals**
- xterm.js 6 uses the WebGL renderer for every terminal, visible or not (`frontend/app/view/term/termwrap.ts:828-864`). Scrollback defaults to 2,000 rows, with a maximum of 50,000.
- xterm.js stores 12 bytes per cell: 200 columns × 10k rows is about 24 MB per terminal.
- The WebGL addon's glyph atlas has no cap in 0.19.x. Atlas pages can double up to 16384 px (1 GiB each). One project measured GPU 17 GB and renderer 867 MB from a single flooded terminal, falling to 1.5–2 GB GPU and 431 MB renderer once pages were capped at 4096 px.
- Chromium allows 16 live WebGL contexts per page.

**srv**
- Per-pane buffers are bounded: a 256 KB terminal blockfile, a 1 MiB agent line buffer, and a PTY channel of about 512 KB worst case.
- The MPS broker keeps up to 4,096 events per (event, scope), and each persisted event is stored under its own scope and a global scope. bashwrap `tool_chunk` and `shell_node` output persist 1,024 each.
- srv is 80–125 MB and isn't today's problem, but its per-agent state has to stay bounded once it takes over history from the renderer (§6, Phase 2).

**Memory handling that already exists**
- A commit-pressure heartbeat and pool eviction under Warn or Critical pressure (`crates/cef/src/memory_heartbeat.rs`, `memory_pressure.rs`).
- The launcher relaunches after an out-of-memory exit.
- Job objects with `KILL_ON_JOB_CLOSE`, and an optional BELOW_NORMAL priority for agents.
- None of this caps memory or makes it shrink.

**Why it climbs with uptime:** Chromium's Blink garbage-collected heap (cppgc) doesn't compact, and on Windows it doesn't return freed regions. VS Code's agents window (Chromium 150, Windows) reached 2.5 GB private after 99 h:
- The Blink heap had 611 MiB allocated but only 39 MiB live, i.e. 93% fragmentation.
- Recreating the renderer reclaimed about 520 MiB, and the new renderer was 338 MiB.

AgentMux's main window is the same workload: a long-lived agent UI that creates and destroys a lot of DOM. Expect the same effect. (microsoft/vscode#337444.)

## 5. What the research says works, and what doesn't

### Works: make the renderer's cost depend on what is visible

| Technique | Evidence | Expected effect for AgentMux |
|---|---|---|
| **Pane hibernation:** unmount hidden panes beyond the K most recently used; srv keeps the state and replays it on show | VS Code is proposing exactly this for its agents window (vscode#327437: 38 background xterms = 80 MB, about 16% of the renderer heap). Slack unloads idle workspaces in one renderer and reports up to 50% less memory. Edge sleeping tabs: −32% on average. | Turns roughly 120 MB per open pane into roughly 0 per hidden pane. The largest single win. |
| **Renderer recycling:** reload the window renderer when idle and over budget, restoring state from srv | VS Code: about 520 MiB reclaimed. Microsoft's WebView2 guidance recommends periodic refresh for long-running pages. Discord (Dec 2025) restarts above 4 GB after 30 min idle. | Removes growth from fragmentation, which no amount of app-level tidiness can fix. |
| **WebGL only for visible terminals,** plus `WEBGL_lose_context.loseContext()` on dispose and a 4096 px atlas page cap | Projects with many terminals measured GPU memory falling from 17 GB to 1.5–2 GB in the worst case. `dispose()` alone doesn't release the context until GC. | Bounds GPU memory and per-terminal renderer memory, and avoids the 16-context eviction. |
| **Terminal state in srv:** `alacritty_terminal` (used by Zed) or `libghostty-vt` (pre-1.0) keeps the screen and scrollback in Rust; the frontend xterm keeps the viewport plus a small window | Native terminals don't use fewer bytes per cell (Ghostty about 12.5 B, the same as xterm.js). They are cheap because the data isn't in a garbage-collected heap duplicated per view. Warp's packed storage saved about 20%. | Moves scrollback out of the renderer. Lower priority than hibernation if hidden terminals are unmounted anyway. |
| **Transcript windowing over srv:** the frontend keeps only the visible window plus a margin, parsed markdown is memoized for that window only, and older history is paged from srv | Codex desktop: opening one long conversation cost +265 MB and settled at about 990 MB (openai/codex#38176). Virtualization alone isn't enough when the store still holds parsed history. | Bounds each visible agent pane to tens of MB. |
| **Fewer idle renderers:** pool size 1 instead of 2–4, and share one renderer across same-origin windows instead of an in-memory context per window | Each app-loaded pool renderer is 21–71 MB (§3.3). `process-per-site` shares a renderer for one site; Chromium's own New Tab Page uses it. | Saves 50–150 MB, and keeps multi-window use from multiplying the floor. The trade-offs (window open latency, shared crash domain, why secondary windows need separate contexts) need checking first. |

### Doesn't work, or only trims the edges

- **`--js-flags=--max-old-space-size`:** lowers the ceiling, so you hit OOM sooner, without shrinking live data. CEF 136 users saw no effect.
- **`--lite-mode` / `--jitless`:** the lite-mode savings were folded into default V8 years ago. jitless saves about 1.7% of heap at up to 80% slower JS. Both disable WebAssembly.
- **`--renderer-process-limit`:** unreliable since site isolation (it mostly limits spare renderers), and the CEF maintainer advises against it. AgentMux removed it on 2026-05-09.
- **Disabling site isolation:** about 10–13% in multi-site browsing. AgentMux's content is one origin, so there is almost nothing to save.
- **`--enable-low-end-device-mode`:** undocumented on desktop, and causes rendering artifacts.
- **`--force-gpu-mem-available-mb`:** one app measured about 50 MB less at peak and nothing at rest, then reverted it because of rendering problems on high-DPI displays. Possibly worth a measured experiment, but it isn't a fix.
- **Memory-pressure purge** (`Memory.simulatePressureNotification`): VS Code saw no reclaim in 35 s. Chromium stopped purging on freeze in 2025 because it bought no real memory.
- **Working-set trimming** (`EmptyWorkingSet`): cosmetic; see §2.
- **Keep-alive-but-hidden patterns (React's `<Activity mode="hidden">`, a SolidJS pane kept mounted behind `display:none`) and CSS `content-visibility`:** save CPU, not memory. They keep the state and DOM. (The AgentMux frontend is SolidJS; the React reference was only an outside example.)
- **Moving to WebView2 or Tauri:** on Windows WebView2 is Chromium. Measured Tauri apps were 370–399 MB against Electron's 312–318 MB for the same content, and WebView2 shares processes only between apps that use the same user data folder.
- **Native GPU terminals** (an Alacritty/Ghostty-style renderer beside CEF): a real but very large change. Moving terminal state to srv gets most of the benefit for a fraction of the work.

## 6. A plan that makes the 500 MB target hold at any pane count

### Budget (private working set, AgentMux's processes only)

| Component | Today (v0.59.20) | Target |
|---|---:|---:|
| Browser process | 34 | 35 |
| GPU process | 394 | ≤ 80 |
| Utility processes | 12 | 15 |
| Idle pool-window renderers | 139 (4) | ≤ 50 (1) |
| Window renderer: app shell | (part of 1,389) | ≤ 80 |
| Window renderer: visible panes | (part of 1,389) | ≤ 150 for up to about 6 visible panes |
| Window renderer: hidden panes | (part of 1,389) | ≈ 0 (stubs only) |
| srv | 124 | ≤ 90 |
| **Total** | **≈ 2,050** | **≤ 500** |

**What this means for "any number of panes":**
- The number of open agents and terminals no longer matters. Only the number on screen at once does, and that is bounded by screen area.
- Extra windows each add a renderer floor, about 60–80 MB each with today's per-window context. Making a fourth or fifth window fit the same 500 MB needs the shared-renderer work below.
- For the commit metric, set the target after Phase 0. Today's GPU commit alone is about 1 GB.

### Phase 0: measure (do this first; about 2–3 days)

- **Per-pane curve:** a repeatable test in an isolated `task dev` build that opens N = 1, 5, 10, 20 agent panes and terminals with a fixed transcript and output, and records each process's private working set and commit.
- **Split the renderer** with Chromium memory-infra tracing (V8, `blink_gc`, PartitionAlloc, malloc, `cc` tiles) and a heap snapshot for each pane type. That answers how much is transcript state, how much is DOM, how much is xterm and how much is fragmentation.
- **Measure the xterm WebGL atlas:** page sizes and context count in a terminal-heavy session.
- **Run a 24 h soak** to separate growth from open panes and growth from uptime.
- The harness is the one `PLAN_PERFORMANCE_CLAIMS_VERIFICATION_2026_10_11.md` §6 already proposes over `crates/procstats`; this phase reuses it.

### Phase 1: quick, low-risk changes (about 1 week)

- **Terminals:**
  - WebGL only for visible terminals.
  - Call `loseContext()` on dispose.
  - Cap atlas pages at 4096 px (or take the xterm.js 6.1 fix once it is released).
  - Shrink scrollback for terminals that are hidden.
- **Inactive tabs:** make `content-visibility:hidden` the default for inactive tabs, or measure whether keeping them laid out costs anything.
- **Occlusion:** re-check why `CalculateNativeWinOcclusion` is disabled. If the original reason no longer applies, enabling it lets occluded windows stop producing frames, which cut GPU memory by about 3% in Chrome.
- **Pool:** reduce `POOL_TARGET_SIZE` to 1 and measure the latency of opening a window.

### Phase 2: pane hibernation (the main fix; about 2–4 weeks)

- **Live-pane limit:** keep the K most recently used panes live, with K set by the budget (around 6–8). Any other hidden pane unmounts down to a stub.
  - **Agent panes:** srv already has the transcript. The frontend stops storing the stream for hibernated panes and reloads the visible window when the pane is shown. Live status, such as the footer line and needs-you prompts, still comes from a small status event, not the full stream.
  - **Terminal panes:** srv keeps the screen state. Either restore from the existing cached serialize, or move to an srv-side emulator (`alacritty_terminal`) and replay the viewport plus recent scrollback. The second also removes the idle 5-second full-scrollback serialization (`termwrap.ts:1014-1038`).
  - **Browser panes:** destroy the CEF browser for long-hidden browser panes and keep the URL and profile. This frees a whole renderer process per pane.
- **Transcript windowing:** lower the per-pane in-memory limits once the history can be paged back from srv cheaply (`LIVE_FEED_MAX_ROWS`, `FULL_NODE_BYTES_CAP`).

### Phase 3: renderer recycling (about 1–2 weeks, after Phase 2)

- **When:** the window has been idle for, say, 10 minutes and the renderer's private working set is above its budget (or above 2× its post-restore size). Reload the window's renderer then.
- **How:** Phase 2 makes it cheap: every pane restores from srv, and with hibernation only K panes rehydrate.
- **Check it holds:** run the 24 h soak again; renderer memory should be flat.

### Phase 4: a budget governor (about 1 week)

- One component (srv or the CEF host) reads Tower's numbers every few seconds and applies the budget in order:
  1. Lower K.
  2. Shrink terminal scrollback.
  3. Evict the window pools.
  4. Recycle the renderer when idle.
- This is what makes the target hold even with very many agents. The existing commit-pressure heartbeat becomes one input to it.
- **Show it in Tower:** an AgentMux budget line, and a "hibernated" state on agent rows, so the user can see what was given back.

### Phase 5: multiple windows (optional; size it after Phase 0)

- Find out why secondary windows need separate in-memory request contexts.
- If they don't need them, share one renderer across same-origin windows (`process-per-site` or a shared context). The cost is a shared crash domain: one window's crash takes the others with it.

## 7. Agent CLIs: a separate budget

- **The CLIs are third-party processes.**
  - Claude Code's native build is Bun/JavaScriptCore, so V8 flags (`NODE_OPTIONS=--max-old-space-size`) don't apply.
  - Bun ignores `--max-old-space-size` and doesn't enforce `BUN_JSC_gcMaxHeapSize` (oven-sh/bun#34917).
  - Several Claude Code leak reports are native or off-heap, so GC tuning wouldn't help anyway.
- **AgentMux can't make them small, but it can stop them growing without limit:**
  - **Measure each agent's own row:** Tower already does this through each agent's job object.
  - **Idle restart above a threshold:** when an agent has been idle for N minutes and is over a set size (for example 600 MB commit), restart its CLI with `--resume`. This is the Discord pattern, applied per agent, and it should show in Tower.
  - **`MEMORY_PRIORITY_LOW` on idle agents' processes:** Windows then trims them first under pressure without changing their commit. (Linux: `MemoryHigh` in a systemd scope.)
- **Hard caps aren't recommended.** A job-object commit limit (`JOB_OBJECT_LIMIT_JOB_MEMORY`) or cgroup `MemoryMax` turns growth into a crash in the middle of a turn. The soft-notification form (`JobObjectNotificationLimitInformation`) is useful as the trigger for the idle restart.
- **The total that matters to the user:** AgentMux ≤ 500 MB, plus about 150–250 MB for each running agent, plus the agents' tools. Tower should show the two parts separately, which it already does.

## 8. Open questions for Phase 0

1. How the main renderer's 1.39 GB splits between transcript state, DOM, xterm buffers, WebGL atlases and fragmentation. This decides whether Phase 2 or Phase 3 goes first.
2. Whether the 394 MB GPU process comes from terminal WebGL contexts, compositor tiles for laid-out hidden tabs, or something else.
3. Why secondary windows get separate request contexts, and why `CalculateNativeWinOcclusion` was disabled.
4. How fast a hibernated agent pane can be restored from srv. This sets K and the user-visible cost of hibernation.
5. What the commit target should be, given Chromium's GPU over-count.

## Sources

**Metric definitions**
- Chromium, "private memory footprint" and the switch from working set: https://groups.google.com/a/chromium.org/g/chromium-dev/c/ELSYMXnvbBc
- Chromium memory docs: https://chromium.googlesource.com/chromium/src/+/HEAD/docs/memory/README.md
- Chromium memory-infra tracing: https://chromium.googlesource.com/chromium/src/+/HEAD/docs/memory-infra/README.md
- Task Manager memory columns: https://scorpiosoftware.net/2023/04/12/memory-information-in-task-manager/
- `EmptyWorkingSet`: https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-emptyworkingset

**Renderer fragmentation and recycling**
- VS Code agents window, renderer fragmentation and recycling: https://github.com/microsoft/vscode/issues/337444
- WebView2 performance guidance (periodic refresh, memory target level): https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/performance
- WebView2 process model: https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model
- Discord's auto-restart above 4 GB: https://www.windowslatest.com/2025/12/06/discord-admits-its-windows-11-app-is-a-resource-hog-tests-auto-restart-when-ram-usage-exceeds-4gb/

**Hibernation and unloading**
- VS Code, background terminal memory: https://github.com/microsoft/vscode/issues/327437
- Slack, one renderer with idle workspaces unloaded: https://slack.engineering/growing-pains-migrating-slacks-desktop-app-to-browserview/
- Edge sleeping tabs: https://blogs.windows.com/msedgedev/2020/12/09/sleeping-tabs-beta-performance/
- Chromium stopping purge-on-freeze: https://developer.chrome.com/blog/freezing-on-energy-saver

**Terminals**
- xterm.js buffer layout (12 bytes per cell): https://github.com/xtermjs/xterm.js/blob/master/src/common/buffer/BufferLine.ts
- xterm WebGL atlas growth, measured: https://github.com/greenfield-inc/Pane/pull/688
- WebGL context release on dispose: https://github.com/notaharness/n10/pull/324
- Ghostty memory per cell: https://github.com/ghostty-org/ghostty/discussions/9982
- Warp's block storage: https://www.warp.dev/blog/block-model-behind-warps-agentic-development-environment
- Windows Terminal per-tab GPU cost: https://github.com/microsoft/terminal/issues/16983
- Zed, agent tool terminal retention: https://github.com/zed-industries/zed/pull/60019
- libghostty: https://mitchellh.com/writing/libghostty-is-coming

**Transcripts**
- Codex desktop, transcript memory: https://github.com/openai/codex/issues/38176

**Chromium process model and flags**
- Chromium process model: https://chromium.googlesource.com/chromium/src/+/main/docs/process_model_and_site_isolation.md
- CEF forum on `--renderer-process-limit`: https://magpcss.org/ceforum/viewtopic.php?f=6&t=17346
- Site isolation cost: https://www.usenix.org/system/files/sec19-reis.pdf
- V8 lite mode: https://v8.dev/blog/v8-lite
- V8 jitless: https://v8.dev/blog/jitless
- V8 memory cage / pointer compression: https://www.electronjs.org/blog/v8-memory-cage
- GPU tile budget experiment: https://github.com/quwisky/trinity-matrix-client/pull/1057
- Tauri vs Electron memory on Windows: https://github.com/tauri-apps/tauri/issues/5889

**Agent CLIs and OS limits**
- Bun heap limits not enforced: https://github.com/oven-sh/bun/issues/34917
- Claude Code memory reports: https://github.com/anthropics/claude-code/issues/67433, https://github.com/anthropics/claude-code/issues/36132, https://github.com/anthropics/claude-code/issues/24840
- Job object notification limits: https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_notification_limit_information
- `SetProcessInformation` (memory priority): https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation
