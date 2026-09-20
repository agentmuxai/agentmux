# SPEC — Consolidate pane loading into one readiness system

**Date:** 2026-09-20
**Type:** Architecture proposal
**Status:** active — phases 1 and 2 and the §5.4 chrome guard shipped in PR #3462.
Landed: `PaneReadiness` (§5.1/§5.2) with named gates, a warn-only stuck-gate report
and an opt-in (default-off) reveal bound;
`<PaneLoadingCover>` (§5.3) now the single owner of `.agent-pane-loading-overlay`,
adopted by both `agent-view.tsx` and `AgentPicker.tsx` — indicator #4 in §2 is gone and
#3 no longer renders its own markup; the header-mic guard inverted to positive (§5.4).
Phase 3 follows in PR #3466: indicators #1 and #2 now route through the same
controller and cover, the cover's styles moved out of the `.agent-view` cascade into
`element/PaneLoadingCover.scss`, and coverage became an explicit input (§5.3).
Not started: phase 4's `isLoading()` subscription and phase 5 (browser pane, §2 #6-#7) —
both blocked on the same missing plumbing, and phase 5 also needs a different
design than originally specified — see §6.1.
**Motivating evidence:** `docs/reports/REPORT_AGENT_PANE_LOADING_UI_2026_09_20.md`
(measured timeline, `spin:2` observation, §F regression).
**Related:** `REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md`,
`SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md`,
`SPEC_AGENT_PANE_TAB_STRIP_OVERLAY_2026_08_10.md`,
`SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md`,
`SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md`.

---

## 1. Goal

> "we need to consolidate the disparate loading brains into a single cohesive system"
> … "it needs to be rock solid."

One authority per pane for "is this still assembling", one cover, one reveal moment.

## 2. Inventory — what exists today

Seven independent loading indicators, five of which can apply to a single agent pane:

| # | Site | Trigger | Covers |
|---|---|---|---|
| 1 | `block.tsx:262` | `<Suspense>` fallback | block content box |
| 2 | `block.tsx:498` | `ready() && subagentBackfillSettled()` | block content box |
| 3 | `agent-view.tsx:2180` | five-stage reveal (§3) | `.agent-view` |
| 4 | `AgentPicker.tsx:947` | `pickerReady()` | `.agent-pane-loading-overlay` — **the same class as #3**, rendered by a different component with its own lifecycle |
| 5 | `AgentShellSubblock.tsx:702` | `zoomSeeded()` / `spinnerMounted()` | the Shell drawer |
| 6-7 | `browser-view.tsx:225,230` | browser load | browser pane |

Measured: **two brains on screen simultaneously** during one agent-pane mount. Four is
reachable (#1/#2 → #3 → #4, plus #5 if the drawer is open).

Each has its own fade duration, its own unmount timer, and no knowledge of the others.
There is no moment defined as "the pane is now allowed to appear" — only several
components independently deciding to stop hiding their own part.

## 3. Why it is not rock solid

**3.1 Readiness is inferred, not stated.** `agent-view.tsx` reveals via:
`historyPainted()` → `authPhaseSettled()` → Long-Task quiet detector → 2×rAF →
`historyLoaded()` (CSS fade) → 220ms timer → unmount. Six async sources feeding one
boolean, each individually justified, none of them an actual statement from the thing
being waited on. Any firing early reveals a half-assembled pane.

**3.2 Coverage is implicit.** `.agent-pane-loading-overlay` is
`position: absolute; inset: 0`, so what it hides is "whatever ancestor happens to be
positioned". This broke silently this week: introducing `.agent-view-zoomed`
(`position: relative`) moved the overlay's containing block and left the Shell drawer
uncovered — see the motivating report §F. Nothing detected it; no test asserts coverage.

**3.3 Chrome has no readiness signal at all.** The overlay deliberately does not cover
the tab strip (`SPEC_AGENT_PANE_TAB_STRIP_OVERLAY_2026_08_10.md §1.4`), so every
header-level affordance must suppress itself. They do so with ad-hoc negative guards —
e.g. `blockView !== "agent"` shows the header mic whenever the view type is *unknown*,
which includes the window where block data is still null.

**3.4 Duplicate class, duplicate owner.** #3 and #4 render the same
`.agent-pane-loading-overlay` class from two components. Whichever unmounts last wins;
neither coordinates.

## 4. Design constraint — the deadlock precedent (do not regress this)

`block.tsx:380-398` records a P0: folding `subagentBackfillSettled()` into `ready()` made
`<Show when={ready()}>` never mount `BlockFull` — but mounting is the *only* thing that
calls `registerAgent`, which is the only thing that triggers the backfill the gate waited
on. Gate and producer deadlocked.

**Therefore the new system must keep two concerns separate:**

- **Mount gating** — may the component tree exist? Must depend only on data needed to
  *construct* it. Kept minimal, as `ready()` is today.
- **Reveal gating** — may the user see it? May wait on anything, because the content is
  already mounted and running underneath the cover.

Every dependency added by this spec is a *reveal* gate. No dependency may ever be added
to mount gating.

## 5. Proposed architecture

### 5.1 One readiness controller per pane

```ts
type PaneReadinessPhase = "assembling" | "revealing" | "live";

interface PaneReadiness {
    phase: () => PaneReadinessPhase;
    /** Declare a reveal-blocking dependency. Returns its completion callback. */
    gate(name: string): () => void;
    /** For chrome: true until `live`. */
    isLoading: () => boolean;
}
```

Created once per block, alongside the ViewModel, and reachable by both the block frame
and the view. `phase()` moves `assembling → revealing` when every registered gate has
reported done, and `revealing → live` when the cover's fade completes.

Gates are **named** so a stuck reveal is diagnosable ("waiting on: history, auth") rather
than an unexplained hang — today a never-firing proxy is invisible.

### 5.2 Gates replace proxies

The five-stage chain (§3.1) becomes explicit registrations:

| today (proxy) | becomes |
|---|---|
| `historyPainted()` | `const done = readiness.gate("history")` → called by the transcript when it has painted |
| `authPhaseSettled()` | `readiness.gate("auth")` |
| `subagentBackfillSettled()` | **NOT a gate** — see below. It is re-entrant, and a one-way gate silently swallows every re-cover after the first |
| Long-Task quiet + 2×rAF | retained, but owned once by the controller as the transition into `revealing`, not re-implemented per view |

A pane with no gates goes `assembling → revealing` immediately, so non-agent panes are
unaffected.

**Only one-way signals may be gates.** `subagentBackfillSettled()` looks like an obvious
gate and is not one: `useSubagentBackfillGate` calls `setSettled(false)` on *every*
"started" event, including one arriving long after the first cycle settled, and the
pre-consolidation code deliberately re-showed its spinner for it. `PaneReadiness` is
one-way — gate release is one-shot, and re-registering after reveal is a documented
no-op precisely so a late dependency cannot yank the cover back over content the user
is reading. Feeding a re-entrant signal into it type-checks, reads correctly, and
silently drops every re-cover after the first. (reagent P1 on #3464, which became #3466.)

The rule this generalises to: **one-time assembly is the controller's job; anything
cyclic keeps its own small cycle and merely renders the same cover.** The same split
already applies to the block `<Suspense>` fallback (§2 #1), the Shell drawer's re-seed
spinner (#5) and the browser pane's post-first-paint badge (§6.2). Before adding a gate,
check whether its signal can go back to false.

**A cyclic signal's effect must track the phase too.** The first attempt at that own
cycle read `readiness.phase()` through `untrack`, so it re-ran only when the cyclic
signal itself changed value. `useSubagentBackfillGate` initialises `settled` to **false**
and leaves it there until an async RPC completes, while the `content` gate releases on
`ready()` — which for a persisted-session agent pane resolves well before the backfill
round trip. So the ordinary case is: the signal is false and never changes, the pane
reaches `live`, the effect never re-runs to notice, and the cover never appears at all.
Strictly worse than the code it replaced, and invisible to any test whose mock defaults
the signal to the convenient value. Track both reads. (reagent P1 on #3466.)

### 5.3 One cover, with an explicit coverage contract

A single `<PaneLoadingCover>`, rendered at **one** place (the block level, wrapping the
pane's content box), replacing #1-#4. It takes the element it must cover as an explicit
target rather than inheriting it from the cascade, so §3.2 cannot recur.

The drawer's own spinner (#5) is a genuinely different thing — it hides a *re-seed* of an
already-live pane, not initial assembly — and stays, but should adopt the same component
for visual consistency.

**Whatever suppresses a kept affordance must ask "is a cover up?", not "is the controller
assembling?"** The `<Suspense>` fallback was first suppressed on
`PaneReadiness.isLoading()`, which is permanently false once the pane is live and knows
nothing about the separate re-cover cycle a cyclic signal drives (§5.2). A mid-life
suspension during a backfill re-cover therefore rendered a spinner *underneath* the
cover — reintroducing the two-brains case on exactly the path the re-cover fix had just
added. The predicate is now derived from the rendered cover's own phase.
(reagent P1 on #3466.)

**Open question this raised.** That `<Suspense>` boundary may not catch view-level
`createResource` reads at all: `block.tsx` builds `viewElem` with an *eager* `createMemo`,
so the view component is constructed during `BlockFull` setup rather than while the
boundary is rendering. An attempt to drive a real suspension through it in
`block.test.tsx` produced a normally-rendered view and no fallback. If that holds, #1 is
dead code rather than a kept mid-life affordance, and should be deleted outright rather
than suppressed — but that is a separate investigation and is NOT assumed here.

### 5.4 Chrome subscribes

`BlockFrame` reads `readiness.isLoading()` and suppresses transient affordances centrally
while assembling. Independently, negative guards become positive: render the header mic
when `blockView === "term"`, not when `blockView !== "agent"`, so an unresolved view type
renders nothing (the safe default).

## 6. Migration — each phase shippable and revertible

1. **Introduce the controller, no behaviour change.** Add `PaneReadiness`; have
   `agent-view.tsx` register gates that mirror its current signals; keep the existing
   overlay rendering from `phase()`. Verify the measured timeline is unchanged.
2. **Collapse #3 and #4.** Move the cover to the block level; delete the duplicate
   `.agent-pane-loading-overlay` render in `AgentPicker.tsx`.
3. **Collapse #1 and #2** into the same cover, keeping `ready()` as mount gating only
   (§4).
4. **Chrome subscribes** (§5.4) and the mic guard is inverted. **The mic guard
   shipped; the `isLoading()` subscription is blocked — see §6.1.**
5. **Browser pane** (#6-7) adopts the controller. **Revised — see §6.1.**

### 6.1 Phases 4 and 5 both need a pane-scoped readiness handle

Two separate attempts ran into the same missing piece, which is worth naming
once rather than rediscovering a third time.

**Phase 4.** §5.4 says "`BlockFrame` reads `readiness.isLoading()`". Threading
it as a prop from `<Block>` does not work, and fails *silently*: every view type
in `pane-leaf-chrome.tsx`'s `HOISTS_OWN_CHROME` — agent, term, browser, editor,
sysinfo, cpuplot, swarm, armory, media, drone, help, warden, i.e. essentially
every real pane — sets `noHeader()`, so `BlockFrame`'s own inline header never
renders. The header those panes actually show is built by `PaneHeaderTabStrip`,
which lives **outside** `<Block>` and constructs its own explicit prop object.
A prop threaded down from Block can never reach it, so the suppression is inert
exactly where it was aimed. (Caught in review on #3464; the inert wiring was
removed rather than shipped as a fix.) The mic guard, which is a pure function
of `blockView` inside `EndIcons`, is unaffected and did ship.

**Phase 5.** See the browser-pane analysis below: the correct form is a
`gate("first-paint")` registered by `browser-view`, which also sits outside the
block's prop tree.

Both need the same thing: **the pane's readiness must be reachable from outside
`<Block>`**, by the hoisted chrome above it and by view code inside it. The
obvious shortcut — the blockId-keyed `BlockComponentModel` registry — is not
safe as-is, because that entry is adopted across mounts (`block.tsx`'s
create-vs-adopt ownership dance, and the P0 it already caused), so a consumer
could pick up another mount's controller. Candidates: hang readiness off the
existing `setActiveViewModel(vm, bcm)` channel that hoisted chrome already uses
as its live pointer into the block, or have `pane-leaf-chrome` own the
controller and pass it *down* into `<Block>`. Either is a deliberate design
change, not a prop rename.

### 6.2 Phase 5 does not fit the controller as written

Phase 5 was specified on the assumption that #6/#7 are assembly indicators like
the rest. They are not, and implementing it as a fold would break the controller.

`browser-view.tsx`'s spinner is driven by `model.loadingAtom()`, which is
**cyclic**: it flips true→false→true again on every reload, back/forward and
redirect chain, for the whole life of the pane. `PaneReadiness` is deliberately
**one-way** — `assembling → revealing → live`, with a gate registered after
reveal treated as a no-op precisely so that a late dependency cannot yank the
cover back over content the user is already reading. Making it re-entrant to
accommodate a page load would delete that guarantee for every other pane.

The same distinction already appears twice elsewhere and is resolved the same
way both times: the block `<Suspense>` fallback (§2 #1) and the Shell drawer's
re-seed spinner (§2 #5) are *mid-life* affordances, kept, and merely suppressed
while the one-time cover is up. #7 (the post-first-paint badge) is that same
category and stays — it also carries deliberate anti-flicker design of its own
(`SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md`) and a
`data-pane-overlay` attribute the native pane flip logic depends on.

Only #6 — the *first* load, when there is genuinely nothing behind it — is
initial assembly. The correct form of phase 5 is therefore **not** a fold but a
gate: `browser-view` registers `readiness.gate("first-paint")` and releases it
on the first `loadingAtom()` true→false transition, after which #6 is deleted
outright and the block-level cover hides that window instead.

That needs a readiness handle reachable from view code, which does not exist
yet: views receive `viewModel`, not the controller. The obvious shortcut — the
blockId-keyed `BlockComponentModel` registry — is not safe to use for it as-is,
because that entry is adopted across mounts (`block.tsx`'s create-vs-adopt
ownership dance, and the P0 it already caused), so a view could pick up another
mount's controller. Phase 5 should introduce that plumbing deliberately rather
than borrow the registry for it.

## 7. Acceptance criteria

- At no point during a pane mount is more than **one** loading indicator in the DOM.
  (Today: `spin:2` measured; this is the headline regression test.)
- The cover's rect equals the pane content rect for the whole `assembling` phase —
  asserted in a test, so §3.2/§F cannot regress.
- With the Shell drawer open, the drawer is fully covered for the whole `assembling`
  phase.
- No element outside the cover changes its rendered content between `assembling` and
  `live` — i.e. nothing "pops in". Checked by diffing a chrome snapshot across the
  transition.
- A pane whose block data resolves late renders **no** header mic at any point.
- Mount gating is unchanged: `BlockFull` still mounts on the same condition as today
  (guards the §4 deadlock).
- Existing fade feel preserved: `revealing` duration matches today's 220ms.

## 8. Non-goals

- Changing *when* data loads. This is about when the user is shown the result.
- The working/activity indicator (a live-state affordance, not assembly).
- The tab strip staying interactive during load — that stays true.

## 9. Risks

- **Deadlock**, per §4 — mitigated by the mount/reveal split, but it is the failure mode
  to review hardest.
- **A single authority means a single point of hang.** If one gate never reports, the
  pane never reveals.

  An earlier draft of this section proposed "a bounded reveal timeout that logs loudly
  and reveals anyway — degrading to today's behaviour rather than an indefinite cover."
  **That was wrong on its own terms and is not what shipped.** Today's behaviour *is*
  an indefinite cover: `agent-view.tsx` waits for `historyPainted && authPhaseSettled`
  with no forced-reveal path at all. A timeout would therefore have been a behaviour
  change introduced under the banner of preserving behaviour — and the case most likely
  to trip it is the legitimate one this cover exists for: a persisted-session pane
  replaying a large transcript while auth settles and subagents backfill. Force-revealing
  that mid-assembly produces exactly the flicker §3 is about. (reagent P1 on #3462.)

  What shipped instead: `warnAfterMs` (default 8s) **logs** which gates are outstanding
  and leaves the cover up, giving full diagnosability at zero behavioural cost.
  `revealTimeoutMs` exists as an explicit per-caller opt-in for a surface where partial
  content genuinely beats waiting; **no call site sets it today.** The default path is
  covered by its own test, since a default nothing exercises is a default nobody checks.

  The two are **independent deadlines on independent timers.** A first attempt armed a
  single timer at `Math.min(warnAfterMs, revealTimeoutMs)` and revealed whenever it
  fired, so a caller asking for a 20s hard bound was revealed at the 8s *warn* instead —
  a silent violation of the bound it had asked for. It survived review once because only
  `revealTimeoutMs < warnAfterMs` was tested; the reversed ordering is now a test.
  (reagent P1 on #3462, round 2.)
- **Fade-feel regressions** across five call sites with individually tuned timings; phase
  1 exists to prove the timeline is unchanged before anything is deleted.
