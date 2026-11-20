# Command palette: top centre, plain rows, every shortcut shown

**Status:** active. Approved with decisions D1 to D6 as recommended. Written against `origin/main` at `5ae357b6ee`; §12 says what changed on main since. Build: task #47900, in the order of §10. Supersedes the look and the "future" notes of `docs/specs/command-palette.md` (v1); the registry and the RunCommand route described there stay.

## 1. What is asked

1. The palette opens from the top of the window, in the centre, laid over the content the way VS Code's is.
2. No shaded backdrop behind it.
3. Rows in normal text.
4. No "OPEN" repeated on every line.
5. Every shortcut shown, and the missing ones filled in.
6. Edge cases found and decided.
7. What VS Code's palette does that is worth taking.
8. A second look at how commands are declared, for one source of truth.

## 2. What the palette is today (read from the code)

- `frontend/app/modals/command-palette.tsx` renders a window-scope `<Modal placement="top">`: a 560 px panel whose top edge is 15% of the window's height down, over a backdrop that dims the window by 55% and blurs it (`element/modal.scss`).
- The list is `commandRegistry.all()` (72 commands from `store/command-registry.ts`, 9 from the agent view, widgets' commands) plus one row per agent (`modals/palette-agents.ts`), sorted by a fixed category order, then by name.
- **The repeated "OPEN"** is the command's category. Every row prints its category at the right, in capitals (`.command-palette-item-category`, `label-caps`). Twenty commands are in the category "Open", so the word runs down the list.
- **Shortcuts** are not written on the commands. A row shows `shortcutFor(cmd.id)`: the first key of that command id in the shortcut table (`keybindings/defaults.ts`), with the person's own changes applied. So a shortcut is missing from a row for one of three reasons: the command has no key (most rows); the key belongs to a different command id that does the same thing; or the command with the key has no row at all (§6).
- Hover moves the selection. The arrow keys stop at the ends. The selected row is not scrolled into view. Enter during text composition runs the selected command. The opening shortcut pressed again does nothing. None of these is covered by a test today.
- The rows are plain `div`s: no list or option roles, and the selected row is not announced.

## 3. The look

### 3.1 Place

- **R1.** The panel hangs from the top of the window: its top edge is 6 px (real pixels, not zoomed) below the window's top edge, over the window header, and it is centred horizontally in the window.
- **R2.** The panel is 600 px wide at 100% zoom, and never wider than the window less 8 px on each side. Its height is what its rows need, up to 12 rows and never more than the window's height less 48 px; past that the list scrolls and the input row stays.
- **R3.** The panel follows the chrome zoom, as the title bar does. Its offset from the top is in real pixels at every zoom.
- **R4.** In a window too narrow for the panel beside the window's own buttons, the panel covers them while it is open. Escape, a click outside, or the opening shortcut closes it.

Tests: a browser layout test at 600, 900 and 1600 px wide and at 100%, 150% and 200% zoom asserts the panel's top (6 px), its centring (left and right gaps equal within 1 px), its width, and that it never leaves the window. A second case with 3 rows and with 80 rows asserts the height rule and that the input row does not scroll.

### 3.2 No backdrop

- **R5.** Nothing behind the panel is dimmed or blurred. The window stays as it was, readable, around the panel. The panel has its border and a drop shadow, so it reads as lying over the content.
- **R6.** The window behind is still locked while the palette is open (it cannot be typed into or tabbed into), and a click outside the panel closes the palette and does nothing else. This differs from VS Code, where the closing click also lands on what is under it; here a stray click must not press a button the person did not aim at. (Decision D1.)
- **R7.** A browser pane's page stays visible around the panel. Only the panel's own rectangle (with its shadow) is cut out of the pane's native view. Today the whole window is cut out, which the dimming hid.
- **R8.** Other dialogs keep their backdrop. Only the palette loses it.

Tests: a layout test asserts the backdrop's computed background is transparent and it has no `backdrop-filter`, and that another modal's is unchanged. A component test asserts a click on the backdrop closes the palette and that elements outside are inert while it is open. A component test asserts the rectangle registered with the pane overlay is the panel's, not the window's. The look check (§10) covers a real browser pane.

### 3.3 Text and rows

- **R9.** Rows are in the app's normal text: the label in the base size and regular weight, in the main text colour. Nothing in the palette is in capitals: the category label and the "ESC" hint go. (Decision D2 for the hint.)
- **R10.** A row is, from the left: the icon when the command has one, the label, the detail when there is one (an agent's seat and state) in muted text, and at the right the shortcut when there is one. A row with no shortcut has nothing at the right.
- **R11.** A shortcut is drawn as key caps, one cap per key, with the kit's `Kbd` (`element/ui/Kbd.tsx`): `Ctrl` `Shift` `P`. The two steps of a chord are two groups with a gap between them. On macOS the caps hold the usual symbols.
- **R12.** Groups. With nothing typed, the list is in groups: "recently used" first (§5), then one group per category, in today's order. A group's name is shown once, on the group's first row, at the right after the shortcut, in muted lower-case text of the small size, and a hairline separates each group from the one above it. Rows inside a group carry no category.
- **R13.** With something typed, the list is in at most two groups: "recently used" for the matches that are recent, then "other commands". Order inside each is the search's ranking (`command-palette-search.ts`, unchanged).
- **R14.** A label too long for the row is cut with an ellipsis. The shortcut and the group name never shrink, wrap or get cut. The full label is the row's tooltip.
- **R15.** No match: one line, "Nothing matches "…"", in normal muted text, not italic.
- **R16.** The row under the pointer is highlighted with the hover colour and the selected row with the selection colour, as two different things. Moving the pointer does not change which row Enter runs. A click runs the row clicked. (Today hover moves the selection, so a pointer resting where the palette opens picks a row, and the list scrolling under a resting pointer changes it.)

Tests: a component test per rule: no element in the palette computes `text-transform: uppercase`; a row with no key has no key cap element and no text at its right; "Ctrl+Shift+P" renders three caps and a chord two groups; a category name appears exactly once per group when browsing and never when typing; hover leaves `aria-selected` where it was; a 200-character label keeps the caps fully visible (layout test, the caps' right edge inside the panel and their width unchanged).

## 4. Keys, focus and assistive technology

- **R17.** Up and Down move the selection and wrap at the ends. Page Up and Page Down move by a page. Home and End stay with the text field. The selected row is always scrolled into view.
- **R18.** Enter runs the selected row. Shift+Enter runs it with "open a new one" where the command has that (as today).
- **R19.** While text is being composed (an IME), Enter, the arrows and Escape belong to the composition: no command runs, the selection does not move, and the palette does not close.
- **R20.** The opening shortcut pressed while the palette is open closes it. So does the "Command Palette" item of the logo menu and of the macOS menu bar.
- **R21.** Opening the palette closes any open menu, popover and tooltip of that window first. A dialog that is open stays, and the palette opens above it.
- **R22.** Closing without running anything gives focus back to what had it. Running a command closes the palette first and leaves focus to the command (a new pane takes it; the palette does not take it back).
- **R23.** The palette is one per window and opens in the window that has focus. It stays open when its window loses focus. (VS Code closes it; ours is also driven by agents and tests, and a palette that vanishes when another window comes forward is harder to work with. Decision D3.)
- **R24.** A pane that is magnified or full screen is under the palette, not over it.
- **R25.** With a browser pane focused, the opening shortcut still opens the palette in that window, typing goes to the palette and not to the page, and a click on the page closes the palette.
- **R26.** Roles: the text field is a `combobox` that controls the list and names the selected row (`aria-activedescendant`); the list is a `listbox`; each row is an `option` with `aria-selected`; each group is a `group` named by its label. A row's accessible name is its label, its detail and its shortcut in words. The no-match line is a `status`.
- **R27.** Colours come from tokens only, in every theme, and inside a pane backdrop the palette is the window's, not a pane's. In High Contrast the selected row is marked by more than a colour (an outline). Label, detail, key caps and group names meet 4.5:1 on the panel and on the selected row.
- **R28.** The palette appears with the dialog's existing short entrance and, under the motion policy's reduced setting, at once. Nothing else in it moves.

Tests: component tests for R17 to R22 and R26 (keys dispatched to the field; composition events with `isComposing`; the shortcut dispatched twice; a menu open before; focus before and after; the roles and names read from the DOM). R24 and R27 in the browser layout test (stacking order against a magnified pane; contrast in each built-in theme and in High Contrast). R25 and R23 in the look check on each platform, since they need a real browser pane and two real windows.

## 5. Recently used

- **R29.** A command run from the palette is remembered. The palette keeps the last 20, newest first, by command id, in the app's local storage, so it is the same in every window and survives a restart. Agents' rows count: an agent opened from the palette is recent.
- **R30.** With nothing typed, the first group is "recently used" with the 8 newest that still exist and can run now. With something typed, every remembered command that matches comes first, under "recently used" (R13). A command is never listed twice.
- **R31.** A remembered id that no longer exists, or cannot run now, is skipped, not shown greyed out, and not forgotten.
- **R32.** A command "Clear Recently Used Commands" empties the list.
- **R33.** Commands run by a key, a menu or an agent are not remembered: the list is what the person picked here.

Tests: unit tests on the store (cap of 20, newest first, a repeat moves to the front, unknown ids skipped, cleared) and component tests (the group and its 8 rows; a recent command absent from its category group; typing; a command run by `commandRegistry.run` does not appear).

## 6. Shortcuts

### 6.1 What is wrong today

| Case | Commands | Fix |
|---|---|---|
| The key is on another command id | "Find in conversation" (`agent:search:open`) shows nothing; Ctrl+F is `pane:find` | R34 |
| The command has a key but no row | Keyboard shortcuts (Ctrl+/, F1); New pane; Rename tab (F2); Move tab left and right; Go to last tab; Next and previous pane (F6); Swap pane (four); Move pane border (four); Refocus pane; Focus the message box; Replace pane with Shortcuts; Change connection; Type into all terminals; Find in pane; Reset zoom on all panes; Voice input | R35 |
| A pane's own commands have keys and no rows | Files (18), editor (6), document tabs (7), terminal (3), browser (1) | R36, later |
| The name differs between the shortcut list and the palette | "Maximize pane" / "Toggle Magnify" / "Magnify Pane"; "Split below" / "Split Down"; "Reset zoom" / "Actual Size" | R37 |
| A widget's command | Cannot have a key (`ext:` ids are refused in the `keybindings` setting, by the widget spec) | unchanged, R38 |

- **R34.** A command can name another command id whose key it shows, for a row that is the same action under a pane's own id. "Find in conversation" shows the key of `pane:find`.
- **R35.** Every app-level command in the shortcut table has a row in the palette, under its table name, in the category its id starts with. Go to tab 1 to 8 and focus pane 1 to 9 get no rows: seventeen rows that say nothing a person would search for.
- **R36.** Later, as its own change: a pane's own commands appear when that kind of pane has focus.
- **R37.** One name per command: where the shortcut table and the registry name the same id differently, both use the registry's name. A test keeps them the same from then on.
- **R38.** Every row whose command has a key shows it: built-in commands, agents' rows and widgets' rows alike (the last two have none today, and show one the day they can), and a key the person set in Settings > Keyboard replaces the default on the row at once.

Tests: a unit test walks the shortcut table and asserts each app-level command id has a registry entry with the same name (R35, R37); a component test sets a user key for a command, for an agent's row id and removes one (`-command`), and asserts the caps on each row (R38); R34 by its row.

### 6.2 Proposed new shortcuts (for approval; none is assigned before that)

Single free keys are nearly used up, and the ones left collide with a browser or a desktop somewhere. So the proposal is one two-step family, "Open": Ctrl+Shift+O (⌘⇧O on macOS), then a letter. The table already has one chord of this kind (Ctrl+Shift+S, then an arrow, to split).

| Command | Windows and Linux | macOS | What else uses these keys |
|---|---|---|---|
| Open Terminal | Ctrl+Shift+O, T | ⌘⇧O, T | First step: nothing in the shortcut table. Chrome uses Ctrl+Shift+O for its bookmark manager, which a browser pane does not have. Not in the measured list of keys a desktop takes (`keybindings/reserved.ts`) |
| Open Files | Ctrl+Shift+O, F | ⌘⇧O, F | as above |
| Open Swarm | Ctrl+Shift+O, S | ⌘⇧O, S | as above |
| Open Kanban | Ctrl+Shift+O, K | ⌘⇧O, K | as above |
| Open Vault | Ctrl+Shift+O, V | ⌘⇧O, V | as above |
| Open History | Ctrl+Shift+O, H | ⌘⇧O, H | as above |
| Open System Info | Ctrl+Shift+O, I | ⌘⇧O, I | as above |
| Connectors | Ctrl+Shift+O, C | ⌘⇧O, C | as above |
| Memory | Ctrl+Shift+O, M | ⌘⇧O, M | as above |
| Back to Previous Pane | Ctrl+Alt+- | ⌃- | VS Code's own keys for "go back" on Linux and macOS. Ctrl+Alt+- is not AltGr text on the layouts in the reserved list (that rule is for letters); to be measured |
| Forward to Next Pane | Ctrl+Shift+- | ⌃⇧- | VS Code's "go forward". Not zoom (zoom out is Ctrl+- and ⌘-) |

Not proposed, on purpose: Reload Window, Close Window, Restart Backend (a key pressed by accident costs too much); the animation speeds, the dev file openers and the workspace actions (rare; the palette is their shortcut).

**Before any of these is assigned**, each is pressed on Windows, macOS and Linux in a test build, with a terminal, an agent pane and a browser pane focused, to see that it arrives. The repo's rule for the reserved list is that a key is not guessed.

## 7. Edge cases

| Case | Behaviour | Rule |
|---|---|---|
| Narrow window | Panel is the window's width less 8 px each side; covers the window's buttons while open | R2, R4 |
| Zoomed window (150%, 200%) | Panel zooms; its top offset does not; it never leaves the window | R3 |
| Very long command name, long agent seat | Ellipsis; caps and group name whole; tooltip has the full text | R14 |
| No match | One plain line | R15 |
| Hundreds of entries (many agents, widgets' commands) | The list is not cut short. Filtering 1,000 entries is one pass of the existing search and stays under a frame (unit test with a time bound); rows are rendered as today, and if the look check shows a slow open with 300 agents the list is windowed in the same PR | R39 |
| A menu, popover or tooltip is open | It closes; the palette opens | R21 |
| A dialog is open (a pane's confirm, Settings) | It stays; the palette opens above; closing the palette returns focus to the dialog | R21, R22 |
| The palette is open and its shortcut is pressed | It closes | R20 |
| A pane is magnified or full screen | The palette is above it | R24 |
| A browser pane has focus | Opens in that window; keys go to the palette; the page stays visible; a click on the page closes it | R7, R25 |
| Text composition (IME) | Enter, arrows and Escape belong to the composition | R19 |
| High Contrast, every theme, a pane with a backdrop | Tokens only; selection has an outline in High Contrast; the palette is never in a pane's colours | R27 |
| Reduced motion | Appears at once | R28 |
| Screen reader | combobox, listbox, option, group; the selected row is announced with its shortcut | R26 |
| A second window | One palette per window; opening one in window B leaves window A's as it is | R23 |
| The window loses focus | The palette stays open | R23 |
| A pointer resting where the palette opens, or a list scrolling under it | The selection does not change | R16 |
| A command that cannot run now (no agent pane focused) | Not listed, as today; not listed under "recently used" either | R31 |
| A command that fails | The palette is already closed; the command's own error toast says so (unchanged) | |
| An agent opens the palette (`RunCommand view:command-palette`) | Allowed today, unchanged; it opens in the person's focused window | |
| Two rows with the same label (two agents of one name) | Both listed; the detail tells them apart (unchanged) | |

- **R39.** Opening and typing stay within a frame with 1,000 entries.

## 8. What VS Code does, and what to take

Sources: VS Code's user interface and contribution-point documentation, its 1.14 release notes, the screenshot supplied with the request, and a reading of its quick-input source for this spec.

| Finding | Take? | Why |
|---|---|---|
| The palette hangs from the top, centred, 600 px at most, and has no backdrop element at all | Take | R1, R2, R5. It is the request |
| "recently used" above "other commands", as labelled separators; a least-recently-used list capped by a setting (50 by default), kept across restarts, filled only by commands run from the palette, with a command to clear it | Take, smaller | §5: 20 kept, 8 shown, no setting |
| The shortcut at the right of the row as key caps, from the live key table, so a changed key shows | Take | R11, R38. Ours already reads the live table |
| A row shows "Category: Title" | Skip | Our labels start with their verb ("Open Terminal"), and the group name carries the category once |
| Hover and selection are two highlights | Take | R16 |
| A gear on the selected or hovered row to set that command's key, and a cross on a recent row to forget it (both visible in the supplied screenshot) | Later | Settings > Keyboard is where a key is set today. The gear is a shortcut to it worth having once the rows are right. Decision D4 |
| Prefix modes of the same box (`>` commands, `@` symbols, `:` line, `?` help) | Later | One mode today. A "go to agent" or "go to pane" mode is the natural first one |
| "similar commands" by text similarity, and "commonly used" | Skip | A second ranking for a list of about 100 |
| The typed text kept for the next opening (a setting, off by default) | Skip | Recently used does the job |
| The palette can be dragged elsewhere and has preset positions | Skip | |
| Extensions add commands in their manifest and are listed by default; a `when` clause hides one; a key is contributed separately and shows on every surface | Have | Widgets' `contributes.commands` reach the registry today. A widget cannot contribute a key (R38), by the widget spec |
| `listbox` and `option` roles | Take | R26 |
| Closes when focus leaves it | Skip | R23 |

## 9. One source of truth for a command

### 9.1 What exists

Three tables, joined by the command id:

- **What a command is and does:** `commandRegistry` (`store/command-registry.ts`): id, label, category, icon, keywords, `when`, `execute`. Widgets register into it. Agents are added to the palette's list live and are not in it.
- **Which key runs it:** the shortcut table (`keybindings/`): id, name, keys per platform, the context a key applies in, the person's changes. A key with no handler of its own runs `commandRegistry.run(id)`.
- **What a key press does:** `store/keymodel.ts`: for about a dozen ids a handler written apart from the registry's `execute`.

And the menus (logo menu, title bar menu, pane menu, the macOS menu bar) each write their own labels and, for some items, call the action directly.

### 9.2 What is duplicated

- **Two implementations of one command**, one for the key and one for the palette: close pane, magnify, next and previous tab, focus pane by direction, zoom, new tab, new window. Close pane takes two different paths.
- **Two or three names for one command** (R37).
- **Hand-written labels in menus** for registry commands: New Tab, New Window, Reload Window, the workspace actions, Online Docs (whose address is written twice).
- **Hand-written key text** in about a dozen places (F2 in the editor tree menu, Enter in the files menu, the browser's Incognito key, viewers' hints). The test that forbids it does not see a bare key or a `keyLabel("mod+…")`.
- **The macOS menu bar** shows no keys, and its "Settings…" opens the settings file while the palette's "Settings" opens the Settings pane. Reported to the owner; not part of this work.

### 9.3 The smallest change

One registry in meaning, without merging the tables: the registry says what a command is, the shortcut table says which key, and everything else reads those two by id.

1. **The palette** keeps reading the key with `shortcutFor(id)` at render time. The registry gains one optional field: the id whose key a row shows (R34). No `shortcut` field: a second copy of a key is how it goes stale.
2. **One name** (R37) and **a row for every app-level key** (R35).
3. **Menus** take their label, key and action from the registry by id, through one helper, where the item is a registry command. Items that are not commands (colour pickers, checkboxes) stay as written.
4. **One implementation:** a key whose command is in the registry runs the registry's `execute`. Each of the dozen handlers is removed in its own small change, with a test that the key and the palette row do the same thing, because two of them differ today (close pane) and the difference has to be looked at, not assumed away. The owner of panes and tabs reviews these.
5. **Key text** everywhere comes from `shortcutFor`; the test is widened to bare keys and `keyLabel` literals.

### 9.4 Left alone

- The shortcut table's own structure: its contexts, pane scopes and platform split do work the palette does not need.
- The agents' live list, and widgets' registration.
- The agent pane's slash commands: a second command system with arguments and per-agent availability. Listing them in the palette is a possible later mode (§8), not a merge.
- The RunCommand route and what an agent may run.

## 10. Order of the build

1. **The look:** §3 and §4 (R1 to R28), R39. One PR. Look check on macOS and Linux: place and size at three widths and two zooms, no backdrop over an agent pane, a terminal and a browser pane, the keys, a second window, High Contrast, light theme.
2. **Recently used:** §5.
3. **Shortcuts shown:** R34, R35, R37, R38.
4. **New shortcuts:** §6.2, after approval and after each key is measured on the three platforms.
5. **One source:** §9.3 steps 3 to 5, each a small PR.

## 11. Decisions wanted

- **D1.** A click outside closes the palette and does nothing else (proposed), or also lands on what is under it (VS Code).
- **D2.** Drop the "ESC" hint in the input row (proposed), or keep it in normal case.
- **D3.** The palette stays open when its window loses focus (proposed), or closes (VS Code).
- **D4.** The gear on a row to set its key, and the cross to forget a recent command: later (proposed), or in this build.
- **D5.** The shortcut table in §6.2: which to assign.
- **D6.** Recently used: 20 kept and 8 shown (proposed).

## 12. Changes since this was written (main at `7e36ec6909`)

Two changes landed on main after the spec was drafted, and one reading of the spec was settled while building the look.

- **Every pane view, widget and key-bound command already has a row** (#7713, `store/palette-open-commands.ts` and `store/palette-key-commands.ts`). R35 is therefore partly met; the shortcuts-shown change re-audits the shortcut table against the registry and fills only what is still missing.
- **Keyboard scrolling** (#7714): the selected row stays in view, and Page Up, Page Down, Home and End move the selection through the list. R17 said Home and End stay with the text field; they keep the behaviour that shipped, which selects the first and the last row. Up and Down wrap at the ends (R17) in the change for §4.
- **The tooltip of R14** shows the full label when the label is cut by the row, not on every row: a hover label on a row whose text is whole says nothing and hides the row below it.
- **The group column** (R12): every row keeps the width of the group name, empty, while browsing, so the shortcuts line up down the list. While typing there is no group, and no column.
- **`Modal`** gains `placement="dropdown"` and `backdrop="none"` for this; no other dialog changes.
