# Spec: schedule the deferred redock/floating-pane structural fixes (P3, P6)

> **Correction (2026-07-29 review pass):** this spec originally scoped P4
> ("unowned floaters") as deferred/not-yet-shipped alongside P3 and P6. That
> premise was stale even at authoring time: P4 shipped over a month earlier
> in `cf71cd06` / #1574 ("remove owned-window z-order invariant; add floater
> cascade + diagnostics", 2026-06-19) — floaters are already unowned
> `WS_POPUP` + `WS_EX_TOOLWINDOW` HWNDs with Z-order-on-activation, per
> `agentmux-cef/src/floating_pane.rs`'s own module doc ("Why no owner
> (issue #1560)"). Former Phase 3 below is marked done rather than scheduled.
> Scoping new work against the false premise risked duplicating shipped work
> or misdiagnosing the real redock issue — flagged by automated review before
> merge, fixed here rather than shipping the stale claim.

## Context

`docs/retro/retro-redock-ghost-landing-reliability-2026-07-27.md` (2026-07-27)
diagnosed the current "redock no longer lands reliably" complaint as the
latest symptom of a subsystem that has regressed before. The prior regression
already produced a living architecture doc —
`docs/architecture/ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md` — with a
concrete, already-scoped modularization proposal (§9, P1-P6). **P1, P2, and
P4 shipped** (canonical window resolver; explicit target-window threading
through pane creation; unowned floaters — `cf71cd06`/#1574, 2026-06-19,
predates this spec by over a month). **P3 and P6 did not**, and this spec's
own gate framing below now applies only to those two. A companion spec,
`docs/specs/SPEC_FLOATING_PANE_DND_RETHINK_2026_06_22.md`, independently
proposed the same P4 idea under the name **R2** ("unowned floaters") and
deferred it at the time — that gate has since been opened by the same commit
that shipped P4.

Nothing here is a new idea — it's scheduling work that was already designed
and explicitly deferred, plus folding in what the 2026-07-27 retro found has
changed since (a second RPC caller, an HWND-timing change, a layout-geometry
rewrite) so the harness in P6 covers today's actual dependency surface, not
just May's.

## Why now

The retro's own framing: this class of bug (a shared, load-bearing piece of
floating-pane infrastructure taking silent collateral damage from an
unrelated, well-intentioned refactor) has now happened **three times** to
this feature cluster — the original May regression that produced the
architecture doc, the `FLOATER_EDGE_RESIZE_BORDER` shrink (retro'd
2026-07-27), and now this redock reliability complaint. Each time the fix has
been a point patch or a re-verification pass. P3/P4/P6 are the two structural
changes and one guardrail that were proposed specifically to make the *next*
unrelated refactor unable to silently break this feature — which is a
different kind of fix than anything shipped so far.

## Plan

Two remaining independent, single-concern PRs (P4/R2 below is already
shipped, kept here only as a marked-done reference so this doc still reads
as the complete P1-P6 picture), each gated on the fresh-profile checklist the
architecture doc already prescribes (dock · redock · terminal+browser resize
· z-order · survival · browsers-render) before the next one starts — same
sequencing discipline §9 already specifies.

### Phase 1 — P6: redock/floating integration test harness (do this first)

Land the guardrail *before* the structural changes, so P3/P4 have a
regression net to develop against instead of relying on manual re-verification
the way every prior fix in this history has.

- Script tear-off → redock ×N for both terminal and browser panes, asserting
  `load-end` (not `load-error`) on every redock — the exact assertion the
  architecture doc specifies, chosen because it's the signal that's gone
  silently missing in this feature's history (retro notes "a test harness was
  silently broken for weeks" previously).
- Extend coverage beyond the original P6 scope to the two things the
  2026-07-27 retro found are new since May and not yet exercised:
  - Redock via the cross-tab pane-drag path (`crossTabDrag.ts`'s
    `redockDraggedPane()`, added by #2079) — the harness should exercise
    **both** callers of `RedockFloatingPane`, not just the floater-window
    gesture, since the retro's framework-level finding is specifically that
    two independent callers sharing one RPC is a new risk surface.
  - Redock onto/adjacent-to a minimized pane, given the 2026-07-16/17
    minimize-as-display-mode rewrite touched the exact shared-pool sizing
    math (`layoutGeometry.ts`) that P4c's landing-rect fix depends on.
- Wire into CI (existing `check --tests + test` jobs) so a future unrelated
  PR that happens to break this gets a red check instead of a silent merge —
  the actual goal, not just having the test exist locally.

### Phase 2 — P3: model redock as an atomic backend MOVE

- Introduce the `PaneLocation` state machine the architecture doc specifies:
  `Docked(window, tab) ⇄ Floating(floater)`, with a single `Redock{from_floater,
  to_window, to_tab}` transition owned by the reducer/saga layer, replacing
  today's five independently-scheduled steps (block move, floater auto-close,
  pane close-in-floater, pane create-in-target, frontend `onNodeDelete`).
- Where the browser HWND can be re-parented to the target window without a
  destroy+recreate, do that. Where a recreate is unavoidable, sequence
  close-then-create deterministically and suppress the frontend `DeleteBlock`
  for a block that is *moving* — enforced in the reducer/move itself, not a
  frontend-side flag (the architecture doc explicitly rejects the
  frontend-flag approach as the wrong layer for this guarantee).
- This directly addresses the retro's top structural finding: "no single
  source of truth for window identity" and the hover-resolver's fragility are
  symptoms of orchestration living in five uncoordinated places instead of
  one state machine.

### Phase 3 — P4 / R2: decouple floater OS-window lifetime (unowned floaters) — ✅ ALREADY SHIPPED, 2026-06-19

- **Done, not scheduled.** `cf71cd06` ("remove owned-window z-order
  invariant; add floater cascade + diagnostics", #1574) shipped the unowned-
  floater model this section originally proposed: floaters are raw
  `WS_POPUP` + `WS_EX_TOOLWINDOW` HWNDs with **no Win32 owner** (see
  `agentmux-cef/src/floating_pane.rs`'s module doc, "Why no owner
  (issue #1560)"), closing a window no longer cascade-destroys floaters, and
  Z-order-on-activation is handled explicitly
  (`agentmux-cef/src/browser_pane/callbacks.rs`'s `[pane-zorder]` raise-on-
  activate, `floating_pane.rs`'s cascade-hook) rather than via the owned-
  window invariant. Taskbar suppression already goes through
  `WS_EX_TOOLWINDOW` independent of ownership, exactly as originally
  proposed here.
  - **Cross-reference:** `docs/specs/SPEC_FLOATING_PANE_MULTI_MONITOR_TASKBAR_2026_07_27.md`
    (new, same day as this spec) proposes *reintroducing* real taskbar
    presence for floating panes under specific multi-monitor conditions.
    Since the owner-independent `WS_EX_TOOLWINDOW` flag this phase would have
    established already exists, that work is **unblocked now**, not gated on
    a phase here.
- This was the change the DnD-rethink spec called "the recurring footgun" —
  it eliminated the Z-order pinning, owner-destroy cascade, and `GA_ROOT`
  quirks that every point-fix in this feature's history before it (P1/P2, R1,
  R3, Phase 4b/4c) had to work *around* rather than remove.

## Explicit non-goals

- Not re-litigating P1/P2/P4/R1/R3/Phase-4b/4c — confirmed still correctly in
  place by the 2026-07-27 retro's static analysis (P4 confirmed shipped via
  direct source inspection during this doc's 2026-07-29 correction pass);
  this spec only schedules what's still actually deferred (P3, P6).
- Not a rewrite of the hover-detection Z-order walk
  (`resolve_window_at_cursor`) — P1's canonical resolver already consolidated
  it; P3's atomic-move model reduces how often it's the *only* thing standing
  between a correct and incorrect redock, which is the more leveraged fix.

## Verification

Each phase individually passes the fresh-profile checklist before the next
phase starts (architecture doc's existing discipline). Phase 1's harness
becomes the standing regression gate for phases 2 and 3, and for every
floating-pane-adjacent PR after this ships.

## Files (expected, not exhaustive — scoped further per phase)

- Phase 1: new test harness under whatever this repo's existing e2e/integration
  test convention is (`npm test -- app.e2e.test.ts` pattern per `CLAUDE.md`);
  CI workflow wiring.
- Phase 2: `agentmux-srv/src/sagas/redock_floating_pane.rs`, a new
  `PaneLocation` state module, `agentmux-srv/src/server/service/tear_off.rs`,
  frontend `onNodeDelete`/`DeleteBlock` suppression path.
- Phase 3: already shipped — `agentmux-cef/src/floating_pane.rs` (owner/
  Z-order code, `WS_EX_TOOLWINDOW` handling, cascade hook),
  `agentmux-cef/src/browser_pane/callbacks.rs` (`[pane-zorder]` raise-on-
  activate). No new files for this phase.
