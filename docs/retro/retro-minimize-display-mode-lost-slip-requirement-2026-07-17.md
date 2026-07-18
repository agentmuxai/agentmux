# Retro — The display-mode minimize redesign deleted a real product requirement

**Date:** 2026-07-17
**Trigger:** User report: "if minimized, it is supposed to slip under/over the other panes, that isnt happening." Direct questions: *why did we go backward?* and *is this a very hard problem?*
**Status:** Root cause confirmed by re-reading the original spec chain. **Fixed** — slip-docking implemented as derived geometry (§4's proposed shape, built as designed): `resolveRowSlipTargets` + a docking pass in `updateTreeHelper` (`frontend/layout/lib/layoutGeometry.ts`). No tree mutation, no new persisted state; verified end-to-end against the exact user repro (agent/cpu/swarm — minimize cpu then swarm — asserts agent absorbs the full row width and right_col's chip stack docks onto agent's top, zero dead space) plus 11 unit tests for the slip-target resolution algorithm. 112 frontend tests green.

---

## TL;DR

**We went backward because I never checked what AgentMux's own minimize feature was originally spec'd to do.** I researched how *other* systems (i3, tmux, IDEs, docking frameworks) implement minimize, found none of them do anything like AgentMux's "slip" (a minimized pane's header visually merges into an adjacent pane, freeing its space), and concluded from that absence that slip was accidental complexity worth deleting. It wasn't accidental — it's a deliberately designed, twice-refined product requirement, documented in two specs from 2026-06-24 and 2026-06-27, predating this whole bug-fixing arc by two weeks. I deleted the requirement along with the buggy mechanism that implemented it, instead of keeping the requirement and fixing the mechanism.

**Is this a very hard problem? No.** The hard part — get rid of in-tree size arithmetic and structural tree surgery so minimize can't corrupt the tree — is already solved and correct (four real bug classes closed, verified by tests, reagent-reviewed twice). What's missing is a rendering-level feature on top of that already-correct foundation: derive a minimized chip stack's position to overlay/dock onto an adjacent pane's rendered box instead of rendering as its own separate narrow column. That's a moderate, well-scoped addition, not a redesign.

---

## 1. What the product actually required (the spec chain I should have read first)

Four specs, in order, each building on the last:

| Spec | Date | What it added |
|---|---|---|
| `SPEC_PANE_MINIMIZE_AND_TOOLCALL_FAILCOLLAPSE_2026_06_21.md` | 2026-06-21 | The original minimize button. Explicitly scoped as **pure visual hide** — "Minimized state does NOT affect layout sizing of neighbouring panes." No slip, no space reclamation. |
| `SPEC_PANE_MINIMIZE_CARET_BUG_2026_06_24.md` | 2026-06-24 | Bug fix (chevron reactivity). Confirms by this date collapse *already* "shrinks to its header bar **or slips into the adjacent column**" — slip existed before this spec even started. |
| `SPEC_PANE_MINIMIZE_REFINEMENTS_2026_06_24.md` | 2026-06-24 | **The slip requirement, explicit and deliberate**: *"a pane that occupies its own column slot ... should collapse vertically and slip its header into the adjacent column rather than shrinking horizontally into a thin strip."* Full algorithm spec'd: pane header pins to the top of the neighbor column, width released to that neighbor, fully reversible on restore. |
| `SPEC_PANE_MINIMIZE_COLUMN_DISSOLVE_2026_06_27.md` | 2026-06-27 | **Cascading slip**: when every pane in a column is minimized, *"the user's intent when collapsing every pane in a column is to reclaim that column's width for adjacent content"* — the whole column's headers migrate into the neighbor, the column's width is released, and this cascades to further neighbors.

These are not throwaway notes. They're two rounds of deliberate refinement with explicit before/after diagrams, restore semantics, and edge-case tables (adjacent-sibling-is-a-leaf, cascading dissolve, multiple slips into one column). Someone thought carefully about what "minimize" should *feel* like in this product: **panes visually consolidate into a neighbor, not just shrink in place.**

## 2. What I actually did, and where the reasoning broke

Session arc, condensed:

1. Root-caused four real bugs in the slip/dissolve mechanism (direction flip #2176, resize-through-lock #2180, marker-fields stranded on branches, cascading dissolve computing negative sizes — caught live by the layout doctor).
2. Ran deep research asking: *"how do established systems model minimized panes — in-tree size state, out-of-tree docks, or container display modes?"*
3. Research came back unanimous: **no surveyed system does anything like slip.** i3's scratchpad, Eclipse's trim stacks, IntelliJ's tool-window bars, AvalonDock's anchor groups — all either move the pane fully out of the tree or use a discrete, render-derived display mode. The research's own finding 8 said outright: *"AgentMux's 'slip' behavior ... has no analog in any surveyed system and should be reconsidered."*
4. I read "no analog" as "this was over-engineering, cut it" and implemented the display-mode redesign (#2197) **without** the slip/merge-into-neighbor visual — just a minimized pane rendering as an isolated chip (or chip-stack) in its own narrow slot.

The break is at step 4. "No other system does X" is evidence that X is *unusual*, not evidence that X is *wrong for this product*. The research was answering "how do other systems solve the corruption-prone mechanism problem" — it was never asked "should AgentMux keep its own product-specific slip requirement," because I never went back to check whether that requirement existed in writing. It did, in exactly the specs listed above, and I had access to them the whole time — I just didn't look, because by the time I was implementing the redesign, the framing in my own head had shifted from "fix minimize's corruption bugs" to "minimize should look like other apps' minimize." Those are different projects, and I shipped the second one while believing I was doing the first.

**The honest failure mode: conflating "the mechanism that implements this requirement is what's corrupting the tree" with "the requirement itself must be the source of the corruption."** They're not the same claim. The corruption came from *how* slip/dissolve was implemented (structural tree surgery: splicing nodes out of the Row, converting leaves to Columns, stealing flex units from a neighbor's first child) — not from *what* it visually accomplished (a header docking into a neighbor's presentation). I could have kept the second while replacing the first.

## 3. Why this wasn't caught earlier

- The research report (§7, recommendation) does flag the gap honestly in one line — *"the 'slip' mechanism has no analog... and should be reconsidered"* — but "reconsidered" reads as neutral in isolation; I converted it to "removed" without a distinct reconsideration step, and never surfaced "this deletes an existing requirement" as its own decision point to the user.
- Every review pass on PR #2197 (three rounds of reagent findings, all real) checked *correctness of the new mechanism against itself* — gap compensation, migration math, effective-minimize guards. None of them, and none of my own test-writing, checked *behavioral parity against the old spec'd requirement*, because I never framed "does this still slip into a neighbor" as a requirement to preserve. The tests I wrote assert the new (narrower) behavior is internally consistent, which is a different thing from asserting it matches what was asked for two weeks ago.
- The live smoke-testing loop this session (multiple rounds: "we still are not getting proper operation," the dead-space bug, the negative-size cascade) was all diagnosing *correctness* bugs in slip/dissolve — never once "does the visual result match intent," because until this message, the visual intent question hadn't been asked directly. The diagnostics I built (the layout doctor) check structural tree invariants, which is exactly why it stayed silent through this — a chip rendering in its own isolated column instead of merged into a neighbor is not a structural invariant violation, it's a requirements gap.

## 4. Is this a very hard problem?

No — and it's worth being precise about which part is hard and which isn't, because they're genuinely different in kind:

**Already solved, correctly, and this part *was* hard:** don't store minimize as mutable size arithmetic in the tree. That was the actual four-bug-class problem, and the fix (one leaf-only `minimized` flag, geometry derived fresh every render pass, `computeMainAxisAllocation` as a pure function) is sound, tested, and matches how every mature system in the research handles the "don't let a display mode corrupt persistent state" problem. Keep this. It is not what needs to change.

**Missing, and this part is moderate, not hard:** the *rendering* of a minimized chip (or chip-stack) currently gets its own slot in its parent's flex allocation — narrow, but still a sibling slot next to the expanded pane, not merged into it. What the spec asked for is a **positioning** change: instead of allocating the chip stack a slot of its own, derive its rect to be positioned *at the top of* (docked onto) the adjacent expanded pane's own rendered box, and give that whole freed slot's width/space to the expanded neighbor. This is achievable within the exact same derived-geometry model already in place — it doesn't need new tree state, doesn't need structural surgery, and doesn't reopen any of the four closed bug classes, because it's still "derive positions fresh every render pass from a single boolean flag," just with a different positioning rule for the minimized-branch case. The `computeMainAxisAllocation`/`minimizedFixedPx` machinery already correctly computes "how much space should this chip stack's slot claim" (the width-reclaim math that gives the neighbor breathing room) — what's missing is telling the *renderer* to draw that chip stack overlaid on the neighbor's box rather than in a separate box beside it.

Rough shape of the fix, for when it's time to implement (not started yet):
- The neighbor (the pane the chips should slip onto) needs to be identified the same way `_dissolveColumn`'s spec did — the adjacent sibling in the Row.
- The neighbor's own rect gets computed as normal (full remaining width/height).
- The minimized chip stack's rect gets positioned as an overlay pinned to the top of the neighbor's rect (same width as the neighbor, header-stack height), likely via a z-index layer or a nested-rendering trick, rather than claiming its own slot in the parent's main-axis allocation at all.
- Restore is already correct — it's just clearing the flag; no restore-context bookkeeping needed since there's no structural move to undo.

## 5. What should have happened, and the actual lesson

Before implementing the display-mode redesign, I should have run one extra check: grep `docs/specs/` for the feature I was about to change, read what was already promised, and treated any deletion of documented behavior as its own explicit decision to surface to the user — not something to fold silently into a "these bugs are fixed" narrative. Research into how *other* systems solve a *class* of problem is valuable for choosing an implementation *mechanism*; it is not a substitute for reading this product's own requirements history before removing something. The fix for next time isn't "do more research" — it's "check what was actually promised before deciding a requirement is disposable," and when research and existing spec disagree, that disagreement is exactly the kind of thing to ask about rather than resolve unilaterally.
