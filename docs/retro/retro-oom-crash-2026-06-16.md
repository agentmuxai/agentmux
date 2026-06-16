# Retro / Post-mortem — AgentMux OOM crash (2026-06-16)

**Status:** root cause confirmed · recovery gaps identified · recommendations below
**Author:** AgentA
**Date:** 2026-06-16
**Severity:** P1 — the live dogfood session was lost (hard crash, no graceful exit)

---

## 1. Executive summary

The AgentMux instance we were working in — the **v0.44.1 local portable**
(`local-main-b28b7a`, build `gc7df50a6`) — **hard-crashed at 07:33:16 local
(14:33:16 UTC)**. The crash was an **out-of-memory abort inside the CEF/Chromium
host process**, not a logic bug in AgentMux code.

Windows Error Reporting recorded the faulting process raising exception
**`0xe0000008`** — Chromium's intentional, non-continuable *out-of-memory*
exception code (`base::win::kOomExceptionCode`) — through
`KERNELBASE!RaiseException`. Chromium raises this deliberately when an allocation
fails; it is the allocator's "I cannot get memory, abort now" signal.

The machine had **32 GB of RAM but its system commit limit (page file + RAM) was
exhausted**. All session we had been hitting `errno 1455`
(`ERROR_COMMITMENT_LIMIT`) on builds — the same condition. The exhaustion was
driven by **several AgentMux instances running at once** plus a **1.3 GB Vite dev
server** plus **repeated production builds** (each esbuild/Go worker needing
~1–2 GB). When the commit pool ran dry, the next allocation in the v0.44.1
Chromium host failed and Chromium aborted the process.

AgentMux already has a **host-crash relaunch ladder** (3 restarts / 60 s,
escalating to `--disable-gpu`). The gap is that **the ladder is memory-blind**:
it relaunches straight back into the same starved condition, and nothing warns
the user or sheds load *before* the OOM. The result is a hard crash that reads,
to the user, as "AgentMux just disappeared."

---

## 2. Timeline (2026-06-16, local time)

| Time | Event |
|------|-------|
| (all morning) | Active dogfood session in the **v0.44.1 portable** (`agentmux-0.44.1+gc7df50a6` on the Desktop). Simultaneously: a `task dev` instance for branch `agenta/agent-failure-recovery-ui`, a `0.46.0` local build, and a long-running **Vite dev server (PID 10376, ~1.3 GB RSS)**. |
| ~07:2x–07:33 | Repeated **production frontend builds** attempted; each failed with `runtime: VirtualAlloc … errno=1455` / `fatal error: out of memory` and `fork: retry: Resource temporarily unavailable` — direct evidence the **system commit limit was exhausted**. |
| **07:33:15** | The crashed instance's backend `srv-events.log` stops writing (`channels/local-main-b28b7a-…/versions/0.44.1/data/srv-events.log`). |
| **07:33:16** | **CEF host `agentmux-0.44.1.exe` (PID 3796 / `0xed4`) crashes** — exception `0xe0000008`, faulting module `KERNELBASE.dll`, fault offset `0x25369`. (WER Event ID 1000.) |
| 07:33:53–54 | Windows Error Reporting writes the `APPCRASH` reports and a **minidump** (`…\WER\Temp\WERD071.tmp.mdmp`). (Event ID 1001 ×2.) |
| ~07:33–08:02 | The instance's `cef-debug.log` continues to grow to ~164 MB until ~08:02, consistent with the **launcher relaunch ladder firing** and the host coming back up — but the backend `srv-events.log` never resumes, so the relaunched host likely had a degraded/headless backend. |
| (now) | Memory has fully recovered: 25.7 GB physical free, 7.9 GB committed of a 65.4 GB limit. The pressure was transient and load-driven. |

---

## 3. Root cause

### 3.1 The crash itself — Chromium OOM abort

WER (authoritative):

```
Faulting application name: agentmux-0.44.1.exe, version: 0.0.0.0, time stamp: 0x6a2d9e2e
Faulting module name:      KERNELBASE.dll, version: 10.0.19041.7417
Exception code:            0xe0000008
Fault offset:              0x0000000000025369
Faulting process id:       0xed4   (3796)
Faulting application path:  …\agentmux-0.44.1+gc7df50a6.20260613T180900.363-x64-portable\runtime\agentmux-0.44.1.exe
```

`0xe0000008` decodes as a **customer-defined** SEH code (top nibble `E` → severity
ERROR + customer bit set). It is **not** an access violation (`0xC0000005`) or a
C++/CLR throw (`0xE06D7363` / `0xE0434352`). It is **Chromium's `kOomExceptionCode`**:
when PartitionAlloc / the Chromium allocator cannot satisfy a request, Chromium
calls `RaiseException(0xE0000008, EXCEPTION_NONCONTINUABLE, …)` from inside
`base::internal::OnNoMemory*`. That call lives in `KERNELBASE.dll` — exactly the
faulting module reported. **This is a deliberate OOM abort, not memory
corruption.** A crash dump is available if deeper analysis is wanted
(`%ProgramData%\Microsoft\Windows\WER\Temp\WERD071.tmp.mdmp`, may be GC'd).

### 3.2 Why memory ran out — load, not a leak

The host did not leak; the *system* ran out of committable memory. Concurrent
consumers at crash time:

- **Multiple AgentMux instances** — the v0.44.1 portable **and** the
  `agent-failure-recovery-ui` `task dev` instance **and** a `0.46.0` build, each a
  full launcher + host + multiple CEF subprocesses (browser/GPU/renderer/utility)
  + an `agentmux-srv`. CEF/Chromium is memory-heavy per instance.
- **A 1.3 GB Vite dev server** (PID 10376, `vite --port 5287`) left running from
  hot-reload testing, ballooned over a long session.
- **Repeated `task build:frontend` runs**, each spawning Go/esbuild workers that
  need ~1–2 GB; these were *themselves* OOM-failing with `errno 1455`.

32 GB RAM was nominally plenty, but the **commit limit** (RAM + page file) is the
real ceiling, and it was saturated. The first instance to ask for memory and lose
was the v0.44.1 Chromium host.

> Note: this is fundamentally a **dev-machine load pattern** (N instances + Vite +
> builds), not a defect a normal end user would hit with a single instance. The
> recommendations below still apply because (a) dogfooding *is* this load pattern,
> and (b) end users on smaller/older machines can hit per-instance OOM, and the
> *graceful-handling* gaps are the same.

---

## 4. What recovery already exists today

AgentMux is not defenceless against a host crash — the launcher
(`agentmux-launcher/src/main.rs`) implements:

- **Host-crash relaunch ladder** — `HOST_RESTART_BUDGET = 3` relaunches within a
  `HOST_RESTART_WINDOW = 60 s` window. On the second attempt it steps into a
  **degraded `--disable-gpu`** (software-rendering) rung; budget exhausted → it
  logs `restart budget exhausted` and gives up. (Both the Windows and Unix
  supervisors share this; lines ~949–1010 and ~1491–1549.)
- **Splash re-dismissal on relaunch** so a host that died pre-first-frame doesn't
  leave a stuck splash.
- **srv crash monitor** — the backend spawns a crash monitor that writes minidumps
  to `C:\CrashDumps\agentmuxsrv` (`[crash-handler] crash monitor spawned …`).
- **Persisted state** — the workspace (panes/layout) lives in the channel's
  `objects.db`; **agents and auth are global** (cross-channel work #1387–#1393).
  So a *successful* relaunch can reload the workspace; the durable data is not
  lost in a crash.

For this incident, the growing `cef-debug.log` after 07:33 suggests the ladder
**did** fire and bring a host back — but the backend `srv-events.log` never
resumed, so recovery was partial at best.

---

## 5. Why it wasn't graceful — the gaps

1. **The relaunch ladder is memory-blind.** It relaunches the host *immediately*
   into the **same** commit-exhausted condition. Under sustained pressure that
   means crash → relaunch → crash → … until the 3-restart budget is burned, then
   "give up" — converting a recoverable transient into a permanent down. There is
   no check of available commit before relaunching and no backoff to let memory
   recover.

2. **No pre-OOM awareness or graceful degradation.** Nothing watches system memory
   pressure. AgentMux runs full-speed into the wall: no "memory is low, pausing
   non-essential renderers / agents" step, no flush-and-checkpoint before the
   abort, no clean teardown. Chromium's OOM abort is the *first* signal anything
   is wrong.

3. **No user-facing crash/recovery signal.** From the user's seat the window just
   vanished. There is no "AgentMux recovered from a crash and restored your
   session" (or "couldn't recover — here's your data") affordance. Recovery, when
   it happens, is silent and unverifiable to the user.

4. **Multi-instance memory has no shared budget.** Each instance is independently
   greedy; nothing coordinates total footprint across instances, and nothing
   accounts for co-resident dev tooling (Vite, esbuild). On a dogfood box this is
   the dominant failure mode.

5. **Backend/host recovery aren't coupled.** The host relaunched but the srv
   stopped logging at 07:33:15 and didn't visibly resume — a relaunched host with
   a dead/oom'd backend is a half-recovered, confusing state rather than a clean
   restore-or-fail.

6. **No OOM caps inside Chromium.** The host doesn't pass renderer memory limits
   / `--js-flags` heap caps / process-count limits, so a single instance's
   Chromium can grow until the *system* (not Chromium) refuses — the worst place
   to hit the limit.

---

## 6. Recommendations (prioritized)

> These are now designed in
> **`docs/specs/SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md`** (host/instance-level
> memory-pressure supervision + graceful degradation), which complements the
> existing renderer-level `SPEC_GATED_RENDERER_RECOVERY_2026_06_01.md`.

### P0 — make recovery memory-aware (turn the hard crash into a graceful one)

- **Gate the relaunch ladder on available commit.** Before each relaunch, read
  `GlobalMemoryStatusEx` (Windows) / `/proc/meminfo` (Linux). If commit headroom
  is below a threshold, **back off with exponential delay** instead of
  immediately respawning into the wall, and **don't count memory-starved retries
  against the 3-restart budget** (or use a separate, longer window for OOM-class
  exits). Distinguish the OOM exit code (`0xe0000008`) from other abnormal exits
  so the supervisor can apply OOM-specific backoff.
- **On budget-exhaustion, fail *gracefully*:** show a real window/dialog —
  "AgentMux ran out of memory and couldn't recover. Your panes, agents and auth
  are saved; reopen to restore." — rather than a silent disappearance. The data
  is in `objects.db`, so this is a messaging gap, not a data-loss one.

### P1 — degrade before the wall, not at it

- **Add a lightweight memory-pressure monitor** (in the launcher or srv) that
  samples commit headroom. On a low-water mark, **proactively shed load**: pause
  idle agent subprocesses, drop the warm window pool, ask renderers to free
  caches (`Page.clearMemoryCache` / `--purge-memory-on-low`), and surface a
  non-blocking "low memory" banner so the user can close things *before* a crash.
- **Checkpoint on pressure**: flush any unsaved UI/session state to `objects.db`
  when pressure is detected, so even a subsequent hard OOM restores cleanly.

### P1 — verify and harden session restore

- **Confirm the relaunch path actually restores the workspace** (panes + agents)
  from `objects.db` after a host crash, end-to-end, and add a smoke test for
  "kill host → relaunch → same panes". Today we *believe* it restores; this
  incident shows we don't *verify* it.
- **Couple host+backend recovery**: if the srv died too, the relaunched host
  should detect the dead backend and respawn/reconnect it (or fail loudly),
  instead of coming up half-wired.

### P2 — cap Chromium's appetite so it fails inside its own budget

- Pass renderer memory limits / `--js-flags="--max-old-space-size=…"` /
  `--renderer-process-limit` so a runaway instance hits a *Chromium-level* cap
  (recoverable: kill one renderer) before it hits the *system* commit limit
  (unrecoverable: whole-host abort).
- Consider starting in `--disable-gpu` automatically when commit headroom is
  already low at launch (skip straight to the degraded rung).

### P2 — operational / dogfooding guidance

- **Don't run N instances + Vite + prod builds on one box.** Prefer a **headless
  CDP probe** over a GUI dev for verification (already a noted preference); stop
  stale `task dev` Vite servers (they balloon — PID 10376 was 1.3 GB); don't run
  `task build:frontend` while multiple instances + dev are live.
- **Prune accumulated per-build channels** (`channels/local-*`) — each carries a
  data dir + cef-cache and they accumulate; live ones also hold memory.
- Add a `muxlog`/doctor command that reports current commit headroom and the
  number of live AgentMux instances, so pressure is visible before it bites.

---

## 7. Evidence appendix

**Crash (WER, Application log, 2026-06-16):**
- Event 1000 — faulting app `agentmux-0.44.1.exe` PID `0xed4` (3796), module
  `KERNELBASE.dll`, exception `0xe0000008`, offset `0x25369`,
  report id `4896b060-77c0-4e7b-a000-c5648e3c2db3`.
- Event 1001 ×2 — `APPCRASH`, fault bucket `1813149519630564525` (type 4);
  minidump `…\WER\Temp\WERD071.tmp.mdmp`.

**Crashed instance data dir:**
`~/.agentmux/channels/local-main-b28b7a-20260613T180900/versions/0.44.1/`
(`data/srv-events.log` last write 07:33:15; `logs/cef-debug.log` grew to ~164 MB
until ~08:02).

**Memory-pressure evidence (this session):** repeated
`runtime: VirtualAlloc of N bytes failed with errno=1455` / `fatal error: out of
memory` from `task build:frontend`; `bash: fork: retry: Resource temporarily
unavailable`.

**Co-resident load at crash time:** v0.44.1 portable host (PID 3796), the
`agenta/agent-failure-recovery-ui` `task dev` instance, a `0.46.0` build, and a
Vite dev server (PID 10376, `vite --port 5287`, ~1.3 GB).

**Existing recovery code:** `agentmux-launcher/src/main.rs`
(`HOST_RESTART_BUDGET = 3`, `HOST_RESTART_WINDOW = 60s`, `--disable-gpu` degraded
rung, "restart budget exhausted" give-up); srv crash monitor →
`C:\CrashDumps\agentmuxsrv`.

**Decode of `0xe0000008`:** Chromium `base::win::kOomExceptionCode`, raised
non-continuably from `base::internal::OnNoMemory*` via
`KERNELBASE!RaiseException` — a deliberate out-of-memory abort.
