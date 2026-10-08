# A filter for the Help pane, on one shared filter box

**Status:** active — built in #4484.
**Date:** 2026-10-08.
**Requested by:** repo owner (asafebgi): "add a filter to the help pane … write a separate spec to file"; "the new help
pane's filter input naturally would autoselect like all the rest"; "make sure we keep the filter DRY … how DRY is our
filter framework?"
**Author:** AgentO.
**Related:** `SPEC_FOCUS_FOLLOWS_SELECTION_2026_10_08.md` (#4479: `data-pane-focus`, the pane's typing target),
`REPORT_FOCUS_ON_OPEN_AUDIT_2026_10_08.md` (#4481).

---

## 1. Goal

Open Help (Cmd+/ or F1, the widget, "Open Help" in the palette), type, and see only the shortcuts and tips that match.
No click first: the filter box is the pane's typing target, like the Settings search, the My Agents filter and the
Remotes filter.

## 2. What Help shows today

`helpview.tsx` renders `QuickTips` (`element/quicktips.tsx`), five cards of hard-coded TSX:

| Card | Items | Source |
|---|---|---|
| Header Icons | 4 (Maximize, Change connection, Pane Settings, Close pane), with their shortcuts | hard-coded, keys from `shortcutFor` |
| Keyboard Shortcuts | about 75–80 entries in 8 groups, plus a "Mouse" group | `shortcutHelp()`, generated from `keybindings/defaults.ts` with user overrides |
| More Tips | 3 | hard-coded |
| Need More Help? | 5 links | hard-coded |
| Alpha Software & AI Content | a notice | hard-coded |

No search, no anchors, no `giveFocus` and no `data-pane-focus`, so Help never took the caret.

## 3. Design

- **The box.** A `FilterInput` (§4) pinned at the top of the pane, outside the zoomed content so it stays put and keeps
  its size when Help is zoomed. Placeholder "Filter shortcuts and tips". It carries `data-pane-focus`, so focusManager
  gives it the caret whenever Help is opened or selected (#4479). Escape clears it; on an empty box Escape is left to
  the rest of the app.
- **Matching** (`matchesEveryWord`, §4): every word typed must appear, case-insensitively, in the item's words, in any
  order. An item's words are its card or group name, its title and text, and its keys. Nothing is ranked or fuzzy: the
  list keeps its order and what shows is predictable. Matching the group name means "terminal" or "tabs" shows a whole
  group.
- **Keys by name.** macOS key labels are symbols ("⇧⌘W"), so each key label also carries its symbols' names
  (`keyLabelWords`, `keybindings/help.ts`, beside the rest of the key labels): "cmd shift w" finds it. Other platforms' labels are words already
  ("Ctrl+Shift+W").
- **What hides.** Items that don't match; a shortcut group or card with nothing left; the alpha notice while the box
  has text (it is a notice, not help content).
- **Nothing matches.** "Nothing in Help matches “…”." in place of the cards.
- **Content as data.** The hard-coded cards become small arrays (`HEADER_ICONS`, `MORE_TIPS`, `HELP_LINKS`,
  `MOUSE_ENTRIES`) rendered by one loop each, so they filter the same way the generated shortcut list does. The look
  is unchanged.
- **Not persisted.** The filter starts empty each time Help mounts.

## 4. DRY: one filter box and one matcher

Before this change the panes' filters shared almost nothing:

| Pane | Box | Matching |
|---|---|---|
| My Agents | `AgentPickerFilterBar`: own icon, input, clear button, Escape handling | `literalFirstSearch` (ranked) for names |
| Personal Memory | `MemoryAgentFilterBar`: a copy of the above, with CSS copied from the picker | an inline `toLowerCase().includes()` |
| Remotes | a bare `<input type="search">` with its own CSS | its own substring check (`remotes-sections.ts`) |
| Settings | `SettingsSearchBar`: a results dropdown, a different control | `literalFirstSearch` (ranked) |
| Files | an on-demand filter row in `files-view.tsx` | its own |

Now:

- **`FilterInput`** (`element/ui/FilterInput.tsx`, exported from `element/ui`): magnifier, input, a clear button while
  there is text, Escape clears, `data-pane-focus` by default. It renders a bordered `.ui-filter` box (`ui.scss`), or
  with `bare` just its parts, for a bar that lays them out itself. `iconClass`, `inputClass`, `clearClass` and `testId`
  let an existing bar keep its CSS and test ids.
- **`matchesEveryWord(query, ...texts)`** (`app/util/fuzzysearch.ts`, beside `literalFirstSearch`): the plain
  type-to-narrow match. `literalFirstSearch` stays for places that rank results (My Agents, Settings).
- **Moved onto them in this change:** Help (new); My Agents and Personal Memory (`bare`, their own classes, so they look
  the same; their duplicated Escape and clear-button code is gone); Remotes (the boxed form, which adds the magnifier and
  a clear button, and its matcher now uses `matchesEveryWord`, so "user host" matches as well as "user@host"); Personal
  Memory's agent-name match.
- **Not moved:** Settings (a results dropdown, not a narrowing filter; it already shares `literalFirstSearch`) and the
  Files filter (opened on demand by `files:filter`, with list keyboard navigation of its own). Both can take
  `FilterInput` later; the CSS the picker and memory bars still carry can then be folded into `.ui-filter`.

## 5. Tests

- `FilterInput.test.tsx`: the pane-focus default and opt-out, input, the clear button, Escape (handled when there is
  text, left alone when empty), `bare` with the caller's classes and test ids.
- `fuzzysearch.test.ts`: `matchesEveryWord` (blank query, every word in any order across texts, missing texts, no match
  across two texts).
- `quicktips.test.tsx`: no filter shows everything; a filter keeps matching items and drops empty groups, cards and the
  alpha notice; a group name shows its group; keys match by symbol name; the empty state. `keyLabelWords`.
- `helpview.test.tsx`: the box is the pane's typing target and typing filters.
- Existing My Agents, Memory and Remotes suites pass unchanged.
