# SPEC — Consolidate pane loading into one readiness system

**Date:** 2026-09-20
**Type:** Architecture proposal
**Status:** active — phases 1 and 2 and the §5.4 chrome guard ship with this spec.
Landed: `PaneReadiness` (§5.1/§5.2) with named gates and a bounded reveal timeout;
`<PaneLoadingCover>` (§5.3) now the single owner of `.agent-pane-loading-overlay`,
adopted by both `agent-view.tsx` and `AgentPicker.tsx` — indicator #4 in §2 is gone and
#3 no longer renders its own markup; the header-mic guard inverted to positive (§5.4).
Not started: phase 3 (folding the block-level `ready()` spinner and `<Suspense>`
fallback, §2 #1-#2, into the same cover), phase 4's chrome `isLoading()` subscription,
phase 5 (browser pane, §2 #6-#7).
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
| `subagentBackfillSettled()` | `readiness.gate("subagents")` — a *reveal* gate, never a mount gate (§4) |
| Long-Task quiet + 2×rAF | retained, but owned once by the controller as the transition into `revealing`, not re-implemented per view |

A pane with no gates goes `assembling → revealing` immediately, so non-agent panes are
unaffected.

### 5.3 One cover, with an explicit coverage contract

A single `<PaneLoadingCover>`, rendered at **one** place (the block level, wrapping the
pane's content box), replacing #1-#4. It takes the element it must cover as an explicit
target rather than inheriting it from the cascade, so §3.2 cannot recur.

The drawer's own spinner (#5) is a genuinely different thing — it hides a *re-seed* of an
already-live pane, not initial assembly — and stays, but should adopt the same component
for visual consistency.

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
4. **Chrome subscribes** (§5.4) and the mic guard is inverted.
5. **Browser pane** (#6-7) adopts the controller.

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
