# Retro: closing the LAST window ("main") never notified srv at all

**Date:** 2026-07-16
**Severity:** Medium — permanent srv-side `db_window`/`db_workspace` row leak, resurrected by crash-reproject on every subsequent launch
**Tracking:** `docs/specs/SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md` §4c Round 3 (this fix)
**Fix (confirmed, live-verified):** `agentmux-cef/src/ui_tasks/window.rs` (`CloseWindowTask::execute`, "main" branch)
**Fix (defense-in-depth, not the live Windows path):** `agentmux-cef/src/client/lifecycle.rs` (`on_before_close`) — see §5

---

## 1. How it was found

While smoke-testing a batch of merged PRs in a `task dev` instance, the user closed all open windows one at a time via the normal UI close button, then relaunched. Crash-reproject (Pillar 1) recreated a window titled "Starter workspace" that the user had explicitly closed — a "ghost" window reappearing after being deliberately closed.

Live inspection of the instance's `objects.db` (`db_window` table) and `srv-events.log` confirmed:

- `db_window` still contained a row for the closed "Starter workspace" window (`winsize: {width:0, height:0}`, stale placeholder geometry).
- Tracing that specific `window_id` through the *entire* `srv-events.log` found **zero events of any kind** for it — not even an attempted-and-failed close. Every *other* window closed in the same session produced a full saga: `tab_deleted → active_tab_changed → workspace_deleted → saga_completed → srv_window_closed`.

This ruled out a transient race (which would at least show a failed attempt) and pointed at a structural gap: this window's close never even tried to notify srv.

## 2. First hypothesis (wrong location, right instinct)

The first read of `AgentMuxHandler::on_before_close` (`client/lifecycle.rs`, Pillar 2's Phase B.9.3 two-stage close cascade) found what looked like the bug: its Stage 2 branched on whether the closing browser was the *last* one —

```rust
if self.browser_list.is_empty() && !self.is_browser_pane {
    quit_message_loop();
} else {
    // ...notify srv via backend_close_window, with retry logic...
}
```

— with the `backend_close_window` notify living **only** in the `else` arm, so the last window's close would skip it. This diagnosis was fixed (see §5) and is real, but **live testing (`getApi().closeWindow()` on the last window, then grepping the host's own `tracing::` log) proved `on_before_close` never actually runs for this case on this CEF build.** The Rust host log showed the real path taken:

```
[close-window] main close initiated — dispatching UnregisterBrowser (parked browser fires no on_before_close)
```

— a completely different function, `unregister_after_parking_close` (`ui_tasks/window.rs`), explicitly documented (`SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md`) as existing *because* CEF 148 Views parks (hides + recycles) the browser on `window.close()` instead of destroying it, so `on_before_close` never fires for the dominant Windows close path. `on_before_close`'s own fix is real and harmless to keep (see §5), but it was not the live bug.

## 3. Actual root cause (confirmed via `tracing::` log, not `dlog()`)

**Lesson learned mid-investigation:** this codebase has two logging sinks that look similar but aren't — `cef-debug.log` (forwarded JS `console.*` calls only) and the real Rust `tracing::` output (`agentmux-host-v*.log`, found via `muxlog ls`, one file per running instance). `dlog()` calls are a *third*, silent-by-default sink (`AGENTMUX_DEBUG_CLOSE=1` gated). Diagnosing this correctly required grepping the actual host log, not the CEF debug log.

`getApi().closeWindow()` → the `close_window` IPC command → `CloseWindowTask` (`ui_tasks/window.rs`). That task has explicit, well-documented exclusions for the `"main"` label throughout — e.g.:

```rust
// Scope: NOT for "main" — main's close feeds the tuned wrr
// last-window quit sequence and process exit reaps everything there.
```

and a `self.label != "main"` guard around the entire block that calls `demote_srv_cleanup` (the function that actually does `backend_close_window` → srv `CloseWindow` RPC → `delete_workspace` cascade) for every `window-*` label. **`"main"` was the one label that never reached `demote_srv_cleanup`, or any equivalent, anywhere in this function** — on the documented assumption that closing main is the same as quitting the app, and "process exit reaps everything." It doesn't: `agentmux-srv` is a *separate* process with its own persistent SQLite-backed object store. Nothing else ever tells it that main's window closed.

## 4. The fix (confirmed, live-verified)

Added a `self.label == "main"` branch at the top of `CloseWindowTask::execute` that runs the same backend-notify logic (`backend_window_id` lookup with bounded retry, then `backend_close_window`) every `window-*` label already gets via `demote_srv_cleanup`.

**Critical detail — this call is synchronous, not fire-and-forget on a background thread** (unlike `demote_srv_cleanup`'s own pattern, which is fine for `window-*` labels since the process stays alive for them). First attempt used a background thread and it lost the race *every time*: the host's own shutdown sequence (`lib.rs`, right after `run_message_loop()` returns) logs `"Killing backend sidecar"` and kills the `agentmux-srv` child process — confirmed live to happen **within ~50ms** of the close starting, well before an async HTTP round-trip (even a fast localhost one) reliably completes.

The fix instead blocks the UI thread. This works *because* CEF's message loop is single-threaded: this task, the WRR win-event callback that eventually calls `quit_message_loop()`, and everything else downstream all run on that same thread — so blocking here transitively delays the entire quit sequence (and the sidecar kill) until the notify finishes, capped by the existing bounded retry (~1s) and `backend_close_window`'s own timeouts (~2s). Acceptable: this only affects the final moments of an already-closing app.

## 5. Defense-in-depth fix kept alongside (not the live Windows bug, but real)

The original `on_before_close` fix from §2 was kept: the notify block there was *also* skip-on-last-window structured, and while CEF-148-Windows never reaches it for `"main"`'s own close, `on_before_close` **does** fire for other browsers in the same handler (confirmed live: floating-pool windows closing as part of the Stage-1 pool-drain cascade go through it normally), and per its own comment is the *correct, reliable* path on macOS/Linux (`window.close()` → `can_close` → `on_before_close` runs the full chain there, no parking). Leaving the old skip-on-last-window branch in place would still be a live bug on those platforms/paths. Implementation: the notify logic now runs unconditionally; `quit_message_loop()` (UI-thread-only) is deferred via a new `QuitMessageLoopTask` posted from the background notify thread once it completes, instead of firing immediately and racing ahead of it.

## 6. What this doesn't fix

- **A genuinely unreachable srv** (already dead, or slower than the bounded retry/timeout) still leaves the row orphaned — this fixes the *common* case, not every failure mode. The durable fix is the reconciliation-pass idea already tracked in `SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md` §2.3 (`SystemProcessInfo`-style periodic cross-check), still not implemented.
- **macOS/Linux were not live-tested** — the `on_before_close` fix (§5) is reasoned from its own doc comments (`window.close()` → `can_close` → `on_before_close` runs the full chain there) but not empirically re-verified on those platforms this session.
- Shutdown latency: closing the last window now briefly blocks the UI thread on the notify call before the process exits. Bounded (~1s retry + ~2s HTTP timeouts, worst case), judged acceptable for the final moment of an already-closing app.

## 7. Verification

- `cargo check -p agentmux-cef` — clean, both fixes.
- Live repro (before fix): closing "main" (the last window) — zero events for its `window_id` anywhere in `srv-events.log`.
- Live repro (after fix, synchronous branch): closing "main" now produces the full `srv_window_closed → saga_started(delete_workspace) → tab_deleted → active_tab_changed → workspace_deleted → saga_completed` sequence. `db_window` row count confirmed to drop to `0` afterward (was carrying a leaked row from before this fix — closing "main" with the fix in place cleaned up that pre-existing leak too, not just prevented new ones).
