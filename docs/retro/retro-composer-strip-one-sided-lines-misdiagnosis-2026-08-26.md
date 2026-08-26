# Retro: Composer Strip "One-Sided Line" Bug — Six Revisions, Still Undetected Until the User Spelled It Out

**Date:** 2026-08-26
**Severity:** Medium — no data loss or functional break, but a visibly broken layout shipped to `main` (PR #2808) and stayed undiagnosed across at least three separate verification passes by the agent, including two AFTER a real, correct screenshot was already in hand.
**Observed by:** Manoz (Claude agent), composer-strip zone-balance work
**Related:** `docs/specs/SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md` (Rev 1-6 full history), `docs/status/STATUS_COMPOSER_STRIP_ZONE_BALANCE_HANDOFF_2026_08_25.md`, `docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md` (the fix this retro leads into)

---

## TL;DR

Six revisions of this file's left/right balancing logic (Rev 1 through Rev 6, spanning two days and multiple sessions) all treated "balance" as a property of two AGGREGATE totals — "is zone A's total width close to zone B's," "is either zone completely empty." None of them ever checked the property a human actually looks at: **does every individual rendered LINE have content on both its left and right.** A layout can have perfectly balanced aggregate totals and zero empty zones while still looking badly broken, because the two zones each independently overflow onto their OWN full-width, one-sided lines instead of sharing lines together. That is exactly what shipped in PR #2808's Rev 6, and it survived three separate look-at-it-and-explain-it passes — including two after a real, correctly-captured screenshot was already sitting in front of the agent — until the user stated the invariant in explicit mathematical terms.

---

## What Happened

1. **Rev 6 (this session)** replaced Rev 5's fixed left/right slot pairing with `computeBalancedLeftKeys` — a real-DOM-measurement search that picks whichever GLOBAL 2-way partition of the slot pool minimizes `|leftTotalWidth − rightTotalWidth|`. This was visually verified at the ≥482px "everything fits on one line" tier via a real `task dev` screenshot, which looked correct, and shipped.
2. The user asked to see a fresh screenshot and describe the problem. The agent's FIRST capture was garbled (overlapping window content) — root-caused correctly as a screen-capture methodology bug (`SetForegroundWindow` silently failing, `CopyFromScreen` grabbing a different, unrelated overlapping window) and fixed by switching to `PrintWindow`.
3. With a clean, correct capture in hand, the agent examined it and reported the "problem" as: the Mode/Model/Effort trigger had moved to the right side of the strip instead of staying left — a UX-consistency regression, re-treading a concern already flagged (and dismissed as acceptable) back in Rev 3's own spec section. **This was not the actual defect** — nothing was clipped, nothing overlapped, the described "problem" was cosmetic positioning, not the structural bug.
4. The user corrected this twice, progressively more explicitly: first "it is very visually broken... for every line X there are 2X sections from which you must evenly distribute the elements... do you see how there are 2 lines but only 2 elements?", then, after the agent's restated understanding was still slightly off, the precise form: **"there are only 2 sections. the elements on 2 lines would need to be evenly distributed across 4 sections. but the screenshot only has 2 sections."**
5. Only at that point did the actual defect become visible: line 1 was 100% the left zone's content (auth + context group), entirely left-justified with nothing on its right half; line 2 was 100% the right zone's content (runtime trigger + HOST/Shell), entirely right-justified with nothing on its left half. Two lines, four possible corners, only two ever filled.

---

## Root Cause

**Every verification method used across Rev 1 through Rev 6 operated on a proxy metric, not the real one.** The real requirement — every rendered line has both a left and a right occupant — was never written down anywhere in this file's own spec history as an explicit, checkable acceptance criterion. What WAS written down and checked, repeatedly:

- "Neither zone is completely empty" (Rev 4's override rule, still present in Rev 6)
- "Left/right total width difference is minimized" (Rev 2/3's weight search, Rev 6's `computeBalancedLeftKeys`)
- "Everything fits on one line at the widest tier" (verified with a real screenshot in Rev 6)

All three can be simultaneously true while the actual per-line layout is exactly the zigzag pattern the user found: two zones, each individually well-formed (non-empty, globally width-balanced against its sibling), that simply never share a rendered line with each other because each one independently decides when IT needs to wrap onto its own dedicated full-width block. The zone-based architecture (`.agent-composer-strip-controls` and `.agent-composer-strip-right`, each an independent flex container that defaults to `justify-content: flex-start`/`flex-end` respectively and gets `flex-basis: 100%` at narrow widths) has no concept of a shared "row" at all — there was never a code path that could have produced a jointly-occupied line, so no amount of tuning the width-balance math inside that architecture could have fixed this.

### Why three look-and-describe passes missed it

1. **Rev 6's own visual verification had a silent scope gap.** The real screenshot check confirmed the ≥482px single-line case (the specific case the session's tracking issue was about) and correctly reasoned that check was sufficient for THAT tier. It never independently re-screenshotted a narrow, multi-line tier — the code comments describing that tier as "3 lines: controls / stats / right, each its own line, by design" were inherited from Rev 4/5 as settled fact rather than re-examined against the actual requirement being solved in Rev 6. A claim written in an EARLIER revision, to describe a DIFFERENT problem, was carried forward unquestioned.
2. **The first correct screenshot triggered confirmation bias, not fresh observation.** Faced with real pixels, the agent reached for the most readily-available prior explanation on file (Rev 3's already-documented "runtime trigger position stability" concern) instead of asking "what does this specific image structurally show." The explanation was plausible, previously validated, and wrong for this instance — a textbook case of pattern-matching to a familiar prior finding instead of re-deriving from the actual evidence in hand.
3. **No test operationalized the real invariant.** Even the NEW unit tests written for `computeBalancedLeftKeys` in this same session (hand-verified arithmetic, tie-break behavior, etc.) all asserted properties of the aggregate 2-way split — none of them, and nothing else in the test suite, asserted "does this look like N well-formed rows" because that data shape didn't exist yet. A regression in the CSS/JS interaction that produces one-sided lines would have sailed through every existing check.

### The corrective mechanism that actually worked

Not a screenshot, not a test, not the agent's own re-examination — **the user restating the requirement in a form precise enough to be checked against the image mechanically** ("2 lines should have 4 filled sections; count how many are actually filled"). That framing is exactly what an acceptance criterion should have looked like from Rev 1 onward and didn't.

---

## Prevention / Follow-ups

- **Write the real invariant down before touching this file again.** `docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md` states it explicitly: every row must have both a left and right occupant, except the one mathematically unavoidable case (an odd total slot count leaves exactly one singleton row, and a total of exactly 1 slot has nothing to pair with at all). Any future change to this file's balancing logic must be checked against that written invariant, not against "do the aggregate widths look close."
- **Add a test that asserts the row-level invariant directly** on the new row-building function's output — not just spot-checking individual widths, but literally counting rows with an empty side and failing if that count exceeds the mathematically-justified minimum for the given slot count.
- **Screenshot more than one width before calling a fix verified.** A single confirmed screenshot at the tier the original bug report was about does not confirm behavior at OTHER tiers, especially when the change touches shared logic (the balance algorithm) that ALL tiers depend on. Treat an inherited code comment describing "how a different tier behaves" as a claim to re-verify, not a fact to build on.
- **When a human's description of a bug doesn't immediately parse, don't reach for the nearest previously-documented adjacent explanation.** Ask them to restate more precisely, or force yourself to re-derive the geometry from the actual pixels again, before offering a diagnosis that happens to match something already on file.
- **This file now has a "Rev 7."** Six revisions of the SAME symptom class ("looks unbalanced") is itself a signal that the specification, not just the implementation, was under-specified. `SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md` is written as the first attempt at a genuinely complete, checkable spec for this component's layout behavior — if an eighth revision is ever needed, start by asking whether that spec's invariant itself was wrong, not just whether the code satisfies it.
