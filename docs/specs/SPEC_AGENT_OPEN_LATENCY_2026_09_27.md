# SPEC: opening an agent from My Agents should be near-instant — where the time goes, and what to change

**Date:** 2026-09-27
**Status:** proposed
**Author:** Manoz
**Repos touched:** `agentmux` (`agentmux-srv/src/server/cli_handlers.rs`, `agentmux-srv/src/backend/history/`, `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx`, `frontend/app/view/agent/hooks/useHistoryPagination.ts`, `frontend/app/notification/sound/`, `docs/MUXLOG.md`)
**Related:** `SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md` (live feed, K turns), `SPEC_MUXLOG_AGENT_ADMISSION_TIMELINE_2026_09_27.md` (the muxlog recipe pattern reused in §4.6)

## 1. Report

Opening an agent from **My Agents** sometimes takes an unusually long time and looks like heavy processing, even after the caching and live-feed work. It should be close to instant.

## 2. Method

All numbers come from the sources below; nothing here is estimated unless marked.

1. **The live 0.58.0 portable's logs.** Read only; nothing was tested in that instance.
2. **Logs from every retained instance, 2026-09-20 → 27:** 12 dev builds and 3 portables. That's 65 agent opens with frontend `[perf]` data and 32 with backend launch-path lines.
3. **A `task dev` build of main (`7042bb4`), profiled over CDP.** Each open was a real mouse click on a My Agents card, with a 200 µs sampling CPU profile, a `longtask` observer, and timing to the first row painted in the *new* pane and to "quiet" (no frame gap > 50 ms for 1.5 s). Agents: Lazo (58 MB transcript) and Lzop (48 MB), neither running anywhere else.
   Two caveats for anyone repeating this. (a) The probe must not read layout itself; see the F3 correction. (b) The dev window must be **visible**. When it's behind other windows, Chromium's occlusion tracking reports the page `hidden` and stops `requestAnimationFrame`. Then a pane's reveal gate (history "painted" is a double rAF) never opens and the pane sits behind its loading cover, which looks exactly like a stuck open (`[pane-readiness] … gate(s) pending: history`), and any rAF-based timing stalls. Check `document.visibilityState` before trusting a run.
4. **The real restore replay, run offline.** `parseHistoryLines` from main was run under Node on the actual last 5,000 lines of AgentA's 1.18 GB transcript, read directly from `shared/agents/transcripts/filestore.db` (read-only).

## 3. Findings, largest first

### F1 — First open per provider per AgentMux version installs the CLI with npm, on the open path (≈ 2.5–3.2 s)

`ResolveCli` looks for the provider CLI under **`…/versions/<agentmux-version>/data/instances/v<agentmux-version>/cli/<provider>`**. So every new AgentMux version (each upgrade, each portable, each `task dev` build) starts with no CLI. The **first open of each provider** runs `npm install @anthropic-ai/claude-code@<pinned>` synchronously before `WriteAgentConfig`, and nothing renders until it finishes:

| Instance | `CLI not found locally` → `CLI installed` | Open → first row |
|---|---|---|
| 0.58.0 portable, Manoz | 03:38:50.58 → 03:38:53.84 (3.3 s) | n/a (log only) |
| dev `7042bb4`, Lazo (profiled) | 04:01:53.18 → 04:01:56.38 (3.2 s) | **4,068 ms** (quiet at 6,677 ms) |

**16 such installs** appear in the retained logs, one per provider per version per channel. The pinned CLI version (`2.1.280`) is identical across all of those AgentMux versions, so each install repeats the same work. The npm cache made it a revalidation, not a download, and it still cost about 3 s.

### F2 — The startup history index competes with the first opens (23.5 s of scanning)

`history: index built … discovered=61066 duration_ms=23542` (`agentmux-srv/src/backend/history/mod.rs:246`, 0.58.0 portable, 03:38:36 → 03:38:59). It runs at startup and covers the window when the user is most likely to open agents. It rescans all 61,066 sessions on every launch.

### F3 — A warm open is ~1.3 s to "quiet"; mount-time layout reads are a small part of it

With the CLI installed and no startup work running (Lzop, dev, profiled):

- click → quiet: **1,341 ms**; JavaScript busy: **329 ms**; 6 long tasks, **504 ms** in total (max 91 ms).
- Self time attributable to **app** code: `dispatchScrollMargin` **≈ 52 ms** (`AgentDocumentVirtualList.tsx:707`), the composer strip's `zoomRatio` / slot re-measure **≈ 25 ms** (`AgentComposerStrip.tsx`), `parseHistoryLines.feed` **16 ms**, GC 20 ms.

**Correction (2026-09-27, after this spec merged):** the first version of this finding attributed **105 ms** of forced layout (Lazo: **268 ms**) to the app and called it "half the JavaScript time". That figure had **no app caller** in the profile. It was the profiling probe's own per-frame `getBoundingClientRect()` over every row, which it used to detect the first painted row. The probe now detects the first row without layout reads, and the numbers above are the app's own share. The same caveat applies to the long-task and quiet times: they include some probe overhead, so they're upper bounds.

`dispatchScrollMargin` reads `virtualContainerRef.offsetTop`. The profile shows the read happening in the list's `onMount`, which runs inside the block-meta update (`updateMuxObject`) that mounts the pane, so it forces the pane's first layout synchronously. Whether that layout is *extra* depends on how much DOM changes after it before the frame, which wasn't measured cleanly. See §4.2.

Across the 65 logged opens, total frontend long-task time per open was < 0.5 s for 33, 0.5–1.5 s for 19, and 1.5–3 s for 5.

### F4 — Cold-path extras on the main thread (the Lazo open: 1,874 ms of JavaScript)

On top of F3's items (this run's 268 ms `getBoundingClientRect` was probe overhead; see the F3 correction):

- **Markdown + highlighting ≈ 290 ms.** `remark-parse` 126 ms, `rehype-highlight` 67 ms, `shiki` 56 ms, shiki's wasm 38 ms, all for rows in the restored tail. Most of those rows are off-screen or rolled off moments later.
- **Sound engine priming, 46 ms.** `primeOnce` → `sound-player.ts` `prime()` on the first open.
- **Drag-preview rasterization, ~170 ms** (`html-to-image` `toPng` → `toDataURL` + `Blob`, `TileLayout.core.tsx:382`). It's triggered by header **hover**, not by the open (`drag-preview-size.ts`: "runs on every header hover"), but it competes for the same frames whenever the pointer crosses a pane header while an agent opens.

### F5 — Not the bottleneck: the 5,000-line restore replay

This was the first suspect, so here's the measurement. On every open the pane reads `RESTORE_WINDOW_LINES = 5_000` transcript lines (`useHistoryPagination.ts:144/352`) and replays them. The live feed then rolls off everything but the last K finished turns. AgentA's real last 5,000 lines are 2.0 MB, and **83 % are `stream_event` deltas** (4,136 of 5,000). The complete messages are only 154 `assistant` and 77 `user` lines. But **`parseHistoryLines` over all 5,000 takes 16.5 ms** (median of 7, Node 22), and 5.0 ms without the deltas. It's a real inefficiency (§4.5), just a small one.

### F6 — The logs can't tell system time from user time

In the 32 opens with backend launch-path lines, 18 took more than 1 s from `ResolveCli` to `WriteAgentConfig`, the longest 19 s. Some of that is F1. But in most of the long gaps the only lines are `CheckCliAuth` → `claude auth check: no credentials … skipping CLI`, and then nothing: that's the open flow **waiting for the user** at an account/login prompt. Nothing records when the pane started waiting on a human versus on the system, so "opens are slow" can't be quantified from logs today.

### Side finding — pane containers can be scrolled out of place

While profiling, a `scrollIntoView()` on a My Agents card scrolled **two `.tile-node` containers** (`overflow: hidden`) by 17 px each. That shifted a whole pane about 30 px up, under the window header (reported live by the user). `overflow: hidden` elements are still scroll containers, so any `scrollIntoView()`, or a `focus()` that scrolls into view, from inside a pane can do this. The trigger here was the probe, but app code that calls either inside a pane can hit the same trap.

## 4. Changes

Ordered by user-visible impact.

### 4.1 Take CLI install off the open path, and share installs across AgentMux versions (F1)

- Key the CLI install directory by **provider + pinned CLI version**, e.g. `~/.agentmux/shared/cli/claude/2.1.280/`, instead of by AgentMux version. An upgrade that keeps the same pinned CLI then does no install at all.
- When an instance starts and the pinned CLI isn't installed, **install it in the background** at low priority, before anyone opens an agent.
- **Never block the pane on `ResolveCli`.** Render the restored history and the composer immediately. Only the *spawn* waits for the CLI, with the pane showing "preparing <provider> CLI…" in the composer strip instead of a blank pane.

### 4.2 Stop forced layout during mount (F3; small, measure first)

After the F3 correction, this is worth tens of milliseconds, not hundreds. Land it only with a before/after profile, taken with a visible dev window and a probe that doesn't read layout, showing a reduction in layout time during the open.

- `dispatchScrollMargin` must not read `offsetTop` from effects driven by block-meta updates. Keep the reads in the ResizeObservers, which fire when layout is already clean, and in user scroll handlers. If a meta change can move the header, schedule one read for the next animation frame, coalescing any number of changes into a single read.
- Add a dev-only assertion or `perf:allow-layout-read` audit so a layout read reached from `updateMuxObject` shows up in tests.

### 4.3 Move the history index out of the startup window (F2)

- **Persist the index** and update it incrementally from session mtimes, instead of rescanning 61,066 sessions every launch.
- Start it after the first agent is interactive, or after a few seconds of idle, on a low-priority thread.

### 4.4 Defer non-visible main-thread work (F4)

- Highlight code blocks and fully render markdown **only for rows in or near the viewport**. Rows restored and then rolled off, or never scrolled to, never pay for it. Load shiki and its wasm lazily, on the first visible code block.
- Prime the sound engine on the **first sound**, or at idle, not on the first open.
- Drag preview: start rasterization on **drag start**, or on a hover that has lasted a few hundred ms, not on every header hover. Cache it per pane until the pane changes.

### 4.5 Size the restore to the live feed (F5, small)

- Skip `stream_event` lines in the restore replay when the matching final `assistant` message is inside the window. The replay produces the same nodes from a sixth of the lines.
- Size the window from the last K turn starts (the backend's output index already has line offsets), not a fixed 5,000 lines.

### 4.6 Measure opens, and teach muxlog to report them (F6)

- Log one structured line per open, `[agent-open] <agent> <block>`, with timestamps for: click; `ResolveCli` done, and whether it installed; config written; history read; parse done; first row painted; quiet; plus any **user-wait** spans (account picker, login prompt, launch modal), recorded as their own phase so they're never counted as latency.
- Add `muxlog opens [<agent>] [--since …]`, in the same style as `muxlog admission`: one row per open with those phases, and a summary of p50/p95 to first row, excluding user-wait.

### 4.7 Make pane containers non-scrollable (side finding)

- `.tile-node` and any other layout container that is `overflow: hidden` purely for clipping gets `overflow: clip`. That clips the same way but isn't a scroll container, so `scrollIntoView()` and `focus()` can't shift a pane under the window header.

## 5. Acceptance

Measured with the §2 profiling method in `task dev`, and with `muxlog opens` once §4.6 lands:

- **Warm open** (CLI present, nothing else starting): first row ≤ **300 ms**, quiet ≤ **800 ms**, no long task > 100 ms, and no forced layout reached from `updateMuxObject` in the profile.
- **First open after an AgentMux upgrade** with an unchanged pinned CLI: no `npm install` at all; same numbers as warm.
- **First open when the pinned CLI changed:** history is visible ≤ **300 ms** after the click while the CLI installs. The spawn follows when it's ready.
- **Opens during the first 30 s after launch** are within 20 % of a warm open (the history index no longer competes).
- The Lazo cold-open profile shows no `remark-parse`/`shiki` work for rows outside the viewport, and no `html-to-image` work unless a drag starts.

## 6. Out of scope

- Admission and take-over latency. Warm admission measured 70–80 ms (AgentA, 0.58.0); see the muxlog admission spec for its failure modes.
- The CLI's own startup time after spawn. It happens after the pane is interactive.
