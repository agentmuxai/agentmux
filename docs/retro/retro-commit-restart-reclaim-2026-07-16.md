# Retro follow-up — restart experiment resolves the 2026-07-02 open question: the ~40 GB driver commit is live (not leaked), tied to AgentMux's renderer fleet, and released cleanly on teardown

**Date:** 2026-07-16
**Severity:** Medium (commit hit 97% of limit — red in the status bar — during a normal multi-day session; no crash)
**Status:** Measured and resolved (as attribution). The remaining work item is the already-specified renderer-reclaim fix (`SPEC_MEMORY_COMMIT_ATTRIBUTION_CORRECTION_2026_07_02.md` §B.5), which this data promotes to the single highest-value memory fix.
**Reporter:** asaf ("we are at red levels in the currently running agentmux instance … why does it grow? can we stop it somehow?")
**Follows:** `docs/retro/retro-commit-charge-pagefile-growth-2026-07-02.md` — answers its §6.1 open question ("whether the driver *leaks* or merely commits a large per-context floor") with a controlled measurement.

---

## 1. What happened

The status bar's `PF` gauge was red. Live measurement showed the same shape as the July 2 incident: commit charge at **74 GB of a 76.2 GB limit (97%)** — later **79.5 of 85.2 GB** as Windows expanded the pagefile *during the investigation* — while physical RAM had ~30 GB free and the pagefile had under 200 MB actually written. High commit, near-zero paging: commit-limit pressure, not memory-in-use pressure.

The commit ledger reproduced the July 2 signature, with one difference — much heavier non-AgentMux load this time:

| Bucket | 2026-07-16 | 2026-07-02 (retro) |
|---|---|---|
| All user-process private bytes | 29–31.6 GB | ~6 GB |
| … of which AgentMux family (all `agentmux*` + `claude` procs) | **~6 GB** | ~6–8 GB |
| … of which other load (vmmemWSL 4.5, parsecd ×2 = 2.8, OBS, Chrome, Discord, …) | ~23 GB | ~0 |
| Kernel pools | 5.25 GB | 2.6 GB |
| **Unattributed (kernel/driver, charged to System)** | **~40 GB** | ~43 GB |

## 2. The experiment

The July 2 retro asserted (from a single observation) that only a full AgentMux restart reclaims the commit, but left open whether the driver commit was *leaked* (orphaned — only a reboot would reclaim it) or *live* (tied to AgentMux's GPU contexts — process teardown would release it). The user's direct challenge — "what if I close agentmux and the PF stays the same?" — is exactly the falsifying question, so we ran it:

- A commit logger (5 s samples: system commit used/limit, Σ process private, per-family private for `agentmux*`/`claude`/`parsecd`/`vmmem`) was started as a **scheduled task**, outside AgentMux's process tree, so it survived the restart.
- The user closed AgentMux fully, waited ~1 minute, reopened it.
- Raw data archived at `C:\Users\asafe\commit-log-2026-07-16-restart-experiment.csv` (machine: claudius, Win11, 62 GB RAM, dual GPU stacks — NVIDIA + Radeon both resident).

## 3. Result — decisive

| t | State | Commit used / limit | AgentMux procs (private) |
|---|---|---|---|
| 03:31:57 | fully up, steady | **79.5 / 85.2 GB** (93%) | 38 (4.4 GB) + 3 claude (1.9 GB) |
| 03:33:05 | closing, 1 proc left | 69.5 / 85.2 | 1 (0.1 GB) |
| 03:33:15 | fully closed | **35.9 / 75.2 GB** | 0 |
| 03:34:53 | fresh instance up | **38.5 / 75.2 GB** (51%) | 14 (0.9 GB) + agents restarting |

- Closing AgentMux released **~43.6 GB of commit**. Its own processes' private bytes were only **~6–7 GB** of that; the other **~36.5 GB was the "unattributed" System-charged driver commit**, which vanished the moment the process fleet died.
- Windows also **shrank the commit limit 85.2 → 75.2 GB while running** (trimmed the system-managed pagefile without a reboot) once commit fell.
- The fresh instance restarted at a sane ~38.5 GB total system commit and began the slow climb again as agents spun up.

## 4. Conclusions

1. **§6.1 of the July 2 retro is answered: the driver does NOT leak.** The ~36–43 GB is live kernel/GPU-driver state (WDDM paging buffers, context/GPU-VA structures) held *on behalf of* AgentMux's live renderer processes and released cleanly at teardown. No reboot is ever needed; GPU-driver updates are not the fix.
2. **Rule of thumb on this machine: ~1 GB of invisible, System-charged driver commit per AgentMux renderer process**, on top of the renderer's own visible private bytes. 38 processes ≈ 36 GB kernel-side. This is why AgentMux "looks small" in Task Manager (sum of private bytes ~6 GB) while being the largest effective commit consumer on the box.
3. **Why it only ever grows during a session:** confirmed again — renderers are pooled/never reclaimed on pane close (§B.5 of the correction spec, empirically re-verified July 2), so a session's renderer count — and therefore its driver commit — ratchets up with pane/window churn and never comes back down until restart.
4. **Restart is a legitimate relief valve** (93% → 51% here), but it's a reset, not a fix.

## 5. Actions

1. **Implement §B.5 of `SPEC_MEMORY_COMMIT_ATTRIBUTION_CORRECTION_2026_07_02.md`** — deterministic renderer/browser destroy on pane close when live count exceeds the pool target, plus pool **eviction** (not just spawn-refusal, which landed in `window_pool.rs:262`) under commit pressure. This data upgrades B.5 from "top priority among hygiene fixes" to *the* memory fix: every reclaimed renderer returns ~1 GB of system commit that no gauge attributes to us.
2. **Fix the pressure-threshold mismatch** (found during this investigation): the status bar goes red at >95% commit *ratio* (`SystemStats.tsx:commitColor`), but `agentmux-cef/src/memory_pressure.rs` enters Warn/Critical on *absolute* free (< 1 GB / < 512 MB). On this 75–85 GB limit, the UI was red while the backend still reported `Normal` — so none of the backend guards (pool spawn-refusal, etc.) engage until far past what the user sees as red. Thresholds should be ratio-aware (or take max(ratio, absolute)).
3. **Explicitly rejected: a manual `--disable-gpu` override.** Owner policy (2026-07-16): *GPU stays enabled except in cases where there is absolutely no option* — the automatic last-resort startup gate (commit-free < 512 MB, `app.rs:747`, "spawning the GPU process would OOM the host before first paint") is the only sanctioned disable path, and it stays as-is. No user-facing or env override will be added. Background: the correction spec's §A.4 proposed the switch as the decisive attribution test; this experiment already answered that question, so the diagnostic justification is gone. And as a *mitigation* it's counterproductive anyway: SwiftShader software rendering allocates CPU shared memory that is itself pagefile-backed commit (see the `window_pool.rs` "SwiftShader = CPU shared memory = pagefile-backed commit" comment, ~90 MB+/window measured), so it degrades the whole UI while plausibly not reducing commit at all.
4. **Non-AgentMux housekeeping observed on this machine** (informational): vmmemWSL held 4.5 GB (`wsl --shutdown` when idle), and two resident `parsecd` instances held ~2.8 GB between them.

## 6. Measurement discipline (reaffirmed)

Same as the July 2 retro: measure **commit** (`Private Bytes`, `Memory\Committed Bytes`), never `VirtualMemorySize`; and attribute by ledger subtraction before architecting. One addition from this session: **the decisive experiment beats the ledger** — a scheduled-task logger that survives the app's death turned a plausible hypothesis into a measured fact in ten minutes.
