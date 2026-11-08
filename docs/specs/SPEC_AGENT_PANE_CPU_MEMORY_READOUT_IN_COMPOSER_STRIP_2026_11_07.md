# Spec: the agent pane's top band is a priority layout, and the CPU and memory readout moves from it into the composer strip

**Date:** 2026-11-07 (task #28995; builds: A first, then #28996 for B).
**Status:** active — approved by the operator 2026-11-07, all four questions as recommended (Q1 after the context reading, on the left; Q2 compact chip, never hidden; Q3 agent panes only; Q4 A before B, as two builds). A1 is built in PR #6814 (task #29008); A2 (#29009) and B (#28996) follow.
**Author:** Agent1@narko
**Scope:** `frontend/app/element/blockstats.tsx` and `blockstats.scss` (the readout), `frontend/app/block/blockframe.tsx` (where it is drawn today), the agent pane's layout (`agent-view-layout.tsx`) and find bar (`AgentSearchBar.tsx`), `frontend/app/view/agent/components/AgentComposerStrip.tsx` and `composer-row-layout.ts`, and `AgentSessionStats.tsx` (the popover the strip's context reading opens).
**Related:** `SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md`, `SPEC_COMPOSER_STRIP_DROP_CENTER_STATS_2026_08_31.md`, `SPEC_JEKT_SIDE_PANEL_RESEARCH_AND_DESIGN_2026_11_04.md` (section 10, #28993), `SPEC_AGENT_PANE_SEARCH_AND_DIM_MODE_2026_11_06.md`.

## 1. The problem

The live CPU and memory readout ("0.0% 66M", `BlockStatsBadge`) is drawn by the block frame, absolutely positioned at the pane's top right corner (`top: 4px; right: 6px`) over the pane's content, at a z-index above the agent pane's own view. Everything that wants the top right of an agent pane lands under it: the find bar's previous, next and close buttons (Maricon and Clare both saw "0.0%▲▼ 66M ✕" overprinted in a 595 px pane, #6759 and #6779) and the Messages panel's close button (#28993). Each fix so far moved the other control around the readout. Nothing owns the band, so every control that arrives there collides at some width.

**Before** (a 595 px pane with the find bar open; the readout is drawn over the corner by the block frame, above everything the pane draws):

```
+-- pane header ------------------------------------------------------+
| bruno                                          [grip] [find] _ [] x |
+---------------------------------------------------------------------+
| [ Find in conversation ______ ]  10 of 10      0.0%^v 66M x         |
|                                                ^^^^^^^^^^^^         |
|   find bar's  ^  v  x  are under the readout, which the block frame |
|   draws at top: 4px; right: 6px over everything in the pane          |
| ... transcript ...                                                  |
+---------------------------------------------------------------------+
```

## 2. Part A: the top band is a priority layout (groundwork)

### 2.1 The band

The band is the row across the top of the agent pane's content, under the pane's own header and above the transcript. It is outside the transcript's scroll, one line high, and exists only while it holds something (today: the find bar is open, or the readout is shown). Its slots, left to right, are fixed:

| Slot | Holds | Today |
|---|---|---|
| `identity` | what the pane is, when the pane's header does not already say it | empty |
| `stats` | the live readout, reserved so nothing is drawn over it | the CPU and memory readout (Part A moves it in; Part B moves it out) |
| `controls` | transient controls, on the right: the find bar, a panel toggle, any control added later | the find bar |

The band, in its **full form** (wide enough for everything; the stats sit left of the controls and nothing overlaps):

```
+-- pane header ------------------------------------------------------+
| bruno                                          [grip] [find] _ [] x |
+-- top band --------------------------------------------------------+
| [identity]  [ 0.0% 66M ]  ..........  [ Find ____ 10 of 10  <  >  x ] |
|  identity     stats       (free space)        controls (highest priority)
+---------------------------------------------------------------------+
| ... transcript ...                                                  |
```

The band when it is **tight** (the rule of 2.2 applied: the lowest-priority item with a compact form went compact first; the controls keep their full size):

```
+-- top band (narrow pane) ------------------------+
| [identity] [o 0.0%]  [ Find ____ 3 of 7  <  >  x ] |
|             ^ compact chip: a processor icon and the CPU figure;
|               hover or focus shows "CPU 0.0%, memory 66 MB, 3 processes"
+--------------------------------------------------+
```

### 2.2 The collapse rule

One rule governs every item, today's and any added later. An item registers with the band (`registerTopBandItem`) with its slot, a priority (a number: higher keeps its full form longer), and two forms: full and compact (an icon or a chip, with the full reading as its tooltip, focusable). The band's layout is a pure function of the band's measured width and each item's measured full and compact widths (the strip's `computeComposerRows` is the model: real widths, not guessed breakpoints):

1. Everything in its full form, if it fits.
2. If not, the lowest-priority item that has a compact form goes compact, then the next lowest, until it fits. Controls have the highest priority: they are the target, so they stay full size.
3. If everything that can be compact is compact and it still does not fit, the lowest-priority item with no compact form is dropped from the band (never a control), and the layout says so, so the owner can put it somewhere else.
4. No item is ever drawn over another: the layout gives each a box, in order, and the band clips nothing.

A compact item keeps its function: it shows its full reading on hover and on focus, and a click does what the full form's click does (the readout's chip opens its detail; the session popover stays on the strip's context reading, section 3.4).

### 2.3 What changes for the pieces

- **The readout** becomes the `stats` item: full ("0.0% 66M") at normal widths, compact (a processor icon and the CPU figure, tinted by the same colour rule) when the band is tight. `BlockStatsBadge` gets a placement (`overlay` for the panes that keep it where it is, `band` for agent panes) and a compact form; the block frame leaves its own readout out for a pane with the capability `statsBadgeIn` (`pane-tab-registry.ts`, next to `statsBadgeSetting`).
- **The find bar** becomes the `controls` item and stops being a sticky row inside the transcript. At a width where the readout is compact and the bar still does not fit, the readout is already a chip; the bar keeps its field, count and buttons.
- **The Messages panel** (#28993) is an overlay along the pane's right edge, over the conversation and the band. Its close button is at its own top right corner; it needs no room made for the readout because the readout is in the band's `stats` slot, left of the controls, never under the panel's corner (the panel covers the band while it is open, as it covers the conversation).
- **Anything added later** registers an item and gets the rule with no layout code of its own.

## 3. Part B: the readout moves into the composer strip (on top of A)

The strip is the pane's status row (the runtime trigger, the context reading, the account tag, the view, NATIVE or CONTAINER and Shell). The readout is a status, so it belongs there, and the band's `stats` slot can then collapse to zero.

### 3.1 Where, in both strip variants

One more slot of the strip, key `stats`, on the left, **directly after the context reading** (`ctx`): `[runtime] [104k / 200k] [0.0% 66M] … [auth] [view] [NATIVE] [Shell]`. It is a slot, not a revival of the emptied centre stats zone (`composer-stats-zone.tsx`), so the strip's row layout places and measures it like the others.

- **Single row** (everything fits): the three left slots in that order.
- **Several rows** (the strip is narrow): the row layout already moves slots down by priority. `stats` has the lowest priority of the left slots, so it is the first to leave the first row and sits at the start of the second, still on the left. The runtime trigger and the context reading keep the first row.

The readout's look is unchanged (tabular figures, the warning and error colours at medium and high CPU, the italic "host" form for a remote pane). Only its mounting changes: an inline element of the strip, the `inline` placement of `BlockStatsBadge`.

The strip **after Part B**, in one row (everything fits):

```
+-- composer strip ---------------------------------------------------+
| [@ runtime v] [104k / 200k] [0.0% 66M] ......... [auth] [view] [NATIVE] [Shell] |
|    runtime       ctx         stats (new)           the right side as today
+---------------------------------------------------------------------+
| Send message to bruno...                                          mic |
```

The strip in a **narrow pane** (several rows; `stats` is the first slot to leave the first row, and it is the chip, never hidden):

```
+-- composer strip (narrow) ------------------+
| [@ runtime v] [104k / 200k]    [auth] [Shell] |
| [o 0.0%]                        [view] [NATIVE] |
+--------------------------------------------+
| Send message to bruno...                 mic |
```

The band afterwards (Part B done: the band's `stats` slot is empty and takes no width; find, and whatever is added later, own the band):

```
+-- top band --------------------------------------------------------+
| [identity]  ........................  [ Find ____ 10 of 10  <  >  x ] |
+---------------------------------------------------------------------+
```


### 3.2 Narrow widths in the strip

The strip is tight in a narrow pane too, so the move must not carry the collision into it. The slot has the same two forms as in A (the compact chip is the same component): full at normal widths; compact when the strip has more than one row or its measured width is below a threshold (the strip's own `ResizeObserver` width; 360 px unless the build finds a better number). The full reading ("CPU 0.0%, memory 66 MB, 3 processes") is the chip's tooltip, which is the readout's existing tooltip. The row layout may drop the chip to the last row; it is never hidden, because the readout is the thing that shows a pane running away. The slot's width in both forms is measured like every slot, so the strip never overflows.

### 3.3 The band afterwards

For an agent pane the capability changes from `statsBadgeIn: "band"` to `"strip"`: the band's `stats` slot is empty and collapses to zero, the frame draws nothing at the corner, and the band holds the find bar (and whatever is added later) alone. The Part A rule stays: it is what keeps the next control from colliding, with the readout or without it.

### 3.4 The stats popover stays reachable

`AgentSessionStats` (cost, turns, duration, tokens, Archive, Export, Restore) still opens from the context reading, which does not move. The readout is not a second way to it. Its own detail (the processes and their figures) stays the hover tooltip it is today on both forms; the chip is focusable and shows the same detail on focus, so it is not hover-only.

## 4. Build slices

| Slice | Contents | Notes |
|---|---|---|
| A1 | The band (`agent-top-band`): the three slots, `registerTopBandItem`, the pure layout function and its tests (fits, compacts lowest priority first, never a control, drops the last resort, never overlaps, a priority tie keeps registration order), the find bar moved into it | Visual: macOS and Linux look checks. The long-transcript search tests (PR 1) must still pass: the bar leaves the sticky position in the transcript |
| A2 | The readout as the band's `stats` item: the `band` placement and the compact chip of `BlockStatsBadge`, the capability, the frame leaving its overlay out for an agent pane | Visual. Fixes the overlap at its root for find and for the Messages panel |
| B (#28996) | The strip's `stats` slot after `ctx`, its compact form, the capability moving to `"strip"`, the band's `stats` slot collapsing to zero | Visual. Reuses A2's component; only the mounting changes |

Each slice has its own tests (the slot's place in single and multi-row strips, the compact form below the threshold and with several rows, the frame leaving its overlay out for an agent pane and keeping it for a terminal, the tooltip text on both forms, the band and strip never wider than their container) and a look check on macOS and Linux at 100%, 125% and 150% chrome zoom, dark and light, in a wide, a medium and a narrow pane, with the find bar and the Messages panel open.

### 4.1 A1 as built (PR #6814)

The band is `frontend/app/view/agent/top-band/`: `top-band-layout.ts` (the pure rule of 2.2, with its tests), `top-band-registry.ts` (`registerTopBandItem`, the slots `identity`, `stats` and `controls`, the priorities) and `AgentTopBand.tsx` (it measures a hidden, inert copy of each form of each item with `getBoundingClientRect()`, and draws each item in a box of its own). Where this differs from section 2:

- The band sits **above the row that holds the conversation and the Messages panel**, as a direct child of the pane's view, not inside the conversation's column. It follows the pane's zoom itself. So the Messages panel, which lies over the conversation, starts under the band and never covers it: the band's find bar and (from A2) the readout stay visible and reachable while the panel is open, and the panel's close button is no longer near the readout. Section 2.3's "the panel covers the band while it is open" is therefore not what is built.
- The find bar is the `find` item of the `controls` slot (`AgentFindBandItem`), registered while the search is open. It left its sticky place in the conversation's column; the band is its ground. Its measuring copy has the field at its narrowest (96 px), so the bar is not asked for more room than it needs.
- A tie in priority keeps the earlier registered item in its full form longer.
- The z-index token `--z-pane-search-bar` is gone with the sticky bar.

## 5. Questions for the operator

_Answered 2026-11-07, all as recommended; kept for the record._


| # | Question | Recommendation |
|---|---|---|
| Q1 | In the strip the readout goes after the context reading, on the left (3.1), not on the right with NATIVE and Shell | After the context reading: both say "how much is this pane using", and the right side is "what runs it and the one action" |
| Q2 | When tight it becomes a chip with the CPU figure and the full reading as its tooltip, never hidden (2.2, 3.2) | Yes. Hiding it would remove the only sign a pane is running away |
| Q3 | Agent panes only; terminals and the other panes keep the overlay readout (2.3) | Yes. Their top band is not shared with find and panel controls the way the agent pane's is |
| Q4 | A before B, as two builds, or both in one | Two: A is the framework the next control will use, and B is a one-slot move on top of it |
