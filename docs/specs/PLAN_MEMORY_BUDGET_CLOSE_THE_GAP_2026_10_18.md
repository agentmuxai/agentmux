# Plan: close the gap to the 500 MB memory budget

**Date:** 2026-10-18
**Status:** active. Shipped: the instance-only commit attribution log (#5083, track E); the chrome zoom across a reload (#5084, track B); Reload Window (Free Memory), Phase B of the recycle spec (this PR, track B). In review: the memory lab (#5085, track A). Left: the lab's CI gate (A), the recycle governor, Phase C (B), terminal WebGL and per-terminal renderer memory (D1), the Windows GPU process (D2), srv's append churn and allocator retention (E), hibernation (C).
**Coordinator:** Camper@narko
**Builds on:** `docs/reports/REPORT_MEMORY_FOOTPRINT_500MB_BUDGET_2026_10_12.md` (the budget), `docs/reports/REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md` (the measured causes), `docs/specs/PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md` (items 1–3, 7 and 8 have shipped: #4751, #4756, #4767, #4847, #4858).

## 1. Where it stands

The target is AgentMux's own processes (CEF host, GPU, utility, renderers, srv, launcher) at **≤ 500 MB private working set** (Tower's metric), at any pane count. Agent CLIs have their own budget (budget report §7).

Measured on narko, 2026-10-18, two instances in daily use:

| | v0.59.20 (report) | 0.59.30 | 0.59.31 | Budget |
|---|---:|---:|---:|---:|
| Window renderers | ~1,530 | 719 | 776 | ≤ 310 (shell 80 + visible panes 150 + pool 50, plus margin) |
| GPU process | 394 | 165 | 89 | ≤ 80 |
| srv | 124 | 111 | 96 | ≤ 90 |
| Browser, utility, launcher | ~50 | 71 | 47 | ~50 |
| **Total** | **~2,050** | **1,066** | **1,008** | **≤ 500** |

The rest of the gap is the renderer's uptime growth (Blink's heap keeps freed pages) and hidden panes that cost as much as visible ones.

## 2. Tracks

One owner per track, and one owner per file. A change to a file in another track's list goes through its owner.

| Track | Work | Owner | Files it owns |
|---|---|---|---|
| A | **Measurement and a CI gate.** The lab workload from the root-cause report (§2) as a repeatable script: production build, scripted fake-claude agents, private working set and commit per process, memory-infra dump, heap snapshot. Then a soak job that fails a PR when the renderer or the total grows past a set line. | Loap-3@narko | `scripts/perf/`, `scripts/benchmarks/`, `crates/procstats/`, a new workflow under `.github/workflows/`, a report under `docs/reports/` |
| B | **Renderer recycling** (fix plan item 5): Phases B and C of `SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md` (Loap-3's spec; Phase A shipped). When a window has been idle for a while and its renderer is over budget, reload its page, with every pane restoring from srv so the user sees nothing change. | Camper@narko, reviewed by Loap-3 | `crates/cef/src/memory_heartbeat.rs`, `crates/cef/src/memory_pressure.rs`, a new recycling module in `crates/cef/src/`, the frontend's reload-and-restore path |
| D1 | **Terminal WebGL.** WebGL only for visible terminals, `loseContext()` on dispose, a 4096 px atlas page cap, and staying under Chromium's 16 contexts per page. The same on every OS. | Opaz@charlie (Linux), once cleared by the operator | `frontend/app/view/term/termwrap.ts` and its WebGL addon setup |
| D2 | **The GPU process on Windows** (fix plan item 11). Measure what it holds (the disabled `CalculateNativeWinOcclusion`, Sysinfo and Tower charts, laid-out hidden tabs, browser and media panes), then cut it toward 80 MB. | Posa@Area54 (Windows), once cleared | `crates/cef/src/app/mod.rs` (command-line switches), `frontend/app/view/sysinfo/` |
| E | **srv ≤ 90 MB, and correct attribution** (fix plan item 4). Find what srv holds under load (MPS broker retention, per-agent buffers, caches) and bound it; make `agentmux_mb` count only this instance's processes. | Maricon@charlie (Linux; Windows confirmation from D2's agent or Camper) | `crates/srv/` except the agent pane's history paging, which C will need |
| C | **Hibernate panes in hidden tabs** (fix plan item 6). Hidden panes beyond the most recently used few unmount to a stub; srv keeps their state and they restore on show. | after B lands and Naki-2's hidden-pane fix merges; owner to be named | the agent pane's live feed and virtual list, srv history paging |

## 3. Rules for every track

- **Measure in your own build, never in an instance the operator uses.** Use a production build (`task package`) for numbers; a `task dev` build inflates them. Give it its own data folder and CDP port.
- **Every PR states its saving:** before and after, same workload, same machine, private working set and commit for each process group. Until track A lands, use the method in the root-cause report §2.
- **Windows is the reference platform for the headline number.** A track measured on another OS reports its own before and after, and says which OS.
- **Nothing that only moves the number:** no `EmptyWorkingSet` or working-set trimming, no V8 heap caps, no flags that trade rendering correctness for memory (budget report §5).
- **The user must not notice.** A reload, a hibernated pane or a dropped WebGL context restores exactly what was there: scroll position, composer text, selection, a running turn.
- Post a heads-up in the repository's coordination channel before touching a file outside your list.

## 4. Order

1. A, B, D1, D2 and E start now, in parallel (D1 and D2 once their operators clear them).
2. Each track lands in small PRs with their measured savings.
3. C starts once B's restore path exists, so the two share one way of rebuilding a pane from srv.
4. When all five have landed, re-measure both instance shapes (one window with about 10 agent panes, and four tabs with 12) and update this plan with the result.
