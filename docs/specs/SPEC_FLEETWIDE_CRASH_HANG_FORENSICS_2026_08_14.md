# SPEC: Fleet-wide crash & hang forensic logging — implementation plan

**Status:** proposed — not built as of 2026-09-26. Written 2026-08-14 and checked in on 2026-09-26 unchanged apart from this header; re-verify its code references before implementing.
**Supersedes scope of, and incorporates by reference:**
`SPEC_HOST_UI_THREAD_HANG_WATCHDOG_2026_08_14.md` (that spec's design for the
host process specifically is Phase 1+2 below, unchanged — this doc widens the
same problem to every process type in the fleet).
**Motivated by:** the 2026-08-14 host hang incident, where nothing captured
any forensic evidence before the wedged process was killed — the root cause
of the deadlock itself is still unknown as a direct result.

## Goal

Every process type in the app — backend (`agentmux-srv`), host/browser
process, CEF renderer/GPU/utility subprocesses, and the frontend JS running
inside the renderer — should produce actionable forensic evidence (a stack
trace and/or minidump, correlatable to a specific incident) on both **hard
crashes** and **hangs**. Today only the backend has crash (not hang)
coverage; everything else has partial or no coverage. This plan closes that
asymmetry.

**Non-goals:** fixing the specific 2026-08-14 deadlock (that's a follow-up,
blocked on this plan actually producing evidence); building a remote
crash-reporting/upload pipeline — this is local forensic capture only unless
a separate decision is made to ship dumps off-machine.

## Grounding: what exists today (verified by code inspection, not assumed)

- **`minidumper`'s `request_dump` cannot be used for hangs.** It requires a
  real `CrashContext` produced by an actual exception on the faulting
  thread (`agentmux-srv/src/crash_monitor.rs:214-231`). There is no
  "dump now" message in its API. A hang produces no exception and the
  wedged thread can't be made to call anything. **A hang dump must come
  from a separate, external dumper** (same technique as `procdump -h`:
  another process calls `MiniDumpWriteDump` against the target PID from
  outside — this does not require the target's threads to run). This is
  why Phase 1 below is a new, standalone primitive, not a reuse of
  `crash_monitor.rs`.
- **On Windows, host + renderer + GPU + utility processes are literally the
  same executable**, re-exec'd with a `--type=` flag
  (`agentmux-cef/src/lib.rs:105-111,307-310`); only macOS has a distinct
  helper binary. Practically: any PID-targeted dumper or WER registration
  built once covers every process role for free — there's no per-role
  binary to wire up separately on Windows.
- **Renderer crashes are already handled and logged** (not silently
  swallowed): `RequestHandler::on_render_process_terminated`
  (`agentmux-cef/src/client/handlers.rs:569-577`) →
  `crash_recovery.rs:25-100` logs `tracing::error!(target:"crash",
  kind="renderer_terminated", reason, error_code, ...)` and drives a
  Reload/Quit recovery UI under a crash budget. **No stack trace or dump is
  captured**, just status/exit-code text. GPU-process and utility-process
  termination handling is **unconfirmed** — needs a follow-up check before
  Phase 4 is scoped precisely (open question below).
- **Frontend JS already forwards uncaught errors to the host log**:
  `frontend/log/error-forwarder.ts` hooks `window.onerror` (L172) and
  `unhandledrejection` (L201), resolves source-mapped stacks, and ships them
  via `invokeCommand("fe_log_structured", …)` (L89, L115). This is
  logging-only — no dump, and it doesn't catch a hung (not throwing) JS main
  thread.
- **WER LocalDumps is registered for the backend only**, keyed to the exact
  versioned srv exe filename (`tools/enable-crash-dumps.reg:13`,
  `scripts/register-crash-dumps.ps1:63`). Nothing registers the host/CEF
  exe. The handful of `agentmux-cef.exe` `.dmp` files already found on this
  machine are Windows' own unconfigured default WER queuing — no guaranteed
  path, retention, or dump-type configuration behind them.

## Plan

### Phase 1 — External hang-dump primitive (foundational)

Build a small, process-role-agnostic capability: given a target PID, output
path, and a reason tag, capture an external minidump via `OpenProcess` +
`MiniDumpWriteDump` (the `procdump -h` technique), without requiring any
cooperation from the target's own threads. Natural home: `agentmux-launcher`,
which already tracks the host's PID via the supervisor and already talks to
it over IPC (`supervisor/windows.rs`, `ui_liveness.rs`). Write dumps to a
distinctly-named directory (e.g. `CrashDumps\agentmux-hang\`), separate from
exception-triggered crash dumps — "we don't know what happened, we grabbed
a snapshot" and "we caught the actual exception" mean different things to
whoever investigates next, and should never land in the same bucket.

This one primitive, once built, is reused unchanged by every later phase
that needs a hang dump for any process role, since they're all the same
exe.

### Phase 2 — Host (browser/main) process: hang detection → dump → recycle

As detailed in `SPEC_HOST_UI_THREAD_HANG_WATCHDOG_2026_08_14.md`: extend
consumption of `ui_liveness.rs`'s probe misses beyond `teardown_backstop`'s
zero-window gate; tighten cadence to match srv's existing 10s/3-miss
pattern (`SPEC_SRV_HANG_WHILE_ALIVE_DETECTION_2026_08_03.md`) instead of the
60s interval that's tuned for a different consumer. On a declared wedge:
call Phase 1's dumper against the host PID with `reason="ui_thread_hang"`,
then force-kill and let the existing supervisor relaunch — the same outcome
the manual `taskkill` had in the 2026-08-14 incident, just automatic and in
~30s instead of ~16 minutes.

### Phase 3 — Host/browser process: real crashes (not just hangs)

Two complementary changes, both cheap relative to Phase 1/2:

1. Extend `register-crash-dumps.ps1` / `enable-crash-dumps.reg` to also
   register WER LocalDumps for the host/browser exe name(s)
   (`agentmux-<version>.exe` packaged, `agentmux-cef.exe` dev builds —
   see `binary_resolution.rs:8-46`), with an explicit path/count/type
   configuration instead of relying on whatever Windows' unconfigured
   default happens to do.
2. Optionally also wire `crash_monitor.rs`'s `crash-handler`+`minidumper`
   approach directly into the host's browser-process role, since that path
   *does* work correctly for genuine exceptions (just not hangs) — gives an
   immediate, structured, correctly-tagged dump the instant it happens,
   the same guarantee srv already has, rather than depending solely on
   WER's best-effort out-of-band queue.

### Phase 4 — CEF subprocess (renderer/GPU/utility) crash forensics

- Renderer termination already has structured status/error-code logging
  (`crash_recovery.rs`) — no detection changes needed there.
- **Open question, needs a follow-up code check before scoping further:**
  is GPU-process or utility-process termination handled/logged at all
  today, or only renderer? If unhandled, decide whether it needs the same
  treatment as renderer or is low enough impact to defer.
- Since these are the same exe as the host (Phase 3's premise), Phase 3's
  WER registration covers renderer/GPU/utility crashes for free — no
  additional registration work required here, just verification that it
  actually fires for a `--type=renderer` crash and not only `--type=` unset
  (browser).

### Phase 5 — Frontend (JS): hang detection + evidence correlation

`error-forwarder.ts` already covers *thrown* errors well. Two additions:

1. **JS-side responsiveness heartbeat**: a cheap `requestAnimationFrame` or
   `setTimeout`-based ping posted to the host at a fixed interval. If the
   host stops seeing it for N seconds, that's an independent signal from
   the native `ui_liveness` probe — useful because a JS-thread hang (e.g. a
   pathological render or synchronous loop) and a native message-pump hang
   can have different root causes, and having both signals lets triage
   immediately narrow which layer to look at.
2. **Correlation IDs**: tag `fe_log_structured` payloads with the same
   PID/instance-id scheme introduced in Phase 6, so a JS stack trace and a
   native dump from the same incident can be tied together without manual
   cross-referencing (which is what the 2026-08-14 investigation had to do
   by hand against `netstat` output).

### Phase 6 — Cross-cutting: naming, correlation, retention

Design alongside Phases 1-3, not bolted on after:

- Every dump and every relevant log line (srv, host, renderer/GPU/utility,
  frontend forwarder) gets PID + instance/channel ID + build version + a
  reason tag (`hang` / `crash` / `manual`), so one incident's full evidence
  trail is greppable without manual correlation. This also satisfies the
  postmortem's standing action item about per-instance log tagging in the
  shared `agentmuxsrv-v{version}.log.{date}` file.
- Define an explicit retention/rotation policy for every dump directory
  introduced here, matching or extending whatever currently governs srv's
  dumps (check `register-crash-dumps.ps1`'s configured limits rather than
  assuming).
- **PII/security**: minidumps of host/renderer processes can contain
  in-memory pane and session content. Access control and retention for the
  new dump directories must be reviewed against whatever policy governs
  srv's existing crash dumps before this ships — do not assume today's
  unconfigured WER defaults are an acceptable bar.

### Phase 7 — Verification

- A debug-gated hook to inject an artificial deadlock in the host, to
  confirm Phase 1+2 actually detects it, dumps it, and recycles within the
  expected ~30s window.
- A debug-gated hook (or CEF's existing crash-trigger mechanism, if any) to
  force a renderer/GPU crash, confirming Phase 3/4's WER registration
  actually captures it end to end.
- An injectable JS throw/hang to confirm Phase 5's heartbeat and
  correlation IDs work.
- Add all three to whatever manual QA/smoke checklist exists today, or flag
  explicitly if none exists yet — this class of bug is exactly the kind
  that's invisible until it happens live, so it needs a standing check, not
  a one-time verification.

## Sequencing

1. **Phase 1 + 2 first** — directly closes the exact gap that caused the
   2026-08-14 incident; smallest surface, highest leverage.
2. **Phase 3** next — cheap (mostly config: extend one script with more exe
   names), and removes the "hangs get evidence but crashes still don't"
   asymmetry this plan would otherwise leave for the host.
3. **Phase 4** is largely free once Phase 3 lands (same exe), modulo the
   GPU/utility open question.
4. **Phase 5** is independent and parallelizable with 1-4.
5. **Phase 6** threads through all of the above — assign correlation-ID/
   retention design before Phase 1 starts, don't retrofit it after.
6. **Phase 7** throughout, not deferred to the end — each phase should ship
   with its own verification hook, not rely on the next real incident to
   prove it works.

## Open questions

- GPU/utility-process crash handling coverage today (Phase 4) — unconfirmed,
  needs a direct follow-up code check.
- Whether Phase 3's "wire `crash_monitor` into the host too" sub-option is
  worth the complexity given WER registration alone (Phase 3.1) already
  closes most of the gap — recommend starting with WER-only and revisiting
  once real data exists on how often WER's best-effort path actually fires
  vs. misses.
- Where correlation IDs (Phase 6) should be minted from — likely the same
  instance/channel ID already used elsewhere in the system (e.g. the
  `local-main-b28b7a-cdf87dde` style channel names seen in this incident) —
  confirm the canonical source before implementing rather than inventing a
  new ID scheme.
