# PLAN — WRR quit watchdog kills the whole host on a live, non-draining window

- **Status:** Draft → implementing
- **Date:** 2026-08-03
- **Reported by:** user (this machine), v0.54.9
- **Scope:** A minimal, bounded extension of Step D from
  `docs/specs/SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md`. Does not touch Steps A-C, the
  `should_quit_on_last_window` gate itself, or any of the reducer/`reconcile_quit` machinery.
  Deliberately does not attempt the larger L1 "retire the win_event parallel authority" effort
  those specs reference — this is a targeted fix for one specific gap in the existing bounded
  fallback.

---

## 1. Symptom

AgentMux (v0.54.9) closed unexpectedly with the main window and ~9 open panes (agent panes,
terminal panes, one Browser pane) all disappearing at once — not a crash (no dump, no Windows
Event Log entry, no error in `agentmux-launcher.log`), just a clean process exit (code 0).

## 2. Evidence

Reconstructed from the per-session structured host log
(`<channel>/versions/0.54.9/logs/agentmux-host-v0.54.9.log.<date>`, target `wrr`), all times UTC
2026-08-03:

| Time | Event |
|---|---|
| 14:51:08.43 | Browser pane navigates to `accounts.youtube.com/accounts/SetSID` |
| 14:51:09.27 | → `accounts.google.com/signin/oauth/id` (Google OAuth handoff) |
| 14:51:10.87 | → `accounts.google.com/signin/oauth/consent` |
| 14:51:10.99 | `accounts.google.com/gsi/transform` request **aborted** (`ERR_ABORTED`) |
| 14:51:11.03 | That pane's browser is torn down: `BrowserUnregistered` / `BrowserPaneClosed` |
| 14:51:11.04 | `[wrr] arming 3000ms quit watchdog (reducer counts 1 live)` |
| 14:51:11.04–14:51:14.07 | A burst of unrelated window-pool churn lands in the same window: multiple `PoolWindowLeft { reason: DestroyedBeforePromote }`, `DriftDetected { kind: Pool, host_count: 2, mirror_count: 3 }`, and repeated pool window spawn/destroy cycles (the drag/tear-off warm-pool refilling itself) |
| 14:51:14.04 | `[wrr] quit watchdog fired: 0 visible for 3000ms but reducer disagrees (registered=1 draining=false) — quitting on OS signal alone (reducer desync, investigate)` |
| 14:51:14.05 | `cef::quit_message_loop()` called — entire host exits |

The layout at the time had ~10 blocks across multiple panes in a single `main` window — nowhere
near an actual last-window-close. `registered=1 draining=false` is as unambiguous a "the reducer
thinks a real, running window is still open and nobody asked to quit" signal as this codebase
produces.

## 3. Root cause

`agentmux-cef/src/wrr/win_event.rs`'s `QuitWatchdogRecheckTask::execute` (Step D of
`SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md`) has exactly **one** re-arm path today:

```rust
if visible != 0 {
    if draining && registered == 0 {
        // "post-drain debris" — re-arm
        arm_quit_watchdog(registered);
        return;
    }
    // stand down — a window is visible again
    return;
}
// visible == 0 falls straight through to here — NO re-arm option at all,
// regardless of what `registered`/`draining` say:
if QUIT_INITIATED.swap(true, SeqCst) { return; }
tracing::warn!(... "reducer desync, investigate" ...);
cef::quit_message_loop();
```

The existing re-arm only covers the *mirror-image* desync (OS says something's still visible,
reducer says drained-and-zero). There is **no equivalent retry budget** for the flavor that
actually fired here: OS says zero visible, but the reducer says a live, non-draining window
(`registered > 0 && !draining`) is still registered. That flavor gets exactly one 3-second grace
period, then the code unconditionally trusts the OS `EnumWindows` snapshot and kills the whole
process — even though the reducer's own bookkeeping is the *more* trustworthy signal in this
specific case (a window nobody decided to close, still counted as live).

`count_visible_user_windows` is a synchronous `EnumWindows` pass on the UI thread
(win_event.rs:248-291); the same spec doc that introduced Step D already documents this pass as
capable of transiently misreading during "window-pool refill/promote churn on the same UI
thread" (§2 of `SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md`). The captured log shows exactly that
churn (repeated `PoolWindowLeft`/`DriftDetected`/pool-respawn events) landing in the same 3-second
window as the Browser pane's OAuth-triggered close — the same class of transient misread the
spec already anticipated, just on the `visible == 0` side instead of the `visible != 0` side, and
with no retry budget to absorb it.

The Browser pane's close is the trigger (its underlying browser tore down right after the
`gsi/transform` request aborted — consistent with Google's GSI/OAuth completion script closing
its own popup-style frame), but it is not itself the bug: any pane close that happens to land
inside a window-pool churn burst can hit this same one-shot fallback.

## 4. Fix

Give the `registered > 0 && !draining` desync flavor the same kind of bounded retry budget the
`draining && registered == 0` flavor already has, instead of one immediate unconditional quit:

1. **`should_extend_lag_retry(registered, draining, retries_used) -> bool`** — new pure decision
   function (same style as `should_quit_on_last_window`/`is_reducer_lagging_os`, unit-testable
   without Win32/CEF): `registered > 0 && !draining && retries_used < WATCHDOG_LAG_RETRIES_MAX`.
2. **`WATCHDOG_LAG_RETRIES_MAX: u32 = 3`** — bounded, so this remains "quit a few seconds late
   with a loud log" and not a regression back toward the invisible-zombie failure mode Step D was
   built to avoid (must not regress #1676). Worst case adds
   `WATCHDOG_LAG_RETRIES_MAX * QUIT_WATCHDOG_GRACE` = 9s of extra grace, only when a live,
   non-draining window is genuinely still registered.
3. **`WATCHDOG_LAG_RETRY_COUNT: AtomicU32`** — tracks consecutive re-arms granted to this flavor;
   reset to 0 whenever the watchdog stands down cleanly (a window is OS-visible again) or when it
   ultimately fires (fresh budget for the next occurrence).
4. **Diagnostic dump on the anomalous paths only** (`diag_dump_app_windows`) — a one-off
   `EnumWindows` pass (not the hot per-event path) that logs hwnd/class/title/visible/iconic/rect
   for every app-class window belonging to the process, called right before each lag re-arm and
   right before the final quit-fire. Turns "reducer desync, investigate" from a bare pair of
   counts into an actual list of what the OS thinks exists, for the next time this fires.
5. The `registered == 0 && !draining` flavor (a genuinely missed `request_drain` consumption
   site — no live window to protect) is unchanged: it still fires on the first watchdog cycle,
   same as today.

## 5. Why this preserves the #1676 / Step-D guarantees

- Still bounded — a genuinely stuck reducer (never updates `registered`/`draining` again) quits
  after `WATCHDOG_LAG_RETRIES_MAX + 1` cycles instead of 1, not never.
- Does not touch `should_quit_on_last_window` (the primary, non-watchdog quit gate) or the
  `registered == 0` desync flavor.
- Only extends grace for the specific case where the reducer reports a live window that nobody
  asked to close — the one case where "quit anyway" is actively wrong, not just premature.

## 6. Risks / things to verify live

1. **Extra shutdown latency on a genuine last-window recycle-close that races this exact
   condition** — bounded to +9s worst case; needs live verification it doesn't regress perceived
   quit responsiveness for the common case (should be unaffected — the common case resolves on
   the very first re-check, same as today).
2. **The diagnostic dump's cost** — one extra `EnumWindows` + per-window property reads, only on
   the already-anomalous path (at most a few times per occurrence), not the per-WINEVENT hot
   path.
3. Cannot live-repro the original Google-sign-in-timed pool-churn race in this environment
   (requires a live Windows CEF build + an actual OAuth flow); this fix is verified by unit test
   plus `cargo check -p agentmux-cef`, matching the verification bar the originating spec
   (`SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md` §7) used for its own initial land.

## 7. Testing plan

- Unit tests for `should_extend_lag_retry` covering: live+non-draining+under-budget (extend),
  live+non-draining+at-budget (don't extend, fall through to fire), draining (don't extend —
  that's the other flavor / straightforward quit), `registered == 0` (don't extend — no live
  window to protect).
- `cargo check -p agentmux-cef`.

## 8. Sources

- `docs/specs/SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md` (Step D, the fallback this plan extends)
- `docs/retro/retro-last-window-close-quit-race-2026-07-16.md` (related but distinct window-close
  gap, same subsystem, confirms this area was still being hardened through mid-July)
- Code read for this plan: `agentmux-cef/src/wrr/win_event.rs:196-573`
- Log evidence: this session's incident triage (Google-sign-in-in-Browser-pane → full app exit),
  cross-referenced against the current `agentmuxai/agentmux` `main` (`1899c5e`)
