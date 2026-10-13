# Report: where AgentMux's renderer memory goes — measured root causes

**Date:** 2026-10-12
**Status:** analysis — measurement only. Nothing has changed in the repo; the patches described here were built only into throwaway lab copies.
**Follows:** `REPORT_MEMORY_FOOTPRINT_500MB_BUDGET_2026_10_12.md` (the budget and plan). That report's open questions, Phase 0, are answered here.

## 1. Summary

An AgentMux v0.59.20 instance showed about 2 GB in Tower: one window, 4 tabs, 12 agent panes, 26 hours of uptime. Two thirds of that was its main window renderer (1.39 GB).

That renderer was reproduced in an isolated lab, and its memory was attributed with Chromium's memory-infra dumps, V8 heap snapshots and A/B builds. A lab run with the same layout and 12 scripted agents reached the same size (1.35 GB) after one turn per agent, and 2.3 GB after 40 more.

Four causes account for almost all of it. All four are in the agent pane or in how Chromium handles its memory; none is in srv, and none is caused by the number of panes as such.

| # | Cause | Where | Measured effect |
|---|---|---|---|
| 1 | **A tool's output stays on the page after the tool finishes.** A tool panel opens by itself while the tool runs. Its body (shiki-highlighted diff, file preview or Bash output) then stays mounted for the rest of the row's life. In a live session that means every tool call. | `frontend/app/view/agent/components/ToolBlock.tsx:240-249` | **95%** of each agent pane's DOM elements sat inside collapsed tool panels. Unmounting the body after collapse (together with #2) cut the renderer by **53–56%**, the JS heap by **64–69%** and the DOM nodes by **94–97%**. |
| 2 | **Rows that left the list stay in memory.** In `@solid-primitives/keyed`'s `keyArray` (behind `<Key>`), every slot's root closure keeps the whole result array of the pass that created it. That array holds the DOM of every row on screen at that moment. Rows that later roll off stay alive, detached. | `node_modules/@solid-primitives/keyed/dist/index.js`, `addNewItem` (v1.5.3), used by `AgentDocumentVirtualList.tsx:1729, 1808` | 728k detached DOM objects in one snapshot. Fixing it halved the DOM node count (497k → 249k) and cut the renderer by about 90 MB in a 4-pane run. |
| 3 | **Freed Blink memory is never returned.** Blink's garbage-collected heap (Oilpan/cppgc) doesn't compact, and on Windows it keeps empty pages. After a burst of DOM, the renderer stays at its high-water mark. | Chromium | Blink heap **89–92% empty**: 544 MB reserved for 61 MB of live objects, and 200 MB for 17 MB. Reloading the page gave it back: **1,009 → 338 MB** (12 panes) and **407 → 198 MB** (4 panes). |
| 4 | **Hidden tabs cost as much as visible ones.** Agent panes in a background tab receive, build and keep their full DOM. | the agent pane (dormancy only defers rendering) | Two panes in a hidden tab held 23k–32k elements each, the same as visible panes. Switching tabs changed nothing (788 → 795 MB). |

Smaller, fixed costs:

- The pre-warmed spare windows: 3 renderers at 52–58 MB each, **about 165 MB**.
- Shiki's oniguruma WebAssembly heap: 28 MB.
- The app's own script: about 10 MB.

**An idle agent pane costs about 1 MB.** The count of panes isn't the problem; what they hold and keep is.

**Typing latency** under the 12-agent load was mostly fine on both builds: key-to-paint p95 24–64 ms at 58–60 fps. It degraded when many turns started at once: p95 96–104 ms, 46–49 fps, 11–19% of the time in long frames. Fixes #1 and #2 didn't measurably change it in this test. A freshly reloaded renderer was the best case: p95 24–32 ms, no long frames.

## 2. Method

**Lab instance**
- A production portable of `main` at 9088493f5 (v0.59.21 + main, CEF 154, codec build), built with `task package` into the agent's own workspace.
- Run with its own data folder (`AGENTMUX_HOME_OVERRIDE`) and its own CDP port (`AGENTMUX_CDP_PORT=9333`). It never touched the operator's instances.
- A production build was used on purpose. A `task dev` build (Vite plus SolidJS dev mode) would inflate every number.

**Agents**
- The repo's scripted Claude stand-in (`scripts/ui-screenshots/fake-claude/`), so the panes ran the real pipeline: srv, the translator, the stores and the renderer. No model, no account, no tokens.
- The script was a generated, tool-heavy conversation: 150 turns of about 133 KB each.
  - Each turn has 8–20 tool calls: Bash with 40–300 lines of ANSI output, Reads of 80–400 code lines, Greps, Edits, TodoWrites.
  - Each turn ends with a 2–6 KB markdown reply with code blocks.
- The stand-in streams far faster than a model (4 ms per chunk). A run of 40 rounds compresses hours of agent work into about 15 minutes.

**Measurements**

| What | Tool |
|---|---|
| OS numbers per process (private working set as Tower shows it, commit) | `Win32_PerfRawData_PerfProc_Process` |
| Chromium allocators per process (V8, blink_gc, PartitionAlloc, malloc, cc, gpu…) and their live-object sizes | memory-infra memory dump over CDP (`Tracing.requestMemoryDump`, detailed, after a forced GC where noted) |
| What the JS heap holds and what keeps it alive | V8 heap snapshots, analyzed with memlab (`@memlab/heap-analysis`: dominators, retainer paths, detached DOM) |
| DOM and layout counts | CDP `Memory.getDOMCounters` and `Performance.getMetrics`, plus in-page counts |
| Typing | 10 s at 20 keys/s into a visible composer during the load. Key-to-paint from Event Timing (`keydown` entries; keys under 16 ms counted as 8 ms, as the repo's full-conversation bench does), rAF gaps and long animation frames. |

**Variants**, all with the same build and workload:
- **A / L-base:** unmodified.
- **B:** `keyArray` patched (§4.2).
- **C / L-c:** `keyArray` patched, plus a prototype that unmounts a tool body 1.5 s after its panel collapses (§4.1).

The scripts and raw data are in the agent's workspace, not the repo: `memlab/` (`memsnap.mjs`, `typing.mjs`, `run*.sh`, `analyze-heap.mjs`, `snaps/*.json`, `typing.jsonl`, `heap/*.analysis.json`).

## 3. Results

### 3.1 Floor and idle panes

| State | Main renderer | GPU | AgentMux CEF total |
|---|---:|---:|---:|
| Empty window | 88 MB | 43 MB | 189 MB |
| 1 idle agent pane | 86 MB | 62 MB | 216 MB |
| 4 idle agent panes | 87 MB | 59 MB | 215 MB |
| Spare windows (when present) | +3 × 52–58 MB | | |

These are private working set. srv adds 30 MB at idle and 120–130 MB under load. The launcher is 4 MB.

### 3.2 Four panes, 30 rounds (about 120 turns), after forced GC

| | A: as shipped | B: `keyArray` fix | C: B + tool-body unmount | C, then reload |
|---|---:|---:|---:|---:|
| Main renderer | 850 MB | 760 MB | **407 MB** | **198 MB** |
| JS heap (used) | 246 MB | 225 MB | 76 MB | 31 MB |
| Blink heap (reserved) | 415 MB | 381 MB | 200 MB | 64 MB |
| DOM nodes, live and detached | 497k | 249k | 16k | 7k |
| Elements on the page | 137k | 123k | 5.3k | 2.6k |
| Layout objects | 248k | 222k | 12k | 4.7k |
| AgentMux's own total (Tower's figure) | 1,136 MB | 1,025 MB | 653 MB | — |

### 3.3 Live-like: 12 agents over 4 tabs (4/2/4/2), 40 rounds

| | L-base: as shipped | L-c: both fixes | L-c, then reload |
|---|---:|---:|---:|
| Main renderer after the launch turns | 1,347 MB | 193 MB | |
| Main renderer peak (snapshots without GC) | 3,100 MB (round 10) | 1,531 MB | |
| Main renderer after 40 rounds, forced GC | **2,318 MB** | **1,009 MB** | **338 MB** |
| JS heap (used) | 599 MB | 218 MB | 66 MB |
| Blink heap: reserved / live | 1,248 MB / — | 544 / 61 MB | 147 MB |
| V8 old space: reserved / live | — | 242 / 192 MB | |
| DOM nodes | 1.28M | 80k | 29k |
| Agent-pane elements (12 panes) | 276k (21k–25k each) | 28k (0.6k–5.3k each) | 9k |
| GPU process | 95 MB | 94 MB | 94 MB |
| AgentMux's own total | **2,638 MB** | **1,305 MB** | about 630 MB (CEF 512 + srv 120) |

The baseline's 1,347 MB after one launch turn per agent matches the operator's 1,389 MB renderer with 12 agent panes. It went far beyond that under the lab's compressed, tool-heavy load.

### 3.4 Typing under the 12-agent load

Each window is 10 s at 20 keys/s while every agent streams a turn.

| Window | Key-to-paint p50 / p95 / max (ms) | Keys over 50 ms | fps | Time in long frames |
|---|---|---:|---:|---:|
| L-base, launch turns streaming | 32 / 96 / 168 | 72 / 200 | 45.9 | 10.5% |
| L-base, round 1 | 24 / 64 / 104 | 20 | 52.4 | 4.3% |
| L-base, rounds 10–40 | 16–32 / 40–56 / 48–104 | 0–11 | 57.6–58.9 | 1–8% |
| L-c, idle | 24 / 24 / 40 | 0 | 60.1 | 0.7% |
| L-c, round 10 | 32 / 104 / 224 | 53 | 48.7 | 18.9% |
| L-c, rounds 20–40 | 16–32 / 24–32 / 48–72 | 0–1 | 59.6–59.9 | 0.5–1.3% |
| L-c after reload: idle / streaming | 16–24 / 24–32 / 32–40 | 0 | 60 | 0% |

**How to read this:**
- The bad windows line up with bursts (many turns starting at once), not with heap size.
- One window per point is noisy: L-c round 10 was the worst single window in either run.
- The memory fixes are not a typing fix by themselves.
- The clean result after a reload suggests that the DOM and heap left behind by a long session cost some frame time. It doesn't prove it.

## 4. The causes in detail

### 4.1 Tool bodies are never unmounted (`ToolBlock.tsx`)

```ts
// The panel's body (full tool output: diffs, file previews, Bash output,
// all highlighted with shiki) mounts the first time the panel opens and
// then stays — never unmounted on collapse ...
let bodyEverShown = false;
const bodyMounted = createMemo(() => bodyEverShown || (bodyEverShown = expanded()));
```

**Why every tool call ends up with a body**
- `rowDisclosure` opens a tool's panel automatically while it runs or awaits approval, and holds it open after finishing on screen.
- So in a live session every tool call's body is built, and then never released.
- The 2026-09-27 change that introduced "mount on first open" (`SPEC_AGENT_OPEN_LATENCY_2026_09_27.md` F4) helps restored history, where rows are never opened. Live rows were always opened.

**Measured**
- In one pane, 24,187 of 25,479 elements (95%) were inside collapsed `.agent-tool-overlay` subtrees, mostly `agent-preview-line` rows and their highlight spans.
- The panel is `inert`, `aria-hidden`, and collapsed by style, but its text is still laid out and shaped. The heap holds `LayoutText`, `ShapeResult` and `HarfBuzzRunGlyphData` in step with the element count.
- Each element also carries its SolidJS computations, about 8 JS objects each: in A's heap, 1.0M `Object`, 1.5M arrays and 680k closures.

**The prototype used in variant C (not a proposed patch)**
- It unmounts the body 1.5 s after the panel collapses and remounts it on open.
- It took per-pane elements from about 23k to 0.6k–5.3k.
- A real fix has to keep what the comment above protects: the collapse transition, and the virtualizer's measured heights. It also has to pay shiki's cost again on re-open (about 94 ms on first open per the F4 spec).
- Options:
  - Unmount when the row leaves the screen, or after the hold ends.
  - Keep only the most recent few bodies.
  - Render collapsed bodies as plain text, with highlighting only when open.

### 4.2 `keyArray` keeps the reconcile result array alive (`@solid-primitives/keyed` 1.5.3)

```js
function addNewItem(list, item, i, key) {
    createRoot(dispose => {
        ...
        prev.set(key, save);
        list[i] = save.mapped;   // ← closes over `list`, the result array of this pass
    });
}
```

**What the heap snapshot shows**
- The context `scope @308599` holds exactly `addNewItem`'s four parameters: `c` = key, `l` = index, `a` = item, `o` = the result array.
- That context is reachable from each slot's `dispose`, which is stored in the `prev` Map for as long as the row is in the list.
- The agent document re-partitions on almost every flush, so slots are created constantly. Each one pins a snapshot of the rows on screen at that moment.
- Rows that have since rolled off stay alive through those arrays, with their whole tool-preview subtrees: 118k detached text nodes, 74k spans and 14.5k preview lines in one snapshot.
- The retention is bounded by how long the oldest live slot lives, not unbounded. It is still large because the pinned rows carry entire tool bodies (§4.1).

**The fix** is to assign the slot's output to a local variable and write `list[i]` after `createRoot` returns, so the closure no longer captures `list`. It's a two-line change in the library (B), and it should go upstream. Until a release ships, AgentMux can carry it with `patch-package` or a vendored copy.

### 4.3 Blink's heap doesn't give memory back (Chromium)

**Reserved against live**, from the detailed dumps:

| | Blink heap reserved | Live objects | Empty |
|---|---:|---:|---:|
| C, 4 panes | 200 MB | 16.7 MB | 92% |
| L-c, 12 panes | 544 MB | 61 MB | 89% |

- V8 compacts and stays dense: 192 MB live of 242 MB.
- malloc keeps 30–45% slack.
- This is the effect VS Code measured in its agents window (microsoft/vscode#337444: 93% fragmentation, about 520 MiB reclaimed by recreating the renderer).
- Here, reloading the page in the same renderer process returned it: 1,009 → 338 MB, and 407 → 198 MB.

**Why it matters for uptime**
- Every burst (a big turn, a restore, a tab full of tool output) raises the high-water mark, and the renderer never comes back down.
- That fits the operator's instance reading 1.4 GB after 26 hours with ordinary use.

### 4.4 Hidden tabs keep everything

- Agent panes in a background tab stay mounted and keep building DOM as their agents stream.
- Measured: 23k and 32k elements in the two hidden panes, the same as visible ones. The renderer didn't change when the tab was shown (788 → 795 MB).
- The operator's instance has 12 agent panes in 4 tabs, so most of its cost is in tabs that aren't on screen.

### 4.5 Smaller items found along the way

- **Spare windows:** `POOL_TARGET_SIZE = 2` window renderers plus 1 pane-pool renderer, measured at 52–58 MB each when warm. That's about 165 MB of the 500 MB budget for windows that may never be used.
- **Tool output in the stores:** 97 MB of strings in L-c's heap, mostly tool output still held by 12 panes' stores. With fixes #1 and #2 in, extending the existing tool-result unloading (`tool-result-unload.ts`, ≥ 8 KB collapsed results) to cover finished, collapsed rows would cut most of it.
- **The commit attribution log can't separate instances.** srv's 30-second `commit attribution` line (`sysinfo.rs`, `classify_commit_attribution`) adds every process whose name starts with "agentmux" into `agentmux_mb`, so it mixes all instances on the machine. Today it reads about 7 GB, which isn't this instance's number. It should use the instance's own process tree, as Tower does.
- **A browser process under the lab tree:** in one baseline run a CEF process launched 9 `msedge.exe` processes (100 MB). That's probably a URL handed to the system browser. It wasn't reproduced and isn't AgentMux's memory, but it lands under AgentMux's tree, so Tower may count it in the AgentMux row.
- **The GPU process** stayed at 81–98 MB in every lab run, against 394 MB in the operator's instance. The difference is unexplained. Likely contributors the lab didn't have: 3 browser panes, 2 media panes, 4 Sysinfo charts, and 26 hours of compositor churn at 2689×1470.

## 5. What this means for the 500 MB target

**Measured in the lab:**

| | AgentMux total |
|---|---:|
| 12 agents, as shipped | 2.6 GB |
| 12 agents, with #1 and #2 fixed | 1.3 GB |
| 12 agents, with both fixed, after a renderer recycle | about 630 MB |

The remaining distance to 500 MB:
- the spare windows (about 165 MB when warm);
- the renderer's post-reload floor for 12 restored panes (338 MB);
- the hidden-tab panes inside that floor.

**Priority order, by measured effect per unit of work:**

1. **Unmount tool bodies** (§4.1). This is the largest single cut: −53% to −56% of the renderer with #2. Choose the policy that keeps the collapse transition and height measurement correct, and test re-open latency.
2. **Fix the `keyArray` capture** (§4.2). A two-line library patch with a clear heap-snapshot proof; send it upstream.
3. **Recycle the renderer when idle and over budget** (§4.3). It reclaims 60–70% after a long session. The pane restore it relies on already works (the lab reloads restored all 12 panes). The work is choosing the moment (idle, unfocused, over threshold) and making it invisible.
4. **Hibernate agent panes in hidden tabs** (§4.4). Stop building DOM for a pane nobody can see, and rebuild from the store or transcript when its tab is shown. This is what keeps the cost flat as tabs are added.
5. **Shrink the spare-window pool** (§4.5): one window, or create it lazily.
6. **Unload tool output from finished rows** (§4.5).
7. **Fix the commit attribution log** (§4.5), so field data can be trusted.

Items 1, 2 and 4 also lower the peaks (the baseline reached 3.1 GB in round 10), and peaks are what drive #3.

## 6. Caveats

- **The workload is synthetic.** The lab compresses hours of agent work into minutes and is more tool-heavy than most real sessions. Absolute sizes are higher than a real hour would give, but the attribution (which allocator, which component, what retains it) doesn't depend on that.
- **Windows only.** macOS and Linux weren't measured. Blink's handling of empty pages differs there; on Windows, freed regions aren't discarded.
- **Variant C's tool-body unmount is a prototype.** It wasn't checked for visual or measurement regressions.
- **Mid-run snapshots aren't comparable.** They were taken without forced GC, so they include garbage; compare the "forced GC" rows.
- **One typing window per point.** Treat §3.4 as indicative.
