---
type: patch
---

feat(menu): hamburger menu tweaks — reorder, DevTools item, Documentation link

Three tab-bar hamburger (≡) menu changes:

- **Reorder.** "New Tab" is now the topmost item. "Command Palette" moves
  from the top down to just below Settings.
- **DevTools is no longer a widget.** Removed `defwidget@devtools` from
  `widgets.json` (and the now-dead devtools special-case in
  `handleWidgetSelect`). DevTools toggling moves into the hamburger menu as
  a "DevTools" item between Settings and Command Palette — same
  `toggleDevtools()` action. (The Command Palette's "Toggle DevTools"
  command is unaffected.)
- **"Help" → "Documentation".** The menu item is renamed and now opens
  `https://docs.agentmux.ai` in the external browser via
  `getApi().openExternal(...)`, instead of opening the in-app help pane.
  The `help` widget / `view: "help"` pane are unchanged.

New bottom-of-menu order: Documentation · Settings · DevTools · Command
Palette · — · Exit.
