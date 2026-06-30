# Win10 Commit-Limit OOM — Addendum: Free-Disk Regression & the 0xE0000008 Crash Class

**Date:** 2026-06-29
**Status:** Addendum to existing memory-pressure corpus — corrects a stale premise, adds two new findings
**Affected:** AgentMux (CEF/Chromium 148) on Windows 10 22H2 (build 19045).

> **Read first — this builds on prior work, it does not replace it:**
> - `docs/incident/INCIDENT_2026_06_26_APP_CLOSED.md` — the overnight silent OOM kill
> - `docs/specs/SPEC_MEMORY_ANALYSIS_2026_06_26.md` — process inventory, commit math, P0–P3 list
> - `docs/specs/SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md` — `mem_supervisor.rs` design
> - `docs/specs/SPEC_GATED_RENDERER_RECOVERY_2026_06_01.md` — renderer OOM recovery
> - `docs/specs/SPEC_WINDOWS_LIFECYCLE_ROBUSTNESS_2026_06_26.md`
>
> **Already shipped** (confirmed in tree): commit gauge in StatusBar (`SystemStats.tsx`, #1799);
> launcher memory-aware relaunch (`mem_supervisor.rs`, #1493); `CreateMemoryResourceNotification`
> watcher (`agentmux-launcher/src/main.rs`); gated renderer OOM recovery (`memory_heartbeat.rs`,
> `client/mod.rs`, #1229); proactive `--disable-gpu` under low commit (#1496); WAL checkpoint +
> saga cancel (#1799); `avail_page_gb` heartbeat (`backend/sysinfo.rs`).

This addendum exists because new machine data shows the **commit budget the prior analysis
assumed (87.7 GB) has since collapsed to ~52–56 GB**, and because the Windows Event Log reveals
a **second, distinct crash manifestation** (`0xE0000008`) not covered by the 6/26 incident.

---

## 1. What changed since 2026-06-26 — the premise is now stale

`SPEC_MEMORY_ANALYSIS_2026_06_26.md` computed everything against:

```
Commit budget (RAM + page file): 31.9 + 55.8 = 87.7 GB
→ "Max safe concurrent agents: 3 (leaves 17.7 GB margin)"
```

That 55.8 GB page file assumed C: had room to host it. **It no longer does.** Today's readings:

| Metric | 2026-06-26 (prior spec) | 2026-06-29 (now) | Change |
|--------|------------------------|------------------|--------|
| C: free space | (enough for 55.8 GB pf) | **20.1 GB free** / 446 GB | drive filled up |
| Page file allocated | 55.8 GB | **~20.75 GB** | shrank ~35 GB |
| System commit limit | 87.7 GB | **~52–56 GB** | **nearly halved** |
| Pagefile growth ceiling `min(3×RAM, ⅛ vol)` | ~55.8 GB | ~55.8 GB wanted, **gated by 20 GB free disk** | blocked |

**The system-managed page file cannot grow.** It wants to climb toward ~55 GB when commit hits
90%, but there is only ~20 GB of free disk, so it is physically stuck near 20 GB. The commit
limit ceiling is therefore pinned at roughly RAM (32 GB) + ~20 GB ≈ **52 GB and cannot rise**.

**Consequence:** every "safe concurrent agents" number in the 6/26 spec is now optimistic by
roughly the ratio 52/87.7 ≈ 0.6×. With a tighter budget the system reaches the wall faster and
more often — consistent with the user's report that Win10 now crashes "like clockwork, even
idle." The single most impactful variable today is **free disk on the pagefile volume**, which
none of the prior specs tracked.

---

## 2. New finding — the `0xE0000008` crash class (distinct from the 6/26 silent kill)

The 6/26 incident was a **silent Windows OOM process-kill** (no dump, no WER entry — Windows
reclaimed memory by terminating the host). That is failure mode **(A)**.

The Application event log shows a **second** mode **(B)** that recurs across *every* version
(0.43.2 → 0.49.6), independent of 6/26:

```
Faulting application: agentmux-0.49.6.exe (libcef 148.0.9)
Faulting module:      KERNELBASE.dll +0x25369   ← RaiseException (the raiser, not the cause)
Exception code:       0xE0000008                ← Chromium kOomExceptionCode (out of memory)
OS:                   10.0.19045 (Win10 22H2)
```

`0xE0000008` is **Chromium's hard-coded OOM exception** (`base::win::kOomExceptionCode`; chosen
because `0x8 = ERROR_NOT_ENOUGH_MEMORY`, top nibble `E` to avoid collisions). When any
allocation fails, `base::TerminateBecauseOutOfMemory` calls
`::RaiseException(0xE0000008, EXCEPTION_NONCONTINUABLE, …)` and the CEF process self-aborts.
Breakpad labels it `MD_EXCEPTION_OUT_OF_MEMORY`. (Sources §6.)

So mode (B) is the app's **own renderer/host process hitting a failed commit and aborting
itself** — the same root pressure as (A), surfaced as a real WER crash rather than a silent
kill. The existing gated-renderer-recovery (#1229) is meant to catch renderer deaths, but these
`0xE0000008` faults still reach WER, so either (i) they're hitting the host/browser process (not
recoverable as a renderer), or (ii) recovery isn't firing for this exception code. **Action:
verify whether mode (B) crashes are classified and recovered by `memory_heartbeat` / the gated
recovery path, or whether they bypass it.**

Secondary codes also present, all downstream of pressure: `0x80000003` (Chromium
`IMMEDIATE_CRASH`/breakpoint, in `libcef.dll`), `0xC0000409` (`__fastfail`), `0xC0000602`
(`FAST_FAIL_FATAL_APP_EXIT`).

### Note on the 6/26 commit attribution
The 6/26 spec attributed ~10.5 GB *commit* to each `claude.exe`, derived from
`VirtualMemorySize64`. The `Resource-Exhaustion-Detector` (Event 2004) data from the same
morning lists each `claude.exe` at **~0.49–0.67 GB** of consumed virtual memory, and `Traktor.exe`
at ~0.9 GB. `VirtualMemorySize64` counts reserved address space (not commit); the true
per-agent **private commit** is sub-GB, not 10.5 GB. This doesn't change the conclusion (the
budget was exhausted) but it means the dominant consumer was the **aggregate of many processes
against a now-smaller budget**, not a few giant agent processes. Worth re-measuring with
`PrivateUsage` before sizing the commit-aware scheduler's reserve.

---

## 3. New finding — runaway logging is eating the scarce disk (feedback loop)

`~/.agentmux/logs/agentmux-launcher.log` is **244 MB**, dominated by per-second repeats of:

```
agentmux_srv::server::websocket: SetMeta oref=block:…   (INFO, many/sec)
```

On a drive with only ~20 GB free, this directly **tightens the page-file ceiling** that mode
(A)/(B) crashes depend on — a self-reinforcing loop (more uptime → bigger log → less disk →
smaller commit budget → earlier OOM). Not the root cause, but a cheap, high-leverage fix that
also protects the pagefile headroom.

**Action:** throttle/sample the `SetMeta` INFO line; enforce rotation + size cap on
`agentmux-launcher.log` (e.g. 50 MB × 3).

---

## 4. Why Windows 11 is unaffected (updated)

Not a kernel-version bug. The Win11 box almost certainly has **more free disk**, so its
system-managed page file actually grows toward 3×RAM and the commit limit stays ahead of demand
— the allocation that aborts on Win10 never fails there. Lower steady-state commit charge
(fewer concurrent agents / no Traktor) and marginally more aggressive Win11 memory compression
add headroom but are secondary. The dominant lever is environmental (free disk → pagefile
growth), which is exactly the variable that regressed on this Win10 machine since 6/26.

---

## 5. Recommendations

### 5.1 Immediate (user, no code) — should stop the crashes today
1. **Free C: to ≥ 60–80 GB.** Quick wins: the 244 MB launcher log, old portable build folders
   on the Desktop (`agentmux-0.49.x+…-portable`), stale dev instances under `~/.agentmux/dev`.
   This lets the page file grow back toward ~55 GB and restores the 87.7 GB budget the prior
   analysis assumed.
2. **Or set an explicit page file** (e.g. 32 GB init / 48 GB max) on a volume with room, so it
   can't get stuck mid-grow. Requires the disk first.
3. Keep concurrent agents ≤ what the *current* budget supports (≈ 2 until disk is freed).

### 5.2 Code — gaps still open after the shipped corpus
| Pri | Item | Why it's still needed | Where |
|-----|------|----------------------|-------|
| **P0** | **Track free disk on the pagefile volume**, not just `avail_page_gb`. Warn when free disk < ~15% *and* page file is system-managed ("Windows can't grow virtual memory — crash risk"). | The entire regression here is invisible to the current `avail_page` gauge — it only sees the symptom (commit near limit), not the cause (disk can't back a bigger pagefile). | `backend/sysinfo.rs` + StatusBar |
| **P0** | **Confirm `0xE0000008` is caught by gated renderer recovery.** If these faults hit the host/browser process, they bypass `memory_heartbeat` recovery and kill the session. | Mode (B) crashes still reach WER across all versions despite #1229. | `agentmux-cef/src/client/mod.rs`, `memory_heartbeat.rs` |
| **P0** | **Commit-aware turn scheduler** — gate new agent spawn on commit headroom (6/26 P0). Re-derive the reserve from `PrivateUsage`, not `VirtualMemorySize64`. | Verify whether this actually shipped or only monitoring landed; the 6/26 reserve (12 GB) was sized off the inflated 10.5 GB/agent figure. | `agentmux-srv` |
| **P1** | Throttle `SetMeta` INFO firehose + rotate/cap `agentmux-launcher.log`. | Disk-eating feedback loop (§3). | srv logging + launcher |
| **P1** | Shut down old version on upgrade (6/26 P1 — still see multiple versions coexisting). | Stacks CEF overhead across months. | launcher |
| **P2** | Per-agent commit (`PrivateUsage`) in Swarm; first-run budget advisory **driven by free disk**, not a static RAM table. | The static "3 agents" table is wrong once disk shrinks the budget. | srv + frontend |

---

## 6. Sources
- [Chromium `base/process/memory.h`](https://chromium.googlesource.com/chromium/src/base/+/master/process/memory.h)
- [Add OOM exception code for Chromium (breakpad-dev)](https://groups.google.com/g/google-breakpad-dev/c/PJOG-iLhSjg) — `kOomExceptionCode = 0xe0000008`
- [`base::TerminateBecauseOutOfMemory` → `RaiseException` on Windows (codereview 2173463002)](https://codereview.chromium.org/2173463002)
- [Page file sizing for 64-bit Windows (MS Learn)](https://learn.microsoft.com/en-us/troubleshoot/windows-client/performance/how-to-determine-the-appropriate-page-file-size-for-64-bit-versions-of-windows) — auto-grow at 90% commit, ceiling `min(3×RAM, ⅛ volume)`
- [Introduction to the page file (MS Learn)](https://learn.microsoft.com/en-us/troubleshoot/windows-client/performance/introduction-to-the-page-file) — commit limit = RAM + page files

---

## 7. Verification
- **Confirm the disk lever (single-variable test):** free C: to ≥ 80 GB, run idle 24 h → expect
  no `0xE0000008`. Refill to ~20 GB free → expect the crash to return. This proves disk/pagefile
  is the regression vs. the shipped supervision corpus.
- **Re-measure per-agent commit with `PrivateUsage`** (not `VirtualMemorySize64`) to correctly
  size the scheduler reserve.
- **Watch both failure modes:**
  ```powershell
  # Mode B — Chromium OOM self-abort (WER)
  Get-WinEvent -FilterHashtable @{LogName='Application';Id=1000} |
    Where Message -match 'agentmux.*e0000008' | Select TimeCreated
  # Pressure precursor (fires before either mode)
  Get-WinEvent -FilterHashtable @{LogName='System';
    ProviderName='Microsoft-Windows-Resource-Exhaustion-Detector'}
  # The regressed variable the gauge doesn't track
  Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='C:'" | Select Size,FreeSpace
  ```
