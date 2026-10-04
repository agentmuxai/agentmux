# Keyboard shortcuts

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
| Swap pane with neighbour | ⌃⌥⇧↑/↓/←/→ | Ctrl+Alt+Shift+↑/↓/←/→ |
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

While a Browser pane's page has focus, AgentMux forwards only the default window, tab and pane keys. Remapping one of those to another command is respected there. Unbinding one stops it running, but the page still doesn't get the key; a key you add yourself works everywhere except inside a web page.
