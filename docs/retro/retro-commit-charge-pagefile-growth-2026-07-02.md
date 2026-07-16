# Retro — "Pagefile growing too large" traced to ~43 GB of unattributed committed shared memory (not AgentMux)

**Date:** 2026-07-02
**Severity:** Medium (no crash this session; commit at 88–90% of limit, pagefile auto-grew to 27 GB — a
degraded state that *precedes* the OOM kills seen in prior incidents)
**Status:** Diagnosed; root cause is largely **outside AgentMux** (system/driver committed shared
memory). Corrects a measurement error in `SPEC_MEMORY_ANALYSIS_2026_06_26.md`. AgentMux-side
follow-ups captured in `SPEC_MEMORY_COMMIT_ATTRIBUTION_CORRECTION_2026_07_02.md`.
**2026-07-16 update:** §6.1's open question (driver *leak* vs live per-context commit) is resolved by a
controlled restart experiment — the commit is **live, not leaked**, tied to the renderer fleet, and
released cleanly on teardown (~43.6 GB reclaimed). See
`docs/retro/retro-commit-restart-reclaim-2026-07-16.md`; recommendation §6.2 (GPU driver update) is
thereby de-prioritized, and §B.5 renderer reclaim is promoted to the primary fix.
**Reporter:** asaf ("the pagefile is growing too large … architectural problem in AgentMux")
**Component:** Windows commit/pagefile behavior; AgentMux CEF fleet + agent subprocesses (contributors,
not the dominant cause)

---

## 1. What happened

The Windows pagefile (`C:\pagefile.sys`) had grown to **27.2 GB on disk**, and the user attributed it to
an architectural memory problem in AgentMux. Live measurement reframed the symptom and largely
exonerated AgentMux.

## 2. What the pagefile growth actually is

- **Physical RAM: 31.9 GB total, 22.5 GB free.** Not a physical-memory shortage.
- **Commit charge: 52 GB used / 64.9 GB limit (~88%)**, and the **commit limit grew from 58.6 → 64.9 GB
  during the ~40-minute investigation** — i.e. Windows was *actively expanding the pagefile* to keep the
  limit above rising commit.
- **`pagefile.sys` = 27.2 GB allocated, but only 0.1 GB actually used.** Windows must reserve pagefile
  *backing* for committed pages even when they're demand-zero and never written. So the "huge pagefile"
  is a **symptom of high commit charge**, not of memory actually being paged out.

The pages driving commit are **committed-but-never-touched**: they consume commit charge (forcing the
limit, and thus the pagefile, to grow) while consuming ~0 physical RAM and ~0 pagefile writes.

## 3. Where the 52 GB of commit actually lives (measured)

| Category | Committed | How measured |
|---|---|---|
| All process **private** bytes | 6.0–6.2 GB | `Process(_Total)\Private Bytes`; cross-checked via `Win32_Process.PrivatePageCount` (incl. protected procs) |
| Kernel pools (paged + nonpaged) | 2.6 GB | `Memory\Pool Paged/Nonpaged Bytes` |
| System driver total | 0.04 GB | `Memory\System Driver Total Bytes` |
| agentmux CEF **GPU process** | 0.6 GB (74 MB shareable) | VMMap PID (gpu-process) |
| Each **claude** agent | **1.05 GB** (776 MB private) | VMMap PID (claude.exe) |
| **GPU / WDDM** system commit | 0.9 GB | `GPU Process Memory(*)\Total Committed` |
| Virtualization VM (`vmmem`/WSL/Docker/Hyper-V) | **0** | process + service scan |
| **`dwm.exe`** (elevated VMMap) | **≤2.1 GB total, 4 MB private** | VMMap PID 2800 (elevated) — **ruled out** |
| **Committed total** | **52–54.5 GB (rising)** | `Memory\Committed Bytes` |
| **→ Unattributed to any user process** | **~43 GB** | by subtraction |

Two independent methods put **total user-process commit at ~6 GB**. By the Windows commit accounting
identity (`commit = Σ process-private + Σ pagefile-backed shared sections + kernel commit`), the ~43 GB
residual is **committed page-file-backed memory**. VMMap of every plausible owner ruled them out: the CEF
GPU process (74 MB shareable), a claude agent (2.5 MB shareable), and — with **elevation** — **`dwm.exe`
(≤2.1 GB total, 4 MB private, ~2 GB shareable *reserved*)**. No user-mode process holds it. The commit is
therefore **kernel/driver-committed memory attributed to the System process** (PID 4, no user-mode address
space for VMMap to inspect) — i.e. **a driver commits pagefile-backed system memory**, not AgentMux.

## 4. Root-cause analysis

### Primary — the ~43 GB is kernel/GPU-driver-committed system memory, *provoked by* AgentMux's GPU workload
Every AgentMux-owned process was measured small: host commit flat ~55 MB (prior 30.5 h data,
`SPEC_MEMORY_ANALYSIS_2026_06_26.md:50-59`, consistent with this session), CEF GPU process 0.6 GB, each
agent ~1 GB, all CEF renderers/utilities tens–hundreds of MB private. **AgentMux's entire footprint is
~6–8 GB of commit.** Elevated VMMap ruled out DWM (≤2 GB). No user-mode process holds the 43 GB, so it is
**kernel/driver-committed pagefile-backed system memory** charged to the System process.

**Why it correlates with AgentMux being open (the user's key observation):** AgentMux's per-pane isolated
`RequestContext` design (`agentmux-cef/src/commands/mod.rs:25-30`) gives every window/pane its own
**GPU-accelerated Chromium renderer**. Each renderer/context makes the GPU driver stack (dxgkrnl + vendor
KMD) allocate kernel-mode, pagefile-backed system memory (WDDM paging buffers, context save areas, GART/
GPU-VA structures). That commit is charged to **System, not AgentMux** — explaining small AgentMux private
bytes, low GPU *dedicated* memory (0.9 GB — this is system-memory commit, not VRAM), demand-zero
(untouched) pages, and commit that tracks AgentMux's lifetime. Whether the driver *leaks* (fails to
reclaim on context/renderer teardown) or merely commits a large per-context floor is the open question
(§6.1); the per-pane renderer multiplier makes either scale with pane/window count.

### Contributing (AgentMux, real but minor) — per-window renderer design multiplies the Chromium floor
`agentmux-cef/src/commands/mod.rs:25-30`: every window and every browser pane gets its own isolated
`RequestContext` → its own Chromium renderer process, each with its own GPU transfer/discardable/
shared-image buffers (page-file-backed shared sections). Bounded by warm-pool caps (~4 browsers → ~10
subprocesses observed), so it sets a per-browser floor rather than growing unbounded — but it *is* the
knob that scales AgentMux's shared-memory commit with pane/window count.

### Contributing (AgentMux) — browser-pane teardown can strand CEF browsers
`agentmux-cef/src/browser_panes.rs:365-387` deliberately skips `close_browser` on Windows and relies on
`on_before_close` possibly not firing; and the `DeleteBlock` saga (`sagas/delete_block.rs`) has no
browser-specific teardown, so a browser block deleted while its view is unmounted (inactive tab / not
rendered) can orphan the CEF Browser (and its shared sections). Not observed accumulating in this
snapshot (only one instance, ~10 subprocesses), but a real reliability gap.

## 5. The measurement error this corrects

`SPEC_MEMORY_ANALYSIS_2026_06_26.md` (TL;DR, lines 8-15, 25, 36-46) concluded the OOM was "caused
entirely by Claude Code agent processes, each of which commits ~10 GB." **VMMap disproves this:** a live
`claude.exe` shows **1.05 GB committed** with **10.5 GB merely *reserved*** (V8's pointer-compression
sandbox / partition-allocator address-space reservation). The spec measured `VirtualMemorySize`
(reserved VA), not commit — its own footnote (lines 32-34) even states CEF's huge VA "does NOT count
against the commit limit," but the same conflation was then applied to the agent row. So "4 agents ×
10.5 GB = 42 GB commit" is arithmetic on the wrong counter; real agent commit is ~1 GB each (~5 GB for
5 agents). The prior spec's *remediations* (commit-aware scheduler, low-memory handler) are still
worthwhile defensively, but its attribution of the commit budget is wrong.

## 6. Recommended actions (ranked)

1. **Confirm the GPU-driver hypothesis with the `--disable-gpu` A/B test** (decisive, AgentMux already has
   the switch at `app.rs:615-626`). Launch AgentMux with GPU disabled and compare system `Committed Bytes`
   at equivalent pane/window counts against a GPU-enabled run. If commit drops by ~tens of GB → confirmed
   GPU-driver system-memory commit provoked by AgentMux's renderers. If unchanged → look upstream (a
   non-GPU driver / non-AgentMux load). *(This also doubles as an immediate mitigation on commit-tight
   machines.)*
2. **Update the GPU driver** — a driver that leaks committed system memory on context create/destroy is a
   known failure mode; a vendor driver update often fixes it. Cross-check with a second machine / different
   GPU vendor to see if the 43 GB reproduces.
3. **Reclaim now (user action): only a full AgentMux restart works — closing panes does NOT.**
   Empirically verified 2026-07-02: closing panes down to a single open pane left **all 5 renderer
   processes alive (identical PIDs)** and commit **unchanged** (54.61 → 55.04 GB). Renderers are held in
   the warm pool / not torn down on pane close, so per-pane close reclaims nothing. Restarting AgentMux
   tears down the renderer fleet + shared GPU process and drops the commit; the pagefile shrinks after.
   Do **not** shrink/cap the pagefile manually while commit is ~88% of limit — that risks hard failures.
4. **Reconsider the per-pane RequestContext/renderer design** (`commands/mod.rs:25-30`) or bound the
   live GPU-accelerated-browser count — this is the AgentMux-side multiplier for the driver commit. Sharing
   a `RequestContext` (fewer renderers) or capping warm pools on low-commit machines directly reduces it.
5. **AgentMux hygiene fixes worth doing regardless** (see the correction spec): prune the WPS broker
   `persist_map` on block/shell close (`wps.rs:172`), bound the per-connection WS egress channels
   (`eventbus.rs:84`), and make browser-pane teardown deterministic in the `DeleteBlock` saga rather than
   relying on `on_before_close`.

## 7. Prevention
- **Measure commit, not virtual.** Any future memory analysis must use `PrivateUsage` / VMMap "Committed",
  never `VirtualMemorySize` — the two differ by ~10× for V8/Chromium and caused the prior misdiagnosis.
- **Attribute before architecting.** The reported "AgentMux architectural problem" was ~85% not AgentMux;
  a 20-minute commit-ledger measurement (this retro) would have reframed the earlier incident too.
- The existing `mem_heartbeat` (every 20 s) already logs host commit — extend it to log **system** commit
  + top-N process private commit so the ledger is captured continuously, not reconstructed post-hoc.

## 8. References
- Measurements: this session (`Get-Counter \Memory\Committed Bytes` = 52 GB; `Process(_Total)\Private
  Bytes` = 6 GB; VMMap of CEF gpu-process = 0.6 GB, claude = 1.05 GB; `GPU Process Memory` = 0.9 GB;
  no `vmmem`).
- Code: `agentmux-cef/src/commands/mod.rs:25-30`, `agentmux-cef/src/browser_panes.rs:365-387`,
  `agentmux-srv/src/backend/wps.rs:172`, `agentmux-srv/src/backend/eventbus.rs:84`.
- Prior art (corrected/built-on): `docs/specs/SPEC_MEMORY_ANALYSIS_2026_06_26.md`,
  `docs/specs/SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md`, `docs/analysis/oom-filestore-cache.md`.
