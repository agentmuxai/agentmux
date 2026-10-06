# Report: the My Agents tile's expand arrow and edit panel sit in the wrong place

Date: 2026-10-05. Code read at `origin/main` `6943d5fd5`. Status: option B (section 4) is implemented in the PR that carries
this report. Option C was left out on purpose: it is a visual refinement, not part of the fix.

## 1. What is wrong

In the starting agent pane (the picker), the "My Agents" tiles each have a chevron at the bottom right that opens the
actions panel (Rename, Duplicate, View History, Delete). In a row of tiles that differ in height, the chevron, and the
panel it opens, line up with the **tallest** tile in the row instead of with their own tile. On the shorter tiles the
chevron floats below the tile's border, and the panel opens with a gap under it.

I read the code to find this. I did not reproduce it on screen, so the mechanism below is from the CSS, not a screenshot.

## 2. Root cause

The tile is built from two elements that do not agree on what "the tile" is.

| Element | Selector | What it owns |
|---|---|---|
| `<li>` (the grid item) | `.agent-recent-sessions-row` | Its height, which the grid stretches. It is also the positioning anchor (`position: relative`) for the chevron and both panels. |
| `<button>` inside it | `.agent-recent-sessions-entry` | The visible tile: border, background, hover and padding. Its height is only its content (`width: 100%`, no height). |

1. `.agent-recent-sessions-list` is a CSS grid with no `align-items`, so the default `stretch` applies. Every `<li>` in a
   grid row is stretched to the tallest tile in that row.
2. The `<button>` is not stretched with it. It stays at its natural height, so the visible tile is shorter than its `<li>`.
3. `.agent-recent-sessions-menu-toggle` is `position: absolute; bottom: var(--space-2)`. That is measured from the `<li>`,
   so on a short tile it lands below the tile's visible border.
4. `.agent-row-menu` and `.agent-fork-prompt` are `top: 100%` of the same `<li>`. They open at the bottom of the stretched
   `<li>`, not at the bottom of the visible tile.

Tile heights vary more than they used to. #4285 made the tile twice as large and let the summary run to three lines. An
agent with no history shows a one-line note. The account line and the message count are each optional. So a row of
tiles with different heights is now the normal case. The spec for #4285 says "tiles in one row keep equal height", but
the test it planned was a DOM test on class names and computed style, which cannot catch this. jsdom does no layout.

The Templates tier (`AgentCard`) does not have the problem. There the card is the grid item itself, so the stretch
applies to the thing that has the border.

## 3. Best practice

For a card with one main action and some secondary actions:

1. **The grid item is the card.** The element that stretches in the grid owns the border, background and hover. Anything
   positioned relative to the card is then positioned relative to the box the user sees. This is the rule `AgentCard`
   already follows and `MyAgentsList` breaks.
2. **Inside the card, use a flex column.** The content takes the space it needs and a footer row sits at the bottom
   (`margin-top: auto`). A control at the bottom of every card then lines up without absolute coordinates.
3. **Put secondary actions in flow where you can.** The timestamps and the chevron share one footer row. That removes the
   `bottom`/`right` offsets, the 40 px reserved right padding, and the "top-right overlaps the HOST/SANDBOX badge" problem.
4. **Make the main action reach the whole card without nesting buttons.** Keep the chevron as a sibling of the main
   button, which the code already does (nested buttons are invalid HTML), and let the main button cover the card.
5. **Float panels with the floating-panel component the app already has.** Don't open them with `position: absolute`
   inside a grid item.

On point 5: `frontend/app/element/anchored-popover.tsx` (`AnchoredPopover`) is already used by the status bar and the
agent pane's model and session panels. It portals to `document.body`, positions with floating-ui against a live anchor
element, follows chrome zoom, dismisses on an outside click or Esc, and handles the native-pane overlay case. The row
menu has its own outside-click handling and relies on `z-index: 5` plus a blur on every other tile to stay readable.

For aligning inner rows **across** tiles (every tile's name, summary and footer on the same baselines), the CEF build is
recent enough for CSS `subgrid`. I would not start with it. Equal outer height plus a bottom-pinned footer fixes the
reported bug, and subgrid is a refinement.

## 4. Options

### A. Hotfix (about 5 lines of CSS, one file)

Make the `<li>` a flex column and let the button fill it:

```scss
.agent-recent-sessions-row { display: flex; flex-direction: column; }
.agent-recent-sessions-entry { flex: 1; }
```

The visible tile then fills its `<li>`, so the chevron and the panels line up with the tile's own bottom edge on every
tile. The tiles also become equal height, as the spec intended.

- Pros: tiny and safe, and it keeps the delete animation, the spotlight blur and the panels as they are.
- Cons: the chevron is still absolutely positioned over padding that was reserved for it. The structure that caused the
  bug is unchanged.

### B. Restructure (recommended)

1. The `<li>` becomes the card: it takes the border, background, hover and active tint from `.agent-recent-sessions-entry`.
2. The main button covers the card as a transparent hit area (`position: absolute; inset: 0` or a stretched `::after`).
   The content sits above it and is non-interactive.
3. A footer row holds the timestamps on the left and the chevron on the right. The chevron is in flow, with
   `margin-top: auto` on the footer. The 40 px right padding and the absolute chevron go away.
4. The actions menu, the rename input and the fork prompt render in `AnchoredPopover`, anchored to the chevron or the
   tile. The spotlight blur can stay as a visual effect, but the panel no longer depends on `z-index: 5` or on the row
   staying in its grid cell.

- Pros: removes the cause, not just the symptom. Alignment no longer depends on content height. The panels get zoom,
  dismiss and pane-overlay handling for free. A chevron at the bottom right of an equal-height tile is easy to find.
- Cons: touches `MyAgentsList.tsx` and `_recent-sessions.scss`, plus the delete "poof" and reflow animation, which
  clones and absolutely positions the `<li>`. The rename/fork panel moving into a popover needs a decision on where the
  rename input should appear.

### C. Subgrid on top of B

Each card spans a fixed set of grid rows so name, summary and footer align across every card in a row. This is only worth
doing if the summary line's varying height starts to look ragged once heights are equal.

## 5. Recommendation

Ship **A** now, as it is small and fixes what you see. Do **B** as the follow-up, because the root problem is the card
having two owners, and A leaves that in place. Leave C until B is on screen.

## 6. How to test it

This cannot be covered by a jsdom test. Layout needs a real browser. The check I would add runs in an isolated
`task dev` build: render a row of tiles with different heights (a one-line, a three-line and an empty summary) and assert
that each tile's chevron bottom is within a pixel or two of its own entry's bottom edge, and that the panel's top equals
that tile's bottom. A pure-CSS unit test on class names would pass today and catch nothing.

## 7. What I did not check

- I did not run the app or take a screenshot of the broken layout.
- I did not look at the narrow single-column case (a pane under 480 px). Equal height does not matter there, but the
  chevron and panel placement should be checked.
- I did not check how the delete animation behaves with the structure in option B.

## 8. Decision and what was built

Option B was chosen, with engineering time not a constraint. Option C was not built: it aligns inner rows across tiles,
which is polish, and it adds risks (row gaps inside every tile, placeholder rows for optional lines, the delete animation
pulling a tile out of the grid).

- The `<li>` is the tile. It owns the border, background, hover and active tint, and the grid stretches it.
- The open button wraps only the name. Its `::after` covers the tile (stretched-button pattern), so a click anywhere opens the
  agent. The chevron is its own control above it.
- The tile is a three-column grid: icon, body, actions. The body takes the leftover height, and the footer row (timestamps
  and chevron) sits at the bottom. There are no absolute offsets left in the tile.
- The actions menu and the Rename / Duplicate panels render in `AnchoredPopover`. Their styles moved out of the `.agent-view`
  wrapper, because a portaled panel is not a descendant of it. The menu keeps this list's pane-scoped Escape and
  outside-press handling; the popover's own document-wide dismissal is not used for it.
- `MyAgentsList.layout.test.tsx` measures the real markup and the real stylesheet in headless Chrome or Edge. Run against the
  old markup, it showed the bug: on a tile with no history the chevron sat 57 px below the visible tile. It skips when no
  Chromium-based browser is installed.
