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
- **The physical-capacity exception (added 2026-08-26, PR #2812 review — Codex P1).** If no two remaining slots' combined width (plus gap) fits within the strip's real available width, forcing a pair onto one line would silently overflow it — the row's own `flex-wrap` safety net would then split that single row onto two physical lines, reproducing the one-sided-lines bug through a different mechanism than the one this revision set out to fix. When this happens, the wider of the two candidates is emitted as its own one-sided row instead of an overflowing pair. This is genuinely unavoidable when content simply cannot fit two-up in the available width — not a gap in the pairing algorithm itself, and (unlike the two exceptions above) can affect more than one line in a single render when the pane is narrow enough.

Outside those three named exceptions, **a line with an empty half is a bug**, regardless of how balanced the aggregate left/right totals look, and regardless of whether either zone is "technically" non-empty when considered in isolation. This is the exact property none of Rev 1-6's acceptance criteria ever stated, and the gap the retro above documents. (The physical-capacity exception was itself missed by this spec's own first draft — see the PR #2812 review history below §3.2 for how it surfaced.)

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
6. **(Added 2026-08-26, PR #2812 review — Codex P1) Per-pair capacity.** Step 2's pairing only proceeds if `sorted[i].width + sorted[j].width + gapPx <= availableWidth`; if it doesn't fit, `sorted[i]` is emitted as its own one-sided row instead (the **physical-capacity exception**, §1) and `i` advances alone. `sorted[j]` is always the smallest still-unplaced width at that point (descending sort, closing in from the end), so if it doesn't fit next to the widest remaining slot, no other remaining slot fits any better — the greedy bail-out is provably correct given the sort order, not a heuristic.
7. **(Added 2026-08-26, PR #2812 review — Codex P1) Shed slots excluded from pairing.** A slot hidden by this file's own SCSS shed-content queries (`.agent-composer-strip-auth`, `.agent-composer-strip-process-badge` collapsing to `display:none` below a container-width threshold) measures a real 0px. Feeding it into the pairing algorithm anyway let it consume a pairing partner as if it were real content — a row could come back with both sides "filled" while one side rendered nothing at all. Zero-width, non-`hostShell` slots are excluded from the width list passed into the algorithm above, then folded back into the last produced row via their own fallback `side` afterward (harmless, since a 0-width element changes nothing about how that row visually renders) — this keeps them mounted and re-measurable so they pick back up automatically once the container widens past its own shed threshold again, rather than being dropped from rendering entirely.
8. **(Added 2026-08-26, PR #2812 review — Codex P1) Stats zone reservation.** The single-row fit check (§3.1) must also account for the stats zone's own real measured width plus one extra gap, since it shares that line as a third flex child whenever everything fits on one line (§3.4) — slots alone fitting `availableWidth` doesn't mean slots-plus-stats do. Passed to the row-builder as a `reservedWidth` parameter, added only to the single-row total, never to individual multi-row pair budgets (the stats zone never shares a line with a multi-row pair — see §3.4).

This is deliberately the SAME kind of small, brute-force-adjacent, easy-to-verify approach Rev 6 used for `computeBalancedLeftKeys` (sort + pointer walk, not a search or heuristic solver) — consistent with this file's own established lesson (`SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md`'s Rev 2/3 postmortem: cleverer search algorithms are where this file's bugs have historically come from). Steps 6-8 are refinements found by ReAgent/Codex review on PR #2812, not a redesign — the core sort + two-pointer mechanism is unchanged.

### 3.3 Deciding single-row vs. multi-row: a new capability this file didn't have before

Every prior revision relied EXCLUSIVELY on CSS `@container` queries to decide wrapping — the JS never needed to know the strip's actual pixel width. Deciding "does the whole pool fit on one line" requires knowing the real available width, which now needs a `ResizeObserver`, feeding a `stripWidth` signal. This is a genuinely new capability, not a refactor of an existing one — call this out explicitly in review, since it changes a documented architectural property of this file (the file-header comment's own claim that this design is "NOT deliberate pixel-breakpoint tiers" — the multi-row decision now DOES need a real pixel measurement, though the actual row layout inside is still content-sized/organic, not a fixed breakpoint table).

**Two measurement pitfalls found during PR #2812 review, both fixed by observing `.agent-composer-strip-rows` (not the outer `.agent-composer-strip`) and reading `getBoundingClientRect()` (not `entry.contentRect`) inside the `ResizeObserver` callback:**
- **Zoom unit mismatch (reagent P1, task #48's own narrow-tier verification).** `entry.contentRect` reports the element's own LOCAL CSS pixels, unaffected by an ancestor's CSS `zoom` (e.g. the agent view's own `zoom: 0.8`), while per-slot widths use `getBoundingClientRect()`, which IS zoom-scaled. Comparing the two directly made the single-row decision systematically over-generous under any non-1 ancestor zoom.
- **Padding inclusion (reagent P1).** `.agent-composer-strip`'s own `getBoundingClientRect().width` is its BORDER-BOX width, including its own horizontal padding (`var(--space-1) var(--space-2)`, 16px total) — slot widths have no padding component, so comparing them against that inflated width overstated how much fits on one line. `.agent-composer-strip-rows` has no padding/border of its own, so its border-box and content-box widths are identical.

Both pitfalls independently produced the same symptom — the single-row decision firing when the real content didn't fit, and the row's own `flex-wrap` safety net silently reproducing the one-sided-lines bug — discovered via real dev-build screenshot verification at the narrow tier (task #48), not by inspection alone. This is the exact class of gap the retro at the top of this document warned about: a fix that looks correct by inspection or by unit test can still miss a real integration bug that only a real rendered screenshot exposes.

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
| 3. Implement `ResizeObserver` + `stripWidth` signal | ✅ Done | |
| 4. Implement row-builder pure function + single-row delegation | ✅ Done | `computeComposerRows` |
| 5. Implement `hostShell` last-row-right reordering | ✅ Done | |
| 6. Rewire component render output to `<For each={rows()}>` | ✅ Done | Iterates `rowIndices()` (primitives), not `rows()` directly — same identity-stability fix as PR #2808's reagent P1, one level up |
| 7. Update SCSS (`.agent-composer-strip-row{,-left,-right}`; remove old zone tier rules) | ✅ Done | |
| 8. Pure-function tests (§7.1-7.2) | ✅ Done | 26 tests incl. the parameterized invariant test |
| 9. Component regression tests still pass (§7.3) | ✅ Done | |
| 10. `npx tsc --noEmit` + `npx vitest run` + `npx stylelint` clean | ✅ Done | |
| 11. Real screenshot, wide tier (≥482px) | ✅ Done | Single row, matches Rev 6 |
| 12. Real screenshot, narrow tier (<482px) — the tier Rev 6 never re-checked | ✅ Done | Found and fixed a real integration bug here: `ResizeObserver`'s `entry.contentRect.width` is reported in local (pre-zoom) CSS pixels, while per-slot widths use `getBoundingClientRect()` (post-zoom, viewport-relative) — under the agent view's `zoom: 0.8`, this made the "does everything fit on one line" check systematically over-generous, silently deciding single-row and letting the row's own `flex-wrap` safety net reproduce the one-sided-lines pattern via a different mechanism. Fixed by reading `entry.target.getBoundingClientRect().width` instead, so both sides of the comparison share the same coordinate space regardless of ancestor zoom. Confirmed via CDP DOM inspection and a live screenshot at 500px width: 2 correctly-paired rows. |
| 13. Changeset + PR opened | ✅ Done | PR #2812 |
| 14. Review (ReAgent + Codex) addressed | ✅ Done | ReAgent P1 (padding), P2 (wrong gap token) and Codex P1×3 (shed slots, stats reservation, per-pair capacity) — see §3.2 steps 6-8, §3.3, and §1's new physical-capacity exception. 2 new pure-function tests + 5 existing tests recalibrated to realistic `availableWidth` values instead of an artificial "force multi-row" constant. Re-verified live via CDP: still 2 correctly-paired rows at the narrow tier after all fixes. |
| 15. Merged | ✅ Done | PR #2812 merged as `a4058141` |
| 16. Post-merge regression: zoom/gap unit mismatch + stale shed measurement | ✅ Done | Found via live user testing minutes after #2812 merged — the widest tier split into 2 lines, and narrower tiers grew genuinely empty rows. Two root causes: (1) `--space-2` read via `getComputedStyle()` (unzoomed) compared directly against `getBoundingClientRect()`-based widths (zoomed) — the SAME bug class task #12 fixed for width, reintroduced via the gap term; fixed with a `zoomRatio()` helper applied everywhere the two measurement styles mix, including a second pre-existing instance in the per-slot internal-gap measurement (PR #2808, not previously flagged). (2) the per-slot width measurement effect only depended on `slots()` (props), never `stripWidth()` (resize) — a pure resize crossing a CSS shed threshold never re-triggered remeasurement, so a shed slot's stale nonzero width kept defeating the Codex P1 shed-slot exclusion. Fixed by adding `stripWidth()` as a dependency. Verified live via CDP at 630px (1 row), 300px (2 correctly-paired rows), and 150px (3 rows, zero empty sides) against a fresh dev build off updated `main`. PR #2813. |
| 17. Post-merge regression #2: stats zone WRAPPER measured instead of content | ✅ Done | The multi-row-position stats zone is a stretched (default `align-items: stretch`) full-strip-width child of the column-flex strip even when empty — measuring the wrapper fed `reservedWidth ≈ stripWidth` into the single-row fit check, making it unsatisfiable at any width. A one-way trap: any visit to multi-row state locked the strip multi-row forever; width SWEEPS (one direction, starting single-row) structurally cannot catch it — verification now requires a ROUND-TRIP (wide → narrow → wide, asserting return to the original layout). Fixed by measuring the zone's content child (`firstElementChild`), absent exactly when there's nothing to reserve. PR #2814. |
| 18. Edge priority for interactive elements (user-directed constraint) | ✅ Done | On every rendered line, interactive slots (buttons/dropdowns) sit flush against the strip's outer edges; passive content (auth status, ctx text) sits inward. Two mechanisms: `orderKeysForEdgePriority` (stable partition per row side — interactive first on the left, last on the right; ordering only, applied after all row-membership/shed decisions, so it cannot affect §1 or any fit/pairing decision) and side-mirrored internal order for the composite ctx/hostShell slots (Compact/Shell render on the outer end of whichever side the slot lands in — e.g. Compact sits flush-left when ctx is the left edge slot). "Shell always outermost" is preserved (hostShell is interactive AND last in pool order; the partition is stable). 8 new tests (4 pure-fn, 4 DOM-order incl. the degenerate hostShell-on-left flip). Verified live via CDP: single-row tier (`Compact | ctx | auth --- runtime | HOST | Shell`), narrow 2-row tier, and both round-trips. |
| 19. ELEMENT-level edge priority + fit-check conservative rounding (user-directed) | ✅ Done | Two follow-ups to task 18, same day. (a) Slot-level ordering couldn't place a passive element from one slot inward past a NEIGHBORING slot's interactive element (single-row right read `ctx · Compact · HOST · Shell` — passive HOST closer to the edge than interactive Compact). Fixed with CSS `order` scoped to each slot's top-level elements (`.agent-composer-strip-slot-measure > :is(...)`): interactive elements order toward the outer edge across slot boundaries; visual-only, so measurement/pairing/focus order untouched. Required dissolving the `.agent-composer-strip-host-shell` wrapper (HOST + Shell now direct siblings, same shape as ctx's elements) — the user explicitly chose strict edge priority over the older "HOST just left of Shell" pairing directive when they collided. JSDOM can't see stylesheets, so the CSS layer is CDP-verified (single-row right now reads `ctx · HOST · Compact · Shell`); the JS layers keep their unit tests. (b) User-reported "two breakpoints within a couple px": slot widths fed the fit check rounded to NEAREST 8px — up to 4px/slot optimistic, letting the row's CSS flex-wrap safety net overflow a few px BEFORE the JS split. Both measurement effects now round UP (`Math.ceil`), guaranteeing the JS decision fires at-or-before real-layout overflow — one clean breakpoint for any content. |
| 20. Stats eviction middle tier (user-reported 1→3-line jump) | ✅ Done | With live stats ticking, the strip jumped from 1 visual line straight to 3: `reservedWidth` (stats) inside `computeComposerRows`'s fit check made a too-wide stats zone split the SLOTS (2 rows) while the stats simultaneously moved to their own dedicated line. Missing middle tier: slots on one line + stats evicted below (2 lines). `computeComposerRows` no longer takes `reservedWidth` — row membership is decided from slot widths alone; whether the stats share the single row is the component's separate `statsInline` decision (slots+stats+gap ≤ width). Codex P1 #2812's original overflow concern cannot recur: when slots-plus-stats don't fit, the stats leave the row instead of overflowing it. Tier progression is now 1 line → 2 (slot row + stats row) → 3 (2 slot rows + stats row). Unit-tested (contract test rewritten); live tier verification pending an open agent pane (closed mid-session during this change). |
