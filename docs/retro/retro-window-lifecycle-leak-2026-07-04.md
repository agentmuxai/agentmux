# RETRO — window close never reaches the reducer; launcher's open-window mirror also drifts

**Date:** 2026-07-04
**Author:** AgentA
**Severity:** High — `state.windows` (the reducer's canonical window map) is effectively write-only in current production usage; the user-visible "(N)" window-count chip and its expanded panel drift upward and never recover; directly relevant to the pagefile/memory-growth investigation that prompted the test that surfaced this.
**Area:** `agentmux-cef/src/client/lifecycle.rs` (`on_before_close`), `agentmux-cef/src/client/helpers.rs` (`backend_close_window`), `agentmux-srv/src/server/service/window.rs` (`CloseWindow`), the launcher's `WindowOpened`/`WindowClosed` ledger (`launcher-ipc`), `frontend/app/statusbar/InstancePanel.tsx`.

---

## Summary

The user opened 4 additional windows (deliberately, to stress-test pagefile growth — the same investigation that led to today's renderer-leak PR #1957) and closed all 4. The status-bar chip and its expanded panel should have returned to 1 window; instead they showed 5, listing all 5 as still open.

Investigating live: `Layout(query=windows)` (reading directly from the srv reducer) showed **9** window entries, only one with a real workspace name — confirming the leak is not just a UI display bug, it's the backend's own canonical state. Tracing both the srv and host logs for this entire session found **zero** `window.CloseWindow` RPC calls ever reaching `agentmux-srv`, despite 6 `WindowOpened` launcher events and 4 confirmed `WindowClosed` events. The reducer's window map has been strictly append-only for the whole session.

## Timeline (this session, host log timestamps)

| Time (UTC) | Event | Note |
|---|---|---|
| 08:39:27 | `WindowOpened` label=`main`, version=1 | Session start |
| 19:12:00.987–19:12:01.001 | 3× `[on_before_close] no backend window ID registered for label=... — shells may orphan` | Pre-promote pool-window churn (background pool refill), not user-visible windows |
| 19:14:41.109 | `WindowOpened` label=`main`, **version=4** | The *same* main window re-registering (version jumped 1→4) — consistent with a window reload/reconnect, e.g. the DevTools-toggle / close-reopen workaround suggested earlier this session for the stale empty layout cell (see the earlier "refresh the window" exchange). **No matching close for any of versions 1–3 exists anywhere in the log.** |
| 19:43:46.802–19:43:52.065 | 4× `PoolWindowPromoted` + `WindowOpened` (~2s apart) | The deliberate 4-window pagefile test |
| 19:43:54.437–19:44:01.888 | 4× `WindowClosed` | The user closing all 4 — clean 4-open/4-close pairing on the launcher's own ledger |
| (this investigation) | `Layout(query=windows)` → 9 entries; srv log → 0 `window.CloseWindow` calls ever | Confirms the reducer-side leak is universal, independent of whether the launcher's own ledger looks paired or not |

## Impact

- The status-bar window-count chip and expanded panel drift upward indefinitely and never self-correct — every window ever opened in a session appears to remain "open" from the reducer's point of view.
- `workspace.DeleteWorkspace`'s cascade-cleanup (tabs, blocks) is gated inside the same `window.CloseWindow` service handler (`window.rs:355-368`) and never runs for any window closed this way — meaning **closed windows' workspaces, tabs, and blocks are never cleaned up server-side either**, not just the window entry itself.
- Directly relevant to the pagefile/OOM investigation (`docs/specs/SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md`) that motivated the user's test: orphaned workspace/tab/block state accumulating in the reducer for the life of the process is exactly the kind of unbounded growth that investigation was looking for, separate from (and in addition to) the CEF renderer-process leak PR #1957 fixed today.

## Root cause

Two distinct, compounding bugs — one universal (affects every window close), one narrower (affects a subset and explains part of the visible chip drift):

### 1. The only path to `window.CloseWindow` is gated on a lookup that isn't reliably populated

`state.windows` is only ever pruned by the srv `window.CloseWindow` service method (`agentmux-srv/src/server/service/window.rs:308-379`), which correctly dispatches `Command::CloseWindowInternal` and cascades to `delete_workspace` when appropriate. Across the whole codebase there are exactly two callers:

- One frontend JS call site (`frontend/app-init.ts:361`), fired only inside a narrow app-init self-heal path (recreating a window whose workspace is missing) — never during normal use.
- `backend_close_window` (`agentmux-cef/src/client/helpers.rs:53`) — a fire-and-forget raw-TCP POST to `/agentmux/service` with `service=window, method=CloseWindow`. This is called from `on_before_close` (`agentmux-cef/src/client/lifecycle.rs`, ~line 887), **only if** a `backend_window_id` lookup for that window's label succeeds. When it misses, the call is skipped entirely — logged only as a warning (`"no backend window ID registered for label={:?} — shells may orphan"`), no retry, no fallback, no user-facing signal.

This session reproduced the miss 3 times (19:12, pre-promote pool churn). But even the 4 *clean* closes at 19:43–19:44 (no warning logged, launcher ledger perfectly paired) never produced a `window.CloseWindow` call in the srv log either — `dlog()`-level tracing inside `backend_close_window` (which would show the actual TCP attempt) isn't captured in the log surface available for this investigation, so it's unconfirmed whether those 4 attempted the call and failed silently (it's fire-and-forget with no response check — a wrong port, stale auth key, or srv-side rejection would be invisible), or didn't attempt it at all. Either way, the observable fact holds: zero `CloseWindow` calls reached srv across the entire session.

### 2. The launcher's own `WindowOpened`/`WindowClosed` ledger separately drifts on window reload

`report_window_closed` (the call that keeps the status-bar chip / `InstancePanel` in sync — a *different* mechanism from #1, gated only on the CEF browser identity resolving to a `label`, not on `backend_window_id`) is nested inside `if let Some(ref lbl) = label { ... }` in the same `on_before_close`. When `label` itself can't be resolved (the `label=None` case, 1 of the 3 warnings at 19:12), this call is skipped too — a second, independent gap in the same function.

Separately, and probably the more user-visible contributor here: the main window re-registered as `WindowOpened` at 19:14:41 with **version=4** (having started the session at version=1), with no corresponding close logged for versions 1–3 anywhere. A window reload/reconnect (e.g. via DevTools toggle or close-and-reopen — the exact workaround suggested earlier this session for the stale empty layout cell) appears to register as a brand new "open" in the launcher's ledger without the previous version being paired off first.

## Why it wasn't caught

- `backend_close_window`'s fire-and-forget design (explicit in its own doc comment: *"we write the request and don't read the response"*) means a failure at the srv side, or even a failure to send at all, produces zero error signal anywhere a normal test would notice — only a debug-level `dlog()` call not captured by standard log tailing.
- The `on_before_close` → `backend_window_id` gate's failure mode is a `tracing::warn!`, not `error!`, and its own message ("shells may orphan") undersells the actual blast radius — it reads as a shell-process concern, not "this window's entire workspace/tab/block state, and its entry in the window-count chip, will never be cleaned up."
- No test or CI check exercises the open→close→count-returns-to-baseline round trip for real (non-pool, non-error-recovery) windows — the existing coverage (per `SPEC_BROWSER_PANE_LIFECYCLE_TESTS.md`, referenced from today's earlier PR #1957 investigation) is scoped to browser *panes*, not whole windows.
- This is architecturally the same shape of bug PR #1957 fixed today for browser panes — CEF's close/teardown callbacks not firing reliably, or firing without enough information to complete cleanup — just one level up the hierarchy (windows, not panes), and not caught by that PR's own validation protocol because that protocol was pane-scoped.

## Action items

1. **Make `backend_close_window` observable and retryable.** Read the HTTP response (even just the status code) instead of fire-and-forget; log an `error!` (not `warn!`) on failure with the window_id, and consider a retry or a periodic reconciliation pass (compare srv's `state.windows` against the launcher's live window list; prune anything srv thinks is open that the launcher/OS no longer has).
2. **Fix or remove the `backend_window_id` gate's silent-skip.** If the lookup can legitimately miss for windows that were never meant to have one (pre-promote pool churn), the gate should distinguish "this window never had a backend_window_id by design" (no-op, correct) from "this window had one and we lost track of it" (real bug, must not silently skip cleanup).
3. **Investigate the main-window version-increment-without-close pattern.** A window reload (DevTools toggle, close/reopen) registering as a brand-new `WindowOpened` without pairing off the prior version is a separate, concrete bug worth its own trace — tie to the earlier "refresh the window" request in this session if that's confirmed as the trigger.
4. **Add an open→close→count-returns-to-baseline test at the window level**, mirroring what `SPEC_BROWSER_PANE_LIFECYCLE_TESTS.md` already does for panes — this class of bug keeps recurring one level of the hierarchy at a time (browser panes today, whole windows here) and each level has needed its own dedicated validation protocol to catch it.
5. **Cross-reference `SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md`.** This orphaned-workspace/tab/block accumulation is a plausible independent contributor to that investigation's memory-growth findings, separate from the CEF-renderer-process leak PR #1957 addressed. Worth checking whether that spec's data already shows this pattern or needs a fresh look with this mechanism in mind.
6. **Add a `SystemProcessInfo`/window-reconciliation surface**, per the still-undecided `docs/specs/SPEC_AGENT_SYSTEM_MANAGEMENT_API_2026_07_04.md` proposal from earlier today — this entire investigation was only possible because I could manually cross-reference `Layout(query=windows)` against raw log greps; there's no first-class way for an agent (or a human) to ask "does srv's window count match reality" without this kind of manual archaeology.

## Open questions / not yet resolved

- Whether `backend_close_window`'s TCP POST is actually attempted and failing silently for the 4 "clean" closes at 19:43–19:44, or never attempted at all — needs `dlog()`-level output, which isn't in the log surface this investigation had access to.
- Whether the main-window version=4 re-registration at 19:14:41 is actually caused by the DevTools-toggle/close-reopen suggestion from earlier this session, or something else — not confirmed with the user.
- Exact current live window count reconciliation (9 in `Layout`, vs. whatever the user's original "5" reflected at the time they reported it) — these are different snapshots at different times in an ongoing, cumulative leak, not a discrepancy to resolve, but worth noting neither number should be treated as "the" bug count.

## Remediation of the current live session

Not attempted in this session — closing/pruning the 8 orphaned window entries directly (e.g. via manual `window.CloseWindow` calls or a restart) would itself be a live-session mutation of the kind flagged as needing confirmation earlier today. Left for the user to decide: restart the instance (clears all in-memory reducer state cleanly) vs. leave it running for further diagnosis.

## Code fix

Action items 1–2 (make `backend_close_window` observable, fix the silent-skip gate via a bounded retry) are implemented in **PR #1965** (`fix(window): retry backend_window_id lookup on close so CloseWindow reliably reaches srv`), design in `docs/specs/SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md`. Action items 3–6 remain open, tracked as tasks #8–#10 in this session (main-window reload/version bug, window-level test coverage, pagefile-spec cross-reference) plus the still-undecided `SystemProcessInfo`/reconciliation proposal (task #4).
