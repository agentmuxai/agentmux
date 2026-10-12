# Verifying the performance numbers on agentmux.ai, on all three platforms

**Date:** 2026-10-11
**Status:** active. Shipped in #4712: this plan and its first tool, `crates/procstats/examples/memtree.rs`, a memory-only probe used for a pilot on all three platforms. Still to do: the owner's answers in §9, the full harness (§6), and Phases 1–6.
**Author:** Agent4@narko, at the operator's request
**Scope:** every number on the live [agentmux.ai/compare](https://agentmux.ai/compare) and [agentmux.ai/sources](https://agentmux.ai/sources) pages: AgentMux's own, and the competitors'.
**Runs with:** [SPEC_CROSS_PLATFORM_TESTING_PROTOCOL_2026_10_11.md](../specs/SPEC_CROSS_PLATFORM_TESTING_PROTOCOL_2026_10_11.md) (one lead, one tester per platform).

## 1. Goal

Each number on the site is either confirmed by a measurement we can repeat, or corrected, with the method and raw data published next to it. "Rigorous" here means:

- defined: each claim says what is measured, on what, from when to when;
- repeatable: scripted, run many times, with the spread reported, not one lucky run;
- fair: competitors measured the same way, on the same machine, in the same session, at stated versions;
- honest: a worse result for AgentMux is published too, and changes the site.

## 2. The claims

As shown on 2026-10-11. The "today's basis" column is what /sources says each one rests on.

### 2.1 AgentMux

| # | Claim | Today's basis | First look (2026-10-11) |
|---|---|---|---|
| A1 | Download ~148–206 MB | v0.59.9 release assets (high) | v0.59.20 assets are 147.0–206.7 MB (setup.exe 147.0, DMG 167.3, AppImage 176.5, rpm 190.6, deb 190.7, tar.gz 191.5, portable zip 205.8, msix 206.7). Holds; the floor is 1 MB low. |
| A2 | Install ~453–464 MB | extracted portable zip / tar.gz, v0.59.9 (high) | Not re-measured. The installed forms (setup.exe, DMG app, deb, rpm, AppImage) are not covered. |
| A3 | RAM 150–350 MB at idle, one terminal pane | "self-reported from the development team", CEF host + `agentmux-srv` (medium) | **Suspect.** A running app is about 8 Chromium processes (renderers per window and pool window, GPU, utility, crashpad) plus srv and the launcher. The 2026-06 data had CEF processes at 20–207 MB working set each (SPEC_MEMORY_ANALYSIS_2026_06_26). Counting two processes undercounts; summing working sets overcounts shared pages. Unmeasured as defined. |
| A4 | Backend `agentmux-srv` ~15–40 MB | self-report | 2026-06 heartbeat: about 30 → 45 MB over 30 h. Plausible, unmeasured on macOS and Linux. |
| A5 | Startup < 1 s | self-report: "CEF host launches frontend from local dist/" (medium) | **Ambiguous.** The splash's own staged telemetry puts launch-to-first-paint at 1.5–9 s (SPEC_SPLASH_STARTUP_TELEMETRY_2026_06_25). The claim needs an endpoint (§4.2). |
| A6 | Sidecar ready < 500 ms | self-report | Unmeasured. srv start to "listening" is in the host log. |
| A7 | "Chromium + Rust, not Chromium + Node" (no unpredictable heap growth, no daily restarts) | narrative | Untested. A soak run (§4.5) is what would support it. |

### 2.2 Competitors

| App | Download | Install | RAM | Startup | Spikes (reported) | Basis on /sources |
|---|---|---|---|---|---|---|
| Warp | ~205 MB | ~450 MB | 150–400 MB | 1–2 s | 3+ GB | DMG measured; RAM from Discord and Reddit |
| Wave Terminal | ~155–200 MB | ~300 MB+ | 200–500 MB | 2–3 s | — | v0.14.1 assets; RAM "typical Electron" |
| Zed | ~80 MB | ~220 MB | 100–250 MB | < 1 s | — | zed.dev, Zed's blog |
| Cursor | ~230 MB | 500 MB+ | 400 MB–3 GB+ | 2–6 s | 20–40+ GB, multi-day | forum and Reddit |
| Windsurf | ~220 MB | ~800 MB | 500 MB–4 GB+ | 3–8 s | 10 GB+ language server | Reddit and Discord |

None of the competitors' RAM or startup numbers were measured by us. The "spikes" are reports of specific users' sessions, and no lab run can confirm or refute them. For those, the plan checks that each cited report exists and says what /sources says it does, and links it (§4.6); it doesn't try to reproduce them.

## 3. Platforms, machines and people

| Platform | Tester (protocol roster) | Machine | Note |
|---|---|---|---|
| Windows 10 (19045) | AgentA | Area54 | The owner's choice (2026-10-11). Results are labelled Windows 10. Windows 11's memory compression and scheduling differ, so they aren't presented as Windows 11 numbers. Needs a quiet window (§5.2). |
| macOS, Apple Silicon | Masty | starpower (MacBook Air) | A fanless Air throttles under sustained load: record the thermal state, keep runs short, and run on AC power. |
| Linux | Maricon | charlie | Run on both Wayland and X11 (XWayland apps behave differently). |

The lead (Agent4@narko) designs the harness, reads and analyses the data, and doesn't measure, as in the protocol. narko itself runs many agents, so it is a noisy machine.

## 4. Method, per metric

The research basis for each rule is in §7.

### 4.1 Memory (A3, A4, competitors' RAM)

- **One metric per OS, the one the OS itself uses for "this app's memory", summed over the app's whole process tree:**
  - Windows: private commit (Private Bytes, Task Manager's "Commit size"), which is what Chromium itself reports as a process's memory footprint, with private working set ("in RAM now") as a second column. Working set alone under-reports once pages are compressed or paged out [W1, W2]. Chromium notes that commit over-counts the GPU process by 50 MB or more [C2], so the GPU process is also shown on its own line.
  - macOS: `phys_footprint`, which is what Activity Monitor's Memory column shows and what `footprint` reports. It includes compressed and swapped pages, and GPU memory on Apple Silicon [M1, M2]. Measure all the tree's PIDs in one `footprint` call, which reports shared objects once.
  - Linux: PSS from `/proc/<pid>/smaps_rollup`, which splits shared pages fairly across processes, so a sum over a tree isn't inflated, plus USS (what quitting the app would give back) [L1, L2]. Record the kernel version: kernels built with `CONFIG_NO_PAGE_MAPCOUNT` only estimate PSS for large folios [L2].
- **Cross-check for the Chromium-based apps** (AgentMux, Wave, Cursor, Windsurf): Chromium's own total, `Memory.Total.PrivateMemoryFootprint`, which counts each shared region once [C1, C2]. If the harness and Chromium disagree by more than 10%, find out why before publishing. (Electron's own `getAppMetrics()` gives private bytes on Windows only, so it can't be the cross-OS metric [E1].)
- **Also RSS and working set**, because community reports quote those. They let us compare like with like, but they are never the headline: they double-count shared Chromium pages across processes.
- **The tree:** everything from the app's top process down, at a stated moment. For AgentMux, that is the launcher, the CEF host and every `--type=` subprocess, and `agentmux-srv`. The shell and any agent CLI running in a pane are reported separately, which matches the site's "agent processes excluded" note. For an Electron app, that is the main process and all its helpers. For Zed and Warp, it is the app and any language-server or helper children.
- **Reuse `crates/procstats`.** It already reads exactly these metrics on all three OSes (Windows `WorkingSetPrivateSize`, macOS `ri_phys_footprint`, Linux `RssAnon + VmSwap`, with `smaps_rollup` on demand), and the Tower pane's "agentmux" task already sums AgentMux's tree with it. The harness (§6) is a thin CLI over it: given a root process, it samples the tree. One code path for the app's own Tower view and for the published numbers.
- **"Idle, one terminal pane" means**, for every app:
  - a fresh profile, default settings, no extensions or plugins;
  - signed out where the app allows; where it doesn't, signed in and idle;
  - one window with one terminal or integrated terminal open at the default shell, and nothing typed;
  - for editors, a small fixed sample folder open (the same one for all of them).

  Measure after a 60 s settle, sampling every 1 s for 60 s, and report the median of that window.
- **States, beyond idle:** (a) idle as above; (b) a fixed "working" layout of 4 terminal panes, each having run a scripted command; (c) after the soak (§4.5). The site only claims (a); (b) and (c) are what users will compare against.

### 4.2 Startup (A5, A6, competitors' startup)

- **Endpoints, measured from the moment the process is spawned:**
  - T1, first window: the app's first top-level window is visible on screen. This is measured from outside the app (OS window list), so it is the same for every app.
  - T2, first content: the window shows its terminal or editor, not a blank or splash frame. Measured from a screen capture of the window (the first frame that matches a reference crop), which also works for every app.
  - T3, usable: a key typed into the terminal is echoed. For AgentMux, also read the splash's staged timeline (srv ready, CEF init, first paint) from the host log, so the total can be explained, not just stated.
- **The site's "< 1 s" is about one of these, and the owner picks which (§9).** The plan reports all three.
- **Cold and warm:**
  - Warm: a second launch straight after a quit, with the files cached.
  - Cold: after a reboot, or after flushing the OS file cache (macOS `purge`, Linux `drop_caches`, Windows standby-list flush), with the first-run-after-install effects (Defender or Gatekeeper scans, migrations) reported separately as "first launch".
- **Runs:** at least 30 warm and 10 cold launches per app per platform, alternating apps run by run, so a slow drift in the machine doesn't favour one app [S3]. hyperfine drives the launches (`--warmup`, and `--prepare` for the cache flush) [T1]. It times only until the command exits, so the command it runs is the harness's `launch`, which exits at the endpoint.
- **First launch is its own number.** On macOS, a first launch after download (Gatekeeper, notarization and XProtect checks) has taken over 10 s on macOS 26, and it can repeat after the app is moved [T3]. On Windows, Defender scans new binaries; record its effect with `New-MpPerformanceRecording` rather than adding exclusions [T4]. Both apply to every app, so measure them the same way for all and report them apart from warm and cold.

### 4.3 Download and install size (A1, A2, competitors)

- **Download:** the byte size of each official asset of the current release, read from the vendor's own release listing (GitHub releases API for AgentMux and Wave), and for the others by downloading the asset itself. Record version, date and URL. Report decimal MB.
- **Install:** the apparent size of everything installed, per form, by the same script on each OS: the portable folder, `Program Files\…` plus `%LOCALAPPDATA%\…` for an installer, the `.app` bundle, and the deb, rpm or AppImage file set. Also the size of the per-user data the first launch creates (profile, caches), as its own column, since apps differ a lot there.

### 4.4 Statistics and reporting

- Report the median, the interquartile range or p95, and n, with a bootstrap 95% confidence interval for each median. Never a single run, "best of N", or a mean on its own (start-up times are skewed) [S1].
- A comparison ("AgentMux uses less memory than X") is reported as a ratio with its own confidence interval [S2]. It is only stated on the site when that interval excludes 1.
- Warm-up runs are dropped, and the drop is stated. Outliers are kept in the raw data and flagged, never silently removed.
- **A "number on the site" is the median. The range on the site spans the platforms' medians,** or else names the platform.
- Every result records: the machine (CPU, RAM, GPU, display and scale), OS build, power source and plan, each app's exact version, the date, and the harness commit.

### 4.5 Soak (A7, and the "daily restarts" narrative)

An 8-hour run per platform: AgentMux and one Electron competitor side by side, each with the "working" layout, a scripted light activity every minute (a command in each pane, a tab switch). Sample memory every 30 s. Report the slope (MB per hour) and the start and end medians. This is the only run that can support "no unpredictable heap growth"; until it is done, the site shouldn't imply it.

### 4.6 Checking the cited reports

For each competitor number based on reports (RAM ranges, spikes), the plan looks at each source /sources gives:
- it exists;
- it says what is claimed;
- it is for a version less than a year old.

Then it records the link. A claim whose source is gone, misquoted or stale gets reworded or removed. This is reading, not measuring, so it can start at once.

## 5. Running it

### 5.1 Phases

| Phase | What | Who | Done when |
|---|---|---|---|
| 0 | Owner answers §9; inventory frozen | owner, lead | definitions agreed |
| 1 | Harness (§6), with its own tests; source check (§4.6) | lead | harness passes on a lab app on each OS |
| 2 | Pilot: AgentMux + one competitor, Windows only, n = 10 | Windows tester | spread known; n confirmed or raised |
| 3 | Full runs: all apps × all platforms × idle and working states, cold and warm | three testers, in parallel | every cell has a median and a CI |
| 4 | Soak | three testers, overnight | slopes recorded |
| 5 | Report; a proposed site diff; the scripts and raw data published | lead | owner approves the site change |
| 6 (optional) | Regression tracking: AgentMux's own idle memory and warm start, per release, on fixed hardware, with alerts that are re-run before anyone is told [S7] | lead | a number per release in the release notes |

### 5.2 A quiet machine

These are shared development machines with several agents on each. A build or an agent burst on the same machine moves memory and start-up numbers far more than the differences we are measuring. So a measurement run takes the protocol's input lock **and** a quiet lock: the other agents on the machine pause builds, tests and dev instances for the length of the run, and the harness logs the machine's total CPU load during each sample. A sample taken while the load was above a set threshold is discarded and retaken. Runs go in short blocks (15–20 min), which suits the lock rules. If the noise is still too high (Phase 2 will show this), the run moves to a machine with no agents on it.

### 5.3 Per-run discipline (from the protocol, applied to measurement)

- On AC power. The OS power plan fixed (Windows "Best performance", macOS no Low Power Mode, Linux the `performance` governor where allowed), and turbo state recorded [S4, S5]. No other apps open. Notifications off. Wait for Spotlight or Windows Search indexing to settle. The same display, scale and refresh rate, and the same font and window size, for every app.
- Each app at a stated version, installed fresh, with a fresh profile per state.
- Results are posted on the tracking issue (one comment per platform per phase), each naming the harness commit. Raw data goes in the results folder (§6).

## 6. The harness

`scripts/perf/claims/` (new), in this repo, so anyone can rerun it:

- `perfprobe` (a small Rust binary over `crates/procstats`) does three things:
  - `tree --root <pid>` samples the tree's per-process and summed private, resident and commit memory, with Linux PSS and USS;
  - `launch -- <cmd>` spawns an app and times T1 from the OS window list (Windows `EnumWindows`, macOS `CGWindowListCopyWindowInfo`, Linux X11 `_NET_CLIENT_LIST`; on Wayland, through XWayland for apps that use it, and for native Wayland apps by screen capture only);
  - `sample` writes JSON lines.
- A screen-capture step for T2 and T3, per OS.
- `run.mjs` drives the matrix: apps × states × cold/warm × n. It alternates the apps and writes `results/<platform>/<date>/*.jsonl`.
- `analyse.mjs` turns those into medians, IQRs and bootstrap CIs, and into the tables for the report and the proposed /sources text.
- App definitions (`apps/*.json`), one per app and platform: how to install, launch and quit it, and which process is its root.

The stale `scripts/benchmarks/measure-performance.{ps1,sh}` (Tauri-era, a dead binary path, one process only) is removed in the PR that adds this harness (not #4712, which ships only `memtree`), so there is one way to measure.

## 7. Best practices this follows

### 7.1 Sources

Memory:
- [W1] Microsoft, `PROCESS_MEMORY_COUNTERS_EX2` (private commit vs private working set): https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex2
- [W2] Chromium's switch of its Task Manager column to memory footprint: https://groups.google.com/a/chromium.org/g/chromium-dev/c/ELSYMXnvbBc
- [C1] Chromium memory docs: https://chromium.googlesource.com/chromium/src/+/HEAD/docs/memory/README.md
- [C2] Chromium memory key concepts (one owner per shared region; the GPU commit over-count): https://chromium.googlesource.com/chromium/src/+/main/docs/memory/key_concepts.md
- [M1] WWDC22, memory footprint: https://developer.apple.com/videos/play/wwdc2022/10106/
- [M2] `footprint(1)`: https://keith.github.io/xcode-man-pages/footprint.1.html
- [L1] Kernel docs, `/proc` (PSS, `smaps_rollup`): https://www.kernel.org/doc/html/latest/filesystems/proc.html
- [L2] `proc_pid_smaps(5)`: https://man7.org/linux/man-pages/man5/proc_pid_smaps.5.html
- [E1] Electron `MemoryInfo`: https://www.electronjs.org/docs/latest/api/structures/memory-info

Startup:
- [T1] hyperfine (`--warmup`, `--prepare`): https://github.com/sharkdp/hyperfine
- [T2] Linux `drop_caches`: https://docs.kernel.org/admin-guide/sysctl/vm.html. On macOS, `purge`; on Windows, an elevated `RAMMap64 -Et`. A reboot is the only true cold start.
- [T3] Slow first launches on macOS: https://eclecticlight.co/2025/09/30/do-apps-launch-faster-in-macos-tahoe/ and https://mjtsai.com/blog/2025/04/30/why-some-apps-sometimes-launch-extremely-slowly/
- [T4] Defender performance recording: https://learn.microsoft.com/en-us/powershell/module/defenderperformance/new-mpperformancerecording

Statistics and environment:
- [S1] Georges et al., statistically rigorous performance evaluation: https://dri.es/files/oopsla07-georges.pdf
- [S2] Kalibera and Jones, rigorous benchmarking in reasonable time (effect sizes with CIs): https://kar.kent.ac.uk/33611/45/p63-kaliber.pdf
- [S3] Mytkowicz et al., producing wrong data without doing anything obviously wrong (randomize the setup and order): https://users.cs.northwestern.edu/~robby/courses/322-2013-spring/mytkowicz-wrong-data.pdf
- [S4] LLVM benchmarking tips: https://llvm.org/docs/Benchmarking.html
- [S5] Google Benchmark, reducing variance: https://google.github.io/benchmark/reducing_variance.html
- [S6] Shared CI runners detect changes of about 4.4% at best, against about 1.9% on bare metal: https://arxiv.org/html/2411.05491v1
- [S7] Mozilla perf sheriffing (regression alerts with re-runs and an owner): https://firefox-source-docs.mozilla.org/testing/perfdocs/perf-sheriffing.html

For later, if typing latency joins the site:
- [K1] Typometer: https://pavelfatin.com/typometer/
- [K2] Dan Luu, terminal latency: https://danluu.com/term-latency/
- [K3] vtebench (throughput only, not latency): https://github.com/alacritty/vtebench
- [K4] Warp's published method: https://docs.warp.dev/terminal/comparisons/performance/

### 7.2 The pitfalls this plan guards against

| Pitfall | Where the plan handles it |
|---|---|
| Summing RSS or working set across a Chromium process tree, which counts shared pages many times | §4.1: commit, footprint and PSS are the headline; RSS only for like-with-like |
| A different metric per app or OS, unstated | §4.1: one harness, one metric per OS, for every app |
| Leaving out child processes (GPU, utility, renderers, backend, helpers) | §4.1: the whole tree, with the pane's shell and agents split out |
| A "cold" start that wasn't, or one app's first-run scan in its numbers | §4.2: cold, warm and first launch reported separately for all apps |
| Timing to process exit, or only to a window | §4.2: T1, T2 and T3 |
| Best of N, or a mean of a few runs | §4.4: median, spread, CI, n ≥ 30 warm |
| Turbo, thermals, battery, display settings | §5.3 |
| Tuned AgentMux against competitors' defaults | §4.1: default settings and fresh profiles for every app |
| Throughput standing in for responsiveness | not claimed; typing latency stays out unless measured with [K1] |
| Shared-CI timings, undated or without data | §5.2 (quiet machines, never CI), §6 (scripts and raw data published) |

## 8. Risks

- **Noise on shared machines**, see §5.2. This is the largest risk. Shared CI runners are no answer: they only detect changes of about 4% or more [S6].
- **Wayland window enumeration.** There's no portable way to list another app's windows on native Wayland, so T1 there falls back to screen capture, which is coarser.
- **Sign-in walls.** Cursor and Windsurf may not reach an idle editor without signing in, and AI features may start background work when signed in. Each app's state is recorded as it actually was.
- **Versions move.** A competitor's update mid-run splits the data. Pin the version per phase, or record it per sample.
- **The answer may be worse than the site.** That's the point; §9 asks the owner to agree up front that the site follows the data.

## 9. Questions for the owner

1. **Startup endpoint.** Which one is "< 1 s" meant to be: first window (T1), first content (T2), or usable (T3)? The plan measures all three either way.
2. **RAM definition.** Is it OK to change the claim to the whole app tree in each OS's own metric (Task Manager / Activity Monitor / PSS), even if the number goes up?
3. **Competitor accounts.** Do we sign in to Cursor and Windsurf for the runs (that needs accounts), or measure signed out where possible and say so?
4. **Publishing.** The scripts and raw data go in this public repo, and the site links them. Agreed?
5. **Quiet windows.** Can we have a quiet window (other agents paused) on each test machine? (Windows is settled: Windows 10 on Area54.)
6. **The site follows the data.** If a number comes out worse, /compare and /sources change to the measured value, with its date.
