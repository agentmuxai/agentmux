# SPEC — WRR quit gate fires on a live window (false exit on non-last window close)

- **Status:** Draft → implementing
- **Date:** 2026-07-08
- **Author:** AgentA
- **Scope:** A minimal, independently-shippable slice of L1 from
  `SPEC_REDUCER_SSOT_CONSOLIDATION_2026_06_22.md` ("retire the win_event parallel authority").
  Deliberately does **not** attempt L4 (typed pane-pool `BrowserKind`), L2/L7 (reducer-wide
  `reconcile_quit` wiring), L3 (six-count collapse), or L8 (fold `orphan_reconcile`) — those stay
  exactly as documented in that spec and `STATUS_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_07_07.md`.
- **Reported by:** user (this machine), build `77ce3113` (mid-branch of PR #2032, merged into `main`
  2026-07-08T16:02:11Z): opened 4 windows, closing one (not the last) killed the main window/whole
  host. Separately, the window count also read stale after closing all 4 — tracked separately by
  `SPEC_WINDOW_COUNT_STALE_ON_VIEWS_CLOSE_2026_06_22.md` (already largely fixed; not this spec's
  concern).
- **Confirmed NOT caused by PR #2032** — the buggy code (`wrr/win_event.rs`) is untouched by that
  PR. It's a pre-existing gap PR #2032's added window/pool churn made easier to hit.

---

## 1. Symptom

Opening several windows and closing one that is **not** the last live window can kill the entire
host process (main window included) — an unrecoverable false exit, not merely a display glitch.

## 2. Root cause

`wrr/win_event.rs::maybe_quit_on_last_user_window` (win_event.rs:284-324) is the **live** quit
trigger on Windows (confirmed by `STATUS_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_07_07.md` §2 — the
main-window close path never reaches `reconcile_quit`/`on_before_close` at all). It runs on every
`EVENT_OBJECT_HIDE`/`EVENT_OBJECT_DESTROY` of any app-class window and decides using **only** a raw
`EnumWindows` snapshot (`count_visible_user_windows`, win_event.rs:236-279):

```rust
let registered = app_state().get().map(|s| s.count_live_user_windows()).unwrap_or(0);
let armed = HAD_VISIBLE_USER_WINDOW.load(SeqCst);
if !armed { return; }
let visible = count_visible_user_windows();
tracing::debug!(... "registered={} os_visible={}", registered, visible);
if visible != 0 { return; }          // <-- decision uses ONLY `visible`
...
cef::quit_message_loop();
```

`registered` — the reducer's own `count_live_user_windows()`, the single source of truth
`reconcile_quit` is built around — is **already computed and logged right here**, but is never
consulted for the decision. `visible` is a synchronous `EnumWindows` pass that can transiently
misread during the pool-refill/promote churn PR #2032's reproject feature (and ordinary multi-window
use) generates on the same UI thread these events are delivered on — a window that is genuinely
still open can momentarily fail the `IsWindowVisible`/pixel-heuristic filter (e.g. a promoted pool
window not yet moved on-screen), reading `visible == 0` while real windows remain. Because the check
is edge-triggered on *any* window's HIDE/DESTROY, closing a non-last window can hit this transient
zero and quit the whole process.

### 2.1 — Why "just also check `registered`" isn't safe as a one-line AND (found during design)

The obvious fix — `if visible != 0 || registered != 0 { return; }` — is **not safe today** without a
prerequisite fix. `registered` is not yet reliable in one specific, already-documented case: the CEF
Views main-window "recycle-on-close" (hides/reuses the browser instead of destroying it — the
original motivation for `win_event.rs` existing at all, see its own header comment,
win_event.rs:201-214). On that path `on_before_close` never fires, so `UnregisterBrowser` is never
dispatched to the CEF reducer's own `browsers` map — `registered` stays stuck ≥1 forever.

This case is *already* detected today, precisely — `EVENT_OBJECT_LOCATIONCHANGE`'s "Gap A" pool-move
handler (win_event.rs:503-546) recognizes the recycle (HWND jumps off-screen post-HIDE) via
`label_for_hwnd` + `is_live_top_level_browser` (the authoritative typed check, not a label-prefix
guess) and calls `report_window_closed(label)` — but **only** to the launcher mirror (fixing the
frontend's "(N)" count, `SPEC_WINDOW_COUNT_STALE_ON_VIEWS_CLOSE_2026_06_22.md` Part 1). It never
tells the CEF-side reducer. So naively ANDing `registered` in today would reintroduce the exact bug
`win_event.rs` was built to prevent (#1676 — orphaned process tree, quit never fires) for this one
case.

**Conclusion: fix the recycle-close gap first, then the AND is safe.**

## 3. Fix — two ordered, small changes

**Step A — teach the CEF reducer about a recycle-close (closes the `registered`-never-decrements
gap).** Extend the existing, already-precise LOCATIONCHANGE pool-move detector
(win_event.rs:520-541) to also dispatch `UnregisterBrowser` to the CEF reducer, not just report to
the launcher:

```rust
if state.is_live_top_level_browser(&label) {
    tracing::debug!(target: "wrr", "[wrr] LOCATIONCHANGE pool-move → report_window_closed label={}", label);
    crate::launcher_ipc::report_window_closed(label.clone());
    // NEW: keep the CEF-side reducer's own bookkeeping in sync too — this is the
    // one path where on_before_close never fires (Views recycle-on-close), so
    // `count_live_user_windows()` would otherwise never see this window go away.
    state.host_dispatch(crate::reducer::HostCommand::UnregisterBrowser { label });
}
```

Idempotent by construction — `handle_unregister_browser` (`reducer/browsers.rs:106-123`) already
no-ops (`DispatchOutput::default()`) when the label isn't present, so this is harmless if a genuine
`on_before_close` also fires for the same label (it won't, for the recycle case, but the guard means
we don't have to prove that — see §5).

**Step B — make the quit decision require reducer agreement.** Now that `registered` is kept
accurate for every real close (normal `on_before_close` path *and* the recycle path via Step A),
change the decision at win_event.rs:312:

```rust
// before:
if visible != 0 { return; }
// after:
if visible != 0 || registered != 0 { return; }
```

Both values are already computed on lines 298-306 for the debug log — this is a pure decision-logic
change, no new state, no new threading surface. `registered` reads
`crate::reducer::count_live_user_windows` — the exact function `reconcile_quit` itself uses — so this
is "consult the single source of truth" in spirit, without pulling in the full `reconcile_quit`/
`request_drain`/`QuitState` machinery (deferred — see §6).

## 4. Why this preserves the #1676 fix (must not regress)

The AND in Step B can still resolve `true` (both zero) in every case that used to correctly quit:
- Normal window closes: `on_before_close` already fires and dispatches `UnregisterBrowser` today —
  unaffected.
- Views main-window recycle-close: Step A makes this dispatch `UnregisterBrowser` too, so `registered`
  reaches 0 exactly when `visible` does (both driven by the same underlying OS transition, just two
  different detectors of it).

No case exists (post-Step-A) where a real "last window gone" event leaves `registered` stuck
non-zero forever — only transiently, until the corresponding detector (on_before_close or
LOCATIONCHANGE) catches up, which is bounded by normal event delivery, not indefinite.

## 5. Explicitly out of scope / non-goals

- **Not retiring `count_visible_user_windows`/the EnumWindows heuristic.** Kept as a live
  confirming signal (defense-in-depth), not deleted — full retirement is the larger L1 as originally
  scoped in the SSOT doc; this slice only fixes the false-positive direction.
- **Not touching `commands/window_pool.rs`'s discarded `request_drain`** (quit.rs:69-75) — the
  pool-refill-completion race the Pillar 2 spec flagged as its own remaining gap. Separate, larger
  work; this fix reduces exposure (the transient miscount is a symptom of the same underlying churn)
  but does not close that gap directly.
- **Not touching `orphan_reconcile.rs`'s independent third decider** (L8) — untouched, as before.
- **Not L4** (typed `BrowserKind` pane-pool variant) — not needed here: `is_live_top_level_browser`
  and `count_live_user_windows` already exclude pool/floater/pane windows **by type**
  (`BrowserKind::TopLevel{is_pool:false}` only), not by label prefix, so this fix rides on
  already-typed, already-correct exclusion logic.

## 6. Risks / things to verify live (not just by inspection)

1. **Ordering between HIDE and LOCATIONCHANGE for the recycle case.** If `EVENT_OBJECT_HIDE` (which
   also calls `maybe_quit_on_last_user_window`) fires before the LOCATIONCHANGE pool-move event lands,
   `registered` may still read stale for one event cycle. Expected effect: a brief delay in quitting
   on a genuine last-window recycle-close, not a hang — LOCATIONCHANGE fires shortly after per the
   existing (already-shipped) Gap A mechanism. Must live-verify this doesn't measurably regress
   quit latency or, worse, race into a stuck-open zombie process.
2. **Re-run the original #1676 repro as this fix's own regression test**: close the last window via
   the normal path, and separately via the recycle path, and confirm the host still exits cleanly
   both ways (per SSOT §6 hard constraint: "must not regress #1676").
3. **The reported repro itself**: open 4 windows, close a non-last one, confirm main survives and the
   remaining 3 stay open.
4. **Double-dispatch of `UnregisterBrowser`** for the same label from both the normal close path and
   Step A's new call — confirmed safe by inspection (idempotent, §3), but worth a log-level sanity
   check during live verification (should never observe two `BrowserUnregistered` events for one
   label in a single close in practice, but harmless if it happens).

## 7. Testing plan

- Unit test: extract the two-condition decision (`visible == 0 && registered == 0`) into a small
  pure function (mirroring `should_begin_drain`'s pattern, `reducer/quit.rs:150-162`) so the truth
  table is testable without Win32/CEF — table: (visible, registered) × 4 combinations → quit y/n.
- Live verification (matching this session's established methodology): build a portable, then run
  the three scenarios in §6.2-6.3 with `tracing` at debug level for target `wrr`, confirming the
  `registered`/`visible` log line values at each step match expectations before/after the fix.

## 8. Rollout

Single phase — this is intentionally small (two changed call sites, no new state, no new commands).
Land both steps together (Step B is unsafe without Step A, per §2.1); do not split across releases.

## Addendum 2026-07-09 — first cut regressed; redesigned around the real gap

**The §3 design as first implemented caused a worse regression than the bug it fixed**, caught by
live verification (fresh isolated instance, only `main` open, `closeWindow('main')`): the process
hung forever with `registered_user_windows=1 os_visible=0`. Steps A+B alone are insufficient
because §2.1's premise — "the LOCATIONCHANGE pool-move detector reliably catches every
recycle-close" — is false for the case that matters most:

- **`main` is deliberately excluded from every close-cleanup round.** `CloseWindowTask`
  (`ui_tasks/window.rs`) documents "Scope: NOT for main — main's close feeds the tuned wrr
  last-window quit sequence and process exit reaps everything there." Main's `window.close()`
  parks the Views browser: no `on_before_close`, no demote, no off-screen pool-move for Step A to
  catch. The reducer was **designed** never to learn about main's close — safe only while the WRR
  quit ignored the reducer. Step B broke that hidden contract.
- The earlier multi-window "success" was the pool **demote** path (`DemotePoolWindow` flips
  `is_pool`, correctly excluding recycled windows from the count) — not Step A, which never fired.
- Bonus root-cause collapse: `orphan_reconcile`'s "freshly-promoted" skip of main
  (`orphan_reconcile.rs:127-134`: live HWND + absent from launcher shadow = blocks drain) is the
  SAME stale-registration, not a separate bug — main post-recycle-close is exactly "live hidden
  HWND, launcher already removed it."

**Amended design (implemented):**

- **Step C (the real §3.3 transition):** `CloseWindowTask::execute` dispatches
  `UnregisterBrowser { "main" }` when closing main (Windows-gated; on macOS/Linux
  `on_before_close` fires properly and pre-unregistering would break its label-by-identity
  cleanup chain). The UI-thread executor of the close is the one place the label is known with
  certainty and CEF callbacks are known not to fire. `UnregisterBrowser` is quit-relevant, so
  `reconcile_quit` runs under the same dispatch.

  **Ordering is load-bearing (second live-caught bug, 2026-07-09):** the dispatch MUST run
  AFTER the close is initiated. The first cut ran it at the top of `execute()` — but
  `get_window_on_ui` and the `get_browser` fallback both resolve through `state.browsers`, so
  the pre-close dispatch deleted the registration the close itself needed and the whole
  `CloseWindowTask` silently no-opped: the window just stayed open on screen. Now in
  `unregister_main_after_close`, called at both close-path exits.

  **And the HIDE event is NOT guaranteed:** live run showed main's close can produce zero
  further HIDE/DESTROY win-events (the Views park is a *move*), so nothing re-runs the WRR
  gate even with correct counts. `unregister_main_after_close` therefore also arms the Step D
  watchdog explicitly — the guaranteed quit evaluation. Same run also confirmed the predicted
  root-cause collapse: with main unregistered, `orphan_reconcile` stopped misclassifying it as
  freshly-promoted, correctly entered drain, and closed all pool browsers — the launcher-side
  `HostShouldQuit` path works again for the first time since the recycle-close era began.
- **Step D (bounded fallback, restores the resilience the AND removed):** when
  `armed && visible == 0 && registered > 0` (pure fn `is_reducer_lagging_os`), arm a one-shot
  3s watchdog (`QUIT_WATCHDOG_GRACE`); after the grace a UI-posted task re-checks and, if the OS
  still reports zero visible windows, quits on the OS signal alone (pre-fix behavior) with a loud
  "reducer desync, investigate" warning. Any future missed-dispatch path degrades to
  quit-3s-late-with-telemetry instead of hang-forever. Safe for virtual-desktop switches: cloaked
  windows remain `IsWindowVisible`, so `visible` never reads 0 there (same property the pre-fix
  code relied on).
- **Step A retained** as a backstop for non-RPC off-screen recycles, now followed by an immediate
  `maybe_quit_on_last_user_window()` re-check (the pool-move may be the last event that window
  ever fires).
- Steps A+B unchanged otherwise. §4's regression analysis stands, now actually satisfied by C.

Also fixed en route: `wrap_task!` in `win_event.rs` needs explicit `use cef::rc::Rc as _` +
`use cef::{ImplTask, Task, WrapTask}` (the macro is unhygienic; other call sites get these via
`use cef::*`), and the task struct needs ≥1 field for the generated refcount impl.

## 9. Sources

- `docs/specs/SPEC_REDUCER_SSOT_CONSOLIDATION_2026_06_22.md` (L1, the item this slices from)
- `docs/specs/SPEC_PILLAR2_WIRE_RECONCILE_QUIT_2026_06_29.md` (§3.3, §7 — WRR gap previously scoped,
  not started)
- `docs/status/STATUS_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_07_07.md` §2 (confirms WRR is still the
  dominant, unwired quit path)
- `docs/specs/SPEC_WINDOW_COUNT_STALE_ON_VIEWS_CLOSE_2026_06_22.md` (the sibling frontend-count bug,
  same recycle-close root event, different symptom)
- Code read for this spec: `agentmux-cef/src/wrr/win_event.rs:196-324,494-546`,
  `agentmux-cef/src/reducer/quit.rs:116-172`, `agentmux-cef/src/reducer/browsers.rs:106-123`,
  `agentmux-cef/src/state.rs:1429-1479`, `agentmux-cef/src/commands/window/lifecycle.rs:38-80`,
  `agentmux-cef/src/client/lifecycle.rs:580-692`.
