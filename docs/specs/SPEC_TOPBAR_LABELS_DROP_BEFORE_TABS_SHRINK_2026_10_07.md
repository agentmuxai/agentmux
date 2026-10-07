# Top bar: widget labels drop before tabs shrink

**Status:** active — the tier decision (`frontend/app/window/top-bar-tier.ts`) and the measuring hook shipped in #4442
and were checked live on macOS by the owner; the 1/3/8-tab CDP sweep and the Windows/Linux check (§4) remain.
**Date:** 2026-10-07.
**Requested by:** repo owner (asafebgi): "when widgets automatically switch from text label to icon-only because of
size, the trigger should be when the widgets intersect an unreduced-sized tab. Currently the switch to icon only only
happens once the tab gets as thin as possible. Instead, it should happen as soon as the widget touches the unshrunken
tabs. Tabs should start shrinking only when the widgets already changed to icon only."
**Author:** AgentO.
**Amends:** `SPEC_TOPBAR_PROGRESSIVE_COLLAPSE_2026_06_05.md` (the tier 1→2 trigger, and the tier 2→3 trigger for
consistency). Tier meanings are unchanged.
**Related:** `SPEC_TAB_CONTENT_AWARE_SIZING_2026-06-14.md` (each tab's natural width).

Line numbers are against `main` at `22857409f`.

---

## 1. What the owner sees

As the window narrows, the order today is:

1. The tabs shrink, from their natural width (about 232 px each) down to about 100 px.
2. Only then do the widget labels drop (Agent, Swarm, Memory, … become icons).
3. The tabs keep shrinking toward their 60 px floor.
4. Widgets overflow into `…more`.

The owner wants steps 1 and 2 the other way round: **labels drop the moment the labeled widgets would touch a tab at
its natural width; tabs only start shrinking once the widgets are already icon-only.**

## 2. Why it happens

`frontend/app/window/use-widget-bar-responsive.ts:20-27, 64-76` decides tier 1→2 from a **fixed reserve per tab**,
not from the tabs' real width:

```ts
const tabsNeeded = Math.max(MIN_TAB_WIDTH /*120*/, tabCount * TAB_COLLAPSE_RESERVE_PX /*100*/);
setTooNarrow(labeledW + buttonsW + tabsNeeded > headerW);
```

A tab's real resting width is its `--tab-natural-width`: its measured label width, floored at `TAB_STANDARD_WIDTH` =
232 px and capped at 260 px (`frontend/app/tab/tab-measure.ts:23-53`, `tabbar.scss:61-73`). Tabs are
`flex: 0 1 var(--tab-natural-width)`, so they start shrinking as soon as the tab bar is narrower than their natural
total. The label trigger waits until each tab would be under 100 px. Between those two points (about 132 px per tab)
the tabs shrink while the labels are still shown. The comment at line 22-24 says this was deliberate ("labels drop
first, then tabs continue shrinking") but the 100 px reserve is far below the 232 px natural width, so in practice tabs
shrink first.

The calculation also leaves out most of the header: it subtracts only `.window-action-buttons`, not the macOS
traffic-light spacer, the left drag strip, the hamburger (in the tab bar on Windows/Linux, at the far right on macOS),
the tab separators, or the 32 px drag gutter after the last tab (`--tab-bar-drag-gutter`). So the threshold is off by
a varying amount per platform as well.

## 3. Design

### 3.1 Measure the tabs, not a reserve

Two numbers describe the tab strip, both in the header's own (unzoomed) CSS px:

- **`tabsNaturalW`**: what the tab strip occupies when no tab is shrunk. The sum of every tab's `--tab-natural-width`,
  plus the separators between them, plus the drag gutter, plus the hamburger when it sits inside the tab bar.
- **`tabsFloorW`**: the same with every tab at `--ws-tab-min` (60 px).

Read the natural widths from the drop wrappers' inline `--tab-natural-width` (set by `DroppableTab`), falling back to
232 px for a wrapper not measured yet, exactly as the CSS does. Separator and gutter widths come from the same CSS
variables the layout uses, read once from the computed style. No reserve constants.

### 3.2 Measure the room the tab strip would have

The tab bar and the widget bar share whatever the rest of the header leaves them:

```
shared         = header.clientWidth − spent(header, keep: tab bar, status area)
                                    − spent(status area, keep: widget bar)
roomIfLabeled  = shared − labeledW
roomIfIconOnly = shared − iconOnlyW
```

`spent(row, keep)` is what a flex row spends on everything but `keep`: its padding, the gaps between its laid-out
children, every child's margins (a kept child's margins still take room), and the width of every in-flow child not
kept. Hidden and absolutely positioned children take no space. On macOS that is the traffic-light spacer, the left
drag strip, the hamburger and the status area's margin; on Windows/Linux the window controls instead.

`labeledW` and `iconOnlyW` are the two hidden mirrors the hook already keeps (`mirrorRef`, `iconMirrorRef`). Nothing in
the formula reads the live tab bar or widget bar, so the result doesn't depend on the tier currently shown and can't
oscillate (the property the 06-05 spec designed the mirrors for).

**Why not the live bars.** The first version used `tabBar.clientWidth + liveWidgetsW`. That is wrong whenever the live
widget bar overflows: a window that jumps straight to a width narrower than the labeled bar (a snap or a restore)
keeps the bar at its content width while the tab bar collapses, so the sum overstates the room. Measured on a macOS
dev build: header 1097 px, shared room 973 px, live sum 974 px (the bars already overflowed by 1 px). The hook also
observes the tab bar, the widget bar and the status area, so a tier change re-checks; that is idempotent because the
decision doesn't read them.

### 3.3 The tiers

| Tier | Condition | Widgets | Tabs |
|---|---|---|---|
| 1 Full | `roomIfLabeled ≥ tabsNaturalW` | icons + labels | natural width |
| 2 Icon-only | otherwise, while `roomIfIconOnly ≥ tabsFloorW` | icons | shrink from natural toward the floor |
| 3 Overflow | `roomIfIconOnly < tabsFloorW` | as many icons as fit, rest in `…more` | at the floor |

- **Tier 1→2 is the owner's rule.** Labels drop the moment the labeled widgets would make any tab narrower than its
  natural width. In tier 1 no tab is ever shrunk.
- **Tier 2→3 now follows the same principle.** Icons only go to `…more` once the tabs are already at their floor,
  instead of the separate 70 px/80 px reserves. That keeps one rule for both steps: the widgets give up space first,
  then the tabs, then the widgets again. (Decision for the owner, §5.)
- The tier 3 icon count keeps today's per-icon arithmetic (`use-widget-bar-responsive.ts:77-93`), with
  `headerW − buttonsW − tabsNeededIconOnly` replaced by `roomIfIconOnly − tabsFloorW`.
- A 1 px tolerance on each comparison absorbs sub-pixel rounding at a fractional header zoom.

### 3.4 When to re-measure

Today: a ResizeObserver on the header and the mirrors, and a MutationObserver on the tab list's children. Natural widths
also change when a tab is renamed or its label re-measured, which changes the wrapper's inline style, not the children.

- Add `attributes: true, attributeFilter: ["style"], subtree: true` to the MutationObserver (it only fires on the
  wrappers' style changes in practice), or observe the drop wrappers with the existing ResizeObserver.
- Batch to one measurement per frame (`requestAnimationFrame`), since a rename can touch several wrappers.

### 3.5 Code shape

- A pure `decideTopBarTier({ roomIfLabeled, roomIfIconOnly, tabsNaturalW, tabsFloorW, … }) → { tier, clipCount }` in
  `use-widget-bar-responsive.ts` (or a sibling `top-bar-tier.ts`), table-tested.
- The hook only measures and calls it. The four constants `MIN_TAB_WIDTH`, `TAB_COLLAPSE_RESERVE_PX`,
  `MIN_TAB_WIDTH_ICON_ONLY` and `TAB_COLLAPSE_RESERVE_ICON_PX` are deleted.

## 4. Verification

- **Unit:** the decision table: just wide enough for natural tabs plus labels (tier 1); 1 px less (tier 2); the floor
  boundary (tier 2/3); clip counts; the 1 px tolerance; one tab vs. many; long tab names (natural width up to 260 px).
- **Live (dev build, CDP):** for 1, 3 and 8 tabs, narrow the window in 10 px steps and record, per step, whether labels
  are shown and each tab's rendered width. Pass when no tab is narrower than its natural width at any step where
  labels are shown, and no widget is in `…more` at any step where a tab is above the floor. On macOS and on
  Windows/Linux (the hamburger moves).

## 5. Decisions for the owner

1. **Tier 2→3 trigger.** Built as recommended: icons overflow only once tabs are at their 60 px floor (§3.3), the same
   principle as tier 1→2. The owner saw it live on 2026-10-07 and asked for the PR.
2. **Hysteresis.** Not proposed: the mirrors make the decision independent of the current tier, so there is nothing to
   flap. If a boundary ever flickers in practice, add a few px of hysteresis to tier 1→2 only.
