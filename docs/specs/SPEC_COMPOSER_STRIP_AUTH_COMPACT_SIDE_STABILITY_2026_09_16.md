# SPEC: Composer Strip — Auth/Compact Side Stability Across the Row-Count Boundary

**Date:** 2026-09-16
**Status:** implemented — §4's design shipped as written (the `pinnedPairs` — #3282
generalization, §4.3), with `[{ leftKey: "auth", rightKey: "ctx" }]` wired
at the one call site. Logic validated standalone (vitest is blocked locally
by a pre-existing, unrelated `solid-refresh`/JSDOM environment issue — CI is
the authority there) and visually against the real compiled app via `task
dev`, resizing the actual window across the boundary — no swap in either
state. PR #3282 review (ReAgent P2, Codex P1) found two gaps §4 didn't
originally cover, both fixed: the unmeasured-fallback path's static `side`
fields still had the pre-fix values, reintroducing the swap at every mount
before the first real measurement (fixed: `auth.side` → `"left"`, `ctx.side`
→ `"right"`, matching the pinned invariant); and the multi-row pinned-pair
row was emitted unconditionally, bypassing the per-pair capacity check the
generic two-pointer pairs already have — an over-width pair would still
`flex-wrap` into two physical one-sided lines despite the row's data
formally having both sides filled (fixed: same capacity check, split into
two one-sided rows preserving each member's own side when it doesn't fit).
**Affects:** `frontend/app/view/agent/components/AgentComposerStrip.tsx` —
the `auth` slot ("Logged in" / "Not logged in") and the `ctx` slot's Compact
button.
**Read first:** `AgentComposerStrip.tsx`'s own file-header comment (Rev
1-8 history — this is the 9th revision of this file's zone-balancing logic)
and `docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md` (Rev 7,
the row-based model this spec builds on).
**Trigger:** user report, verbatim: *"in the agent pane, when cross approx
342 px width, the Logged in and Compact switch sides. Can you have them
retain the side they were on before the thinning of the pane? Logged in
should stay left, compact should stay right."*

---

## 1. The bug, confirmed

Captured directly (`UIQuery` against a live agent pane's composer strip,
already narrower than the reported threshold — `.agent-composer-strip` at
280px width):

```
"agent-composer-strip-compact-btn"  x=8.0   ("Compact")
"agent-composer-strip-ctx"          x=47.4  ("431k / 1.0m")
"agent-composer-strip-auth--ok"     x=232.7 ("Logged in")
```

At this width: **Compact is the leftmost element, "Logged in" is the
rightmost.** Per the user's report, above ~342px the arrangement is the
opposite: "Logged in" left, Compact right. That wider-state arrangement is
the one the user wants preserved permanently — i.e. the fix is not "always
put auth left," it's "stop letting the two layout algorithms disagree with
each other."

---

## 2. Root cause

Two *independent* algorithms decide which of `auth` and `ctx` renders left
vs. right, with no shared state between them, and they disagree by
construction:

### 2.1 Wide state — single row (`computeBalancedLeftKeys`, ≥ ~342px here)

When everything fits on one line, `computeComposerRows` delegates to
`computeBalancedLeftKeys` (lines 154-194), which brute-forces every subset
of the *movable* pool (`badge?`, `auth`, `ctx` — `hostShell` is always
fixed-right, `runtime`/anchorLeft is always fixed-left) to minimize
`|leftWidth - rightWidth|` against those two fixed anchors.

`runtime` (the model/effort dropup, e.g. "Bypass · Sonnet 5 · high") is
usually wider than `hostShell` (HOST badge + Shell button) as a fixed base.
Since `ctx` (context text + optional countdown + Compact — up to 3
sub-elements) is reliably the widest movable item and `auth` ("Logged
in"/"Not logged in") is one of the narrowest, the diff-minimizing subset
tends to land `auth` on the smaller-fixed-base side (left, offsetting
`runtime`) and `ctx` on the larger-fixed-base side (right, offsetting
`hostShell`) — **auth left, ctx (Compact) right**. This is *incidental*,
not guaranteed: it falls out of whatever `runtime`/`hostShell`'s actual
measured widths happen to be on a given render, not a rule the function
states anywhere.

### 2.2 Narrow state — multi-row (`computeComposerRows`, < ~342px here)

Below the fit threshold, `runtime` and `hostShell` are reserved out as
their own anchored final row (Rev 8, lines 320-370), and the *remaining*
pool (here: just `auth` + `ctx`) is sorted descending by width and paired
by a two-pointer walk (lines 331-344): widest with narrowest,
`pairs.push([sorted[i].key, sorted[j].key])` → `sorted[i]` (wider) always
becomes the row's `left`, `sorted[j]` (narrower) always becomes `right`.

`ctx` (Compact + context text [+ countdown]) is reliably wider than `auth`
alone, so with exactly two items in the pool this pairing is deterministic
and unconditional: **ctx (Compact) left, auth right** — the opposite of
§2.1, and unrelated to `runtime`/`hostShell`'s widths entirely, because
those two are no longer even in the same pool by this point.

### 2.3 Why ~342px specifically

That number is not a designed threshold anywhere in the code — it is
wherever `computeComposerRows`'s `totalWidth <= availableWidth` check
(line 300) flips from true to false for *this* pane's actual current
content (provider, auth state, context fill, whether a process badge is
showing). It will differ agent-to-agent and render-to-render as those
inputs change. The bug is the discontinuity at whatever that width is, not
the specific value — this spec's fix must not hardcode 342px anywhere.

---

## 3. Precedent already in this file

This is not a new problem shape. Rev 8 (the "ANCHORED ELEMENTS" comment at
lines 229-251) already solved exactly this class of bug for `runtime` and
`hostShell`, which used to travel between sides as the pane resized and
were pinned after a direct user directive:

> "This deliberately reverses the earlier 'the model selector moving sides
> is acceptable' call recorded in the retro's step 3 — it was dismissed
> once as cosmetic and has now been made a hard constraint."

The mechanism: `runtime`/`hostShell` are excluded from both balancing
algorithms' free-floating search entirely and instead always resolve to a
fixed position — `runtime` as `fixedLeftWidth` in `computeBalancedLeftKeys`
and the outermost-left anchor in the single row; `hostShell` as
`fixedRightWidth` there and reoriented-right-of-its-pair in
`computeComposerRows`'s multi-row branch (lines 371-385, the existing
`hostPairIdx` reorientation logic).

`auth`/`ctx` need the identical treatment, extended to a **second**
anchored pair — reusing the mechanism, not inventing a new one.

---

## 4. Proposed fix

### 4.1 Single-row (`computeBalancedLeftKeys`)

When both `auth` and `ctx` are present in the movable pool, pull them out
of the free `2^n` subset search and fold them into the existing fixed-width
accumulators exactly like the current anchors do:

- `auth`'s width folds into `fixedLeftWidth` (alongside `runtime`, when
  present).
- `ctx`'s width folds into `fixedRightWidth` (alongside `hostShell`).
- The brute-force search runs only over whatever's left (today: `badge`
  alone, when a process count exists).

When only one of `auth`/`ctx` is present (the other's visibility condition
isn't met — e.g. auth status still `"unknown"`, or no context tracked yet),
there's nothing to pin against; it stays an ordinary free movable slot,
unchanged from today.

### 4.2 Multi-row (`computeComposerRows`)

Extend the anchor-reservation branch (§3, lines 320-370) to accept this as
a *second* reserved pair, analogous to `{anchorLeftKey, hostShellKey}`:
when both `auth` and `ctx` slots exist, reserve them out of the two-pointer
sort/pair pool and emit them as their own row — `{left: [auth's key],
right: [ctx's key]}` — unconditionally, not subject to the width-sort. The
remaining pool (`badge`, if present) continues through the existing
two-pointer pairing untouched.

If only one of `auth`/`ctx` exists, it's not reserved and falls through to
the existing generic pairing — same escape hatch as §4.1.

### 4.3 Signature change

Both functions currently take one anchor (`anchorLeftKey` on
`computeComposerRows`; the implicit runtime/hostShell fixed widths on
`computeBalancedLeftKeys`). Cleanest shape: generalize to an explicit list
of pinned pairs — `pinnedPairs: { leftKey: string; rightKey: string }[]` —
rather than hardcoding a second named pair (`authKey`/`ctxKey`) alongside
the first. Two pairs today (`runtime`/`hostShell`, `auth`/`ctx`); the
mechanism should not need a third bespoke parameter if a future slot needs
the same treatment. `computeBalancedLeftKeys` gains the analogous
generalization: fold every pinned pair's left member into
`fixedLeftWidth`, right member into `fixedRightWidth`, and brute-force only
the true remainder.

### 4.4 Explicit tradeoff (same one Rev 8 already accepted)

Reserving `auth`/`ctx` out of the balance search reduces how much the
remaining free slots (`badge`) can do to equalize left/right totals when
both anchors AND both `auth`/`ctx` are pinned simultaneously — in the
degenerate case where `badge` is the *only* free slot, it alone cannot
meaningfully rebalance `runtime` vs. `hostShell` the way `auth`/`ctx` used
to help with incidentally. This is the identical tradeoff Rev 8's own
comment names for `runtime`/`hostShell` ("dismissed once as cosmetic and
has now been made a hard constraint") — stability of these specific
elements' sides is prioritized over marginal width-balance in the row(s)
they occupy. Not a regression to silently accept; a deliberate, named
choice, same as last time.

---

## 5. Out of scope

- **Three-or-more-item pools** (e.g. `badge` + `auth` + `ctx` all present
  together) are not addressed beyond falling through to whatever the
  now-smaller free pool naturally does — the user's report and the
  reproduction in §1 are both the two-item case. If a future report shows
  `auth`/`ctx` disagreeing with `badge`'s placement, that's a new spec, not
  a silent scope-creep of this one.
- **No new visual design.** This does not change which side is "correct"
  in any absolute sense — it only removes the width-dependent
  discontinuity, converging on whichever arrangement the single-row
  balance search already tends to produce (auth left, ctx/Compact right,
  matching the user's stated preference) and holding it at every width.

---

## 6. Testing

Both functions are already pure, exported, and unit-tested without a real
layout engine (`AgentComposerStrip.test.tsx`). Extend, mirroring the
existing `describe("computeComposerRows — anchored model selector + Shell
(Rev 8)", …)` block (lines 454+) and the plain `describe("computeBalancedLeftKeys", …)`
block (lines 203+):

- `computeBalancedLeftKeys`: given `auth` and `ctx` both in `movable`, at
  every relative width ratio between them (including cases where the old
  unconstrained brute-force would have picked the opposite split), assert
  `auth` always resolves left and `ctx` always resolves right.
- `computeComposerRows`: given a pool requiring 2+ rows with `auth` and
  `ctx` both present, assert they always land in the same row together,
  `auth` on `left`, `ctx` on `right` — across a spread of widths that
  straddle the single-row/multi-row boundary, confirming no discontinuity
  at the crossover itself (the actual defect: render at width `W-1` then
  `W` where `W` is the fit-boundary width for a fixed set of slot widths,
  assert `auth`'s side is identical in both).
- Regression case: an agent pane snapshot test (or the existing zone-
  assignment identity-stability describe block, lines 135+) resized across
  the fit boundary with real widths, confirming no DOM remount of either
  slot (the same identity-stability property Rev 6 already protects for
  other slots via the `slotByKey`/`untrack` pattern).
