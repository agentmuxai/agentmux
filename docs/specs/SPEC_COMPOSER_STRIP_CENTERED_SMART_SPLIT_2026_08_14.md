# SPEC — Composer strip: stable width + deliberate edge-split tiers

**Date:** 2026-08-14
**Type:** Responsive layout fix
**Status:** Implemented
**Scope:** `frontend/app/view/agent/components/AgentComposerStrip.tsx`,
`frontend/app/view/agent/styles/_composer-strip.scss`

## Related history

- `SPEC_COMPOSER_STRIP_LEFT_JUSTIFIED_TIERED_WRAP_2026_08_03.md` — the
  immediately-prior design. Fixed a real clipping bug (PR #2393/#2408) by
  switching from a hard `grid-template-areas` 2-row swap to `flex-wrap`
  with no line cap, and made every tier below the widest (≥482px)
  left-justified with lines breaking wherever content organically ran out
  of room. **This spec reverts the left-justify decision** — flagged
  directly by the user as a mistake — but keeps every part of the
  clip-safety design (no `max-height`/`overflow:hidden`, internal
  `flex-wrap` on `-controls`/`-right`) unchanged.

## Problems

1. **1↔2 line flicker with no width change.** `AgentComposerStrip` ticks
   `turnTokens`/elapsed every second while a turn is in flight
   (`useTick(1000)`). `formatElapsedCompact`/`formatCompactNumber` don't
   produce fixed-length strings (`"9s"` → `"10s"`, `"↑890 ↓340"` →
   `"↑1.2k ↓340"`), so the stats zone's rendered width drifts slightly
   every tick. With no floor on that width, a wrap decision sitting right
   at the fit boundary can flip on every tick even though the pane itself
   never resized — the "toggles annoyingly between 1 and 2 lines"
   complaint.
2. **Left-justify-always was a mistake.** Below 482px, content wrapped
   wherever `flex-wrap` happened to run out of room, always left-justified.
   The user wants deliberate split points instead, preserving the *same
   edge-split visual language as the widest tier* (something pinned left,
   something pinned right, sometimes something centered between) as the
   pane narrows through more line tiers — not raw left-packing, and,
   after an intermediate revision below, explicitly **not** a centered
   blob either.

## Design

### 1. Stable width for the stats zone

`.agent-composer-strip-stats` gets `min-width: 12ch` (it's already
monospace, so `ch` sizing is exact). This doesn't make the box perfectly
immutable — a genuinely long-running turn can still exceed it and cause a
real reflow — but it absorbs the common tick-to-tick jitter (seconds
rolling over, token counts crossing a k/m suffix) that was the actual
trigger, not a pane resize.

### 2. Three deliberate, edge-split tiers

Two zones (`-stats-zone`, `-right`) get a default `flex-basis: 100%` —
forces each onto its own line. One `@container agent-pane` query
progressively un-forces `-stats-zone` as width grows:

| Width | Lines | Layout |
|---|---|---|
| < 280px | 3 | `[controls]` / `[stats]` / `[right]`, each its own line, each keeping its own identity (left / center / right) exactly as it would sit at the widest tier — just stacked instead of side by side |
| 280–481px | 2 | `[controls (left) \| stats (right)]` on line 1 / `[right]` (right-anchored) alone on line 2 |
| ≥ 482px | 1 | `controls` left / `stats` true-centered / `right` right (unchanged from the 08-03 addendum) |

Each zone keeps a fixed identity at every tier instead of the whole strip
being blob-centered:

- **Controls** — never forced to `flex-basis: 100%` (it's the anchor the
  other two zones' splits are relative to: as the first zone in DOM order
  it's already alone on line 1 whenever `-stats-zone` is also forced onto
  its own line). No `justify-content` override — stays left-anchored by
  the flexbox default at every tier, including the ≥482px tier's own
  `flex: 1 1 0` half of the row.
- **Stats** — `text-align: center`, unconditionally. Centered whether it's
  alone on a full-width line (<280px) or true-centered between controls
  and right (≥482px). At 280-481px it shares line 1 with controls — the
  outer strip's `justify-content: space-between` (not `center`) pins it
  to that line's right edge, matching its role as the "other end" of that
  2-zone line.
- **Right** — `justify-content: flex-end`, unconditionally. Always
  right-anchored, whether alone on a full-width line (<482px) or occupying
  its own `flex: 1 1 0` half at the widest tier.

`.agent-composer-strip`'s own `justify-content` is `space-between` — on a
line with exactly 2 zones sharing it (the 280-481px tier's controls+stats
line) this pins the first to the line's left edge and the last to its
right edge, not centered as a pair. Lines with a single zone forced to
`flex-basis: 100%` have no slack left for the parent's `justify-content`
to matter either way, so that zone's own internal alignment (above) is
what actually determines its position.

### 3. Why this doesn't reintroduce the PR #2393/#2408 clipping bugs

The `flex-basis: 100%` toggle is a "smart split point" implemented as a
regular flex property, not a return to `grid-template-areas`:

- Still no `max-height`/`overflow: hidden` on `.agent-composer-strip`.
- `.agent-composer-strip-controls`/`-right` keep their own internal
  `flex-wrap: wrap` — if a zone's content still doesn't fit the line it's
  been assigned (e.g. a very long resolved model name in `-controls` on
  the 2-line tier), it wraps onto an internal sub-line instead of
  overflowing, exactly as the 08-03 fix established.
- If a tier's breakpoint estimate is off for some real content
  combination, the result is an extra reflow, not a clip — the same
  safety net every breakpoint in this file already relies on.

### Breakpoint estimate

280px is estimated from documented content widths (controls ≈ 120-150px
including its gap, stats ≈ 84-96px with the new 12ch floor plus gap),
matching the file's existing practice of estimating from content width
rather than measured DOM output (see the ≥482px tier's own history: first
shipped as an estimated 640px, corrected to a live-measured 482px after
testing in `task dev` found the real 1-line/2-line wrap point). **280px
has not yet been live-verified in `task dev`** — same outstanding caveat
as most breakpoints in this file; adjust if real content doesn't match.

## Addendum (same day) — two corrections from live user review

The design above shipped, was run in `task dev`, and got two rounds of
direct user feedback:

**Round 1 — dead/wrong `justify-content: center` on `-controls`.** The
first implementation added `justify-content: center` to
`.agent-composer-strip-controls`, reasoning it would matter symmetrically
with `-stats-zone`/`-right` whenever the zone was forced alone onto a
full-width line. But controls is *never* forced to `flex-basis: 100%` (it
never needs to be — see "Controls" above), so its box always matches its
own content width exactly, except at the ≥482px tier's `flex: 1 1 0`,
where the box genuinely is wider than its content. There, `center`
visibly pulled the Mode/Model/Effort trigger away from the left edge —
the most commonly-seen tier, so immediately visible. **Fix:** removed the
rule entirely rather than add a targeted override; controls now relies on
the flexbox default (`flex-start`) everywhere, which is correct at every
tier including ≥482px.

**Round 2 — the whole design was still wrong: blob-centering instead of
edge-splitting.** The first implementation used `.agent-composer-strip`'s
`justify-content: center` (centering whichever zones share a line as a
group) and internal `justify-content: center` on `-right` (centering its
content when alone). User feedback: this wasn't the intent — "as the
responsive [design] triggers more lines, the unit at which the left
justified and right justified portions change... right now you are
centering everything. We need it split just like at the largest
responsive stage, just what gets split is thought through intelligently."
I.e.: preserve the widest tier's left/center/right *edge-split* language
at every tier — never collapse to a centered blob. **Fix:** `-strip`'s
`justify-content` changed `center` → `space-between` (pins a 2-zone
line's endpoints to that line's edges instead of centering them as a
pair); `-right`'s internal `justify-content` changed `center` → `flex-end`
(always right-anchored, matching its widest-tier identity, not merely
centered filler when alone). `-stats-zone`'s `text-align: center` was
already correct under this model — its identity *is* "centered," matching
its widest-tier role — and needed no change.

The design and table in the body above already reflect both corrections;
this addendum exists so the "why" (two real revisions, not typos) has a
record, matching this file's established pattern (see the 08-03 spec's
own same-day addendum).

## Files changed

| File | Change |
|---|---|
| `frontend/app/view/agent/styles/_composer-strip.scss` | `.agent-composer-strip`: `justify-content: flex-start` → `space-between`. `-stats-zone`: default `flex-basis: 100%` + `text-align: center`; `flex-basis: auto` at ≥280px. `-right`: default `flex-basis: 100%` + `justify-content: flex-end` (both apply at every tier, not just ≥482px). `min-width: 12ch` added to `.agent-composer-strip-stats`. No change to `-controls` (an earlier `justify-content: center` addition was added then removed same day — see addendum). Comments updated throughout. |
| `frontend/app/view/agent/components/AgentComposerStrip.tsx` | No structural change — header comment updated to describe the edge-split design. |

## Verification performed

- `npx tsc --noEmit` — clean (2 pre-existing unrelated errors in
  `armory-view.tsx`/`warden-view.tsx` confirmed present on `main` before
  this change, via `git stash` + re-run).
- `npx stylelint frontend/app/view/agent/styles/_composer-strip.scss` —
  one pre-existing hex-color lint error (`--warning-color, #d9a441`
  fallback), same one flagged and left as-is by the 08-03 spec.
- `npx vitest run frontend/app/view/agent` — 1142/1143 passed; the one
  failure (`tool-renderers/registry.test.ts`, a 5s timeout) is unrelated
  to this change and passes cleanly in isolation (confirmed flaky, not a
  regression).
- Run live in `task dev` (Windows) with hot reload; the Round-1 bug was
  caught this way — by the user, not by static checks, since no test
  exercises this component's rendered layout (same gap the 08-03 spec
  shipped with).
- **Not performed:** a full pass through every documented breakpoint (the
  280px tier specifically) confirming pixel-accurate line counts — only
  the ≥482px tier and the Round-1 bug's specific symptom were actually
  observed. Resize a real agent pane through ~500px → ~150px and confirm:
  (a) the 3 tiers land close to the documented widths, (b) the 280-481px
  tier reads as controls-left/stats-right on one line, not centered
  together, (c) no clipping/overlap with the textarea below at any width,
  (d) the stats zone no longer visibly reflows on its own every second
  during an in-flight turn at a width near a tier boundary.
