# SPEC — Commit-attribution correction + genuine AgentMux memory-hygiene fixes

**Date:** 2026-07-02
**Type:** Correction + implementation spec
**Status:** Ready to schedule
**Owner:** asaf
**Scope:** Corrects `SPEC_MEMORY_ANALYSIS_2026_06_26.md`'s attribution; specifies the AgentMux-side
memory fixes that survive the correction. See `docs/retro/retro-commit-charge-pagefile-growth-2026-07-02.md`
for the measured evidence.

> **Headline:** The pagefile-growth symptom is driven by ~43 GB of committed page-file-backed shared
> memory that is **owned by protected/system processes, not AgentMux**. AgentMux's total commit is ~6–8 GB
> and its host is memory-stable. The prior spec's "each claude agent commits ~10 GB" is a measurement
> error (that's *reserved* VA; real commit is ~1 GB). This spec (A) records the correction so it isn't
> re-derived, and (B) specifies the genuine, bounded AgentMux fixes.

---

## A. Correction to SPEC_MEMORY_ANALYSIS_2026_06_26

### A.1 The error
That spec (lines 8-15, 25, 36-46) computed the commit budget as "4 agents × 10.5 GB = 42 GB," using each
`claude.exe`'s **`VirtualMemorySize`** (~10 GB). On Windows, commit charge is **`PrivateUsage`** (perf
counter "Private Bytes" / VMMap "Committed"), which for a live `claude.exe` is **~1.05 GB** — the 10 GB
is V8's reserved-but-uncommitted address space (pointer-compression cage + PartitionAlloc reservation).
The spec's own footnote (lines 32-34) correctly says CEF's huge VA "does NOT count against the commit
limit," but the identical conflation was applied to the agent row.

### A.2 The corrected ledger (measured 2026-07-02)
`commit = Σ process-private (6 GB) + Σ kernel/driver-committed (~43 GB) + kernel pools (2.6 GB)`.
Total = 52–54.5 GB (rising during measurement). Agents contribute ~1 GB each (~5 GB), the CEF fleet
~1–2 GB, the CEF GPU process 0.6 GB. Elevated VMMap **ruled out DWM** (≤2.1 GB total, 4 MB private). No
user-mode process holds the ~43 GB, so it is **kernel/driver-committed pagefile-backed system memory**
charged to the System process — most plausibly the **GPU driver stack (dxgkrnl + vendor KMD)** committing
system memory for WDDM paging/context/GPU-VA structures, **provoked by AgentMux's per-pane GPU-accelerated
renderers** (`commands/mod.rs:25-30`). This explains the user's observation that commit tracks AgentMux
being open despite AgentMux's own private bytes staying small.

### A.3 What stays valid from the prior spec
The *defensive* remediations remain worthwhile even though the attribution was wrong, because commit
*pressure* is real (88% of limit): **P0 commit-aware turn scheduler** and **P0 low-memory
`CreateMemoryResourceNotification` handler** (prior spec §"P0"). They should gate on
`GlobalMemoryStatusEx.ullAvailPageFile` as specified — that logic is correct regardless of *who* consumes
the commit. **Reframe their justification** from "agents commit 10 GB each" to "the system can reach
commit exhaustion from mixed load; AgentMux must not spawn a turn it can't back."

### A.4 Confirming test — `--disable-gpu` A/B (decisive, do this first)
AgentMux already passes `--disable-gpu` at startup when commit-free < 512 MB (`agentmux-cef/src/app.rs:615-626`).
Launch AgentMux with GPU disabled and compare system `Memory\Committed Bytes` at equivalent pane/window
counts vs a GPU-enabled run:
- **Commit drops ~tens of GB** → confirmed: the 43 GB is GPU-driver system-memory commit provoked by
  AgentMux's renderers. Mitigation: force `--disable-gpu` on commit-tight machines, and/or reduce renderer
  count (B.5). Also update the GPU driver (context-teardown commit leaks are a known vendor-driver bug).
- **Commit unchanged** → not the GPU driver; re-attribute upstream (elevated VMMap of `System` isn't
  possible for kernel memory — use a kernel-debugger `!vm`/`!memusage` or poolmon for pool tags, and
  audit non-AgentMux load: Parsec, Traktor, 7× WebView2).

---

## B. Genuine AgentMux memory fixes (bounded, worth doing)

These are real and scale with long-session churn. None alone explains the 43 GB, but they are the
AgentMux-owned growth this investigation surfaced.

### B.1 — WPS broker `persist_map` never pruned on block/shell close  *(confirmed leak, private heap)*
`agentmux-srv/src/backend/wps.rs:172` — `persist_map: HashMap<PersistKey, PersistEventWrap>`, keyed by
`(event, scope)` with scopes like `block:<id>` / `shell:<id>`. Entries are inserted at `:379`
(`.entry(key).or_insert_with(...)`); the per-key event Vec is capped (≤ `MAX_PERSIST=4096`, `:388`) but
the **key set is never pruned** — no `remove`/`retain`/`clear` exists. `DeleteBlock`
(`sagas/delete_block.rs`) kills the controller but never asks the broker to drop the block's/shell's
persist rings. **Fix:** on block delete (and shell teardown), purge `persist_map` (and any per-scope
replay state) for `block:<id>` / `shell:<id>`. Scales with cumulative pane/shell count for the process
lifetime.

### B.2 — Per-connection unbounded WS egress channels  *(backpressure gap)*
`agentmux-srv/src/backend/eventbus.rs:84-85` — two `unbounded_channel`s per WebSocket connection, drained
by a single `socket.send(...).await` loop (`server/websocket.rs:196-229`). If a client's TCP stalls
(hidden/backgrounded renderer, slow pane), `send().await` blocks, the loop stops draining, and the
unbounded senders queue without limit. **Fix:** bound these channels (or add a high-watermark drop/coalesce
for the `term`/`waveobj:update` lanes) so a slow consumer can't grow sidecar commit. Related unbounded
egress: `messagebus.rs:123`, `rpc/engine.rs:206`.

### B.3 — `SubagentWatcher` per-session event Vec + session map, no cap
`agentmux-srv/src/backend/subagent_watcher.rs:77` — `SubagentState.events: Vec<SubagentEvent>` pushed at
`:419,:599` with no truncation; `sessions: HashMap` (`:91`) grows one entry per Claude session id and
`unwatch_agent` (`:259-263`) doesn't remove from it. **Fix:** cap the event Vec (ring) and remove the
`sessions` entry on unwatch.

### B.4 — Browser-pane teardown determinism  *(reliability, shared sections)*
`agentmux-cef/src/browser_panes.rs:365-387` skips `close_browser` and relies on a possibly-non-firing
`on_before_close`; `sagas/delete_block.rs` has no browser-specific teardown. **Fix:** drive
`DrainBrowserPaneByLabel` (or equivalent CEF Browser destroy) from the `DeleteBlock` saga so a browser
block deleted while unmounted still tears down its CEF Browser + shared sections deterministically. Pair
with the SolidJS `onCleanup` path already in `browser-view.tsx:390-427` (which handles the mounted case).

### B.5 — Renderers are never reclaimed on pane close  *(TOP priority — empirically confirmed)*
**Measured 2026-07-02:** closing panes down to a single open pane left **all 5 renderer processes alive
(identical PIDs)** and system commit unchanged (54.61 → 55.04 GB). So AgentMux's renderers — and the
GPU-driver system-memory commit they provoke — **persist regardless of pane count**; a session's commit
only holds or grows, never shrinks, until full restart. Root: `agentmux-cef/src/commands/mod.rs:25-30`
gives every window/pane its own `RequestContext` → its own renderer, and the warm pool
(`window_pool.rs:149,1328`) / teardown path keeps them alive after close (see B.4 — `browser_panes.rs`
relies on a possibly-non-firing `on_before_close`). **Fixes, in order:** (a) make pane close deterministically
destroy that pane's renderer/browser (not pool it) when the live count exceeds the pool target; (b) add
pool eviction under commit pressure (tie to the P0 low-memory handler); (c) share a `RequestContext`
across panes that don't need isolation to cut the renderer count; (d) surface live-renderer count +
per-renderer commit in Swarm. If closing a pane actually freed its renderer, users could reclaim memory
without restarting — today they cannot. This bounds AgentMux's contribution but does not, by itself,
address the kernel/driver-owned commit (confirm/mitigate via the `--disable-gpu` A/B, A.4).

### B.6 — Agent output virtualization (prior P2, still valid)
`SPEC_MEMORY_ANALYSIS_2026_06_26.md` §"P2" — virtual-scroll the agent output pane so the renderer DOM
doesn't hold the full session (measured 23k nodes / ~15.8 MB in one block). Caps renderer working set
(physical), not the commit gap, but is real long-session hygiene.

---

## C. Test / verification plan
- **Attribution (A.4):** elevated VMMap of `dwm.exe`/`System`/`parsecd`/`msedgewebview2`; record each
  one's Committed + Shareable-committed; confirm the ~43 GB owner.
- **B.1:** open N agent panes + shells, close them, assert `persist_map.len()` returns to baseline
  (add a debug counter or test hook); sidecar Private Bytes returns to baseline after close.
- **B.2:** stall a WS consumer (pause a renderer), flood `term` output, assert bounded memory / documented
  drop behavior rather than unbounded growth.
- **B.4:** delete a browser block on an inactive tab; assert the CEF Browser + child HWND are destroyed
  (no leaked `agentmux-*.exe` renderer, no orphaned browser in `state.browsers`).
- **Commit ledger regression:** a scripted `Get-Counter \Memory\Committed Bytes` + `Process(_Total)\Private
  Bytes` snapshot before/after a long session, to catch any AgentMux-owned commit growth early.

## D. Sources
- Retro with full measurements: `docs/retro/retro-commit-charge-pagefile-growth-2026-07-02.md`.
- Corrected: `docs/specs/SPEC_MEMORY_ANALYSIS_2026_06_26.md` (lines 8-15,25,32-46,84-120,143-153).
- Code: `wps.rs:172,379,388`; `eventbus.rs:84-85`; `server/websocket.rs:196-229`;
  `subagent_watcher.rs:77,91,259-263,419,599`; `browser_panes.rs:365-387`; `sagas/delete_block.rs`;
  `commands/mod.rs:25-30`; `commands/window_pool.rs:149,1328`; `browser-view.tsx:390-427`.
