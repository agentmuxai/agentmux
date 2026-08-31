# SPEC: A single content-resize contract for the agent pane

**Date:** 2026-08-31
**Status:** Proposed. No code changed by this document.
**Supersedes as the recommended next step:** `ANALYSIS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_17.md` §7's "fix B",
which called for this but deferred scoping it.

---

## 0. Why this document exists

Nine documented passes have now been made at one bug class — "content in the
agent pane changes height and the scroll position visibly jumps":

| Date | Doc | Outcome |
|---|---|---|
| 07-05 | `ANALYSIS_TOOL_PREVIEW_RUNNING_TO_COMPLETED_JERK` | FLIP fix recommended; landed partially (#1975) |
| 07-24 | `SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY` | Added a tracked dependency |
| 07-27 | `SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING` | Added another |
| 07-30 | `REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT` | Added RO #2; named the pattern as the root cause |
| 08-05 | `PLAN_AGENT_PANE_RESIZE_SCROLL_PIN` | Disproved its own leading hypothesis |
| 08-17 | `ANALYSIS_TOOL_CALL_SCROLL_OSCILLATION` | Retracted two of its own claims mid-document; shipped diagnostics only |
| 08-21 | `FINDINGS_TOOL_CALL_SCROLL_OSCILLATION` | Corrected twice by PR review |
| 08-22 | `FINDINGS_..._LIVE_INSTANCE_DATA` | Corrected twice by PR review |
| 08-31 | this doc | Two further candidate fixes ruled out (§3); a third ruled out in draft, then withdrawn on review (§3a) |

Every pass was made by someone reasoning carefully from the source, and most
produced at least one confident conclusion that a later pass had to withdraw.
That is the signal this document responds to. The problem is not that any
individual analysis was sloppy; it is that the real behavior lives in the
interaction between a `MutationObserver`, a CSS `allow-discrete` transition, a
synchronous browser scroll clamp, three `ResizeObserver`s, and two throttle
timers — and no single file, contract, or test can currently observe that
interaction end to end.

---

## 1. The gap, stated precisely

**There is no place in this codebase where "a node's rendered height is about
to change" is a first-class event.** There are instead six independent
mechanisms, each deciding on its own whether and how to react to a height
change, none aware of the others. All six verified present as of `216c593c4`:

| # | Mechanism | File | Reacts to |
|---|---|---|---|
| 1 | Itemized pin effect | `AgentDocumentVirtualList.tsx:494-513` | node count / `layoutView().totalSize` / `workingRowHeight` |
| 2 | RO #1 (viewport) | `AgentDocumentVirtualList.tsx:535-555` | `scrollRef.clientHeight` |
| 3 | RO #2 (content) | `AgentDocumentVirtualList.tsx:586-595` | `virtualContainerRef` / `streamingBufferRef` box size |
| 4 | Local FLIP | `ToolOverlayLog.tsx:246-297` | its own `<Switch>` branch changing |
| 5 | Local panel auto-scroll | `ToolOverlayLog.tsx:187-208` | `chunks()` / `panelHidden()`, panel-internal only |
| 6 | Throttled re-render timers | `MarkdownBlock.tsx:62-78`, `output-cap.ts:369-440` | their own timers/heuristics, invisible to 1–5 |

Each was added in response to one reported symptom. The result is what you
would expect from six uncoordinated systems layered over four months:
individually reasonable and individually tested, collectively unable to
guarantee the one property users actually want — **monotonic, one-directional
visual flow while pinned to bottom** — because no component owns that
guarantee.

`scrollToTrueBottom()` is correct *arithmetic* (it always lands exactly at
true bottom) and provides zero *visual continuity* guarantee. Nothing upstream
of it prevents the content it chases from legitimately getting shorter.

---

## 2. The physical constraint any fix must respect

This is the single most-repeated dead end in the history above, so it is
stated here once, prominently:

> **`scrollTop` cannot be eased after a shrink.** The browser clamps
> `scrollTop` down to the new `scrollHeight - clientHeight` synchronously, as
> part of the same layout pass that produces the shrink — before any effect,
> `ResizeObserver` callback, or microtask runs. By the time application code
> observes the shrink, the visible jump has already happened. There is nothing
> left to animate.

Established in `PLAN_AGENT_PANE_RESIZE_SCROLL_PIN_2026_08_05.md` §2 ("H1") for
the grow case and carried to the shrink case in
`ANALYSIS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_17.md`'s addendum.

**Corollary:** every viable fix is a *content-layer* fix — freeze the shrinking
element at its old height before the browser paints the smaller layout, then
ease it down, so the native clamp happens in many imperceptible per-frame steps
instead of one visible jump. This is what `flipHeight()` already does for one
element. The contract proposed below is that same technique, owned centrally
instead of reimplemented per component.

---

## 3. Ruled out — do not re-propose without new evidence

Each of these is superficially plausible, has been proposed at least once, and
is wrong for a specific verifiable reason. Recorded here because the reasons
are non-obvious and keep getting rediscovered.

**3.1 — "Give `scrollToTrueBottom()` a shrink-aware eased variant."**
Ruled out by §2. Proposed in the 08-17 doc's own body, retracted in its
addendum the same day, and proposed again on 08-31.

**3.2 — "Extend the FLIP to `ToolBlock.tsx`'s summary row."**
The summary row cannot change height. `.agent-tool-summary` is
`white-space: nowrap; overflow: hidden` (`_document-nodes.scss:223-234`) — a
forced single-line row. The live-tail ↔ result-pill swap changes horizontal
content only. Ruled out in the 08-17 addendum; re-proposed and re-ruled-out on
08-31.

**3.3 — The recurring ~251–252px shrink is not a fixed-height component.**
`FINDINGS_..._LIVE_INSTANCE_DATA_2026_08_22.md` §5 recommended grepping for a
component with a 251–252px height constant. Done, 08-31: no such constant
exists anywhere under `frontend/app/view/agent/`. Consistent with that doc's
own caveat that a repeated *net* delta between two pin-checks cannot establish
a single discrete element. This lead is closed; it needs mutation-level
instrumentation (§5) or nothing.

### 3a. Open — the `heightStale` FLIP bypass, and a worked example of this document's own thesis

The bypass is real and its mechanism is now understood: `.agent-tool-panel`'s
close transition uses `content-visibility 120ms allow-discrete`
(`_document-nodes.scss:403-413`), so `content-visibility` only flips to
`hidden` at the *end* of the 120ms collapse — the panel is still visible and
rendering throughout. But `ToolOverlayLog`'s `panelHidden` signal is driven by
a `MutationObserver` on the `.agent-tool-panel--hidden` *class*, which is added
instantly. So for the entire 120ms window the code believes the panel is
unmeasurable and skips the inner FLIP.

**The first draft of this document ruled the fix out. That was wrong, and how
it was wrong is worth preserving.** The draft argued that fixing it buys
nothing visible, because a `denied`/`canceled` tool's panel is collapsing to
`max-height: 0` over those same 120ms anyway — "already eased by CSS."

`max-height` is a **constraint, not a height.** The panel's rendered height is
its intrinsic content height, *capped* by the interpolating maximum. So when
the inner body swaps a tall streaming `ChunkList` for a short result while the
interpolated max-height is still well above the new intrinsic height, the
constraint is not yet binding — the panel drops to the new intrinsic height
**immediately**, in one frame. Only the tail of the collapse, once the
interpolated maximum falls below the intrinsic height, is actually eased. The
skipped inner FLIP can therefore produce exactly the visible jump the draft
dismissed.

Caught by Codex in review of this PR. Two things follow:

1. **This fix is open, not ruled out** — and per the same argument, it must be
   settled by measurement, not by a third static argument in either direction.
   It is a natural first consumer of step 1's instrumentation (§5).
2. **The thesis of this document survived a live test.** §0 argues that careful
   reasoning about this subsystem keeps producing confident conclusions that
   later have to be withdrawn, because the behavior lives in interactions no
   single file makes visible. That failure mode reproduced inside the very
   document proposing to fix it — the author read the correct CSS, drew a
   conclusion that followed plausibly from it, and missed that `max-height`
   does not do what "the collapse is animated" implies. Treat every
   height-behavior claim in this subsystem, including the ones above, as
   provisional until instrumented.

---

## 4. Proposed contract

One module owns "a subtree is about to change height." Every DOM-shape-changing
site reports through it instead of implementing its own FLIP or relying on an
outer observer to notice after the fact.

```ts
// frontend/app/view/agent/resize-contract.ts  (proposed)

/** Freeze `el` at its current height, run `mutate`, then ease to the new
 *  natural height. The caller never measures, never touches transitions, and
 *  never needs to know whether an outer scroll container is pinned. */
export function withHeightContinuity(el: HTMLElement, mutate: () => void): void;

/** Same, for a mutation that lands asynchronously (a throttle timer's trailing
 *  commit): capture the "from" height now, ease once the mutation settles. */
export function beginHeightContinuity(el: HTMLElement): (this: void) => void;
```

Properties the single implementation owns, which no current call site owns
consistently:

- **Reduced-motion** — one check, not one per site (today only mechanism #4 checks).
- **Hidden/unmeasurable subtrees** — one policy for `content-visibility: hidden`
  and zero-size elements, rather than #4's `heightStale` flag and #5's
  `panelHidden` guard disagreeing about what "hidden" means (§3a).
- **Node-identity resets** — the streaming-buffer slot-reuse hazard that needed
  bespoke `prevNodeId` guards in both `ToolBlock.tsx` and `ToolOverlayLog.tsx`.
- **Cancellation** — one in-flight transition per element, cancelled on re-entry.
- **Magnitude bounds** — skip the ease entirely past some threshold, so a
  whole-pane teardown does not try to animate 40,000px.

The three pin mechanisms (#1–#3) stay as they are. They are the *safety net*
that guarantees arithmetic correctness; the contract is what makes the motion
they chase continuous. Do not attempt to unify pinning and continuity in one
pass — they are separate concerns and the 07-30 report already unified the
pinning side.

---

## 5. Sequencing

Deliberately incremental. The six mechanisms have their own regression suites
(`AgentDocumentVirtualList.resize.test.tsx`, `.collapse.test.tsx`,
`ToolOverlayLog.test.tsx`, `anchor.test.ts`, `output-cap.test.ts`); a wholesale
replacement trades a known-fragmented system for an unknown one.

1. **Instrument before refactoring.** The 08-17 doc's own recommendation #1,
   still unactioned: a `MutationObserver`/`ResizeObserver` trace at the
   *component* level, finer-grained than the existing pin-check-level
   `[wave-scroll-shrink]` diagnostic. Every conclusion in the 08-21/08-22
   findings was limited by that diagnostic's granularity, and §3.3 closed the
   last lead that log-level data could reach. **This is the gating step — it
   is what tells us which call sites actually matter, rather than which ones
   look like they should.**
2. **Land `resize-contract.ts` with no call sites**, plus its own unit tests.
   Zero runtime effect; reviewable in isolation.
3. **Migrate `ToolOverlayLog.tsx`'s `flipHeight()` to it** — one call site, the
   one with existing test coverage to prove equivalence. `heightStale` (§3a)
   is resolved here or not at all.
4. **Migrate `MarkdownBlock.tsx`'s throttled highlight commit** (mechanism #6)
   — the only fully unmitigated source, and the one that fires on the same
   cadence users describe ("during normal streaming, not just at turn
   boundaries").
5. **Migrate the spinner reclassification's *rendering* site** (mechanism #6) —
   the lowest-amplitude source; do it last, or drop it if step 1's data says it
   never mattered. Note the contract cannot be applied in `output-cap.ts`
   itself: `createSpinnerCollapser` is a synchronous, DOM-independent data
   transform with no element and no mutation callback to hand to
   `withHeightContinuity`. The reclassification reaches the DOM through
   `ChunkList`'s memo in `ToolOverlayLog.tsx`, so that component is the call
   site. Applying it in the utility would couple a pure function to the DOM;
   skipping the distinction would leave the height change unmitigated. (Codex,
   review of this PR.)
6. **Re-evaluate.** If steps 3–5 land and the symptom is gone, stop. Do not
   migrate mechanisms #1–#3.

One PR per step. Each keeps the existing suites green.

---

## 6. What this document does not claim

- **That the refactor will fix the reported symptom.** It removes the
  structural reason the symptom keeps coming back with a different face. Step 1
  exists precisely because nobody has yet confirmed which mechanism dominates
  in a real session.
- **That the six mechanisms are individually wrong.** They are not. Each is a
  correct local response to a real bug.
- **Any frame-level timing claim.** Every such claim in the 08-17/08-21/08-22
  chain was walked back on review. Nothing here rests on one.
