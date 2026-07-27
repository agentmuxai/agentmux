# Retro: `agentmux-srv` Dies ~9m36s-9m40s Into Every Idle Dev Instance

**Date:** 2026-07-26
**Affected versions:** 0.54.4 (dev build, branch `agenta/deterministic-login-ux`); likely others — nothing found is version-specific
**Severity:** High — blocks unattended dev-instance use past ~9.5 minutes; unresolved
**Status:** Root cause NOT found. Multiple plausible mechanisms ruled out with hard evidence. Next diagnostic step identified (Process Monitor), not yet run.

---

## How this was found

Discovered as a side effect while manually verifying an unrelated fix
(`agenta/deterministic-login-ux`, PR #2300) — every dev instance launched to
click through the Claude login flow died before the manual test could
finish. First reproduced in a prior session (`68e3c900-ae30-4835-8a01-
fa3d8b922d7d`, 2026-07-25/26) during an unrelated auth battle-test, where the
first two occurrences were caused by a since-fixed muxbus credential bug
(oversized keychain write) — but a third and fourth occurrence in that same
session had none of those symptoms and were never explained. This retro
covers the follow-up investigation in the next session, which reproduced it
2 more times (6 total) and definitively ruled out several hypotheses.

## Timeline (this session's 3 directly-measured reproductions)

| Instance | First paint (UTC) | Died (UTC) | Elapsed | Notes |
|---|---|---|---|---|
| 1 | 2026-07-26T02:53:12.886 | 2026-07-26T03:02:51.545 | **9m38.7s** | `RUNTIME CRASH — pid=6380` logged. No prior muxbus/pipe warnings. |
| 2 | 2026-07-26T03:04:18.968 | after 03:13:55 (last heartbeat) | **≥9m36s** | No `RUNTIME CRASH` line logged this time (process just stopped appearing in `tasklist`); a non-fatal `srv-ipc` pipe-bind warning was present at startup (see below, ruled out as related). Session was interrupted around this time, so the exact death instant and whether a crash line was ever emitted is not fully certain. |
| 3 | 2026-07-26T06:59:53.268 | 2026-07-26T07:09:30.195 | **9m36.9s** | `RUNTIME CRASH — pid=7264` logged. This run had a correctly-configured WER LocalDumps registration (see below) — still produced **zero** minidump. |

All three: idle instance, no login attempts, no user interaction, plenty of
free memory (~22-23GB available of 32GB). The ~9m37-39s window is
tight enough across independent processes/builds that this reads as
deterministic, not a random fault.

## What we ruled out (with evidence, not guesses)

1. **Any code-level timer/duration in the 540-660s range.** Two independent
   full-workspace greps (`agentmux-srv`, `-cef`, `-launcher`, `-common`) for
   `Duration::from_secs`/interval/deadline constants in that band found
   several candidates (`OOM_RESTART_WINDOW=600s`, session timeouts at
   `300`/`600s`, broker sweep thresholds) — every one of them only flips an
   in-memory status flag or gates a *later* restart decision; none call
   `exit`/`panic`/`abort`/`kill` as a direct consequence of elapsed time.
   `agentmux-launcher/src/mem_supervisor.rs`'s `OOM_RESTART_WINDOW` (the
   closest-named candidate) was read end-to-end and confirmed to be pure
   restart-eligibility bookkeeping, never a killer.

2. **The `srv-ipc` pipe-bind failure** (`[srv-ipc] bind failed on
   \\.\pipe\agentmux-<hash>\srv-command: Access is denied. (os error 5)`,
   seen on instance 2's startup). Traced to `agentmux-srv/src/bootstrap.rs`'s
   `bind_srv_pipe_ipc` — a best-effort, fire-and-forget optional side
   channel (srv runs fine on HTTP/WS without it; nothing waits on it, no
   retry, no timeout path). `ERROR_ACCESS_DENIED` on
   `CreateNamedPipe(FILE_FLAG_FIRST_PIPE_INSTANCE)` specifically means
   *another instance of that exact pipe name already exists* — i.e. a
   still-tearing-down prior dev instance, not a real ACL problem. Cosmetic;
   unrelated to the crash (instance 1 died with no pipe warning at all).

3. **A genuine Windows fault** (access violation, `__fastfail`/
   `STATUS_STACK_BUFFER_OVERRUN`, stack overflow) — the same class as the
   documented `retro-recurring-sidecar-crash-0xC0000409.md` precedent. Ruled
   out two ways:
   - **Zero Windows Application-log events** mention "agentmux" anywhere in
     the 8 hours surrounding multiple reproductions (`Get-WinEvent` swept
     the full Application log, all providers). A genuine unhandled exception
     *always* generates at least a basic "Application Error" event at the OS
     level — this is independent of any WER dump configuration. Total
     silence here is strong evidence there was no fault to report.
   - **Zero minidumps**, even after fixing WER LocalDumps to match the
     actual current binary name (see next section) and confirming `WerSvc`
     running, `ForceDumpsEnabled=1`, and correct folder ACLs. Instance 3 was
     run specifically to test this and still produced nothing.

4. **srv's own graceful stdin-EOF shutdown**
   (`agentmux-srv/src/bootstrap.rs:1511-1534`, `install_shutdown_handlers` —
   exits cleanly on `read()` returning `Ok(0)` or an error, matching Go's
   `stdinReadWatch`). This path unconditionally `eprintln!`s either
   `"stdin closed, shutting down"` or `"stdin read error: {e}, shutting
   down"` before cancelling. **Neither string appears anywhere** in the
   fully-captured stderr log (srv's stderr is forwarded verbatim into the
   host's tracing log by `agentmux-cef/src/sidecar.rs`) leading up to any of
   the three crashes. If this path had fired, we would see it.

5. **Any of srv's own intentional `std::process::exit()` calls.** All 15
   call sites in `bootstrap.rs` are on startup-only failure paths (CLI
   subcommand completion, config/DB/migration errors, port-bind failure) —
   structurally unreachable once the server is up and idle. No
   `process::abort()` or custom `panic::set_hook` exists anywhere in
   `agentmux-srv/src`.

6. **An external `.kill()`/`TerminateProcess` call from `agentmux-cef` or
   `agentmux-launcher`** on the srv child handle. Every such call site was
   found and is startup-only (30s/1800s ESTART timeout in
   `srv_spawner.rs`) or user/shutdown-triggered (`restart_backend`,
   launcher teardown) — none are a background health-check/heartbeat that
   could fire ~9.5 minutes into a healthy, still-open instance. No code
   anywhere polls srv's HTTP endpoint on an interval and kills+respawns it
   on failure — no such construct exists.

## A real, separate bug found and fixed along the way

The March 2026 retro (`retro-recurring-sidecar-crash-0xC0000409.md`)
configured Windows Error Reporting `LocalDumps` specifically so the *next*
crash of this kind would produce a minidump. Checking the registry
(`HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps`)
found it registered only for `agentmuxsrv-rs.exe` — a binary name that
predates the later per-build/version-isolation naming scheme. The actual
binary running today is `agentmux-srv-{version}-windows.x64.exe` (confirmed
from the dev launch log: `...\runtime\agentmux-srv-0.54.4-windows.x64.exe`).
WER matches by **exact filename**, so this registration has silently
matched nothing for months — **no crash of the real binary, this bug or any
other, has been diagnosable via minidump since the rename.**

Fixed by `scripts/register-crash-dumps.ps1` (new, this PR), which reads the
current version from `package.json` and (re-)registers the LocalDumps key
for the exact current binary name. It's idempotent and self-elevates via
UAC. **Limitation:** because WER matches by exact filename and this repo's
srv binary embeds its version in the name, this registration goes stale
again on every version bump — there's no wildcard support in WER LocalDumps.
Re-run the script whenever chasing a sidecar crash, especially after a
version bump. A more durable fix would be a `SetUnhandledExceptionFilter`-
based in-process handler that doesn't depend on OS-level filename matching
at all (suggested but never implemented in the original March retro's
Next Steps #3) — not attempted here; scope was diagnosing THIS bug, not
rebuilding the crash-capture pipeline.

Confirmed the fix works mechanically (registry key verified present,
correct `DumpFolder`/`DumpCount`/`DumpType`) — it just turned out this
particular crash isn't the kind WER would catch anyway (see above).

## Current working theory (unproven)

With every "srv does something to itself" mechanism ruled out, and zero
trace anywhere in Windows' own crash/event infrastructure, the most likely
remaining explanation is that **something terminates the process
instantaneously from outside** — either:

- A Windows Job Object closing/tearing down and killing all its members
  (the launcher explicitly manages srv under a Job Object per this repo's
  I1-I6 isolation invariants — a handle-lifecycle bug here is plausible and
  wasn't specifically audited in this pass, only explicit `.kill()` call
  sites were), or
- Something entirely outside this codebase (antivirus, EDR, or another
  device-management/monitoring tool on this specific development machine)
  killing a process it doesn't recognize or flags as suspicious after some
  fixed grace period.

Both would produce exactly what we observe: no exception (nothing to log,
no event, no dump), no chance for srv's own shutdown code to run (a hard
kill doesn't let `eprintln!` or cleanup handlers execute), and a
consistent ~9.5 minute window if the trigger is itself deterministic
(e.g. a fixed-delay policy in whatever is doing the killing).

## What's Been Done

| Action | Status |
|--------|--------|
| Ruled out timers/intervals as direct cause | Confirmed, two independent sweeps |
| Ruled out the `srv-ipc` pipe-bind warning as related | Confirmed |
| Ruled out a genuine Windows fault/exception | Confirmed (zero event log entries, zero dumps with correct config) |
| Ruled out srv's own stdin-EOF shutdown | Confirmed (expected log line absent) |
| Ruled out srv's own intentional exit calls | Confirmed (all startup-only) |
| Ruled out a known internal `.kill()` call site | Confirmed (all startup/shutdown-only, no heartbeat-driven kill exists) |
| Fixed WER LocalDumps to match the current binary name | Done — `scripts/register-crash-dumps.ps1` |
| Reproduced with corrected WER config to test for a dump | Done — still zero dumps |
| Checked Windows Application event log for any trace | Done — zero agentmux-related entries |

## Next Steps (in priority order)

### 1. Trace the exact termination with Sysinternals Process Monitor
The fastest remaining path to ground truth. Run Process Monitor filtered to
the srv PID through one more full ~10-minute reproduction. This will show
definitively whether the process calls `NtTerminateProcess` on itself, is
terminated by another named process (and which one), or something else
entirely (e.g. a Job Object kill won't necessarily show as a discrete
"terminate" event from another process — cross-reference with
`Get-WinEvent`/Sysmon Event ID 5/25 process-termination events if
Process Monitor alone is inconclusive).

### 2. Audit the launcher's Job Object handle lifecycle on Windows
Specifically: does anything ever close or lose track of the Job handle srv
is assigned to, or could `AssignProcessToJobObject` + a
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` policy fire unexpectedly under some
condition that isn't a deliberate shutdown? Review
`agentmux-launcher/src/supervisor/windows.rs` and wherever the Job Object is
created/held, against invariants I2/I3 in CLAUDE.md.

### 3. Rule out non-AgentMux causes on this specific machine
Check installed antivirus/EDR/device-management software's own logs for any
action against `agentmux-srv-*.exe` around the crash timestamps. This
machine runs several other resource-heavy apps (Traktor, etc.) — also worth
checking Windows' own diagnostic data / Reliability Monitor
(`perfmon /rel`) for anything correlated.

### 4. If Process Monitor shows a genuine unhandled exception after all
Re-examine why WER still produced no dump despite correct configuration —
possible remaining explanations: the dump-writing WerFault.exe process
itself failing silently, a race between fast process teardown and dump
generation, or Credential Guard / another security feature blocking the
minidump write. Cross-check `C:\Windows\System32\LogFiles\WMI\
RtBackup\` or `wevtutil qe Application` with raw XML for a WER fault-bucket
event that didn't reach the friendly log view.

---

## Appendix: Crash Fingerprint

```
Binary:       agentmux-srv-0.54.4-windows.x64.exe
Uptime at death: 9m36.9s - 9m38.7s (3 direct measurements, tight window)
Trigger:      none observed — fully idle instance, no user activity
Exit signature: silent — no Windows Application-log event, no WER minidump
                (even with correct LocalDumps registration), no srv-side
                shutdown log line, no known internal kill call site fired
Reproductions: 6 total (4 in session 68e3c900 2026-07-25/26 — first 2 tied
                to a since-fixed muxbus credential bug, last 2 unexplained;
                2 more in this session, both directly measured above)
Related but distinct: retro-recurring-sidecar-crash-0xC0000409.md
                (documented multi-HOUR uptime crash under load, confirmed
                __fastfail/STATUS_STACK_BUFFER_OVERRUN — that mechanism is
                ruled out here by the zero-event-log/zero-dump evidence,
                so this is very likely a different bug, not a recurrence)
```
