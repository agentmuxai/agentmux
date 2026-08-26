# SPEC: Composer Strip — Row-Based Layout (Rev 7)

**Date:** 2026-08-26
**Status:** Proposed — not yet implemented. This document is written BEFORE any code change, per explicit user direction, so the design can be reviewed against the stated requirement before another revision ships wrong.
**Supersedes (for the left/right slot layout only):** the zone-based model described in `docs/specs/SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md` Rev 1-6. That document's history is preserved for the record; this spec is the direct answer to the defect its own Rev 6 shipped.
**Trigger:** direct user correction, verbatim: *"for every line X there are 2X sections from which you must evenly distribute the elements... there are only 2 sections. the elements on 2 lines would need to be evenly distributed across 4 sections. but the screenshot only has 2 sections."*
**Read first:** `docs/retro/retro-composer-strip-one-sided-lines-misdiagnosis-2026-08-26.md` — why six prior revisions missed this exact requirement.

---

## 1. The invariant (write this down, check every future change against it)

**Every rendered line of the composer strip's slot pool must have content on both its left half and its right half.**

Formally: if the slot pool needs `N` lines to render, there are `2N` "sections" (line 1 left, line 1 right, line 2 left, line 2 right, …, line N left, line N right). The number of NON-EMPTY sections must equal `2N`, with exactly one narrow, named exception:

- **The singleton exception.** If the total slot count is odd, one line cannot be perfectly paired — exactly one line may have only one side filled. This is mathematically unavoidable (you cannot split an odd number of items into pairs with none left over) and is NOT a bug; it is the one line, not every line, that's allowed to be one-sided.
- **The degenerate exception.** If the total slot count is exactly 1 (nothing else exists to pair with — e.g. a non-Claude agent with no tracked processes, unknown auth, and no context yet, leaving only the always-present Shell toggle), that one line is necessarily one-sided. This is the same "fully degenerate one-slot-total case" every prior revision already documented and accepted.

Outside those two named exceptions, **a line with an empty half is a bug**, regardless of how balanced the aggregate left/right totals look, and regardless of whether either zone is "technically" non-empty when considered in isolation. This is the exact property none of Rev 1-6's acceptance criteria ever stated, and the gap the retro above documents.

---

## 2. Why the current (Rev 6) architecture cannot satisfy this

`AgentComposerStrip.tsx`'s current model has exactly two independent containers — `.agent-composer-strip-controls` (left) and `.agent-composer-strip-right` (right) — each its own flex-wrap box that decides, independently of its sibling, when it needs to occupy its own full-width line. There is no code path in which "line 1" is a shared concept between them; each one just renders its own content and the CSS's `flex-wrap`/`flex-basis` rules decide where the overflow lands. Two zones that each independently decide "I need my own line" will, by construction, never coordinate to share that line with each other. No amount of tuning `computeBalancedLeftKeys`'s width-matching (Rev 6) fixes this, because the bug isn't in HOW MUCH goes left vs. right — it's in the complete absence of a shared "row" concept that both sides could occupy together.

---

## 3. New data model: rows, not zones

Replace the two independent zones with an explicit, JS-computed ordered list of **rows**:

```ts
type ComposerRow = { left: string[]; right: string[] }; // slot keys
```

Rendering becomes: for each `ComposerRow`, render one `<div class="agent-composer-strip-row">` containing a left `<span>` and a right `<span>`, each populated from the row's `left`/`right` key arrays via the EXISTING `slotByKey` + `untrack()` one-time-render pattern (Rev 6's own reagent-P1 fix for `<For>` identity stability — unchanged, reused as-is for whichever row a key currently lives in).

### 3.1 Single-row case (unchanged from Rev 6, reused)

If the ENTIRE slot pool's total measured width (plus inter-slot gaps) fits within the strip's actual available width, produce exactly one row using the EXISTING `computeBalancedLeftKeys` global 2-way partition — this is the already-implemented, already-visually-verified ≥482px behavior and does not change. Stats (the centered token/elapsed zone) renders inline alongside this one row, exactly as today.

### 3.2 Multi-row case (new)

If the pool does NOT fit on one line, build rows via a **two-pointer widest-with-narrowest pairing**:

1. Sort ALL slots (movable pool + `hostShell`) descending by measured width.
2. Walk from both ends toward the middle: pair `sorted[0]` with `sorted[last]`, `sorted[1]` with `sorted[last-1]`, and so on. Each pair becomes one row (order within the pair — which side is left, which is right — resolved in step 3).
3. If the total count is odd, the middle element is left unpaired by step 2 — this is the one allowed **singleton exception** row (§1).
4. **`hostShell` placement:** find whichever pair `hostShell` landed in, orient that pair so `hostShell` is the RIGHT-side occupant (swap if necessary — "Shell always outermost," Rev 4/5/6's own established invariant, now expressed as "always the right side of whichever row it's in"), then move that pair to the END of the row list, so Shell also stays in the LAST, most-visually-prominent row (closest to the input box) — matching its historical "always last in the pool" position as closely as the row model allows.
5. If `hostShell` is the sole entry in the pool (the **degenerate exception**, §1), it renders alone on the right of the only row that exists.

This is deliberately the SAME kind of small, brute-force-adjacent, easy-to-verify approach Rev 6 used for `computeBalancedLeftKeys` (sort + pointer walk, not a search or heuristic solver) — consistent with this file's own established lesson (`SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md`'s Rev 2/3 postmortem: cleverer search algorithms are where this file's bugs have historically come from).

### 3.3 Deciding single-row vs. multi-row: a new capability this file didn't have before

Every prior revision relied EXCLUSIVELY on CSS `@container` queries to decide wrapping — the JS never needed to know the strip's actual pixel width. Deciding "does the whole pool fit on one line" requires knowing the real available width, which now needs a `ResizeObserver` on the outer `.agent-composer-strip` element, feeding a `stripWidth` signal. This is a genuinely new capability, not a refactor of an existing one — call this out explicitly in review, since it changes a documented architectural property of this file (the file-header comment's own claim that this design is "NOT deliberate pixel-breakpoint tiers" — the multi-row decision now DOES need a real pixel measurement, though the actual row layout inside is still content-sized/organic, not a fixed breakpoint table).

### 3.4 Stats zone — unaffected, deliberately not folded into the row model

The centered token/elapsed stats zone stays a separate concern:
- **Single-row case:** shares the line, centered between the one row's content — today's behavior, unchanged.
- **Multi-row case:** gets its own dedicated line, exactly as today's narrow-tier behavior already does. It does NOT participate in the row-pairing algorithm — folding a THIRD, always-centered concern into a left/right pairing model adds real complexity for a zone that isn't the source of this bug and has never been reported as one-sided (it's a single centered element, not two sides).

---

## 4. What's unaffected

- The extreme-narrow-width **shed-content** container queries (hiding the process badge below 180px, auth below 220px, HOST tag below 260px) — orthogonal to row-pairing, unchanged.
- Every individual slot's own rendered markup, classes, click handlers, and tooltips — this is purely a layout/grouping change, same as every prior revision's own stated scope.
- The `computeBalancedLeftKeys` function itself — reused verbatim for the single-row case.
- The measurement infrastructure (`display: contents` ref wrappers, the `createEffect` that reads `getBoundingClientRect()` + computed `column-gap`) — reused verbatim; rows just consume the same per-key width map.
- `<For>` identity stability (`slotByKey` + `untrack()`) — reused verbatim, now keyed by row membership instead of zone membership.

## 5. What's removed

- `.agent-composer-strip-controls`'s and `.agent-composer-strip-right`'s `flex-basis: 100%` / tier-toggling container-query rules (the 280px/482px breakpoints that decided when each ZONE got its own dedicated line) — this decision moves to JS (`stripWidth` + total-pool-width comparison), since it's the exact mechanism that produced the one-sided-line bug.
- The `side: "left" | "right"` fixed fallback field on each slot loses its role as the PRIMARY fallback (JSDOM/no-measurement case) mechanism — see §6.

## 6. Fallback behavior (no real layout engine — e.g. this file's own unit tests)

Same principle as Rev 6: when measurement isn't available (first paint, or JSDOM's always-zero widths), don't show an arbitrary or empty split. Fallback: treat the pool as needing exactly the single-row case, using the EXISTING fixed `side` field to build one row (`{left: slots.filter(side==="left"), right: slots.filter(side==="right")}`) — identical to Rev 6's fallback, just reframed as "one row" instead of "the two zones." This keeps the existing 8+ component tests passing unmodified, same as Rev 6 achieved.

---

## 7. Testing plan

1. **Pure-function tests for the new row-builder** (no layout engine needed, same rigor as `computeBalancedLeftKeys`'s own tests):
   - Single-row delegation: pool fits within a given `availableWidth` → exactly one row, using `computeBalancedLeftKeys`'s own result.
   - Multi-row, even total count → `N/2` rows, EVERY row has both sides filled (the core invariant, asserted directly, not inferred).
   - Multi-row, odd total count → `(N-1)/2` paired rows + exactly ONE singleton row (assert the singleton count is exactly 1, not more).
   - `hostShell` orientation: assert it always lands on the RIGHT side of whichever row it's in, and that row is always LAST in the returned array.
   - Degenerate single-slot-total case (only `hostShell` exists): one row, left empty, right = `[hostShell]`.
2. **A new, explicit invariant test** — the one thing missing from every prior revision's test suite: given an arbitrary-ish set of slot widths, assert that the number of one-sided rows never exceeds the mathematically-justified maximum (1 if the total count is odd, 0 if even, and only the fully-degenerate case allows a one-sided row when total count is 1). This is the test that would have caught Rev 6's bug — write it FIRST, watch it fail against the OLD zone-based code path (if feasible to run against), then confirm it passes against the new implementation.
3. **Component-level regression tests** — the existing dropdown-identity-stability test (reagent P1 on PR #2808) and the live-ctx-text-reactivity test must still pass unmodified against the new row-based rendering (both exercise the `slotByKey` + `untrack()` mechanism, which is unchanged).
4. **Required: real-screenshot verification at BOTH a wide (≥482px, single-row) AND a narrow (<482px, multi-row) pane width before calling this shipped** — explicitly listed as a checklist item because the retro's root cause was exactly this check being skipped for the narrow tier in Rev 6. Use direct Win32 `PrintWindow` capture (not `CaptureWindow`, not `CopyFromScreen` alone — see the retro and `docs/status/STATUS_COMPOSER_STRIP_ZONE_BALANCE_HANDOFF_2026_08_25.md`'s "How the fix was verified" section for the established, hazard-free method), and manually count filled sections against the invariant in §1 — not just eyeball "does it look okay."

## 8. Acceptance criteria

1. At every pane width tested, count the number of rendered composer-strip rows (`N`) and the number of non-empty left/right sections across them; the count must equal `2N` minus at most 1 (odd-count singleton) minus at most 1 more only if total slot count is exactly 1 (degenerate case). No other one-sided rows permitted.
2. `hostShell` (the Shell toggle) always renders as the rightmost element of the LAST row.
3. The ≥482px single-line behavior already verified in Rev 6 (real screenshot, `docs/status/STATUS_COMPOSER_STRIP_ZONE_BALANCE_HANDOFF_2026_08_25.md`) is unchanged — this spec does not regress the case that was already correct.
4. All existing `AgentComposerStrip.test.tsx` tests pass unmodified (fallback path preserves current JSDOM behavior).
5. New pure-function tests (§7.1-7.2) pass, including the explicit no-one-sided-rows invariant test.
6. Real screenshots at both a wide and a narrow pane width, taken AFTER implementation, are attached to the PR and manually verified against §1 before requesting review.

## 9. Non-goals

- **Perfect per-row width balance is not the primary invariant.** The two-pointer widest-with-narrowest pairing produces a REASONABLY balanced result as a byproduct, but the PRIMARY, checkable requirement is "no one-sided rows" (§1) — do not chase tighter width-balance at the cost of re-introducing search-based complexity (Rev 2/3's own documented failure mode).
- **This spec does not change the shed-content breakpoints, the stats zone's own behavior, or any individual slot's rendered content** — scope is strictly the left/right row-grouping mechanism.

---

## 10. Progress tracking

| Phase | Status | Notes |
|---|---|---|
| 1. Retro: why 6 revisions missed this | ✅ Done | `docs/retro/retro-composer-strip-one-sided-lines-misdiagnosis-2026-08-26.md` |
| 2. This spec | ✅ Done | Written before any code change, per explicit user direction |
| 3. Implement `ResizeObserver` + `stripWidth` signal | ⬜ Not started | |
| 4. Implement row-builder pure function + single-row delegation | ⬜ Not started | |
| 5. Implement `hostShell` last-row-right reordering | ⬜ Not started | |
| 6. Rewire component render output to `<For each={rows()}>` | ⬜ Not started | |
| 7. Update SCSS (`.agent-composer-strip-row{,-left,-right}`; remove old zone tier rules) | ⬜ Not started | |
| 8. Pure-function tests (§7.1-7.2) | ⬜ Not started | |
| 9. Component regression tests still pass (§7.3) | ⬜ Not started | |
| 10. `npx tsc --noEmit` + `npx vitest run` + `npx stylelint` clean | ⬜ Not started | |
| 11. Real screenshot, wide tier (≥482px) | ⬜ Not started | |
| 12. Real screenshot, narrow tier (<482px) — the tier Rev 6 never re-checked | ⬜ Not started | |
| 13. Changeset + PR opened | ⬜ Not started | |
| 14. Review (ReAgent + Codex) addressed | ⬜ Not started | |
| 15. Merged | ⬜ Not started | |
