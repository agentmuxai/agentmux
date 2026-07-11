# Pillar 2 — Sanitize-Then-Decide: Retiring the Last Two Independent Quit Authorities

**Date:** 2026-07-11
**Type:** Design spec (design-first per program discipline — window/lifecycle code here is
deadlock-sensitive and has repeatedly punished plausible-looking fixes)
**Status:** IMPLEMENTED & MERGED 2026-07-11 — Phases 0–3 landed as #2080, #2081, #2084 (replacement for auto-closed #2082), #2083; live close matrix passed on the merged code (results: PR #2082/#2083 comments), including a zombie-hang reproduction fixed by the watchdog re-arm (#2083 tip)
**Builds on:** `SPEC_PILLAR2_WIRE_RECONCILE_QUIT_2026_06_29.md` (Stage 2 first slice merged,
PR #1993), `SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md` (merged, PR #2043),
`SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21.md` (§5.1/§10 — `reconcile_quit` itself)
**Resolves:** SPEC_PILLAR2 §3.3's two open rows (WRR demotion, `orphan_reconcile` merge) and
§7 rollout steps 3–4, via the **sanitize-then-decide** variant (no new guard field carrying
Race B — rationale in §2.3, which this spec's own research pass strengthened: the alternative
would have modeled a state the reducer already models).

---

## 0. TL;DR

After PR #1993 and #2043, "should the host quit?" still has **three** deciders:

1. `reducer::quit::reconcile_quit` — the intended single authority (pure fn of `HostState`).
   Consumed only by `client::on_before_close`.
2. `wrr::win_event::maybe_quit_on_last_user_window` — decides from
   `armed && visible == 0 && registered == 0` and calls `quit_message_loop()` directly,
   without `QuitState` ever transitioning. Post-#2043 it is *safe* (it can no longer fire
   while the reducer disagrees) but it is still an independent authority: on Windows, the
   dominant quit path never flips `QuitState` and never runs the Stage-1 drain.
3. `commands::orphan_reconcile::plan_reconcile` — computes its own `begin_drain`
   (`safe_to_drain`) from launcher-shadow keys + pool sets + a live HWND probe, with the
   "Race B" `freshly_promoted` guard.

This spec retires #2 and #3 as authorities. Both become **sanitize → decide → execute**:
first repair the reducer's `browsers` projection to match probed reality (dispatching the
*already-existing* commands — `UnregisterBrowser`, or `close_browser` whose cleanup chain
dispatches it), then consume the `DispatchOutput.request_drain` the reducer already computes
after every quit-relevant dispatch, then execute via the shared
`begin_drain_and_cascade` executor. No decision logic remains outside `reconcile_quit`.
The #2043 watchdog stays, permanently, as the bounded desync backstop.

---

## 1. Findings from the 2026-07-11 implementation-readiness pass

These were verified against `main` (d321e6c2) and they simplify the design relative to the
first draft of this spec:

### 1.A — Race B is already modeled by the reducer; no new command or field is needed

`handle_pop_and_promote_front_pool_window` and `handle_promote_pool_window`
(`reducer/pool.rs:104-105, 138-141`) flip the browser handle to
`TopLevel { is_pool: false }` **synchronously, inside the promote dispatch** — before the
window is shown or the launcher echo lands. So a "freshly promoted" window is already
counted by `count_live_user_windows`, and `reconcile_quit` already refuses to drain while
one exists.

`plan_reconcile`'s Race-B guard exists only because its `live_user_count` is derived from
**launcher-shadow membership** (`shadow_window_meta`, which lags by a full
host→launcher→host echo) instead of the reducer's own `browsers` map. The guard is
compensation for reading a *slower* projection — switching the decision to `reconcile_quit`
makes it structurally unnecessary. The first draft's `MarkBrowserPromoted` command is
dropped: it would have "repaired" state that is not stale.

### 1.B — The genuinely stale direction is dead/hostless zombies, and the repair commands already exist

A window that dies without any close flow (crash, external kill) leaves a stale
`TopLevel { is_pool: false }` entry in `browsers` — which blocks `reconcile_quit` forever.
This is the projection staleness that actually matters, and `orphan_reconcile` already owns
both repair mechanisms:

- `Dead` → `close_browser(force=1)` — CEF's cleanup chain runs `on_before_close`, which
  dispatches `UnregisterBrowser` *and* (since PR #1993) consumes `request_drain` itself.
- `Hostless` → direct `HostCommand::UnregisterBrowser` dispatch.

So "sanitize" is not new machinery — it is the existing classification and close actions,
kept exactly as they are. What gets **deleted** is the parallel decision layered on top:
`ReconcilePlan.begin_drain`, `live_user_count`, and the `freshly_promoted` drain-block.

### 1.C — Pending-creation semantics are already identical

`quit.rs::is_background_pending_creation_label` (window-pool-/browser-pane-/floating-)
mirrors `orphan_reconcile`'s pending-creation exclusion (in `ui_thread_reconcile`) exactly —
each side's comment cites the other. No convergence work needed; a shared-predicate refactor
is optional hygiene, not a fix.

### 1.D — One real behavioral delta: floaters and the shadow-based count

`plan_reconcile`'s `live_user_count` counts any non-`browser-pane-` label present in both
`browsers` and `shadow_window_meta` — including `floating-*` entries if the launcher mirrors
them. `reconcile_quit` excludes floaters **by type** (invariant FP-LIFE: floaters die with
the last top-level window), as does WRR's visible-count. Post-migration, a lone live floater
no longer blocks the orphan path's drain. This is an intentional alignment with the
documented invariant and with the other two deciders, not a regression. (Phase 1's tests
pin it.)

### 1.E — A pre-registration arming gap in `reconcile_quit` (must fix BEFORE adding consumers)

WRR's gate is armed by `HAD_VISIBLE_USER_WINDOW` (set on first `EVENT_OBJECT_SHOW`).
`reconcile_quit` has no arming equivalent, and the main window's startup creation path
(`app.rs::on_context_initialized`) does **not** enqueue a `PendingWindowCreation`. So
between process start and main's `RegisterBrowser` (at `on_after_created`),
`count_live_user_windows() == 0` with nothing pending — `reconcile_quit` would return
`Some(LastWindowClosed)` to any quit-relevant dispatch in that window. Today that is
latent (the only consumer, `on_before_close`, cannot fire before a browser exists, and
`init_pool()` runs from `on_after_created("main")`), but Phase 2 adds consumers at dispatch
sites — the latent verdict would become a live startup quit.

**Fix (Phase 0):** a monotonic `saw_live_user_window: bool` on `HostState`, set by
`handle_register_browser` on the first `TopLevel { is_pool: false }` registration, required
by `should_begin_drain`. Registration-armed is strictly earlier than WRR's show-armed, and
safe: after registration the live count itself blocks drain until a real close. Under §7a
this is a derived, reconstructable bit (a reprojecting host re-registers windows and
re-arms), not new authority.

### 1.F — Stage 2 on Windows cannot ride `on_before_close`; WRR's quit becomes the gated Stage-2 executor

`begin_drain_and_cascade` (extracted in #1993) flips `QuitState` and closes pool browsers
(Stage 1) but never calls `quit_message_loop()` — that is Stage 2, gated on
`client::browser_list.is_empty()` **inside `on_before_close`**. On Windows CEF 148, parking
closes never fire `on_before_close`, so `browser_list` never empties: Stage 2 is
structurally unreachable there, which is *why* WRR's direct quit is the real Windows exit
path. Full demotion therefore does not delete WRR's quit — it **gates it on the reducer's
decision**: `should_quit_on_last_window` gains a `draining` input (true iff
`QuitState` is `Draining`/`Quit`). WRR then executes only a quit the reducer already
decided; `visible == 0 && registered == 0 && !draining` means a consumption site was
missed, and it arms the watchdog (which quits late with the loud desync log) instead of
quitting silently on its own authority.

### 1.G — The executor only needs `AppState`; a free-function extraction unlocks every call site

`begin_drain_and_cascade` touches nothing on `AgentMuxHandler` except `self.state`. Moving
the body to `ui_tasks::begin_drain_and_cascade(state: &Arc<AppState>, reason)` (the handler
method delegating) makes it callable from `unregister_after_parking_close`, the
LOCATIONCHANGE recycle detector, the pool settling callbacks, and `orphan_reconcile` — all
of which already hold `&Arc<AppState>` on the UI thread. Zero behavior change.

### 1.H — A level-trigger needs an edge to ride: `HostCommand::ReconcileQuit`

`orphan_reconcile` can reach its decision point with **zero** sanitize dispatches (its
"nothing to close but drain requested" arm — `state.browsers` already empty). With the
decision moving into `DispatchOutput.request_drain`, that case has no dispatch to carry
the verdict. Phase 0 adds `HostCommand::ReconcileQuit` — a pure no-op command whose only
effect is `update()`'s existing quit-relevant recomputation. It introduces no new
transition; it is a poke that lets an executor *ask* the standing question through the
standard channel.

---

## 2. Target design

### 2.1 — `orphan_reconcile`: plan → sanitize → decide → execute

`plan_reconcile` keeps its classification (Dead/Hostless/Live × user-kind × shadow/pool-set
membership) as the **sanitize planner**. Changes:

- `ReconcilePlan.begin_drain` is **deleted**; `live_user_count` and the `freshly_promoted`
  drain-block go with it. `freshly_promoted` remains as a diagnostic list (the log line
  stays — it is useful race telemetry) but carries no authority.
- The hostless bucket **splits by reducer kind** (a new `user_labels` planner input, from
  `state.is_live_top_level_browser`): hostless *live-user* entries are exactly the stale
  projections that block `reconcile_quit` forever, and repairing them is correct at any
  time — they are unregistered **unconditionally**, breaking the pre-Phase-1 circularity
  (cleanup was drain-gated while the drain decision read the very registration being
  cleaned). Hostless *pool-kind* entries keep the drain-gate (outside drain, unregistering
  bypasses `on_pool_window_destroyed` bookkeeping — unchanged rationale).
- The live `drainable` close list is **deleted**: closing warm/spawning pool inventory on
  drain is Stage 1's job (`begin_drain_and_cascade` closes `window-pool-*` /
  `floating-pool-*`), and the planner was duplicating that mechanism.
- The pending-creation flag no longer feeds a local drain predicate; it keeps its one
  *mechanism* role: deferring zombie reaps while a user creation is in flight (the close
  chain itself could otherwise race the creation — preserved verbatim).
- Orchestrator flow (order matters — mirrors the pre-Phase-1
  BeginDrain-before-closes discipline):

```
1. HWND-probe on the UI thread (unchanged)
2. plan_reconcile → { zombie_closes, hostless_user, hostless_pool, freshly_promoted }
3. sanitize: unregister hostless_user (unconditional projection repair)
4. decide:   dispatch ReconcileQuit → request_drain     // THE decision; reflects step 3
5. execute:  if Some → ui_tasks::begin_drain_and_cascade (QuitState flips to Draining
             HERE, before any close below can trigger a pool refill)
6. mechanism: zombie_closes via close_browser(force=1) — self-sanitizing; for user-kind
             zombies the step-4 verdict was still None (they were counted live), and the
             drain instead fires from their own on_before_close request_drain consumption
             (PR #1993): same authority, one event later
7. if drained: unregister hostless_pool, then the two documented direct
             quit_message_loop() Stage-2 executions: any-hostless (UnregisterBrowser
             cannot empty client::browser_list, so Stage 2 can never fire) and
             nothing-will-pump (no zombies in flight, no pool inventory for Stage 1)
```

Platform containment: `classify_hwnd` hard-codes Live on macOS/Linux (#1569), so the
zombie/hostless buckets are Windows-only in practice; the residual Windows mixed case
(user-kind zombies alongside hostless entries, where `browser_list` can never empty) is
bounded by the #2043 watchdog.

### 2.2 — WRR: reporter roles unchanged, quit gated on the reducer's decision

- `should_quit_on_last_window(armed, visible, registered)` →
  `should_quit_on_last_window(armed, draining, visible, registered)`; happy-path quit
  requires all four. The truth-table tests extend accordingly.
- `is_reducer_lagging_os` widens to cover "counts agree but nobody decided"
  (`visible == 0 && (registered > 0 || !draining)`) — both flavors of desync arm the same
  watchdog, whose recheck-then-quit-on-OS-signal behavior is unchanged.
- The `QUIT_INITIATED`/`HAD_VISIBLE_USER_WINDOW` statics, the HIDE/DESTROY/LOCATIONCHANGE
  reporter dispatches, and the watchdog are untouched.

### 2.3 — Why sanitize-then-decide beats a new `HostState` guard field (now conclusive)

The first draft argued this on hygiene grounds. §1.A makes it conclusive: the state the
guard field would have carried (`freshly_promoted`) **already exists in the reducer** as
`TopLevel { is_pool: false }` — a guard field would have been a second, slower copy of a
fact the authority already holds, maintained by a side channel, consumed by one reader.
The Race-B lesson generalizes: when a decision site disagrees with `reconcile_quit`, first
ask whether its extra input is (a) a fact the reducer already models (→ just read the
reducer), (b) staleness in the reducer's projection (→ sanitize, then read), or (c) a
genuinely missing input (→ only then extend `HostState`). Race B was (a); dead zombies are
(b); the arming bit (§1.E) is the one genuine (c) found — and it is a monotonic bool, not
a mirrored collection.

### 2.4 — Consumption sites (Phase 2)

`unregister_after_parking_close`, the LOCATIONCHANGE recycle-close dispatch, and the
`window_pool.rs` settling callbacks currently discard `DispatchOutput.request_drain`. Each
gains: `if let Some(reason) = out.request_drain { ui_tasks::begin_drain_and_cascade(&state, reason); }`.
All run on the UI thread (WINEVENT callbacks and CEF tasks); the cascade dispatches
`BeginDrain` (idempotent — racing consumers are harmless) and posts pool closes
asynchronously, never calling `quit_message_loop` inline. This is also what finally closes
the **racing-pool-refill regression** the original reconcile_quit spec §3.2 exists for: a
refill that kept the close-time count non-zero now gets re-evaluated when its own settling
dispatch lands.

On Windows the sequence for a solo main close becomes: parking close →
`UnregisterBrowser` (#2043) → `request_drain: Some` → **cascade: Draining + pool WM_CLOSE**
→ pool parking closes → LOCATIONCHANGE unregisters → WRR gate (now also `draining=true`) →
`quit_message_loop`. Same exit, but `QuitState` finally transitions on the dominant path,
Stage-1 drain runs, and every decider agreed.

---

## 3. Phased plan

Each phase lands independently and leaves every existing test green.

**Phase 0 — foundations (no live-path behavior change).**
(a) `saw_live_user_window` arming bit (§1.E): `HostState` field + `handle_register_browser`
set + `should_begin_drain` input + reducer truth-table tests.
(b) `HostCommand::ReconcileQuit` no-op poke (§1.H) + test that it computes `request_drain`.
(c) Extract `begin_drain_and_cascade` to `ui_tasks` free function; handler delegates (§1.G).
(d) This spec into `docs/specs/`.
Verification: `cargo check -p agentmux-cef`, full crate unit tests. The arming bit is the
only semantic change and is strictly drain-narrowing, in a window where no consumer exists.

**Phase 1 — `orphan_reconcile` sanitize-then-decide (§2.1).**
Planner: delete `begin_drain`/`live_user_count`/Race-B block; rewrite the plan tests to
assert closes-only planning plus new reducer-level tests asserting the drain verdict for
the same scenarios (the truth table moves, it doesn't shrink). Orchestrator: flow per §2.1.
Verification: unit suites; live orphan scenarios are hard to stage deterministically
(they need launcher-detected orphan states), so this phase leans on the moved truth-table
tests plus the fact that every *mechanism* (probe, close actions, direct quit contexts) is
untouched. The PR must say exactly that.

**Phase 2 — consumption sites (§2.4).** After this, closing the last window on Windows
flips `QuitState` for the first time. Live matrix required before merge (see §4): #2043's
matrix (solo main close ~30ms exit; 4-windows-close-one survives; sequential close-all,
`#1676` orphan check) plus watchdog-fire injection. WRR's own un-gated quit is still active
this phase and acts as the cross-check: every scenario should show the cascade firing
*before* WRR's gate.

**Phase 3 — WRR draining gate (§2.2).** Small diff on top of Phase 2; same live matrix,
plus OS-initiated closes (external `WM_CLOSE`, task-manager end-window) and a
deliberately-broken consumption site in a scratch build to confirm
quit-late-with-loud-log rather than hang.

**Phase 4 — doc closure.** SPEC_PILLAR2 §3.3 rows → done; quit.rs header "NOT YET WIRED"
paragraph rewritten; program status snapshot refreshed.

## 4. Risks / honest caveats

- **Live verification is the gate for Phases 2–3, not unit tests.** #2043 had two
  live-caught intermediate regressions; on_before_close deadlocks (v0.33.498) and silent
  `post_task` drops are documented in this exact code. Phases 2–3 do not merge on green
  unit tests alone.
- **Phase 1's live trigger is rare by construction** (launcher-emitted `HostShouldQuit`
  on orphan detection). Mitigation: mechanisms untouched; decision movement covered by the
  relocated truth table; the diagnostic logs (`freshly_promoted`, close plans) are kept so
  any field regression is attributable.
- **`ReconcileQuit` poke frequency:** dispatched only from `orphan_reconcile`'s pass (rare).
  It must never be wired into hot paths — it is not a polling mechanism.
- **macOS/Linux:** the consumption sites in Phase 2 are Windows-gated where the premise is
  CEF-148 parking (`unregister_after_parking_close`, LOCATIONCHANGE); the pool settling
  callbacks are cross-platform and give macOS/Linux the refill-race fix. Phase 3 is
  Windows-only by construction. `classify_hwnd`'s hard-coded-Live limitation on
  macOS/Linux (#1569) is unchanged.

## 5. Explicitly out of scope

- Pillar 1 Step 6 (saga collapse) — sequenced after crash-reproject bake time.
- The strong reducer-authority intent-flip (discussion doc §7b).
- `orphan_reconcile`'s existence/merging into a periodic reconciler.
- The macOS/Linux orphan liveness gap (#1569).
- `Client.windowids` slow growth from abandoned promotions (Step-4 spec 2026-07-08 addendum).

## 6. Definition of done

Phases 0–4 merged; grep gate: no `quit_message_loop()` call sites outside (a) Stage 2 in
`on_before_close`, (b) the watchdog recheck, (c) `orphan_reconcile`'s two documented
Stage-2 executions, (d) WRR's draining-gated executor; `ReconcilePlan` has no `begin_drain`;
`should_quit_on_last_window` requires `draining`; SPEC_PILLAR2 §3.3 both rows closed.

## 7. Sources

- `agentmux-cef/src/reducer/quit.rs`, `reducer/pool.rs` (promotion kind-flip),
  `reducer/mod.rs` (`is_quit_relevant`, `update`'s request_drain recomputation)
- `agentmux-cef/src/wrr/win_event.rs` (gate, watchdog, LOCATIONCHANGE detector, arming)
- `agentmux-cef/src/commands/orphan_reconcile.rs` (planner + orchestrator + test matrix)
- `agentmux-cef/src/client/lifecycle.rs` (`on_before_close` consumption,
  `begin_drain_and_cascade`, Stage-2 gate, RegisterBrowser kind derivation)
- `agentmux-cef/src/ui_tasks/window.rs` (`unregister_after_parking_close`)
- `agentmux-cef/src/app.rs` (main-window startup path — no pending-creation enqueue)
- `docs/specs/SPEC_PILLAR2_WIRE_RECONCILE_QUIT_2026_06_29.md` §3.3 + 2026-07-08 addendum
- `docs/specs/SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md`; PR #1993, #2043 descriptions
