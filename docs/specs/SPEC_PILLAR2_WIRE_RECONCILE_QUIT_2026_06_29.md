# Pillar 2 — Wire `reconcile_quit` as the Single Lifecycle Authority

**Date:** 2026-06-29
**Status:** Implementation spec (ready to build)
**Parent:** `SPEC_ARCHITECTURE_HEALTH_AND_REFACTOR_2026_06_29.md` (Pillar 2),
`DISCUSSION_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_06_29.md`
**Prior art:** `SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21.md` (§5.1, §10), `retro-lifecycle-teardown-churn-2026-06-22.md`
**Issues:** #768 (host/frontend lifecycle divergence), #1681, #1461, #1662

---

## 1. Goal

Make the **quit decision** level-triggered and single-authority. The pure decision function
`reducer::quit::reconcile_quit` is already written, tested, and `#[allow(dead_code)]` (`reducer/quit.rs:151`).
This spec **wires it in** behind the existing UI-thread drain executor, and **demotes the three
current edge-triggered decision sites to pure executors.** No new decision logic — this is the
"connect the already-built fix + add the safety-net test" step the code comments explicitly defer to
(`reducer/quit.rs:56-60`).

**Non-goals:** changing the Stage-1 (close pool browsers) / Stage-2 (`quit_message_loop`) executor;
touching the saga layer; the host-reproject work (Pillar 1). This is the smallest change that stops
the active orphan/teardown bleeding.

## 2. The bug being fixed (why edge-triggered fails)

Today the "should we quit?" decision is computed **only at the moment a browser closes**, inside
`client::on_before_close` (`client/mod.rs:1008,1084`): after `UnregisterBrowser`, it counts live user
windows and, if 0, dispatches `BeginDrain`. This is **edge-triggered** — evaluated once, at one event.

Failure mechanism (confirmed in the orphan host log, retro §3): if a concurrent pool refill/promote
keeps `count_live_user_windows()` from reading 0 at that exact instant, `BeginDrain` never fires, and
**nothing re-evaluates the decision** when the racing pool window later settles → host never drains →
Job Object never drops → whole process tree orphans. The same race resurfaced under new symptoms 5+
times (#601, #702, #1612, #1647/#1650/#1676).

`reconcile_quit` fixes this by being a **pure function of current `HostState`** that can be re-run
after *any* transition and always reaches the right answer — so a later transition catches what the
close-edge missed.

## 3. The wiring design — one chokepoint, not scattered calls

### 3.1 Trigger: reconcile after every quit-relevant reducer transition
`reconcile_quit` must run after any reducer command that can change its three inputs
(`count_live_user_windows`, `user_creation_in_flight`, `quit_state`). Rather than sprinkle calls at
each call site, add a **single post-dispatch hook in the reducer dispatch path** (`reducer/mod.rs`,
where `host_dispatch` applies a command and returns `DispatchOutput`). After applying a command in
the quit-relevant set, run `reconcile_quit(state)` (pure, already under the host-state lock) and, if
it returns `Some(reason)`, emit a new `DispatchOutput` signal requesting drain.

Quit-relevant command set (the only ones that can flip the answer):
| Command | Why it matters | Site |
|---------|----------------|------|
| `UnregisterBrowser` | a window/pane went away (the classic last-close) | `client/mod.rs:825` |
| `RegisterBrowser` | a promote produced a real `TopLevel{is_pool:false}` (must NOT drain) | `client/mod.rs:459` |
| `PromotePoolWindow` / pop+promote | pool → real window flips `is_pool` (`reducer/mod.rs` H.4 arm ~344) | reducer |
| `Enqueue/DequeuePendingWindowCreation` | a user "New Window" started / resolved / aborted | `client/mod.rs:382`, reducer |
| `DropUnpromotedPoolWindow` | a pre-warm window died externally | reducer H.4 |

All other commands skip the check (cheap guard — match on the variant).

> **Stage 1 implementation note (landed):** the guard is implemented as a **negative**
> match (`is_quit_relevant`) — only the drag-opacity hot path and the browser-pane lifecycle
> are excluded; everything else defaults to relevant. This is fail-safe: a future window/pool
> command can't silently miss reconciliation. `DispatchOutput` gained `request_drain:
> Option<QuitReason>`; `update()` sets it after relevant commands. Behavior-neutral until Stage 2
> consumes it. Tests: `is_quit_relevant_guard_membership`, `update_surfaces_request_drain_only_for_relevant_commands` (reducer suite green, 51 passed).

### 3.2 Action: route through the EXISTING UI-thread drain executor

> **Stage 2 design decision (resolved during Stage 1 — IMPORTANT, supersedes the "post from
> host_dispatch" sketch below):** `host_dispatch` is a `&self` method on `AppState` and `AppState`
> holds **no `Arc<Self>`/`Weak<Self>` handle**, so it cannot cheaply construct a CEF task to post
> the cascade cross-thread. Centralizing consumption there would require threading an `Arc<AppState>`
> through (a wider change). Instead, **Stage 2 consumes `request_drain` at the UI-thread CEF
> callbacks** that dispatch quit-relevant commands — `client::on_before_close` (window/pool close)
> **and the pool spawn/promote/destroy completion callbacks** (the transitions that *settle* the
> count to zero after a refill race). Those run on the UI thread already, so a `Client` method
> `begin_drain_and_cascade(&self, reason)` (the extracted Stage-1 cascade) can be called **inline**
> — no cross-thread post, no new task type, no Arc plumbing. The level-triggered guarantee comes
> from consuming at *every* settling callback, not from a single chokepoint.
>
> **The correctness obligation for Stage 2:** enumerate *all* UI-thread callbacks that can drive
> `count_live_user_windows`→0 (close, pool-ready, promote, pool-destroy) and consume `request_drain`
> at each. Missing one reintroduces the orphan race; acting inline on the wrong one (e.g. calling
> `quit_message_loop` from `on_before_close`) reintroduces the deadlock. This is the deadlock-
> sensitive core and is deliberately a **separate, focused PR** from Stage 1.

The (now-superseded) original sketch, kept for context:
**Critical threading contract** (`reducer/quit.rs:49-54`, spec §10.1): `reconcile_quit` only DECIDES.
It must not call `quit_message_loop()`, re-lock `host_state`, or close anything. Calling
`quit_message_loop` from inside `on_before_close` **deadlocks the UI thread** (confirmed v0.33.498,
`client/mod.rs:1066`).

So the decision and the action are separated:
1. **Decision (any thread, under lock):** post-dispatch hook computes `reconcile_quit`.
2. **Action (UI thread, posted task):** if drain is requested, post a CEF UI-thread task that runs a
   single extracted function:

```
fn begin_drain_and_cascade(state, reason):
    state.host_dispatch(BeginDrain { reason })   // idempotent; flips QuitState→Draining, suppresses refill
    // Stage 1: PostMessage(WM_CLOSE) to every window-pool-*/floating-pool-* browser
    //          (the EXACT code currently inline in on_before_close:1107+)
    // Stage 2 stays where it is: when browser_list empties, quit_message_loop()
```

This function is **extracted verbatim** from the current `on_before_close` Stage-1 block
(`client/mod.rs:1084-1180ish`) so behavior is identical — it's just now callable from both the
close-edge path and the reconcile path. Idempotency is already guaranteed: `handle_begin_drain` early-
returns once not `Running` (`quit.rs:15`), and `BeginDrain` is documented idempotent
(`client/mod.rs:1090`).

### 3.3 Result: the three decision sites become executors
- **`client::on_before_close`** — ✅ **DONE** (Stage 2, first slice). Keeps the `UnregisterBrowser`
  dispatch, now captures its `DispatchOutput.request_drain` instead of re-deriving
  `count_live_user_windows() == 0` locally; the Stage-1 body was extracted verbatim into
  `AgentMuxHandler::begin_drain_and_cascade(reason)`, called only when `request_drain.is_some()`.
  Live-verified (isolated instance, debug tracing): a promoted secondary window closing while main
  stays open correctly fires `on_before_close` with `request_drain: None` (main still counts) and no
  cascade; closing the last window in a normal two-window session exits the whole process tree
  cleanly within 1 second, no deadlock, no orphan (`tasklist` confirmed clean afterward).
- **`wrr::win_event::maybe_quit_on_last_user_window`** — ⚠️ **NOT DONE — bigger gap than this section
  assumed.** Live-verified (2026-07-07) that on Windows, closing the **main window does not fire
  `on_before_close` at all** (confirmed via `RUST_LOG="info,wrr-trace=debug"`: closing "main" produced
  zero `on_before_close ENTER` trace lines for the "main" label itself — only WRR's
  `[wrr] all user windows hidden/closed ... quitting message loop` fired, followed by CEF's own
  shutdown cascade closing the *other* remaining browsers, whose `on_before_close` fired only as a
  side effect of `quit_message_loop()` already having been called). This means, for the single most
  common quit scenario — the user closes the main window — `HostState.browsers` never learns "main"
  is gone (no `UnregisterBrowser` dispatch ever fires for it), so `reconcile_quit`'s
  `count_live_user_windows()` would keep reporting `main` as live even after the OS says otherwise.
  **"Have WRR call the same reconcile path instead of its own count" (as originally written above) is
  not achievable as a simple swap** — `reconcile_quit` has no way to know main closed unless something
  tells the reducer. Closing this gap needs WRR's OS-hook to itself report the main-window-gone
  transition into the reducer (a new command/event, or reusing `UnregisterBrowser` off the OS
  signal instead of the CEF signal) *before* trusting `reconcile_quit`'s count — a real design task,
  not part of this rollout's original scope. Tracked as a separate follow-up; `quit_message_loop()`
  called directly from WRR, bypassing `QuitState` entirely, is unchanged for now.

  **Addendum 2026-07-08 — a minimal, independently-shippable slice of this gap is now scoped and
  in progress.** `SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md` fixes the false-*positive* direction
  only (WRR quits while a live window remains, root-caused by a live user bug report — closing a
  non-last window killed the whole host) by (a) extending the already-existing LOCATIONCHANGE
  pool-move detector to also dispatch `UnregisterBrowser` to the CEF reducer on a Views
  recycle-close (today it only reports to the launcher mirror), then (b) requiring
  `count_live_user_windows() == 0` to agree with the OS-level `visible == 0` before WRR calls
  `quit_message_loop()`. This does **not** close the gap described above (WRR still decides
  independently rather than deferring to `reconcile_quit`/`request_drain`) — it only prevents WRR
  from firing when the reducer disagrees. Full retirement of the parallel authority (this section's
  original scope) remains open.

  **RESOLVED 2026-07-11 — full retirement landed.** SPEC_PILLAR2_SANITIZE_THEN_DECIDE_2026_07_11.md
  Phases 2-3 (PRs #2084, #2083): every count-lowering dispatch site consumes request_drain (the
  parking-close path flips QuitState on main-window close — the channel this section said was
  missing), and should_quit_on_last_window now requires QuitState::Draining, demoting WRR to the
  Windows Stage-2 executor of reconcile_quit decisions. The quit watchdog (re-arming while
  draining-with-zero-registered) is the bounded backstop. Live close matrix on the merged code:
  PR #2082/#2083 comments.
- **`commands::orphan_reconcile`** — **RESOLVED 2026-07-11** (PR #2081): the two-phase "sanitize
  state.browsers, then trust reconcile_quit" design shipped — `begin_drain`/`live_user_count`/Race-B
  authority deleted; the planner classifies sanitize work only and the verdict comes from the
  `ReconcileQuit` poke. (Race B turned out to be already modeled by the reducer: promotion flips
  `is_pool:false` synchronously — see SPEC_PILLAR2_SANITIZE_THEN_DECIDE_2026_07_11.md §1.A.)
  Originally: not started; its independent `plan.begin_drain` carried a "Race B"
  (`freshly_promoted`) guard with no `HostState` equivalent.

## 4. Exact code changes

| # | File | Change |
|---|------|--------|
| 1 | `reducer/quit.rs` | Remove `#[allow(dead_code)]` from `reconcile_quit`, `should_begin_drain`, `user_creation_in_flight`, `is_background_pending_creation_label`. |
| 2 | `reducer/mod.rs` | In the dispatch path, after applying a quit-relevant command, call `reconcile_quit(state)`; surface `Some(reason)` via a `DispatchOutput { request_drain: Some(reason), .. }` field (new). |
| 3 | `client/mod.rs` | Extract Stage-1 drain body into `begin_drain_and_cascade(reason)` (UI-thread-posted). Delete the inline edge gate in `on_before_close`. When `DispatchOutput.request_drain` is set, post `begin_drain_and_cascade`. |
| 4 | `commands/orphan_reconcile.rs` | Drop independent `begin_drain` decision; call shared executor. Keep close-plan. |
| 5 | `wrr/win_event.rs` | `maybe_quit_on_last_user_window` → route to reconcile path or delete if now dead. |

**Key invariant preserved:** the `is_pool`-flag classification (`is_live_user_window`, `quit.rs:98`)
remains the single source of truth — a promoted pool window keeps its `window-pool-*` label but is
`is_pool:false` and correctly counts as a live window (the reagent P1 #1676 fix). Do not reintroduce
label-prefix counting.

## 5. Tests — the safety net the regression slipped through

The retro (§7.4) notes **zero E2E coverage** for "close last window ⇒ tree exits." Add:

1. **Reducer unit tests (pure, fast)** — already partly present for `should_begin_drain`'s truth table.
   Add: reconcile returns `Some` after `UnregisterBrowser` drops the last live window; returns `None`
   while a `window-pool-*` promote is mid-flight; returns `None` with a user pending creation; returns
   `Some` once that pending creation resolves to nothing.
2. **The race regression test (the important one):** simulate close-last-window **concurrent with** a
   pool refill that briefly keeps the count non-zero, then the refill settling → assert reconcile
   eventually returns `Some` and drain fires. This is the exact scenario edge-triggering missed.
3. **E2E (gated in CI):** launch → open 2 windows → close both → assert the process tree exits within
   N seconds (no orphaned host/srv). Mirror for "host killed → relaunch → windows restore" once Pillar 1
   lands.

## 6. Risks & mitigations
- **Deadlock (highest risk):** never run the action inline — always via the posted UI-thread task. The
  post-dispatch hook returns a *request*, it does not act. (Contract: `quit.rs:49-54`.)
- **Double-drain:** idempotent by construction (`handle_begin_drain` early-return). A reconcile firing
  after `BeginDrain` already flipped `QuitState` returns `None`.
- **macOS/Linux parity:** Stage-1 uses `PostMessage` (Win), `performClose:` (mac), `WM_DELETE_WINDOW`
  (X11). `begin_drain_and_cascade` must keep the existing per-platform branch (`client/mod.rs:1080-1083`)
  — extract, don't rewrite. The pane-pool startup window means `browser_list` must include
  `floating-pool-*` or Stage-2 never empties on mac/Linux (`client/mod.rs:1101-1106`) — preserve.
- **Over-triggering:** the quit-relevant command guard keeps reconcile off the hot path (most dispatches
  skip it).

## 7. Rollout
1. ✅ Land #1 (un-dead-code) + #2 (reducer hook + `request_drain`) + tests #5.1/#5.2 — pure, no behavior
   change yet (hook computed but executor still the old inline path).
2. ✅ **Partially landed 2026-07-07.** Land #3 (extract executor, delete inline gate, wire
   `request_drain`) — done for `on_before_close`. Live-verified: single-window close and sequential
   multi-window close both exit cleanly with no deadlock/orphan. **However**, live verification also
   found `on_before_close` never fires for the main window's close on Windows at all (§3.3) — so this
   step alone does not yet close the actual race the spec exists to fix; it only makes the
   already-firing call sites (secondary/pool window closes) single-authority instead of duplicating
   the decision.
3. ⬜ Land #4/#5 (demote orphan_reconcile + WRR; E2E test) — **not started; scope now understood to be
   larger than originally written.** Wiring WRR needs a new mechanism for the reducer to learn the main
   window closed (§3.3), and demoting `orphan_reconcile` needs to either add a `HostState` field for its
   "Race B" (`freshly_promoted`) guard or keep it as an upstream state-sanitize step — neither is a
   simple call-the-shared-executor swap. Re-scope before starting.
4. ⬜ Verify against the retro's reproduction (#1647/#1650/#1676 scenarios) and the orphan-log signature
   (drain marker now always present) — blocked on #3 above, since those scenarios are exactly the
   main-window/pool-refill race that step 2's landing doesn't reach.

## 8. Definition of done
- `reconcile_quit` is the only place "should we drain?" is decided; the other sites are executors.
  **Partial:** true for `on_before_close`'s two verified scenarios; not yet true for WRR or
  `orphan_reconcile`.
- The race regression test fails on `main` (pre-wire) and passes after. **Not written yet** — needs
  step 3 above to be meaningful (the current landing doesn't touch the actual racing path).
- Closing the last window always exits the process tree (E2E green); no orphan host logs. **Verified
  manually** for the two-window and one-window cases (2026-07-07); no automated E2E test yet.
- Net deletion of duplicated drain-decision code in `on_before_close` / `orphan_reconcile` / `wrr`.
  **Done for `on_before_close`** (the inline `count_live_user_windows()==0` + `BeginDrain` block was
  deleted, replaced by consuming `request_drain`); `orphan_reconcile` and `wrr` still duplicate the
  decision.
