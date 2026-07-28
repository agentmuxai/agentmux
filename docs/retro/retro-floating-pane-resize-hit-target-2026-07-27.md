# Retro: floating-pane resize hit-target shrank from 12px to 4px as a side effect

**Date:** 2026-07-27
**Severity:** Low-medium — no crash/data-loss, but a persistent UX regression
(resize handle "really hard to select") that shipped silently for a month.
**Observed by:** user, comparing current behavior to an earlier "working well,
best practice" version they remembered.

---

## TL;DR

`FLOATER_EDGE_RESIZE_BORDER` (`frontend/app/workspace/floater-resize.ts`)
drives two different things with one constant: (1) how wide the invisible
mouse hit-test band is for resizing a floating pane by its edge, and (2) how
much a floating *browser* pane's separate OS child window is inset so that
band stays clickable underneath it. It shipped at a comfortable **12px** in
PR #1177 (2026-05-29). A month later, PR #1829 (2026-06-29) fixed a real
visual complaint — browser panes showing a background-colored matte around
their web content, from the child-window inset — by shrinking the *shared*
constant, first to 6px then to 4px in the same PR. That fixed the matte for
browser panes, but also shrank the mouse target for every other floating pane
type (agent, terminal, editor, sysinfo, drone, ...) — none of which have a
matte problem at all, since they have no separate child window sitting over
the frontend DOM. Two docs (`ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md`,
`SPEC_FLOATING_PANE_EDGE_RESIZE_2026_05_29.md`) still cited the original 12px
and were never updated, so there was no documentation trail pointing at the
regression either.

## Why this shape of bug is easy to miss

The constant's own doc-comment said "nothing is painted at this width... so it
can be widened freely" — true in isolation, but the same file's next paragraph
explains the browser-pane inset shares the constant specifically *to keep the
detector and the inset from ever drifting*. That coupling is exactly what
turned a browser-pane-only cosmetic fix into an every-pane-type functional
regression: there was no way to shrink the value for one consumer without
shrinking it for the other, so PR #1829 shrank it for both, and nobody
re-evaluated whether the resulting width was still fair to grab with a mouse
on the panes that didn't have the visual problem in the first place.

## Fix

Restored `FLOATER_EDGE_RESIZE_BORDER` to **8px** — a deliberate middle point:
roughly double the unusable 4px, and comfortably under the 12px that read as
a border on browser panes. Did not split the constant into two (a
browser-only vs. general-purpose value): the browser-pane inset must
mechanically equal the hit-test width for the exposed band to actually
receive pointer events (a wider hit-test zone than the inset would just have
the CEF child window swallow clicks in the extra region), and per-pane-type
branching in the hot pointer-move path was judged more risk than the marginal
gain over a single well-chosen shared value. Updated both stale docs to match.

## Prevention

Nothing structural changed here — this is a narrow, single-constant fix. The
actionable takeaway for future changes to `FLOATER_EDGE_RESIZE_BORDER` (or any
shared constant with more than one consumer): when a change is motivated by
one consumer's complaint, check what the *other* consumers lose, not just
whether the motivating consumer's problem is solved. Grep for the constant's
usages before changing its value, not just before deleting it.

## Files

- `frontend/app/workspace/floater-resize.ts` — the constant + updated doc-comment
- `docs/architecture/ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md` — stale 12px reference
- `docs/specs/SPEC_FLOATING_PANE_EDGE_RESIZE_2026_05_29.md` — stale 12px reference
