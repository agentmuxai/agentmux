# Retro — Linux window-close/quit chain verification + teardown-backstop parity (issue #2189)

**Date:** 2026-07-17
**Status:** Both parts verified live on a `task dev` Linux instance. Part 2 shipped
new code (this branch); Part 1 needed no code changes — #2186's fix already
works correctly on Linux.
**Component:** `agentmux-cef/src/client/lifecycle.rs` (verification only),
`agentmux-launcher/src/{srv_spawner,host_spawn,supervisor/unix}.rs` (new code)

---

## Part 1 — close/quit chain verification

### What was verified

Live on a `task dev` instance, driving window open/close via a raw HTTP IPC
client (see "Direct IPC" below) rather than the app's own UI, so state could
be inspected precisely between steps:

1. **Non-last window close.** Opened a second window, then closed "main"
   while it remained open. The `on_before_close` → `backend_close_window` →
   srv `CloseWindow` → `delete_workspace` saga fired completely:
   `srv_window_closed → saga_started → tab_deleted → active_tab_changed →
   workspace_deleted → saga_completed`. `db_window`'s row for the closed
   window was removed; the remaining window's row was untouched.
2. **Last window close.** Closing the sole remaining window cascaded through
   the app's pre-warmed pool windows (3 more `on_before_close` firings, all
   with no registered `backend_window_id` since they were never promoted) and
   the app quit cleanly within ~150ms of the trigger. `db_window` ended at
   `[]` (fully clean, no leaks). Confirms the pool-window ~1s retry-tail
   latency noted in pre-implementation research is real but not perceptible
   as a hang.
3. **Floating pane close during the quit cascade.** Exercised for free during
   step 2 — a `floating-pool-*` window was part of the same cascade and
   closed cleanly (`on_before_close` fired, `on_window_destroyed` followed).
4. **`CloseWindowTask`'s "main" double-notify branch.** Confirmed by code
   read (not just live test) that this entire block
   (`agentmux-cef/src/ui_tasks/window.rs` lines ~120-145) is
   `#[cfg(target_os = "windows")]`-gated — it cannot run on Linux, so a
   double-notify is structurally impossible here.
5. **Baseline restoration.** Zero orphaned processes in the isolated process
   group after full quit (see Part 2); `db_window` returned to empty.

**Verdict: #2186's `on_before_close` fix works correctly on Linux, unmodified.**
No code changes were needed for Part 1.

### A false start worth recording

Initial testing (before restarting with `AGENTMUX_DEBUG_CLOSE=1`) appeared to
show `on_before_close` **never firing at all** — a search of the host's
`tracing::` log for `on_before_close`'s own unconditional log line
(`"Unregistered browser: ..."`) came up completely empty across the whole
file. This looked like a serious regression and very nearly got reported as
one.

The actual cause: **two `task dev` instances were running simultaneously**
(this session's new build, plus an unrelated earlier session's build still
alive on the same machine), and both CEF hosts wanted the same default
DevTools port (9223). The first-started instance won the bind; the second
silently fell back to a different port. Nothing about that fallback is
visible from `listWindows()`/`window.api` calls — they succeed either way —
so the driver script kept talking to a live browser process the whole time,
just the **wrong one**, with a completely different, older window/log state.
Re-running the identical test against the correctly-identified instance (its
actual DevTools port found via
`grep "CEF remote-debugging port" agentmux-host-*.log`, cross-checked against
the live `authkey.dev` file's `host_pid`) showed the close working perfectly.

This is the same class of trap the original 2026-07-16 retro warned about
("two logging sinks that look similar but aren't") — just one level up: two
*live app instances* that look similar but aren't. Lesson for next time:
when multiple dev builds might be running, confirm the DevTools port from
the target instance's own log line before trusting anything driven through
it, don't just assume the default port belongs to the instance you just
built.

---

## Part 2 — Linux teardown-backstop parity (new code, this branch)

### What shipped

Linux execution of `SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11`'s Phase 2
(Windows-only until now):

- **Process-group isolation at spawn time.** srv gets `.process_group(0)`
  (Linux-only), becoming the leader of a brand-new group separate from the
  launcher's own ambient group. host joins that same group via
  `.process_group(backstop_pgid)` on every spawn/relaunch. This creates a
  bounded container — `{srv, host, host's CEF-spawned descendants}` — the
  launcher was never a member of, satisfying I2/I3 by construction (mirrors
  how Windows' Job Object scopes `TerminateJobObject`'s blast radius, minus
  the launcher).
- **The missing supervisor tick.** `run_unix`'s `select!` loop gained the
  same two low-rate ticks `run_windows` already had: a 60s UI-liveness
  prober (`Command::ProbeUiThread`/`ReportUiThreadAlive` — already
  platform-neutral, now exercised on Linux for the first time) and a 5s
  `teardown_backstop::should_teardown` check. On `true`:
  `libc::killpg(backstop_pgid, SIGKILL)`, then the SAME
  `break TEARDOWN_BACKSTOP_EXIT_CODE` (86) Windows uses — letting the
  existing post-loop cleanup run rather than exiting directly from inside
  the tick (a specific correction from design review: exiting early would
  have skipped `saga_coord.cancel_all_in_flight`, risking dangling
  `SagaStarted` entries).
- `teardown_backstop.rs` / `ui_liveness.rs` — **zero changes**, reused
  as-is; both were already fully platform-neutral.
- `backstop_pgid` is `Option<i32>`, never a bare `0` fallback — `killpg(0,
  ...)` has the special POSIX meaning "my own process group", so a stray
  literal 0 could have made a future teardown kill the *launcher's* own
  group. `None` cleanly disables the backstop for that session instead.

### End-to-end verification (live, full sequence)

Using `AGENTMUX_DEBUG_HANG=1` + the existing `debug:hang_ui` IPC command
(already implemented, unverified on Linux until now) to park the CEF UI
thread, then closing the last window via **raw HTTP IPC** — not CDP:

```
window.api.X() calls in this app do NOT go through Chrome DevTools Protocol
at all — they hit a plain axum HTTP server the host itself runs
(agentmux-cef/src/ipc.rs), Bearer-token-authenticated via the ipc_token in
authkey.dev. A POST to /ipc returns as soon as the command is DISPATCHED
(for close_window, report_window_closed to the launcher happens
SYNCHRONOUSLY before any UI-thread work is even posted) — it never waits for
the UI thread to process anything, which is exactly why it doesn't deadlock
against a parked UI thread the way a CDP Runtime.evaluate round-trip would.
This is almost certainly what the issue's verification recipe means by
"direct IPC" — recommend documenting this explicitly in
SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11.md as the concrete verification
method, since it wasn't obvious in advance.
```

Live trace (timestamps are unix seconds):

```
[ui-liveness] UI thread alive — probe nonce=1 rtt=1ms        (healthy baseline)
[ui-liveness] UI thread alive — probe nonce=2 rtt=1ms
→ debug:hang_ui invoked, then close_window{label:"main"} invoked (same second)
[teardown-backstop] ARMED (OrphanInstance drift — last user window closed, host still alive)
                                                                (armed instantly — report_window_closed
                                                                 doesn't depend on the wedge at all)
[saga] window_cleanup_cascade: ReapPanes, DrainPoolIfLast, SagaCompleted
                                                                (launcher-side bookkeeping unaffected)
+84s  [ui-liveness] probe nonce=3 unanswered after 60s — UI thread did not pump
+144s [ui-liveness] probe nonce=4 unanswered after 59s — UI thread did not pump
+149s [teardown-backstop] host wedged with zero user windows — terminating process group <srv_pid>
      (armed > 30s grace, 2 consecutive unanswered UI-thread probes)
      terminating children (SIGTERM → grace → SIGKILL)
      launcher exiting with code 86
```

149s total from arm to teardown — inside the spec's predicted envelope
(30s grace + 2×60s probe interval ≈ 150s worst case).

**Post-teardown verification (the actual I2/I3 proof, not just the log
saying so):**
- `ps` for the killed pgid: zero surviving processes (all of srv, host, and
  every CEF renderer/zygote/network-service descendant gone).
- Launcher's own log shows `launcher exiting with code 86` — a clean,
  intentional self-exit through the normal post-loop path (SIGTERM→grace→
  SIGKILL cleanup ran, IPC connections closed gracefully), not the launcher
  itself being killed.
- The launcher's PID was confirmed gone from `ps` — but via its own graceful
  exit, not `killpg`: a completely different process group (its own
  `setsid`-wrapper ancestry) than the one that was torn down.
- The shell driving the whole test (a process outside the launcher's tree
  entirely) was unaffected throughout — the blast radius genuinely never
  left the spawned tree.

### Residual notes (not addressed, out of scope for this issue)

- **macOS parity (issue #2188).** Everything in Part 2 is
  `#[cfg(target_os = "linux")]`-gated, not general `#[cfg(unix)]` — no
  behavior change on macOS. `setpgid`/`killpg` are POSIX and should port
  directly, but this was deliberately left unverified since there's no macOS
  hardware in this environment.
- **`AGENTMUX_DEBUG_HANG`/`AGENTMUX_DEBUG_CLOSE`/direct-IPC verification
  recipe** should probably be written into
  `SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11.md` itself as a documented,
  repeatable procedure — this retro is the first time it's been spelled out
  end-to-end for any platform.

## References

- `docs/specs/SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11.md`
- `docs/retro/retro-last-window-close-quit-race-2026-07-16.md` (#2186 root
  cause + fix; the two-log-sinks lesson this retro's "false start" section
  extends)
- Issue #2189
