# Keyboard shortcuts

**Status:** living — generated from the shortcut table; it changes when the table does.

<!-- Generated from frontend/app/keybindings/defaults.ts by keybindings/doc.test.ts. Do not edit by hand: run `UPDATE_KEYBINDINGS_DOC=1 npx vitest run frontend/app/keybindings/doc.test.ts`. -->

The same list is in the app: press F1, or open the Help pane. A terminal keeps every key for the shell except the window, tab and pane shortcuts below, and on Windows and Linux no global shortcut uses Alt+letter, so in a terminal those always reach the shell.

## General

| Action | macOS | Windows / Linux |
|---|---|---|
| Command palette | ⇧⌘P, ⌘P | Ctrl+Shift+P, Ctrl+P |
| Settings | ⌘, | Ctrl+, |
| Keyboard shortcuts | ⌘/, F1 | Ctrl+/, F1 |
| Voice input | ⌃⇧V | Ctrl+Shift+V |
| Close dialog or find bar | Esc | Esc |

## Tabs & windows

| Action | macOS | Windows / Linux |
|---|---|---|
| New window | ⇧⌘N | Ctrl+Shift+N |
| New tab | ⌘T | Ctrl+Shift+T |
| Close tab | ⇧⌘W | Ctrl+Alt+Shift+W, Ctrl+F4 |
| Next tab | ⌘], ⇧⌘], ⌃Tab | Ctrl+Shift+], Ctrl+Tab |
| Previous tab | ⌘[, ⇧⌘[, ⌃⇧Tab | Ctrl+Shift+[, Ctrl+Shift+Tab |
| Go to tab 1–8 | ⌘1–8 | Ctrl+1–8 |
| Go to last tab | ⌘9 | Ctrl+9 |
| Move tab left | ⇧⌘PgUp | Ctrl+Alt+Shift+PgUp |
| Move tab right | ⇧⌘PgDn | Ctrl+Alt+Shift+PgDn |
| Rename tab | F2 | F2 |

## Panes

| Action | macOS | Windows / Linux |
|---|---|---|
| New pane | ⌘N | Ctrl+Shift+` |
| New agent pane | ⇧⌘A | Ctrl+Shift+A |
| Split right | ⌘D | Ctrl+Shift+D |
| Split below | ⇧⌘D | Ctrl+Alt+Shift+D |
| Split in a direction | ⌃⇧S then ↑/↓/←/→ | Ctrl+Shift+S then ↑/↓/←/→ |
| Close pane | ⌘W | Ctrl+Shift+W |
| Maximize pane | ⌘M | Ctrl+Shift+M |
| Focus pane by direction | ⌃⇧↑/↓/←/→ | Ctrl+Shift+↑/↓/←/→ |
| Next pane | F6 | F6 |
| Previous pane | ⇧F6 | Shift+F6 |
| Focus pane 1–9 | ⌃⇧1–9 | Ctrl+Shift+1–9 |
| Swap pane with neighbour | ⌃⌥⇧↑/↓/←/→ | Windows: Ctrl+Alt+Shift+↑/↓/←/→; Linux: Ctrl+Shift+S then Shift+↑/↓/←/→ |
| Resize pane | ⌃⌥⌘↑/↓/←/→ | Alt+Shift+↑/↓/←/→ |
| Refocus pane | ⌘I | — |
| Focus the message box | ⌘L | Ctrl+L |
| Replace pane with launcher | ⌃⇧K | Ctrl+Shift+K |
| Change connection | ⇧⌘G | Ctrl+Shift+G |

## Find & zoom

| Action | macOS | Windows / Linux |
|---|---|---|
| Find in pane | ⌘F | Ctrl+F, Ctrl+Shift+F (in a terminal) |
| Zoom in | ⌘=, ⇧⌘= | Ctrl+=, Ctrl+Shift+= |
| Zoom out | ⌘-, ⌘Num- | Ctrl+-, Ctrl+Num- |
| Reset zoom | ⌘0, ⌘Num0 | Ctrl+0, Ctrl+Num0 |
| Reset zoom on all panes | ⇧⌘0 | Ctrl+Shift+0 |

## Terminal

| Action | macOS | Windows / Linux |
|---|---|---|
| Type into all terminals | ⇧⌘M | Ctrl+Alt+Shift+M |
| Clear | ⌘K | Ctrl+Shift+L |
| Copy | — | Ctrl+Shift+C |
| Paste | — | Ctrl+Shift+V |

## Documents

| Action | macOS | Windows / Linux |
|---|---|---|
| New document tab | ⌃T | Ctrl+T |
| Close document tab | ⌃W | Ctrl+W |
| Reopen closed document | ⌃⇧T | Ctrl+Shift+T |
| Next document | ⌃Tab, ⌃PgDn | Ctrl+Tab, Ctrl+PgDn |
| Previous document | ⌃⇧Tab, ⌃PgUp | Ctrl+Shift+Tab, Ctrl+PgUp |
| Move document right | ⌃⇧PgDn | Ctrl+Shift+PgDn |
| Move document left | ⌃⇧PgUp | Ctrl+Shift+PgUp |

## Editor

| Action | macOS | Windows / Linux |
|---|---|---|
| Save | ⌘S | Ctrl+S |
| Save as (scratch documents) | ⇧⌘S | Ctrl+Shift+S |
| Find and replace | ⌘F | Ctrl+F |
| Toggle markdown preview | ⇧⌘V | Ctrl+Shift+V |

## Files

| Action | macOS | Windows / Linux |
|---|---|---|
| Back | ⌥← | Alt+← |
| Forward | ⌥→ | Alt+→ |
| Up a folder | ⌥↑ | Alt+↑ |
| Open in new tab | ⌘Enter | Ctrl+Enter |
| New tab here | ⌘T | Ctrl+T |
| Close this tab | ⌘W | Ctrl+W |
| Type a path | ⌘L | Ctrl+L |
| Filter | ⌘F | Ctrl+F |
| New folder | ⇧⌘N | Ctrl+Shift+N |
| Rename | F2 | F2 |
| Refresh | F5 | F5 |
| Move to Trash | Delete | Delete |
| Delete permanently | ⇧Delete | Shift+Delete |
| Select all | ⌘A | Ctrl+A |
| Copy | ⌘C | Ctrl+C |
| Cut | ⌘X | Ctrl+X |
| Paste | ⌘V | Ctrl+V |
| Undo | ⌘Z | Ctrl+Z |
| Mention in agent | ⌥K | Alt+K |

## Mouse and gestures

Gestures that aren't plain keys: modifier + mouse, double- and middle-click, drag and drop, and keys a pane handles itself. The Help pane shows the same list, under Mouse and gestures. Generated from frontend/app/keybindings/tips.ts, where each row is tied to the code it describes.

### Panes

| What it does | Where | macOS | Windows / Linux |
|---|---|---|---|
| Move only the border between the two panes beside it (a plain drag resizes the whole group) | on a pane border | ⇧ + drag | Shift + drag |
| Maximize the pane, or restore it | on a pane's header | double-click | double-click |
| Move the pane somewhere else in the layout | a pane by its header | drag | drag |
| Number every pane, for ⌃⇧1 / Ctrl+Shift+1 to ⌃⇧9 / Ctrl+Shift+9 |  | hold ⌃ + ⇧ | hold Ctrl + Shift |
| Zoom that pane; over the title bar, status bar or a pane's header, zoom the app's frame instead | over a pane | ⌘ + scroll | Ctrl + scroll |
| Zoom every pane in the window together (over a browser pane's page, only that page zooms) | over a pane | ⌘ + ⇧ + scroll | Ctrl + Shift + scroll |
| Hold over the tab to open it and place the pane, or drop on the tab to move the pane there | a pane onto a window tab | drag | drag |
| Open it in a floating window; drag that back over the main window to dock it | a pane or pane tab out of the window | drag | drag |

### Window

| What it does | Where | macOS | Windows / Linux |
|---|---|---|---|
| Give all the size change to the panes along that edge (a plain drag scales every pane) | the window by its edge | — | Shift + drag (Windows only) |
| Maximize the window, or restore it | on empty space in the title bar | double-click | double-click |

### Tabs

| What it does | Where | macOS | Windows / Linux |
|---|---|---|---|
| Rename the tab (Enter saves, Esc cancels) | on a window tab's name | double-click | double-click |
| Set the tab's colour, or rename it | on a window tab | right-click | right-click |
| Scroll the tabs sideways | over the tab strip | scroll | scroll |
| Open the tab in a new window (Esc while dragging cancels) | a window tab below the strip | drag | drag |
| Close the tab | on a pane, document or editor tab | middle-click | middle-click |
| Keep it open instead of letting the next preview replace it | on a preview tab (italic) | double-click | double-click |

### Terminal

| What it does | Where | macOS | Windows / Linux |
|---|---|---|---|
| Open it in the system browser | on a URL in a terminal | click | click |
| Open it (a path with :line opens VS Code at that line), or show it in the file manager | on a file path in a terminal | click | click |
| Copy them into the terminal's folder | files onto a terminal | drop | drop |
| Move the cursor there (the terminal sends the shell arrow keys) | on the prompt line | ⌥ + click | Alt + click (Linux only) |
| Extend the selection to there | in a terminal | ⇧ + click | Shift + click (Linux only) |
| Select a word / a whole line | in a terminal | double-click or triple-click | double-click or triple-click (Linux only) |
| Find in a terminal (⌃F / Ctrl+F there goes to the shell) |  | — | Ctrl + Shift + F |

### Editor

| What it does | Where | macOS | Windows / Linux |
|---|---|---|---|
| A click opens a preview tab, a double-click a tab that stays; F2 renames | on a file in the editor's file tree | click or double-click | click or double-click |
| Add another cursor | in an editor | ⌘ + click | Ctrl + click (Linux only) |
| Make a column (block) selection | in an editor | ⌥ + drag | Alt + drag (Linux only) |
| Select the next match of the selected text | in an editor | ⌘ + D | Ctrl + D (Linux only) |

### Files

| What it does | Where | macOS | Windows / Linux |
|---|---|---|---|
| Add or remove one row / select a range | on rows in the Files pane | ⌘ + click or ⇧ + click | Ctrl + click or Shift + click |
| Add or remove the focused row without moving it | in the Files list | — | Ctrl + Space |
| Preview the file / filter / go back; typing a name jumps to it | in the Files list | Space or / or Backspace | Space or / or Backspace |
| Open it in a new tab | on a folder in the Files pane | middle-click | middle-click |
| Type a path | on the Files pane's folder path | double-click | double-click |
| Send files to an agent, editor, media or terminal pane; dropping copies here, or into the folder under the pointer | Files rows onto a pane, or files onto the list | drag or drop | drag or drop |

### Media

| What it does | Where | macOS | Windows / Linux |
|---|---|---|---|
| Zoom around the pointer / pan / switch between fit and zoomed; + − 0 1 and the arrows too | on an image in the Media pane | scroll or drag or double-click | scroll or drag or double-click |

### Agent

| What it does | Where | macOS | Windows / Linux |
|---|---|---|---|
| Clear it (⌘Z / Ctrl+Z brings it back); in an empty box, interrupt the agent | in the agent's message box | Escape | Escape |
| Take back the last queued message; ↑ and ↓ step through what you sent | in an empty message box | ↑ | ↑ |
| Accept the suggested next prompt | in an empty message box | Tab or → | Tab or → |
| / opens command autocomplete; a message starting with ! runs as a shell command | at the start of a message | / or ! | / or ! |
| Attach them to your next message | files onto an agent pane, or paste them | drop | drop |
| Search the conversation; Enter and ⇧Enter / Shift+Enter step through matches | in an agent pane | ⌘ + F | Ctrl + F |
| Allow / deny / set the scope: once, session, project or global | in a permission prompt | Enter or ⇧ + Enter or O·S·P·G | Enter or Shift + Enter or O·S·P·G |
| Open it in a browser pane (a plain click opens the system browser) | on a link in an agent pane | middle-click | middle-click |

### Find

| What it does | Where | macOS | Windows / Linux |
|---|---|---|---|
| Next / previous match | in a find bar | Enter or ⇧ + Enter | Enter or Shift + Enter |

### Browser

| What it does | Where | macOS | Windows / Linux |
|---|---|---|---|
| Address bar / reload / back / forward | in a browser pane | ⌘ + L or ⌘ + R or ⌥ + ← or ⌥ + → | Ctrl + L or Ctrl + R or Alt + ← or Alt + → |

## Your own shortcuts

Add a `keybindings` list to your settings file (command palette → Open Settings File). Your entries come before the defaults, so they win, and they apply as soon as you save.

```json
"keybindings": [
    { "key": "ctrl+shift+e", "command": "split:right" },
    { "command": "-tab:new" },
    { "key": "ctrl+Tab", "command": "-tab:next" },
    { "key": "meta+k", "command": "term:clear", "platform": "mac", "when": "terminalFocus" }
]
```

- `key`: modifiers `ctrl`, `shift`, `alt`, `meta` and `mod` (⌘ on macOS, Ctrl elsewhere), then a key: a letter, a digit, punctuation, or a name such as `Enter`, `Tab`, `ArrowLeft`, `F6`, `PageUp`. Two keys separated by a space make a chord.
- `command`: the command to run (the ids are in the help pane's list and the command palette). Prefix it with `-` to unbind it: every key, or just the `key` you give. An unbind removes the defaults and your own entries above it, so put a new key for the same command after it.
- `when` (optional): `textInputFocus`, `terminalFocus`, `docTabsHost`, `viewType == <pane>` or `viewType != <pane>`, each optionally negated with `!`, joined with `&&`.
- `platform` (optional): `mac` or `other` (Windows and Linux). Both when omitted.

An entry that can't be used (an unknown command, a bad key or `when`) is skipped and the rest still apply.

In a terminal, a key you add applies when its command is one of the terminal's shortcuts above, when the command isn't in the tables above, or when its `when` includes `terminalFocus`; otherwise the terminal keeps the key for the shell.

While a Browser pane's page has focus, AgentMux forwards only the default window, tab and pane keys. Remapping one of those to another command is respected there. Unbinding one stops it running, but the page still doesn't get the key; a key you add yourself isn't forwarded from a web page.
