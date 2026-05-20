# Crash Analysis — Host `0x80000003` / Chromium GPU-process FATAL

**Date:** 2026-05-20
**Build:** `agentmux-0.34.0.exe` (host, PID 7276, ~29 h uptime)
**Symptom:** Windows "Application Error" modal — *"The exception Breakpoint
(0x80000003) occurred… Click OK to terminate, CANCEL to debug."* Title bar:
`DO NOT BOOTY - tab1 - AgentMux`. Screenshot: `crash.png` on the Desktop.

---

## 1. Summary

The host did **not** crash from a bug in AgentMux's own code. It was killed by
**Chromium's `LOG(FATAL)`** after the bundled GPU process crashed six times in
one host session and Chromium gave up on it.

`LOG(FATAL)` in Chromium ends with `base::debug::BreakDebugger()` → an `int 3`
instruction → `STATUS_BREAKPOINT` (`0x80000003`). That is the exact exception
code in the modal. It is a *deliberate, controlled* abort — not memory
corruption (`0xC0000005`) and not a Rust `panic!`.

The terminal log line:

```
[7276:8176:0520/095204.716:FATAL:content\browser\gpu\gpu_data_manager_impl_private.cc:417]
GPU process isn't usable. Goodbye.
```

The deeper trigger was **system memory exhaustion** — the Windows page file
was nearly full, so the GPU process could not restart.

> **Stability mandate.** Rock-solid stability is a core part of AgentMux's
> value proposition: **the user must never see a crash.** "No crashes ever" is
> not literally achievable at the process level — GPU drivers segfault, the OS
> runs out of commit, hardware misbehaves, and none of that is AgentMux's code.
> What *is* achievable, and is the design target, is that **every fault becomes
> an invisible, sub-second auto-recovery** — no OS modal, no lost work, at most
> a flicker. AgentMux is unusually well-positioned for this: it already has a
> separate supervising `agentmux-launcher` process and already persists all
> state. The recovery layer in §4.3 is therefore **required**, not optional —
> this crash is the proof case of the gap.

---

## 2. Timeline & evidence

Sources: `~/.agentmux/versions/0.34.0/logs/cef-debug.log` and
`agentmux-host-v0.34.0.log.2026-05-20`.

| Local time | Event |
|---|---|
| 05‑19 04:31:22 | GPU process crashed **3×** in ~300 ms — `exit_code=-2147483645`. Chromium reinitialized it each time (104–117 ms). App recovered, ran normally. Crash count = 3. |
| 05‑19 → 05‑20 | ~29 h of normal operation. |
| 05‑20 09:52:04.705–.716 | GPU process crashed **3× in 11 ms** — `exit_code=-1073741523`, 0 ms init each. Crash count → 4, 5, 6. |
| 05‑20 09:52:04.716 | `FATAL: GPU process isn't usable. Goodbye.` → host process breakpoints. |
| 05‑20 09:52:04–09:52:11 | Renderer still emitting `[fe]` console logs for ~7 s, then silent. |
| 05‑20 09:52:11 → 09:53:04+ | Host's Rust `mem_heartbeat` thread keeps ticking behind the modal (host main process only froze the faulting thread). User dismisses modal; process exits. |

The host itself was healthy throughout — `ws_mb` ≈ 98 MB, `peak_ws_mb` 130 MB.
Nothing in AgentMux's Rust code or the frontend faulted.

### Exit-code decode

Chromium reports process exit codes as signed 32-bit. Unsigned = `2^32 + code`:

| Reported | Unsigned hex | NTSTATUS | Meaning |
|---|---|---|---|
| `-2147483645` | `0x80000003` | `STATUS_BREAKPOINT` | A DCHECK/breakpoint *inside* the GPU process (05‑19). |
| `-1073741523` | `0xC000012D` | `STATUS_COMMITMENT_LIMIT` | **Out of virtual memory** — the GPU process could not commit memory (05‑20). |

This is the key finding. The 05‑20 GPU crashes were **not** a driver segfault —
the GPU process exited with `STATUS_COMMITMENT_LIMIT` because the machine was
out of committable memory.

### Corroborating memory pressure

The host's `mem_heartbeat` around the crash:

```
16:52:44Z  system memory  load_pct=54  avail_phys_gb=14.6  avail_page_gb=0.1
```

`avail_page_gb 0.1` — the Windows **page file had ~100 MB free**. When commit is
that scarce, a fresh GPU process cannot reserve its address space and dies
before it finishes initializing (hence 0 ms init, then immediate exit).

---

## 3. Root cause

Two layers, in order of importance:

1. **Primary — system memory/commit exhaustion.** The page file was ~full.
   Each GPU-process restart on 05‑20 failed with `STATUS_COMMITMENT_LIMIT`
   before it could initialize. Three instant failures pushed the cumulative
   crash count past Chromium's limit.
2. **Secondary — a flaky GPU session.** The GPU process had already crashed 3×
   the day before (breakpoint/DCHECK). Chromium's crash counter is
   **cumulative for the whole host session**, so the session entered 05‑20
   "pre-loaded" at count 3 — only three more failures away from FATAL.

**Chromium policy** is what converts these into a hard kill: `GpuProcessHost`
restarts the GPU process after a crash, but only up to a cap. Once the cap is
exhausted *and* a usable GPU (hardware or software) cannot be brought up,
`gpu_data_manager_impl_private.cc` does `LOG(FATAL)` — it takes the whole
browser process down rather than run in an undefined state.

This is **environmental** (memory + driver), not an AgentMux logic defect. But
AgentMux has no supervision or fallback around it, so a recoverable subsystem
failure became a full app crash with a frightening OS modal.

---

## 4. Could it have recovered? (and: can a crashed GPU process be restarted/re-mounted?)

**Short answer: a crashed GPU process *can* be restarted and re-attached — and
Chromium already does exactly that automatically. It did so successfully 3×
the day before this crash. What was missing was (a) enough memory for the
restart to succeed on 05‑20, and (b) AgentMux-level supervision so the
final FATAL didn't have to be terminal.**

### 4.1 Restart + "mount" is built in — and normally works

The GPU process is a *separate* process from the browser process and from each
renderer. When it dies, the renderer processes do **not** die — they keep their
DOM/JS state. Chromium's `GpuProcessHost`:

1. Spawns a fresh GPU process.
2. Re-creates the GPU channels and hands them back to the Viz display
   compositor and to every renderer's compositor.
3. The next compositor frame is produced by the new GPU process.

That re-attachment **is** the "mount" — and it is transparent. The user sees at
most a brief flash. The cef-debug.log proves it worked here:

```
0519/043122.624 Reinitialized the GPU process after a crash … 117 ms
0519/043122.755 Reinitialized the GPU process after a crash … 104 ms
0519/043122.865 Reinitialized the GPU process after a crash … 0 ms
```

So "can it be restarted and mounted?" — **yes, by design, and it was.** The
app survived all three 05‑19 crashes for that reason.

### 4.2 Why the 05‑20 restarts did not mount

A restart can only re-mount if the new process actually *starts*. On 05‑20 each
restart exited with `STATUS_COMMITMENT_LIMIT` in **0 ms** — it died before it
could create a GL/D3D context or open a single channel. You cannot mount a
process that never finished initializing. The failure was *upstream* of
mounting: the OS could not give the process memory.

So under that memory state, **no number of restarts would have recovered** —
the fix there is freeing commit, not retrying.

### 4.3 What "the right stuff inside" would have done

Recovery is possible at four distinct layers. AgentMux currently has none of
them; it is unusually well-positioned to add all four.

**(a) Don't let Chromium FATAL — raise/remove the crash cap.**
Chromium has a switch `--disable-gpu-process-crash-limit`. With it, Chromium
keeps restarting the GPU process indefinitely instead of `LOG(FATAL)`. The app
would have stayed up (GPU flickering until memory freed) rather than dying.
Cheap, one-line mitigation — but it only converts "crash" into "spin", so it
must be paired with (b)/(c).

**(b) Software-rendering fallback.**
Launch (or *re-launch*) the host with `--disable-gpu` /
`--disable-gpu-compositing`. With GPU disabled there is **no separate GPU
process to crash** — compositing runs in-process via SwiftShader. Slower, but
immune to driver breakage and to GPU-process OOM. The ideal design: run with
GPU on, and if the GPU process crashes N times, relaunch the host once with
`--disable-gpu` and stay there for the rest of the session.

**(c) Launcher-level supervision + state restore — the real recovery.**
This is the architecturally correct answer and AgentMux already has the two
hard prerequisites:

  - `agentmux-launcher` is a **separate supervising process** that owns the
    host's lifecycle (Job Object, pipe, single-instance). It survives a host
    crash.
  - AgentMux **persists all app state** (the reducer stack → `objects.db` etc.).

So the launcher could: detect the host exited with `0x80000003`, recognize a
GPU FATAL (tail `cef-debug.log` for the `gpu_data_manager` line, or just treat
a breakpoint exit as relaunch-worthy), and **relaunch the host — with
`--disable-gpu` on the retry**. The relaunched host reloads the same workspace,
tabs, and panes from `objects.db`. The user sees a flicker-and-restore instead
of a crash modal. *That* is recovery: not catching the FATAL (you cannot — it
is a synchronous `int 3`), but supervising, restarting, and restoring.

**(d) Suppress the OS modal.**
Independently: the legacy "Application Error / breakpoint" dialog should never
reach the user. Chromium's Crashpad handler normally suppresses the OS dialog,
but it did not cover this FATAL path in the browser process. Setting
`SetErrorMode(SEM_NOGPFAULTERRORBOX | SEM_FAILCRITICALERRORS)` in the host (and
in the launcher for its children), or registering AgentMux with
`WerAddExcludedApplication`, makes a crash exit silently — so the launcher in
(c) can restart it without the user ever seeing the dialog.

### 4.4 Recommended end-state

```
host runs with GPU on
  │
  ├─ GPU process crash → Chromium auto-restarts + re-mounts (transparent)   ← already works
  │
  ├─ N crashes in a session → launcher relaunches host with --disable-gpu   ← (b)+(c)
  │     and restores workspace from objects.db
  │
  └─ host exits 0x80000003 → no OS modal (SEM_NOGPFAULTERRORBOX),            ← (d)+(c)
        launcher restarts + restores
```

The single most valuable piece is **(c)** — launcher supervision with
state restore — because it turns *any* host crash, not just GPU FATALs, into a
recoverable blip.

---

## 5. Recommendations

### Immediate (environment — to get running and stop recurrence)
- **Relaunch AgentMux.** A fresh host resets the GPU crash counter to 0. No
  data was lost or corrupted.
- **Free system memory / grow the page file.** `avail_page_gb 0.1` is the real
  trigger. Close memory-heavy apps, or increase the Windows paging-file size.
- Update the GPU driver. Clearing `cef-cache` GPU caches is cheap but unlikely
  to matter here (they are only ~548 KB).

### AgentMux-side (required — stability mandate, see §1 and §4.3)
Ordered so the user-visible win lands first:
- **(d)** `SetErrorMode(SEM_NOGPFAULTERRORBOX | SEM_FAILCRITICALERRORS)` in host
  + launcher so a crash never shows the OS modal. Smallest change, immediately
  removes the scariest symptom. Ship first.
- **(c)** Launcher: detect host breakpoint/abnormal exit → relaunch + restore
  from `objects.db`. Highest value; generalizes beyond GPU to *every* host
  crash. This is the core of the "no visible crashes" guarantee.
- **(b)** Relaunch with `--disable-gpu` after repeated GPU-process crashes so
  the retry can't hit the same failure.
- **(a)** Pass `--disable-gpu-process-crash-limit` so Chromium keeps
  auto-restarting the GPU process instead of `LOG(FATAL)`-ing.
- Low-commit-memory guard in the launcher's `mem_heartbeat` path — warn (or
  shed) before the GPU process gets starved.

Tracking: these should become a stability epic, not a one-off fix — the goal
is that no fault, AgentMux's or the environment's, is ever visible to the user.

---

## 6. Diagnosis recipe (for next time)

The breakpoint modal **blocks WER**, so there is usually **no dump** in
`%LOCALAPPDATA%\CrashDumps`. Diagnose from logs instead — they outlive the
crash:

```bash
V=0.34.0
CEF=~/.agentmux/versions/$V/logs/cef-debug.log
grep -nE 'FATAL|GPU process.*(crashed|exited)' "$CEF"      # the FATAL + crash escalation
grep mem_heartbeat ~/.agentmux/versions/$V/logs/agentmux-host-v$V.log.* \
  | grep avail_page_gb                                     # was the page file exhausted?
```

Decode any `exit_code`: unsigned = `2^32 + code`, then look up the NTSTATUS
(`0x80000003` breakpoint, `0xC000012D` out-of-memory, `0xC0000005` access
violation, `0xC0000409` stack-buffer-overrun/fast-fail).
