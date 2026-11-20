# SPEC: Right-click an Editor document tab to show the file in the computer's file manager

**Status:** implemented; approved and built with this spec. Written 2026-11-17 against main `b32b24746f`. Everything in §2 was read from the code, not run.

## 1. The request

Right-clicking a document tab in the Editor should offer to open the computer's own file manager with that file selected, named the way each platform names it. (Dragging files out of the app is a separate research task, #47953.)

## 2. What exists today

**The tab's menu.** `EditorTabBar` (`view/editor/editor-view.tsx:308`) gives the document tab strip a `menuItems` function: "Rename…" (or "Save As…" for an Untitled document), then the "Show As" submenu from `showAsMenuItems` (`editor-tab-show-as.ts`). It has no reveal item. A tab carries `payload.filePath`, `payload.isScratch` (an Untitled document, no file on disk) and `payload.viewer`.

**The label.** `revealInFileManagerLabel()` (`util/file-manager-label.ts`) is the one label already used by the editor tree's file and folder menus, the Files pane's row and folder menus, the viewer bar, the info card and the host popover: "Reveal in Finder" on macOS, "Reveal in File Explorer" on Windows, "Reveal in file manager" on Linux.

**Three ways to reveal a path, one of them the right one for a tab.**

| Call | Where it runs | Limits | Who uses it |
|---|---|---|---|
| `fs.reveal` (`FsRevealCommand`, `srv-core fs_ops::open_or_reveal`) | srv | any existing path, not limited to home; refuses a confined pane (a container agent's folder); errors are returned | Files pane (`revealInOs`) |
| `openinshell` (`file_io::reveal_in_shell`) | srv | **only paths under the home folder**; the editor swallows its errors (`editor-model.ts:1269`) | the editor tree's menus |
| `reveal_in_file_explorer` (`cef commands/platform.rs:890`) | the host | any path, host-local | status bar, widgets, terminal file links, tool previews |

**What each platform does.** Windows: `explorer.exe /select,<path>` (srv quotes the path). macOS: `open -R <path>`. Linux: all three calls run `xdg-open <containing folder>`, which opens the folder but does **not** select the file; there is no portable "select" in `xdg-open`.

**Remote documents.** A block's meta `connection` names an SSH host the file is on (`EditorViewModel.connection()`, `isSshConnection`); a scratch tab's file is always local. `fs.reveal` takes no connection: for a remote path it would try the same path on this computer. A WSL path (`\\wsl$\…`) is a local Windows path and Explorer opens it.

## 3. Design

**3.1 The item.** One row on the document tab's menu, after "Rename…": `revealInFileManagerLabel()` with the reveal icon (`MENU_ICON.reveal`, as in the tree). It calls `fs.reveal` with the tab's `filePath` and the pane's `block_id`, so it behaves exactly like the Files pane's.

**3.2 When it is not offered or not usable.**
- An Untitled (scratch) tab: no row, the file does not exist yet.
- A tab on a remote host (`model.connection()` is not empty): the row is shown **disabled**, with the reason as its sublabel ("The file is on <host>"), the same way "Rename…" shows why it is refused (`enabled: false`, `sublabel`). Disabled and explained, not hidden, so the person is not left wondering where the item went.
- A dropped file that has no path on disk (`isDroppedPath`): no row. **(verify)** that a dropped file's tab never holds a real path; if it does, it is a normal local tab.
- The file was deleted or moved since it opened: `fs.reveal` fails; the editor shows its existing toast ("The file is no longer on disk") and nothing is opened. A confined pane's refusal text is shown the same way. Nothing fails silently.

**3.3 The label per platform.** Windows "Reveal in File Explorer", macOS "Reveal in Finder" (both unchanged). Linux: **"Open Containing Folder"**, the name VS Code uses there, because that is what Linux file managers can reliably do. The shared helper changes, so every place that shows the label agrees (§7 decision 1). For a folder (the host popover opens a data folder, not a file) the helper takes `kind: "folder"` and Linux says "Open in File Manager"; macOS and Windows keep their names for a folder.

**3.4 Linux selects the file where it can.** One shared function in `srv-core fs_ops/platform.rs` (`reveal_command` and its Linux fallback) tries, in order:
1. `dbus-send --session --print-reply --dest=org.freedesktop.FileManager1 --type=method_call /org/freedesktop/FileManager1 org.freedesktop.FileManager1.ShowItems array:string:"file://<percent-encoded path>" string:""`, waited for up to two seconds. Success selects the file in Nautilus, Dolphin, Nemo, Thunar and the other file managers that implement the interface.
2. If `dbus-send` is missing, there is no session bus, the call fails or times out: `xdg-open <containing folder>` (today's behaviour).

Both go through the same strict external-process policy as today (`spawn_detached` and `sanitize_external_std_command`, invariant I7).

**3.5 One path through srv (DRY).** The editor tree's two reveal rows move from `openinshell` to `fs.reveal` as well, so the tree and the tab and Files all use the one call: a file outside the home folder works, and a failure is reported instead of swallowed. `openinshell` stays registered (the CLI and agents may call it) and its Linux branch calls the shared function, so there is one place that knows how to reveal. The host's own `reveal_in_file_explorer` is for the host's local paths (installer and data folders) and is left alone apart from the same Linux D-Bus step through a shared helper if the crates can share it; if not, it is a follow-up and the behaviour difference is only on Linux.

**3.6 The Files pane.** It already has the row on both its file and folder menus through `revealInOs`, so it gets no new item. With §3.3 and §3.4 its Linux label and selection match.

## 4. Edge cases

1. Path with spaces, commas, `#`, `%` or non-ASCII characters: Windows passes it quoted (`explorer_quote`); macOS passes it as one argument; the Linux file URI is percent-encoded. Tests include each.
2. A network or UNC path on Windows: Explorer handles it; a missing share returns the "no longer on disk" toast.
3. Windows `explorer.exe` returns a non-zero exit code even when it works: the call is detached and never judged by the exit code, as today.
4. A symlink: the link itself is selected (the entry the person clicked), not its target.
5. A long-lived file manager must not inherit this instance's environment: the strict spawn policy covers it (I7).
6. Two quick right-click choices: each is its own call; the file manager decides how to focus.
7. No file manager installed (a minimal Linux): the fallback `xdg-open` fails; the toast says "No file manager could be opened".
8. A confined pane (a container agent's workspace): refused with the existing refusal text, shown as a toast.
9. A document on the host's "Large text" viewer or any other viewer: same row, since the menu is per tab, not per viewer.
10. Keyboard: the row is reachable from the tab's context menu key. No new shortcut (a shortcut would add a row to remember; the tab menu is enough).
11. Accessibility: a menu row has the platform label as its name; the disabled remote row has its reason as the sublabel, which is read with it.

## 5. Tests

- **Menu (vitest, `editor-tab-reveal.test.ts`):** a saved local tab has the row and choosing it calls `FsRevealCommand` with the tab's path and the pane's block id; a scratch tab has no row; a remote tab has the row disabled with its reason and calls nothing; a rejected call shows the toast and does not throw.
- **Label:** `file-manager-label.test.ts` gains Linux "Open Containing Folder" for a file and "Open in File Manager" for a folder; the existing host popover test is updated.
- **Session test (`editor-view.session.test.tsx`):** a right-click on a document tab in the real view lists the row.
- **Rust (`srv-core fs_ops/platform.rs`):** the Linux command builder with an injected runner: D-Bus success selects and never runs `xdg-open`; D-Bus failure, missing binary and timeout each fall back to `xdg-open <containing folder>`; the file URI encoding for spaces, `#`, `%`, `,` and non-ASCII; Windows and macOS argument tests stay.
- **Tree:** the tree's reveal rows call `fs.reveal` and a path outside home now works (`editor-tree-menu` test).

## 6. Plan and size

One PR (build #47956), about one day.
1. Shared reveal function with the Linux D-Bus step and fallback; tests.
2. Label helper change; update the labels' tests.
3. The tab row (`editor-tab-reveal.ts`, wired in `EditorTabBar`); the tree's two rows to `fs.reveal`.
4. Docs: `keybindings/tips.ts` has no shortcut to add; the Editor help gets one line if it lists tab menu items.
5. Look checks on macOS and Linux (the label, and that the file really is selected in Finder and in a Linux file manager); merge on muxreview approval.

Files: `view/editor/editor-view.tsx`, new `editor-tab-reveal.ts`, `editor-tree-menu.ts`, `editor-model.ts` (the old `revealInExplorer`), `util/file-manager-label.ts`, `statusbar/host-popover-state.ts`, `crates/srv-core/src/backend/fs_ops/platform.rs`, `crates/srv/src/server/editor_handlers/file_io.rs`.

Not in scope: dragging files out of the app (#47953), a reveal for a remote file (it would need the file copied here first), a keyboard shortcut.

## 7. Decisions for the operator

1. **Linux label:** "Open Containing Folder" everywhere the label shows (recommended: it is honest about what Linux does and is VS Code's name), versus keeping "Reveal in file manager" on Linux.
2. **A remote document's row:** shown disabled with the reason (recommended), versus hidden.
3. **The tree's reveal rows move to `fs.reveal`** so a file outside home works and failures are shown (recommended), versus changing only the new tab row and leaving the tree as it is.
