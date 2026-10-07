# Pasting into a terminal from the right-click menu should leave the terminal ready for Enter

**Status:** implemented — PR #4431: §5 layers 1–3 (see §8), verified live. One §6 open item remains (Ctrl+Shift+V at the OS level).
**Date:** 2026-10-07 · **Author:** agent2 · **Base:** `main` @ `22857409f`
**Related:** `SPEC_PANE_CLICK_THROUGH_INPUT_FOCUS_2026_09_23.md` (the one-click rule for inputs), `SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md` (`giveBlockFocus`), `SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md` (the menu every pane shares).

---

## 1. The report

The operator pastes a command into a terminal pane, with **right-click → Paste** or **Ctrl+Shift+V**, and then wants to press Enter. Enter does nothing. They have to click the terminal again with the mouse first.

## 2. Verdict

**Right-click → Paste: reproduced, root cause found.** The right-click menu is an HTML overlay in the page itself (`showJsContextMenu`, `frontend/util/cef-api.ts`; the host's `show_context_menu` IPC is a no-op, "handled in JS overlay"). The bug unfolds like this:

1. Pressing the mouse on a menu row, a plain `<div>`, does what a mousedown on any non-focusable element does: the browser blurs the focused element, the terminal's `xterm-helper-textarea`, and focus falls to `<body>`.
2. The row's `click` handler closes the menu and runs the item. For Paste, that's `buildPaneContextMenu`'s handler (`frontend/app/block/pane-actions.ts`): read the clipboard, then `viewModel.paste(text)` → xterm's `terminal.paste()`. That writes into the terminal but never focuses it.
3. Nothing gives focus back. `closeMenu()` removes the overlay but doesn't restore the element that had focus when the menu opened. So keystrokes, Enter included, go to `<body>` until the user clicks the terminal.

**Ctrl+Shift+V: not reproduced at the page level.** The terminal's key handler (`termViewModel.ts`, `term:paste`) calls `preventDefault`, reads the clipboard and calls `terminal.paste()`. Focus stays in the terminal; xterm's own key-up handler even refocuses its textarea. The host only intercepts keys for browser panes (`handle_pre_key_event` → `host_key_for`, `crates/cef/src/client/handlers.rs`), so the key never leaves the page. See §6 for what's left to check on this path.

## 3. Evidence

Reproduced in an isolated `task dev` build (main @ `22857409f`), driven over CDP with real mouse and key events (`Input.dispatchMouseEvent` / `Input.dispatchKeyEvent`), on a plain terminal pane. "Focus" below is `document.activeElement`.

| Step | Focus afterwards |
|---|---|
| Left-click in the terminal | `TEXTAREA.xterm-helper-textarea` in the terminal |
| Right-click in the terminal (menu open) | `TEXTAREA.xterm-helper-textarea` in the terminal |
| Click the menu's **Paste** row | **`BODY`**: lost |
| (separately) Left-click, then Ctrl+Shift+V | `TEXTAREA.xterm-helper-textarea` in the terminal: kept |

A focus recorder (capture-phase `focusin`/`focusout`, with `HTMLElement.prototype.focus`/`blur` wrapped to log their callers) showed no programmatic focus or blur call from AgentMux code on the paste paths. The only focus calls were xterm's own (on click and on key-up). The menu-path loss is the browser's default blur on a mousedown on a non-focusable element, not code moving focus.

Not a bug, it turned out: in some runs the first left-click on the terminal left focus on `<body>`, and a second click was needed. That was the probe, not the app (§6).

## 4. Who else is affected

Every item chosen from this menu, in every pane type, moves focus to `<body>`. It's most visible after **Paste** into a terminal, because the next action is a keystroke. The same happens after "Copy", "Split …" (the new pane takes focus anyway, so it's harmless there), and the agent pane's **shell drawer** menu, whose Paste (`pasteClipboardIntoShell`, `shell-drawer-menu.ts`) goes through the same overlay.

## 5. Fix

Two layers. The first removes the cause; the second covers every way a menu can close.

1. **Menu rows don't take focus.** In `showJsContextMenu`, add `mousedown` → `e.preventDefault()` on the menu element (rows and submenus). This is the standard pattern for menus and toolbars: the click still fires, but the browser no longer blurs the focused element. The backdrop's own `mousedown` (outside click → close) keeps its current behaviour. A click outside the menu goes to whatever is under it, which is the expected focus target.
2. **Restore focus on close.** At open, record `const restoreTo = document.activeElement`. In `closeMenu()`, after the item has run, if focus ended on `<body>` (or inside the removed overlay) and `restoreTo` is still connected, call `restoreTo.focus({ preventScroll: true })`. This covers Escape, keyboard navigation, and items that don't move focus themselves. Items that do move focus on purpose, like Split (new pane) or Inspect, are left alone, because focus is no longer on `<body>`. Run the restore after the item's synchronous work, in a microtask, so a split's new pane wins.
3. **Paste says where focus belongs.** `pane-actions.ts` Paste calls `viewModel.giveFocus?.()` after `viewModel.paste(text)`. `pasteClipboardIntoShell` focuses the drawer's terminal the same way. This is belt and braces for the case the operator reported, and documents the intent: after Paste, the caret is in the pane that was pasted into.

The `focusin` this produces in the block also fires `handleChildFocus` → `reclaimWindowFocus` (`block.tsx`), which hands Win32 keyboard focus to the main render widget. So the fix keeps OS-level and page-level focus in step, as a click does today.

**Tests:**
- **Unit** (`cef-api.test.ts`): a `mousedown` on a row is default-prevented, and focus restores to the opener after an item click and after Escape. A Split-style item that focuses something else keeps that focus.
- **Pane actions:** Paste calls `paste` and then `giveFocus`.
- **Live**, in an isolated dev build over CDP: real right-click on a terminal, real click on "Paste", then `document.activeElement` is the terminal's textarea and a CDP `Enter` reaches the shell.

## 6. Open items

- **Ctrl+Shift+V at the OS level.** CDP injects keys inside the page, so it can't see Win32 keyboard focus. If the operator still loses Enter after Ctrl+Shift+V once §5 ships, the loss is at the Win32 level: the page still thinks the terminal is focused, but the OS routes keys to another HWND, for example a browser pane's. Clicking restores it because the click fires `reclaimWindowFocus`. Next step: reproduce on the operator's machine with real keys, and log `GetFocus()` before and after the paste. If confirmed, the paste path should call `reclaimWindowFocus` the way `handleChildFocus` does.
- **"First click doesn't focus"** from §3: resolved, a probe artifact. The probe sent Ctrl+Shift+V as one key carrying the Ctrl and Shift flags, and never the Ctrl and Shift key-ups. `registerControlShiftTracking` (`keymodel-dispatch.ts`) therefore stayed in layout mode, where every pane's `.block-mask` shows its number and takes pointer events. The next plain click landed on the mask, a non-focusable `<div>`, so focus fell to `<body>`, and `keyboardMouseDownHandler` left layout mode. After the probe sent the two key-ups, the masks went from 16 to 0, and a click focused each of five terminals on the first try. A real keyboard sends those key-ups, so users don't hit this. Anyone writing a CDP probe should send them too.

## 7. Method

- Code read at `main` @ `22857409f`: `pane-actions.ts`, `cef-api.ts` (`showJsContextMenu`), `contextmenu.ts`, `termViewModel.ts` (`term:paste`, `giveFocus`), `termwrap.ts` (paste handler), `block.tsx` (`handleChildFocus`, `reclaimWindowFocus`), `focusManager.ts`, `crates/cef/src/ipc.rs` (`show_context_menu`, `main_window_focus`), `crates/cef/src/commands/clipboard.rs` (Win32 `OpenClipboard(NULL)`, which doesn't touch focus) and `client/handlers.rs` (`handle_pre_key_event`).
- Live: own `task dev` instance, CDP from a scratch script; nothing driven in the operator's instance. The native OS keyboard was not used, so nothing could reach the operator's windows.

## 8. Implementation

All three layers of §5:

- **`frontend/util/cef-api.ts` `showJsContextMenu`:**
  - The overlay's `mousedown` handler, which covers the backdrop, the menu and every submenu, now calls `preventDefault()`. A press anywhere on the menu no longer takes focus; the click still fires.
  - The menu records the focused element when it opens (`opener`). After `closeMenu()`, in a microtask that runs once the chosen item has run synchronously, it refocuses the opener if focus is on `<body>` or went with the removed overlay, and the opener is still in the document.
- **`frontend/app/block/pane-actions.ts`:** Paste calls `viewModel.giveFocus()` after `viewModel.paste(text)`.
- **`frontend/app/view/agent/components/shell-drawer-menu.ts`:** `pasteClipboardIntoShell` focuses the drawer's terminal after pasting. `getTerminal`'s type gains an optional `focus()`, which xterm's `Terminal` has.

**Tests:**
- `cef-api.test.ts` "showJsContextMenu — focus":
  - a press on a row or the backdrop is default-prevented;
  - focus returns to the opener after an item click and after Escape;
  - an item that focuses something else keeps that focus;
  - an opener that was removed isn't refocused.
- `pane-actions.test.ts`: Paste calls `paste`, then `giveFocus`.
- `shell-drawer-menu.test.ts`: Paste calls `paste`, then the terminal's `focus`.

**Live**, in an isolated dev build of this branch, over CDP with real mouse and key events, on a plain terminal pane:

| Step | Focus afterwards |
|---|---|
| Right-click in the terminal (menu open) | `TEXTAREA.xterm-helper-textarea` in the terminal |
| Click the menu's **Paste** row | `TEXTAREA.xterm-helper-textarea` in the terminal, `document.hasFocus()` true (before the fix: `BODY`) |
| Enter, with no click in between | the pasted command ran: it wrote a marker file, which a control run without the Enter didn't |
| Ctrl+Shift+V | `TEXTAREA.xterm-helper-textarea` in the terminal: kept |
