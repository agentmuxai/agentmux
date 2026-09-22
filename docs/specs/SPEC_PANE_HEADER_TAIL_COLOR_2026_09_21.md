# SPEC: the pane header's tail color follows the pane, not the active tab

**Date:** 2026-09-21
**Status:** implemented in #3492 — see §6 for what shipped and how it was verified
**Author:** AgentO
**Related:**
`docs/specs/SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md` (established
the one header-background rule this refines),
`docs/analysis/ANALYSIS_PANE_TAB_COLOR_COLLAPSE_2026_09_21.md` (the
root-cause analysis for the sibling bug — uncolored *pills* showing the
header tint through — fixed in `agento/pane-tab-color-consolidation`),
`docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md` (the
redesign that made a multi-color pane the normal case),
`docs/specs/SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md`

---

## 1. Problem

A pane's header row is one element (`.block-frame-default-header`,
`blockframe.tsx`), painted by `headerStyle` with the **active** block's
pane color. The tab strip is injected into that row as
`props.leadingTabStrip`, so everything the pills don't cover — the "+"
button, the gap after the last pill, and the run of bar out to the
window/end-icon buttons — is that active tab's color. Call that leftover
region the **tail**.

That was fine when a pane held one widget, or a stack of forks of the
same agent, which shared a color. The universal pane-tabs redesign made
the ordinary pane a mixed bag: an agent tab, a terminal tab, a browser
tab, each with its own identity color. In that pane the tail is now a
large block of color that **changes every time you switch tabs** — click
Swarm and the whole bar goes teal, click Terminal 1 and it goes purple.
The pane's chrome reads as if it belongs to whichever tab happens to be
selected, rather than to the pane.

Note this is a *different* defect from the one fixed on
`agento/pane-tab-color-consolidation` (commit `4b0ad907f`). That one was
about **pills**: an uncolored pill was transparent and let the header
tint show *through it*, so uncolored tabs appeared to take on the active
tab's color. This one is about the **tail**: even with every pill now
painting its own opaque background correctly, the region around them
still tracks the active tab. Both have the same underlying cause — one
row, one background, sourced from one block — but they need separate
fixes, and the pill fix must land first (it already has).

## 2. Rule

Let the pane's **tab color set** be the effective pane color of every tab
in the pane (§3.1 defines "effective"), in tab order.

| Pane state | Tail color |
|---|---|
| Exactly one tab | That tab's effective color |
| Two or more tabs, all the same effective color | That shared color |
| Two or more tabs, two or more distinct effective colors | **App default** — fixed, independent of which tab is active |

Every pill keeps its own color. The **active** pill needed one follow-up
change to make that true — see §2.2.

The "app default" is **one value per theme, for every pane**
(`computeMixedPaneHeaderBg`): `NON_AGENT_DEFAULT_HEADER_BG`
(`hsl(220, 12%, 16%)`) on a dark theme, `var(--block-bg-solid-color)` on
a light one.

It deliberately does **not** reuse `computeBlockTabPillNeutralBg`, which
resolves to two different dark-theme colors depending on `meta.view`
(agent vs not). That split is right for a *pill* — a pill belongs to one
block. It is wrong for the header, which belongs to the whole pane: the
first cut passed it the ACTIVE block's meta, so in a mixed
agent-plus-terminal pane (this spec's own motivating example) the tail
changed color when you switched between them — reintroducing, inside the
neutral branch, the exact bug this spec exists to remove (reagent P1, PR
#3492). A mixed pane has no single `view` to consult, so there must be no
`view` in the input at all.

When the tabs **do** agree, the header gets no override whatsoever and
`BlockFrame_Header` keeps doing exactly what it always did — see §3.2.

### 2.1 Why this rule

A pane with one color is *about* that color — the tail reinforcing it is
information. A pane with several colors has no single color to be about,
so a tail that picks one is not information, it's noise that moves. The
rule degrades to today's exact behavior for every single-tab pane, which
is still the majority of panes, so nothing regresses for a user who
doesn't mix widget types in a pane.

### 2.2 The active pill must paint its own color

This spec originally asserted the pills needed no change at all, and that
the active pill would keep its plain `var(--block-bg-color)` surface
(`PaneTabStrip.scss`'s `&--active`). **That was wrong**, and shipping it
produced the exact inversion of the intended effect, reported as *"the
selected tab is assuming the color of the tail"*:

- Every **inactive** colored pill painted its own hue (`--pane-tab-bg`).
- The **active** pill had its background overridden unconditionally to the
  plain content surface — so the one tab you'd selected was the one tab
  showing no color.

That override was invisible before this spec only because the header row
*behind* the strip was itself painted with the active tab's color, so the
row still expressed it. Neutralizing the tail removed the thing that was
compensating.

So `&--active` now resolves `var(--pane-tab-bg, var(--block-bg-color))`:
its own color when it has one, the plain surface when it doesn't. It
needs a `&--active:hover` restatement to survive: the base `&:hover`
rule is `.pane-tab:hover`, specificity (0,2,0) against `&--active`'s
(0,1,0), so it otherwise wins on the active tab and repaints it with the
generic tint over `--pane-tab-neutral-bg` — throwing the color straight
back away (reagent P1, PR #3492). That restatement carries no hover tint,
for the same reason `&--colored:hover:not(&--active)` skips the active
tab: hover says "you can select this", and this one already is. The
original "the active tab shows the content surface, connecting to the
pane body below" intent is still served — by the underline immediately
below it, which was always the explicit signal for that connection.

Every non-PaneChrome consumer (editor file tabs, the agent History strip)
never sets `--pane-tab-bg` at all, so the fallback keeps their rendering
byte-for-byte identical.

## 3. Implementation

### 3.1 Effective color of a tab

Use the value `computeBlockColorBg(meta, isLightTheme)` already returns
for that block (`blockframe.tsx` — `frame:hue` from the "Pane Color"
picker, else the agent's persisted `frame:activebordercolor`, with the
dark-theme darkening applied). Compare the resolved **strings**, not the
inputs: two blocks that land on the same rendered background are the
same color for this rule even if one got there via `frame:hue` and the
other via its identity color.

A tab with no color of its own resolves to `undefined` from
`computeBlockColorBg`. Treat `undefined` as its own distinct member of
the set, **not** as a wildcard that matches anything:

- One colored tab + one uncolored tab = two distinct colors → neutral
  tail. Correct: the colored tab's color is not the pane's color.
- Two uncolored tabs = one distinct value (`undefined`) → tail is the
  app default, which is also what `undefined` renders as. Same result
  either way; no special case needed.

### 3.2 Where it's computed

`PaneChrome.tsx`'s `renderPaneChromeShell` already has exactly the two
things this needs and `blockframe.tsx` has neither: the full `tabIds()`
list for the pane, and the theme polarity. Add a `headerTailBg` memo
there, next to the existing `tabColors` memo (§ `PaneChrome.tsx:134`),
which walks `tabIds()`, collects the distinct `computeBlockColorBg`
results, and returns:

- `undefined` when the set has 0 or 1 members — the tabs agree, so hand
  back nothing and let `BlockFrame_Header` run its existing logic
  untouched. That covers "they all share a color" (the active block
  resolves to that same color anyway) *and* "none of them has one" (an
  uncolored agent pane keeps its untouched default rather than being
  forced to a value this memo chose). It is also what makes "single-tab
  panes are byte-identical" true by construction rather than by matching
  a value;
- otherwise `computeMixedPaneHeaderBg(isLightTheme)` — no block meta in
  the input, per §2.

Pass it to `BlockFrame` as a new optional prop on `BlockFrameProps`
(`blocktypes.ts`, beside `leadingTabStrip` — the prop is only ever set by
the same caller that sets that one):

```ts
/** Explicit header-row background, overriding the active block's own
 *  pane color. Set by PaneChrome when the pane's tabs don't agree on a
 *  single color — see SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md. */
headerBgOverride?: string;
```

`headerStyle` (`blockframe.tsx:628`) takes it as the first branch:

```ts
const bg = props.headerBgOverride ?? computeBlockColorBg(blockData()?.meta, isLightTheme);
```

and keeps its existing `pickReadableTextColor(bg)` call, so header text
and icons stay legible against the neutral tail without a second rule.

Doing it this way keeps `blockframe.tsx` ignorant of the pane's tab list
— it renders one block and is handed a color — and keeps every
tab-set-aware decision in the one component that already owns the tab
set. It also means every non-PaneChrome `BlockFrame` consumer is
untouched by construction.

### 3.3 What must NOT change

- **Each pill's own color.** No change to `computeBlockTabPillBg`,
  `computeBlockTabPillNeutralBg`, `--pane-tab-bg`, `--pane-tab-underline`,
  or `--pane-tab-neutral-bg`. A neutral tail specifically does *not*
  neutralize the pills — the whole point is that the colors stay
  visible on the tabs that own them. (Originally this clause also claimed
  `&--active`'s background needed no change. It did — §2.2.)
- **The active tab's underline.** `--pane-tab-underline` still carries
  that block's vivid color, and remains the signal that connects the
  active tab to the content below it.
- **Single-tab panes.** Byte-identical rendering to today.
- **Every non-PaneChrome tab strip.** The editor's file tabs and the
  agent History strip never set `--pane-tab-bg`, `--pane-tab-neutral-bg`
  or `headerBgOverride`, so every fallback in this change resolves to
  exactly what they render today.

## 4. Tests

In `PaneChrome.test.tsx`, against the existing `fakeLayoutModel` /
`setObjectValue` harness (the `renderPaneChromeShell — per-tab pane
color` describe block already mocks `blockframe`'s compute functions,
so add `computeBlockColorBg` to that mock):

1. One tab, colored → `headerBgOverride` is that tab's color.
2. One tab, uncolored → `headerBgOverride` is the neutral.
3. Two tabs, same color → that color, and **the same value regardless of
   which of the two is active** (assert across both `activeBlockId`
   values — this is the regression the spec exists to prevent).
4. Two tabs, different colors → the neutral, again asserted for both
   active-tab values.
5. One colored + one uncolored tab → the neutral (§3.1's
   `undefined`-is-distinct rule).
6. Three tabs, two sharing a color and one differing → the neutral.

In `blockframe`'s own tests: `headerBgOverride` wins over the block's
`frame:hue`, and its absence leaves today's behavior untouched.

## 5. Verification plan

Unit tests above, plus a live check against the running dev instance over
CDP (`localhost:9223` — see the `dev-instance-cdp` memory note): open a
pane with an agent tab, a terminal tab, and a Help tab, screenshot with
each tab active in turn, and confirm the tail's computed background is
byte-identical across all three while each pill keeps its own. Then
repeat on a two-tab pane whose tabs share a color and confirm the tail
*does* carry it.

## 6. What shipped

Implemented as specified, with one plumbing detail §3.2 didn't spell out:
`PaneChrome` doesn't render `BlockFrame_Header` directly — it goes through
`PaneHeaderTabStrip`, which is a thin wrapper around it. So
`headerBgOverride` is declared on `PaneHeaderTabStripProps` as well and
passed straight through; that component adds no policy of its own.

- `PaneChrome.tsx` — `headerTailBg` memo beside `tabColors`. Short-circuits
  (`if (distinct.size > 1) break`) as soon as two colors are seen, so a
  pane with many tabs doesn't walk them all to learn what the second tab
  already proved.
- `PaneHeaderTabStrip.tsx` / `blocktypes.ts` — `headerBgOverride` prop.
- `blockframe.tsx` — `headerStyle` takes the override first, keeping its
  existing `pickReadableTextColor(bg)` call, so header text and icons stay
  legible against the neutral tail with no second rule.
- `PaneTabStrip.scss` — `&--active` resolves
  `var(--pane-tab-bg, var(--block-bg-color))` (§2.2). Follow-up to the
  above, not part of the original plan: neutralizing the tail exposed an
  existing unconditional override that left the *selected* tab as the only
  colorless one.

### Verification

8 unit tests in `PaneChrome.test.tsx` (`— header tail color`), covering
§4's six cases plus reactivity. Each multi-tab case asserts the tail across
**every** tab being active in turn, not one — the rule is specifically that
the value doesn't depend on which tab is selected, so a single-active-tab
assertion would pass against the bug. `computeBlockColorBg` is mocked
meta-driven for the same reason the sibling color mocks already are: a
fixed stub makes every pane look single-colored and passes vacuously.

Verified live against the running dev instance over CDP, on a light theme:

- A pane with 5 tabs and 4 distinct colors: clicking each tab in turn left
  the header at `rgb(255, 255, 255)` all five times, while each pill kept
  its own `--pane-tab-bg` (teal / orange / magenta / none / none). Before
  this change the header took the active tab's color on every switch.
- Swept all 4 open panes and asserted the rule directly — a 1-tab
  uncolored pane and a 2-tab two-color pane both render the default; the
  1-tab colored pane still renders its own color (`rgb(212, 133, 53)`).

Re-verified after §2.2's active-pill fix, this time on a dark theme, with
both rules asserted in the same sweep of the 5-tab / 4-color pane:

| Selected | Its own color | Selected pill renders | Tail |
|---|---|---|---|
| Swarm | `hsl(180, 42%, 24%)` | `rgb(35, 87, 87)` | `rgb(36, 39, 46)` |
| CPU | `hsl(30, 42%, 24%)` | `rgb(87, 61, 35)` | `rgb(36, 39, 46)` |
| Terminal 1 | `hsl(300, 42%, 24%)` | `rgb(87, 35, 87)` | `rgb(36, 39, 46)` |
| Terminal 2 | none | `rgba(4, 6, 14, 0.7)` | `rgb(36, 39, 46)` |
| Help | none | `rgba(4, 6, 14, 0.7)` | `rgb(36, 39, 46)` |

The selected pill now carries its own color on every switch, the tail
never moves, and an uncolored selected tab still falls back to the plain
content surface — distinct from the tail, so it still reads as selected.

The JS-side precondition (the active tab keeps its inline
`--pane-tab-bg` and `--colored` class, which the SCSS fallback depends
on) is pinned by a unit test in `PaneTabStrip.test.tsx`; jsdom resolves
no stylesheet, so the computed background itself can only be checked
live.

## 7. Out of scope

- Whether the neutral should instead be a blend or an average of the
  tabs' colors. Rejected up front: a blend is a new color the user never
  picked, and it still moves when a tab is added or closed.
- Any change to how a pane color is *assigned* (the "Pane Color" picker,
  agent identity colors).
- The window tab bar's own colors — a different strip with its own rules.
