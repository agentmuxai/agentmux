# SPEC — Composer strip: left-justified, tiered wrap (up to 3 levels)

**Date:** 2026-08-03
**Type:** Responsive layout fix
**Status:** Implemented
**Owner:** Agent3
**Scope:** `frontend/app/view/agent/components/AgentComposerStrip.tsx` +
`frontend/app/view/agent/styles/_composer-strip.scss`

## Related history

- `SPEC_COMPOSER_STRIP_RESPONSIVE_ARCHITECTURE_2026_07_02.md` — root-caused the
  strip force-hiding the runtime controls (Mode/Model/Effort) via
  `display:none`; established the standing rule that controls are shed last,
  informational content first.
- `SPEC_COMPOSER_STRIP_LAYOUT_MIC_CENTER_MODEL_DEFAULTS_2026_07_10.md` —
  introduced the 3-column grid (`1fr auto 1fr`) with the stats zone
  true-centered in the middle column.
- `SPEC_COMPOSER_STRIP_TWO_LINE_RESPONSIVE_2026_07_30.md` — **already merged**
  as of this spec. Found the narrow-pane container queries were dead code
  (wrong container name, `modal-mount` instead of `agent-pane`) and landed a
  fix: a `grid-template-areas` swap at `≤480px` that moved the strip to a
  fixed 2-row layout (row 1 = controls + right zone, row 2 = stats,
  centered), plus shed queries at `≤300px`/`≤260px`/`≤220px` for the auth
  tag, process badge, and control-trigger cap. This is the baseline this
  spec starts from — not the 07-10 single-row grid.

## Problem

Following the 07-30 two-line fix, the strip still had two issues the user
flagged directly:

1. **Not always left-justified.** The stats zone was centered in Tier 1 (grid
   middle column) and centered again in Tier 2 (`justify-self: stretch;
   text-align: center`). The right zone was pinned to the row's right edge
   via `justify-self: end`. Centering/right-pinning via a grid is also what
   made the two-line fix a fixed, hardcoded 2-row `grid-template-areas` swap
   rather than something that reflows naturally — the grid has no built-in
   notion of "wrap to a 3rd line only if needed."
2. **Only ever 2 lines, hard-capped.** The 07-30 fix bought exactly one extra
   line (row 2 for stats). At widths where row 1 (controls + right zone)
   still didn't fit, the only remaining lever was to hide content (auth tag,
   then process badge) — there was no 3rd line to fall back on first.

## Goal

1. **Always left-justify.** No centering, no right-pinning, at any tier.
   Content flows left-to-right in DOM order (controls → stats → right zone)
   and starts at the strip's left edge on every line, including wrapped
   ones.
2. **Wrap up to 3 levels before shedding content.** Let the browser reflow
   content onto additional lines as needed — normally 1-3, see the
   "revised during review" note below — before falling back to hiding
   informational content (auth tag, then process badge) or compacting the
   runtime controls trigger. **Never clip or overlap** — this is the harder
   requirement and the one that changed the design mid-review (§6).

## Design (as implemented)

### 1. Grid → wrapping flex

`.agent-composer-strip` changed from `display: grid` (with the 07-30 fix's
`≤480px` `grid-template-areas` swap) to a single, permanent `display: flex;
flex-wrap: wrap; justify-content: flex-start`. This removes the fixed 2-row
breakpoint entirely — the browser wraps `agent-composer-strip-controls`,
`agent-composer-strip-stats-zone`, and `agent-composer-strip-right` (still
the same three top-level children, no TSX structure change) onto as many
lines as the content needs, always left-justified, with no explicit
width-keyed rule required for the wrapping itself.

```scss
.agent-composer-strip {
    display: flex;
    flex-wrap: wrap;
    align-content: flex-start;
    justify-content: flex-start;
    align-items: center;
    row-gap: 2px;
    column-gap: var(--space-2);
    min-height: 28px;
    // No max-height/overflow:hidden — see §6, this was in the first
    // version of the fix and removed after review found it could clip
    // Shell again once the zones below wrap internally.
    ...
}
```

### 2. Zone rules: drop grid-only properties

`agent-composer-strip-controls`/`-stats-zone`/`-right` all lose their
`grid-area`/`justify-self` declarations (meaningless on flex children); they
keep `min-width: 0` so the runtime-trigger max-width cap can still shrink
the controls zone under real pressure. No `justify-content: center` or
`justify-self: end` remains anywhere in the file.

### 3. No hard line cap — see §6

The first version of this fix added `max-height: 84px; overflow: hidden`
(~3 lines) to enforce "up to 3 levels" as a hard ceiling. Review found this
reintroduced clipping once `.agent-composer-strip-controls`/`-right` could
also wrap internally (§6), so the cap was removed: `flex-wrap` reflows
content onto as many lines as it actually needs, with no upper bound
enforced by CSS. In practice the shed-content queries below keep real
content to ~1-3 lines by shrinking/hiding it as width drops; "3 levels" is
the normal-case outcome of that shedding, not a mechanism that clips a 4th
line into existence.

### 4. Shed order — revised breakpoints, informational-first (unchanged principle)

The 07-30 shed breakpoints (auth `≤300px`, process badge `≤260px`, controls
cap `≤220px`) were tuned for a layout with only 2 available lines. With 3
lines available — in the worst case, one zone per line — the right zone
(badge + ctx + auth + Shell) has much more total width budget before it
needs to shed anything, so the breakpoints move narrower:

```scss
@container agent-pane (max-width: 220px) {
    .agent-composer-strip-auth { display: none; }
}
@container agent-pane (max-width: 180px) {
    .agent-composer-strip-process-badge { display: none; }
}
@container agent-pane (max-width: 150px) {
    .agent-runtime-dropup-trigger { max-width: 120px; }
    .agent-composer-strip-log-btn { font-size: 9px; padding: 1px 3px; }
}
```

Order and reasoning unchanged from 07-30/07-02: auth tag first (passive
status, least essential), then process badge (a shortcut, swarm view stays
reachable elsewhere), controls last and never `display:none` (07-02's
standing rule) — only compacted via the existing ellipsis cap. Context text
and Shell remain visible at every tier.

**Breakpoint values are estimated from content width** (right zone ≈ badge
28px + ctx 50px + auth 70px + Shell 40px + 3 gaps ≈ 220px), not measured DOM
output — same caveat every prior composer-strip spec has flagged about its
own numbers. Needs live verification in `task dev`.

### 5. Shell button

Stays the last child in DOM order — no TSX structural change. On a wide
pane it's still visually rightmost (last in a left-filling line); on a
narrow, wrapped pane it may be the sole item on its own line, left-justified
like every other line. This is the expected consequence of Goal 1, not a
regression.

## 6. Revised during review (PR #2393)

Two rounds of automated review on the PR found real problems in the first
version of this fix — both are the same underlying tension (a hard clip
boundary vs. content that can legitimately need more room) resurfacing at
a different layer:

1. **Codex (P1):** `.agent-composer-strip-right` moved as a single,
   non-internally-wrapping flex item. When it landed alone on a wrapped
   line narrower than its full content (badge + ctx + auth + Shell), the
   strip's `overflow: hidden` clipped the trailing **Shell button** —
   breaking the "Shell always reachable" invariant from
   `SPEC_COMPOSER_STRIP_RESPONSIVE_ARCHITECTURE_2026_07_02.md`/
   `SPEC_COMPOSER_STRIP_TWO_LINE_RESPONSIVE_2026_07_30.md`. Same latent risk
   on `.agent-composer-strip-controls` (a long resolved model name before
   the `≤150px` trigger cap applies).
   **Fix:** both zones gained their own `flex-wrap: wrap`, so a zone's
   content reflows onto an internal sub-line instead of overflowing past
   the strip's edge.
2. **ReAgent (P2), on the fix above:** adding internal wrap converts
   *horizontal* overflow into *vertical* growth — which competes with the
   outer strip's `max-height: 84px; overflow: hidden` from §1/§3. If two
   zones each need an internal sub-line at once (e.g. controls wraps the
   HOST/SANDBOX tag under a long model name while right zone wraps Shell
   under badge+ctx+auth, at the same narrow width), total height can
   exceed 84px and the outer `overflow: hidden` clips Shell again — same
   failure mode, triggered by height instead of width, and not covered by
   the "not performed" live-verification caveat below (it's a structural
   interaction, not a tuning issue).
   **Fix:** removed `max-height`/`overflow: hidden` from
   `.agent-composer-strip` entirely (§3). There is no longer any
   mechanism in this component that can clip content; the shed-content
   queries are the only thing keeping real-world height to ~1-3 lines, and
   they do so by shrinking/hiding content, not by force.

Net effect: "up to three levels" is now the *expected outcome* of the shed
order under normal content, not a *ceiling enforced by clipping*. That's a
one-word-sounding but real distinction — the goal was always "never
overlay/clip," and enforcing a numeric line cap via `overflow: hidden` was
in tension with that from the start once any sub-component could also wrap.

## Files changed

| File | Change |
|---|---|
| `frontend/app/view/agent/styles/_composer-strip.scss` | `.agent-composer-strip`: grid (+ 07-30's `≤480px` 2-row swap) → permanent wrapping flex, left-justified, no height cap (see §6). Removed `grid-area`/`justify-self` from all three zones. `.agent-composer-strip-controls`/`-right` gained their own `flex-wrap: wrap` (§6, Codex P1). Shed queries (auth/badge/controls-cap) moved from `300/260/220px` to `220/180/150px` to match the extra line of headroom. |
| `frontend/app/view/agent/components/AgentComposerStrip.tsx` | No structural change — updated stale comments referencing the old centered-grid design to describe the new left-justified flow. |

## Verification performed

- `npx stylelint frontend/app/view/agent/styles/_composer-strip.scss` — one
  pre-existing hex-color lint error at line ~217 (`--warning-color,
  #d9a441` fallback), unrelated to this change and present before it.
- `npx tsc --noEmit` — clean.
- `npx vitest run frontend/app/view/agent` — 72 files / 954 tests passed
  (no test exercises this component's layout directly; this is a
  no-regression check, not layout coverage).
- Two rounds of automated PR review (Codex, ReAgent) — findings applied,
  see §6.
- **Not performed:** a live `task dev` visual check of the actual wrap/shed
  behavior at real pane widths. The breakpoint values above are estimated,
  not measured — resize a real agent pane through ~250px → ~120px and
  confirm: (a) content wraps (outer and, if needed, internal) with no
  clipping or overlap with the textarea below, (b) content is always
  left-justified at every width, (c) the shed order (auth → process badge
  → controls cap) fires in that order and only as a last resort, (d)
  resizing back to full width returns cleanly to one line with no leftover
  height.
