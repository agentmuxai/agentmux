# Spec: Drop the composer strip's centered token/elapsed stats

**Date:** 2026-08-31
**Status:** Proposed
**Motivated by:** direct report — the centered stats readout that
sometimes appears in `AgentComposerStrip` (above the textarea) is
confusing because `AgentWorkingRow` ("Worked" row, below the transcript)
already shows overlapping stats, with no label distinguishing the two.

## Problem

Two independent components render a near-identical `↑in ↓out  ·  Ns`
readout, in different places in the same pane, sometimes simultaneously:

| | `AgentComposerStrip` (center, above textarea) | `AgentWorkingRow` ("Worked" row, below transcript) |
|---|---|---|
| **While a turn is loading** | `rightText()` (`AgentComposerStrip.tsx:587-598`): live `turnTokens` (`↑in ↓out`) + live elapsed, ticking every second | `rightText()` (`AgentFooter.tsx:286-296`): live `turnTokens` (`↑in ↓out`) + live elapsed, ticking every second — **the same data, the same format, at the same time** |
| **Once idle (turn complete)** | `sessionTotals`: **cumulative** tokens + duration across every turn in the pane's lifetime, since launch | `workedSummary`/`workedSecondary` (`AgentFooter.tsx:277-284`): **per-turn** tokens/duration/cost/turn-count for only the just-finished turn |

While loading, the two rows show literally the same live numbers twice.
Once idle, they show *different* numbers in the *same visual shape*
(`↑X ↓Y  ·  Ns`) with no label saying "this turn" vs. "total this
session" — a user has no way to tell which is which without already
knowing the two components' separate designs. Confirmed via
`docs/specs/SPEC_AGENT_SESSION_COST_TOTALS_2026_07_02.md`, the spec that
originally split per-turn (`sessionStats`, feeding `AgentWorkingRow`)
from cumulative (`sessionTotals`, feeding `AgentComposerStrip`) — that
split was about fixing *what number* `AgentComposerStrip` showed, not
about whether it should show one there at all.

`AgentComposerStrip`'s stats zone never shows `cost_usd` (confirmed —
`sessionTotals`/`cost_usd` has no other reference in the file beyond
feeding this one readout), so removing it loses no information
`AgentWorkingRow` doesn't already have a version of.

## Decision

Drop `AgentComposerStrip`'s centered stats display entirely.
`AgentWorkingRow` becomes the sole place a user checks for token/cost/
elapsed stats — both the live in-flight numbers and the post-turn
summary already live there, and it already puts the two side by side
with a `$cost · N turns` secondary line the composer strip never had.

## Design

### Scope: remove the content, not the layout scaffolding

`AgentComposerStrip.tsx`'s row-layout algorithm treats the stats zone as
a third, always-centered concern alongside the left/right slot rows —
this is the file's own most heavily documented, most-revised piece of
logic (7 revisions across 2 days per the module doc comment;
`docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md` and
`docs/specs/SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md`). The
scaffolding already handles "the stats zone has no content" as a normal,
existing case — `statsZone()`'s `<Show when={rightText()}>` renders
nothing when `rightText()` is empty, and `computeStatsInline` (line
435-443) degrades to `rowCount === 1` (a harmless, inert value — nothing
downstream renders regardless) once `statsWidth === 0`. **Making
`rightText()` permanently empty is therefore a safe, self-contained
change that doesn't require touching the row-fitting math** — the
opposite (ripping out `statsZone`/`statsInline`/`computeStatsInline`/the
two mount `<Show>`s) would mean editing exactly the part of this file
its own doc comments warn hardest against touching casually, for no
functional gain (dead-but-inert code, not dead-and-wrong code). That
larger cleanup is listed as an explicit non-goal below, not bundled in.

### `frontend/app/view/agent/components/AgentComposerStrip.tsx`

- Remove `sessionTotals` and `turnTokens` from `AgentComposerStripProps`
  (lines ~501-509) — grep-confirmed each has exactly one use site in
  this file, both inside `rightText()`.
- Remove `rightText` (lines 587-598), `elapsedMs` (582-585), and
  `loadStartMs` (574 + its `createEffect` at 576-581) — all three exist
  solely to feed `rightText()`, confirmed by grep (no other reference in
  the file).
- `statsZone()` (1319-1326) keeps its shape but `rightText()` no longer
  exists to call — replace its `<Show when={rightText()}>` body with
  nothing (or delete `statsZone`'s inner `<Show>` + span entirely, since
  it can now never have content — equivalent either way; prefer
  deleting so a future reader isn't left wondering what the dead `<Show>`
  was ever for).
- Leave `statsRefs`, `statsInline()`, `computeStatsInline`, the
  `ResizeObserver` measurement effect (~1031-1091), and the two
  `<Show when={!statsInline()}>`/`<Show when={statsInline()}>` mount
  points (1349, 1379) as-is — per the scope note above, these become
  permanently inert (always measuring/reserving 0 width for an
  always-empty zone) but stay correct, and touching them is the larger,
  separate cleanup called out in Non-goals.

### `frontend/app/view/agent/agent-view.tsx`

- Drop `sessionTotals={agentAtoms().sessionTotalsAtom[0]()}` from the
  `<AgentComposerStrip>` call site (~line 2395).
- Drop `turnTokens={agentAtoms().turnTokensAtom[0]()}` from the same
  call site (~line 2396) — **only this one wiring**, not the
  `turnTokensAtom` itself or its projection (~line 749): `turnTokensAtom`
  still feeds `AgentWorkingRow`'s own `turnTokens` prop elsewhere in this
  file, which stays unchanged.

### Tests

`AgentComposerStrip.test.tsx` has no test that constructs `sessionTotals`/
`turnTokens` props or asserts on `.agent-composer-strip-stats` content
(grep-confirmed — the one hit is a comment, not an assertion), so no
existing test needs updating for the removal itself. Add:

- A render test confirming `.agent-composer-strip-stats` never appears
  in the DOM regardless of `loading`/turn state (guards against the
  props being silently reintroduced later).

## Non-goals

- **No change to `AgentWorkingRow`** — it already correctly shows both
  the live in-flight readout and the post-turn per-turn summary; it's
  the surviving, single source of truth for these stats after this
  change.
- **No removal of the `sessionTotals` state plumbing**
  (`agent-pane-state/reducer.ts`'s `accumulateStats`, the `sessionTotals`
  field in `agent-pane-state/types.ts`, `sessionTotalsAtom` in
  `view/agent/state.ts`, the projection in `agent-pane-state-store.ts`).
  After this change `AgentComposerStrip` was its only consumer
  (grep-confirmed), so this state becomes fully unused — but removing a
  reducer-level accumulator is separate, larger-blast-radius surgery
  than a display change, and not needed to fix the reported confusion.
  Worth a follow-up cleanup pass, not bundled here.
- **No removal of `AgentComposerStrip`'s row-layout scaffolding**
  (`statsZone`, `statsInline`, `computeStatsInline`, the stats-width
  `ResizeObserver` effect) — see the Scope note above. A separate,
  dedicated pass if the dead weight is ever worth the risk of touching
  this file's most fragile logic again.
- **No new label/disambiguation UI** for `AgentWorkingRow`'s existing
  per-turn vs. cumulative numbers — out of scope; this spec only removes
  the duplicate, it doesn't redesign the surviving one.
