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
- Exact current live window count reconciliation (9 in `Layout`, vs. whatever the user's original "5" reflected at the time they reported it) — these are different snapshots at different times in an ongoing, cumulative leak, not a discrepancy to resolve, but worth noting neither number should be treated as "the" bug count.

## 2026-07-07 update — action item 3 root-caused and live-confirmed (task #8)

**The DevTools-toggle/close-reopen theory above (line 22, 50) does not hold up.** Checked
`agentmux-cef/src/client/crash_recovery.rs::on_render_process_terminated` and the recovery page's
Reload button: both navigate the *existing* `Browser` object via `frame.load_url`/`location.href`,
never recreating it. `report_window_opened` fires from exactly two places
(`client/lifecycle.rs::on_after_created`, `commands/window_pool.rs`'s pool-promote path) — both
gated on a genuinely NEW `Browser` object being created. A content reload cannot trigger either.

**Confirmed mechanism, live-reproduced 2026-07-07:** `on_after_created` (`lifecycle.rs:71-83`)
labels a browser `"main"` purely based on `self.state.browsers_is_empty()` — "is this the first
browser *this host process* has registered," not "is this the first-ever main window of the app
session." Separately, the launcher's `handle_goodbye` (`agentmux-launcher/src/reducer/connection.rs:200-221`,
fired on both graceful `Goodbye` and the synthetic one dispatched on an ungraceful host EOF,
`ipc/server.rs:703-728`) only flips `state.processes[pid].state = Exited` — it never touches
`state.windows`, and `WindowMirror` (`state.rs:94+`) has **no field at all** recording which host
connection/session a window entry belongs to, so there is no way for `handle_goodbye` to even find
the right entries to clean up today.

Reproduced directly: launched an isolated instance, then `taskkill`'d the **inner** host process
(not the outer launcher wrapper) — the launcher log showed `CEF host exited abnormally (code 1) —
relaunching (restart 1/3)`, confirming the exact "host dies, launcher survives, host respawns"
precondition this mechanism needs. The respawned host process (fresh `AppState`, `browsers_is_empty()`
true again) created a brand-new `Browser` for its own "Starter workspace" window, confirmed via CDP —
i.e. a **second, genuinely distinct native window gets labeled `"main"` again**, and (since a hard
process kill runs zero in-process CEF shutdown callbacks) `on_before_close`/`report_window_closed`
structurally cannot have fired for the killed host's original `"main"` — there is no code path that
could have paired it off. `window.api.getInstanceNumber('main')` after the respawn returned `1`,
consistent with the launcher's documented duplicate-open-overwrites-not-appends contract
(`reducer/window.rs:57-67`) — the ledger doesn't crash or double-count, it silently drops the prior
entry's history.

**Severity re-assessment: lower than the original "High" above.** This is semantic/ledger corruption
(the launcher's diagnostic view of "main" silently loses continuity across a host restart), not a
resource leak or a count-inflation bug — `state.windows`/`shadow_window_meta`/frontend `knownEntries`
are all label-keyed maps, so a duplicate open overwrites rather than accumulates, and
`instance_registry`'s number is preserved. It's real drift/inaccuracy in observability, not the
unbounded-growth class the rest of this retro is about.

**Fix scope, now understood:** needs `WindowMirror` (or a parallel map) to record an owning
connection/session id, so `handle_goodbye` can find and synthetically close (or otherwise mark)
windows orphaned by a host disconnect before a respawned host's fresh `WindowOpened` silently
overwrites them. This is a real schema addition to `agentmux-launcher`, not a small patch — scoped as
a follow-up, not implemented in this pass. See `docs/specs/SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md`
§2.3 for where this was previously deferred.

## Remediation of the current live session

Not attempted in this session — closing/pruning the 8 orphaned window entries directly (e.g. via manual `window.CloseWindow` calls or a restart) would itself be a live-session mutation of the kind flagged as needing confirmation earlier today. Left for the user to decide: restart the instance (clears all in-memory reducer state cleanly) vs. leave it running for further diagnosis.

## Code fix

Action items 1–2 (make `backend_close_window` observable, fix the silent-skip gate via a bounded retry) are implemented in **PR #1965** (`fix(window): retry backend_window_id lookup on close so CloseWindow reliably reaches srv`), design in `docs/specs/SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md`. Action items 3–6 remain open, tracked as tasks #8–#10 in this session (main-window reload/version bug, window-level test coverage, pagefile-spec cross-reference) plus the still-undecided `SystemProcessInfo`/reconciliation proposal (task #4).

## Code fix, round 2 — PR #1965 was necessary but NOT sufficient (2026-07-05)

Post-merge verification on a fresh isolated build (`g183aecc4`, channel `verify-window-fix-*`) showed the leak **still occurring**: 7 open/close cycles (both rapid and 600ms-delayed) → window records leaked every time, renderer count grew exactly +1 per cycle (5 → 12), and — decisively — **zero `on_before_close` invocations across the entire run** (0 "Unregistered browser" log lines; confirmed at trace level with `AGENTMUX_DEBUG_CLOSE=1`, which showed `register_backend_window` succeeding for the closed window but no close-cascade entries whatsoever afterward).

**Actual root cause, one layer deeper:** on this CEF build, closing a Views window (`window.close()`) destroys the Window/HWND but leaves the hosted browser **hidden/recycled — `on_before_close` never fires**. This was already a *known, documented* property of the codebase: the quit path in `lib.rs` (~line 1050) explicitly works around it ("After a CEF Views last-window close the browsers are HIDDEN/recycled (the close never fires on_before_close)") with a hard `TerminateProcess` (Discussion #1680). What nobody accounted for: **mid-session secondary-window closes hit the same recycle behavior with no compensation at all.** Every open+close of an extra window permanently leaked:

1. a live renderer process (~100MB commit — the pagefile-growth driver for multi-window usage, and a **second, independent renderer-leak mechanism** beyond the browser-pane one PR #1957 fixed);
2. the reducer `browsers` map entry (never unregistered);
3. the entire srv-side window/workspace/tab/block state (because `backend_close_window` lives inside the never-firing `on_before_close` — which is why PR #1965's retry, while correct, never got the chance to run).

Also disproven by the trace: the PR #1965 hypothesis that the `backend_window_id` registration race was the trigger for THIS incident — the mapping was registered and available at close time; the callback that would have consumed it simply never executed. (#1965's fix remains correct and load-bearing for the race it targets, and its cleanup chain is exactly what round 2 re-activates.)

**Round-2 fix:** in `CloseWindowTask` (`ui_tasks/window.rs`), after `window.close()` on the Views path, force the browser itself closed with `close_browser(1)` for non-`main` labels. This makes `on_before_close` actually fire, which re-activates the entire existing cleanup chain (UnregisterBrowser → HWND-cache eviction → launcher mirror → `backend_close_window` with the #1965 retry → srv `CloseWindow` → `delete_workspace` cascade). Safe for secondary windows because they are independent top-level Views widgets — the April pane-cascade failure mode (`WS_CHILD` entanglement, teardown-spike spec §2) structurally cannot occur. `main` is excluded: its close feeds the tuned wrr last-window quit sequence, and process exit reaps everything there anyway.

## Round 2 verification: FAILED. Round 3: break point isolated to inside CEF, between `do_close` and `on_before_close` (2026-07-05)

**Round 2 (close_browser(1) AFTER window.close()) did not work.** Fresh-build verification (channel `verify-round2`): the forcing log line fired 4/4 times, but counts still leaked (srv windows 1→9, renderers 5→9, zero `on_before_close`). Once the Views window teardown has detached the browser, no CEF close API reaches it.

**Round 3 instrumented every link of the cascade** (`can_close`, `do_close`, `on_window_destroyed`, `CloseWindowTask` branches — all dlog-traced) **and swapped the order** to browser-first teardown (`close_browser(1)` *before* `window.close()`). Verification (channel `verify-round3`) still leaks (1→9 windows, 5→9 renderers), but the trace now pins the exact break point. Per closed window:

```
CloseWindowTask(label): close_browser(1) BEFORE window.close()   <- fires
CloseWindowTask(label): window.close()                            <- fires
can_close(label): try_close_browser -> 1                          <- fires, returns "safe to close"
do_close fired; browser_list.len()=N                               <- FIRES — CEF initiated browser destruction
on_before_close                                                    <- NEVER — destruction never completes
```

`browser_list.len()` grows monotonically across closes (4→5→6→7): browsers are appended in `on_after_created` and only removed in `on_before_close`, which never runs. The closed windows' top-level HWNDs *are* destroyed (verified via EnumWindows both rounds — only main + current pool + pane-pool floater remain), so the state after close is: **no Window, no HWND, `do_close` completed, browser parked/detached with its renderer alive, unreachable by any further CEF close call** (a second `close_browser(1)` on it — round 2 — was a no-op).

**Conclusion: the break is inside CEF 148's Views/Alloy teardown, after `do_close` returns false (proceed) — CEF parks/"recycles" the browser instead of completing destruction.** No host-side ordering of `window.close()` / `close_browser()` fixes this; both sanctioned close paths funnel into the same parked state. This matches, and finally explains mechanically, the pre-existing `lib.rs` quit-path comment ("browsers are HIDDEN/recycled (the close never fires on_before_close)").

**Recommended round 4 (not yet attempted — handed off):** bypass the Views close entirely for secondary windows and destroy the top-level HWND natively (`DestroyWindow`, NOT `PostMessage(WM_CLOSE)` — WM_CLOSE routes back through Views' wndproc, which is the old #1680 hide-the-frame failure). The native WM_DESTROY parent→child cascade destroying CEF's browser child HWND is the **exact mechanism PR #1957 proved reliably fires `OnBeforeClose`** for browser panes, and the floating-pane teardown has shipped on it for months. The top-level HWND is resolvable via `state.window_hwnds` / `resolve_window_hwnd` (`commands/window/lifecycle.rs`). Open question for round 4: whether to call `close_browser(1)` first (unload/JS teardown niceties) and then `DestroyWindow`, or `DestroyWindow` alone (what panes/floaters do). Second candidate if that fails: stop leaking at the source — re-enqueue closed promoted windows into the pool (embrace the recycle) — much bigger surgery, pool expects windows with live `cef::Window` handles.

## Rounds 4 + 5: native DestroyWindow — window dies, browser STILL parked (2026-07-05, Agent2)

**Round 4 (`DestroyWindow` alone, strict HWND resolution) executed and verified — INSUFFICIENT.**
Implementation: `resolve_window_hwnd_strict` (validated cache / registry+GA_ROOT only — the
EnumWindows fallback is banned on this path because it can return MAIN) + explicit main-HWND
guard, then native `DestroyWindow` for non-main labels. Forensics per close (channel
`dev/agenta-window-close-force-browser-teardown`):

```
round 4 — DestroyWindow(0x2e0722) owner_tid=90612 my_tid=90612   <- same thread, ownership valid
round 4 — DestroyWindow ret=1 lasterr=0 IsWindow_after=0          <- succeeded, HWND genuinely gone
(no can_close / do_close / on_window_destroyed / on_before_close) <- ZERO CEF callbacks
```

Visible window gone (EnumWindows-verified: only main + pool + floater remain), renderer count
5→6 per cycle — **the leak persists**. Key negative result: **a Views-hosted browser does NOT
tear down when its top-level HWND is destroyed out from under it.** The #1957 WM_DESTROY-cascade
mechanism applies to `set_as_child` browser panes (raw child HWNDs we own), not to
BrowserView/Views plumbing — the browser object survives its own window's native death, parked,
renderer alive.

**Round 5 (`close_browser(1)` to ARM destruction, then `DestroyWindow`) — closer, still
insufficient.** Trace per close:

```
round 5 — close_browser(1) to arm destruction                     <- fires
round 5 — DestroyWindow ret=1 lasterr=0 IsWindow_after=0          <- window death delivered
do_close fired; browser_list.len()=N                              <- browser destruction initiated
can_close(label): try_close_browser -> 1                          <- Views agrees: safe to close
on_before_close                                                   <- STILL NEVER
```

Renderers 5→8 across 3 cycles. The break point is now pinned **one step deeper than round 3
left it: between `can_close → true` and `OnBeforeClose`, i.e. inside CEF 148's
BrowserView→Browser destruction, which parks regardless of whether the window death is
delivered by Views (`window.close()`) or natively (`DestroyWindow`).**

**Solution-space conclusion after five rounds:** every combination of the sanctioned close
APIs and native window destruction funnels into the same parked-browser state. The remaining
viable designs, in recommended order:

1. **Embrace the recycle (pool-demote)** — the handoff's fallback, now the primary candidate:
   `close_window` on a promoted window becomes "demote back into the pool": hide/move
   offscreen, run the srv-side cleanup imperatively at demote time (unregister
   `backend_window_id`, call `backend_close_window` → `CloseWindow` → `delete_workspace`
   cascade — we know the label; no `on_before_close` needed), then re-enqueue as a warm pool
   entry. The renderer is REUSED, not leaked; state is cleaned; and it aligns with what CEF
   148 Views is determined to do anyway. Surgery lives in `window_pool.rs` (promote's inverse).
   Round-5's native-DestroyWindow path must then be REMOVED for pool-eligible labels (the
   window must stay alive to be reused).
2. **Imperative cleanup + accept the parked browser** (fallback if pool-demote stalls):
   keep round 5's close (window dies for real), and do the `on_before_close` work ourselves
   at close time (UnregisterBrowser + backend_close_window with the known label). Fixes
   leaks 2+3 (browsers map + srv state, the count-chip drift) but NOT leak 1 — the ~100MB
   renderer stays until process exit, unless a per-browser renderer-PID reap can be added
   (CEF exposes no cheap browser→renderer-PID attribution; see retro §impact).

Round-5 code + instrumentation stays on the branch: the native close is behavior-improving
(the window actually disappears via one deterministic mechanism), the forensics are
load-bearing for round 6, and the negative results above are the map.

## Round 6: POOL DEMOTE — VERIFIED WORKING (2026-07-05, Agent2)

**Design:** stop fighting CEF's recycle; use it. Closing a promoted pool window now
**demotes** it back into the warm pool instead of destroying it:

1. `demote_srv_cleanup` — the on_before_close cleanup chain, run IMPERATIVELY at close
   time (label known; no callback needed): `backend_close_window` → srv `CloseWindow` →
   `delete_workspace` cascade, with the #1965 registration-race retry, then
   `report_backend_window_id_unregistered` + `report_panes_reaped`.
2. `HostCommand::DemotePoolWindow` — reducer flips `is_pool: true` + re-inserts into
   `unpromoted` (idempotent; leaves the respawn semaphore alone).
3. Park: `SetWindowPos` offscreen + `set_taskbar_hidden(true)`; `window_hwnds` chrome-cache
   evicted; HWND re-cached in `pool_hwnd_cache`; CEF Views `Window` re-cached for the next
   promote's `take_pool_window_view`.
4. Reload to the `pool=1` boot URL — the frontend re-sends `pool_window_ready`, and queue
   re-entry rides the NORMAL `PoolWindowReady` handshake (no mid-reload promote possible).
5. Demote cap `POOL_TARGET_SIZE + 2`: each parked recycle costs the renderer that would
   otherwise LEAK, so keeping it is strictly better up to the cap; beyond it, the round-5
   destroy fallback runs (same cost as the old leak, srv state still cleaned).

**Latent bug found and fixed en route:** `backend_close_window` authenticated via the
`?authkey=` query param — **disabled for HTTP routes in the 2026-05-11 security audit
(C3)** — so it has 401'd on every call since May, unnoticed because `on_before_close`
never fires for these browsers and pre-#1965 the response was never read. Now sends the
`X-AuthKey` header. This bug would have silently broken the cleanup chain even if CEF's
destruction parking were ever fixed.

**Verification (channel `dev/agenta-window-close-force-browser-teardown`, 3 open/close
cycles + reopen):**
- Closes #1/#2: `demoted back into pool`; close #3 hit the demote cap → destroy fallback ✓
- `backend_close_window`: **HTTP 200** on all 4 closes; srv workspace cascade runs
  (closed windows' workspace names are gone from the records) ✓
- **Reopen: the next `open_new_window` promoted a RECYCLED window from the pool, and its
  close re-demoted it — the loop is in equilibrium** ✓
- Renderers 5→8 across the burst: +2 are REUSABLE pool members (pool grew 2→4 by design),
  +1 is the cap-overflow destroy fallback. Steady-state closes leak nothing.

**Remaining follow-ups (out of scope here):**
- srv `CloseWindow` prunes the workspace/tabs/blocks but the WINDOW row itself still
  lingers (visible in `/api/v1/windows` with an empty workspace name) — a pre-existing
  srv-side gap common to every close path; small state, tracked separately (same
  dual-write class as the #864 authority work).
- Non-Windows platforms keep the Views `window.close()` path (no parked-browser evidence
  collected there yet).
- Subwindow children of a demoted window take their own close path (rare).
