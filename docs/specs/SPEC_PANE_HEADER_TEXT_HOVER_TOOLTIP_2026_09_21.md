# SPEC: pane header informational text becomes a hover tooltip

**Date:** 2026-09-21
**Status:** implemented in #3488 — see §5 for what shipped
**Author:** AgentO
**Related:** `docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md`
(the redesign whose side effect this fixes), `docs/specs/PLAN_PANE_TABS_UNIVERSAL_IMPLEMENTATION_2026_09_17.md`,
`frontend/app/element/PaneHeaderTabStrip.tsx` (its own doc comment already
states header text was left untouched by that redesign)

---

## 1. Problem

Before the Pane Tabs redesign, `BlockFrame_Header`'s informational text
(`ViewModel.viewText()`, rendered via `.block-frame-textelems-wrapper`)
had most of the header row to itself. That redesign gave every pane a
`PaneHeaderTabStrip` pill strip by default (`pane:tabstrip` setting
default: `"always"`), which now consumes most of the row's width.
`.block-frame-textelems-wrapper` is still a real flex child, positioned
*after* the tab strip and *before* the end-icon buttons — so it hasn't
moved, but the space actually left for it has shrunk to a thin sliver
against the right edge. Practically, this squeezes and truncates the
text rather than showing it, which reads as text that used to "take up
the entire header" now "floating to the right."

Neither the redesign spec nor its implementation plan anticipated this —
`PaneHeaderTabStrip.tsx`'s own doc comment explicitly lists "header text
elems" among the row elements left untouched, and the redesign spec's
non-goals section excludes "full visual-design polish" without mentioning
header text specifically.

## 2. What actually renders this text today

`BlockFrame_Header`'s `headerTextUnion` memo (`blockframe.tsx`) reads
`block.meta["frame:text"]` first, falling back to
`viewModel.viewText()`. Checked both current producers:

- **Terminal** (`termViewModel.ts`): for a `cmd`-controller terminal, the
  command line plus a restart-spinner or exit-status icon/text; for a
  plain shell, `term:osc_title` block meta (a CLI's own OSC window-title
  — e.g. a `user@host:~` shell prompt title, or an agent CLI setting its
  own title) as a `term-activity`-classed text element. Also, when
  Multi-Input mode is on, a "Multi Input ON" text-button.
- **Agent** (`agent-model.ts`): `viewText` already returns `[]` — emptied
  2026-09-20 as part of the header/border pane-color unification pass
  (its own comment cites that change). No `frame:text` writer exists
  anywhere in the frontend or backend either. **Agent panes render zero
  header text today.** If "the agent's text" was seen floating right, the
  most likely explanation is a terminal-view pane running an agent CLI
  (its OSC-title text, described above), not a dedicated Agent-view pane.

Either way, the fix below covers whatever `headerTextUnion` produces
generically — it doesn't special-case terminal vs. agent, so it also
covers `frame:text` or any future `viewText()` producer without further
changes.

## 3. Fix

`BlockFrame_Header` no longer renders `headerTextElems()` inline in
`.block-frame-textelems-wrapper` (that wrapper still exists, but now only
ever holds the error indicator — see §4). Instead, hovering the header row
shows the exact same content in a floating tooltip, positioned below the
header via `floating-ui`, using the same visual style
(`bg-modalbg border border-border rounded-md px-2 py-1 text-xs
text-foreground shadow-xl z-50`) as the shared `Tooltip` component
(`element/tooltip.tsx`).

A **new**, small `AnchoredTooltip` component (`blockframe.tsx`) does this,
rather than reusing `Tooltip` directly: `Tooltip` always wraps its
children in a brand-new `<div>` to own the hover listeners, but
`BlockFrame_Header`'s own root div already carries several existing
handlers and a conditional ref assignment (`onContextMenu`, `onDblClick`,
`dragHandleRef`, `data-role`/`data-testid`). Wrapping that div in another
one just to reuse `Tooltip` would have meant restructuring a large,
heavily-commented, already-fragile component for no real benefit.
`AnchoredTooltip` instead takes an existing ref accessor directly and
reuses `Tooltip`'s exact positioning approach (`computePosition`/
`autoUpdate`/`offset`/`flip`/`shift`) and CSS classes.

**Scoping decision**: the hover trigger is the header row itself
(`.block-frame-default-header`'s own `onMouseEnter`/`onMouseLeave`), not
the whole pane's content area. The request said "hovering over the
corresponding pane," which the header row satisfies directly (it's part
of, and immediately adjacent to, all of that pane's other content) at
much lower implementation risk: `BlockFrame_Header` renders in two
structurally different places depending on whether a pane's chrome is
hoisted (`PaneChrome.tsx`'s own tree) or not (`BlockFrame_Default_Component`'s
own tree, `blockframe.tsx`) — there's no single "the whole pane" wrapper
common to both today. Extending the hover region to the full pane body
would require touching both trees; deferred as a follow-up if it turns
out the header row alone isn't the natural gesture users reach for.

## 3a. Only *passive* content moves — controls stay on the row

The first cut of this change moved **all** of `headerTextUnion()` into the
tooltip. That was wrong: `viewText()` is not uniformly informational. The
same array also carries real controls, and two things broke (reagent P1 on
PR #3488, raised twice — on open and again on re-review):

1. **A control became unreachable.** `termViewModel`'s "Multi Input ON"
   textbutton (`title: "…click to disable"`, `onClick:
   setIsTermMultiInput(false)`) now existed only inside the tooltip. The
   tooltip is anchored `placement: "bottom"` with `offset(6)` via a
   `Portal`, and closes on the header's own `onMouseLeave` with no delay
   and no hover-bridging — so a pointer travelling from the header toward
   the button leaves the header's bounding box, and dismisses the button,
   before reaching it. Not clickable by mouse at all.
2. **A persistent mode warning got hover-gated.** "Multi Input ON" is
   telling the user their keystrokes are going to *every* connected
   terminal. That has to be visible without being asked for.

Hover-bridging the tooltip (a close delay plus listeners on the floated
content) would have fixed (1) and not (2). The fix is instead to
**partition** the elements — `frontend/app/block/header-elems.ts`:

- `isInteractiveHeaderElem(elem)` — true for anything the user can act on.
  `input`/`toggleiconbutton`/`connectionbutton`/`menubutton` always;
  `iconbutton` only with a real `click` and neither `noAction` nor
  `disabled` (so termViewModel's exit-code glyph stays passive);
  `textbutton` and `text` only with an `onClick`; `div` if it handles
  anything itself or contains anything that does, recursively.
- `partitionHeaderElems(elems)` — `{ inline, tooltip }`, order preserved
  within each. A `div` is never split across the two: its children are
  laid out by the div itself, so one interactive descendant pins the whole
  div inline.

`inline` renders in `.block-frame-textelems-wrapper` exactly as everything
did before this spec; only `tooltip` goes to `AnchoredTooltip`. An unknown
future `elemtype` defaults to passive — a new *control* adds its case,
whereas defaulting the other way would silently pin every new
informational element to the row and re-create the width problem this
spec exists to fix.

This keeps the original goal intact: the squeeze was caused by long
command lines and OSC titles, all of which are passive. The controls that
stay are short and few.

The helper lives in its own import-free module rather than in
`blockframe.tsx` specifically so it is unit-testable — see §5.

## 4. Side effect: `hasSummary`'s meaning narrowed

`hasSummary` (drives the `--has-summary` CSS class, which caps the pane
name's width at 60% on agent panes to make room for text alongside it)
used to be true whenever there was an error **or** non-empty header text.
Since passive text no longer takes any inline row space, squeezing the
name region for it made no sense anymore — `hasSummary` now reflects
`props.error != null` **or** a non-empty `inline` partition (§3a), i.e.
exactly the things still rendered inline in
`.block-frame-textelems-wrapper`.

## 5. What shipped / verification

- `AnchoredTooltip` component and its wiring into `BlockFrame_Header`
  (`frontend/app/block/blockframe.tsx`).
- `frontend/app/block/header-elems.ts` — the interactive/passive partition
  (§3a), with 13 unit tests in `header-elems.test.ts` covering every
  `elemtype`, the `noAction`/`disabled`/missing-handler cases, nested
  divs, and order preservation. Being an import-free pure module, it
  needs no mocking, which is what makes the part of this change that
  actually regressed testable at all.
- `hasSummary` widened back to cover inline controls (§4).
- No changes to `termViewModel.ts`/`agent-model.ts` — this only changes
  *where* `viewText()`'s output renders, not what any view type produces.

Verified live against the running dev instance via its CDP debug port
(`Runtime.evaluate`/`Page.captureScreenshot`, the same technique used in
`docs/analysis/ANALYSIS_PANE_TAB_COLOR_COLLAPSE_2026_09_21.md`):
- `.block-frame-textelems-wrapper`'s `textContent` is empty across every
  pane after the change (previously held the command/OSC-title text).
- Dispatching `mouseenter` on a terminal pane's header (`[data-role="block-header"]`)
  produces a `[data-pane-overlay]` tooltip whose text matches that pane's
  previously-inline text exactly (confirmed with a real OSC-title-derived
  `asafebgi@starpower:~` shell prompt).
- Dispatching `mouseleave` removes it.

Not covered by an automated test: `BlockFrame_Header` has no existing
unit-test file (it's a very large, heavily-mocked-dependency component;
no test scaffolding for it exists anywhere in this codebase today), and
`floating-ui` positioning plus real hover timing are exactly the class of
interaction this codebase's own comments already note as impractical to
verify in jsdom (see `PaneTabStrip.tsx`'s own comment on drag-gesture
testing for the same reasoning). Verified live instead, per above.
