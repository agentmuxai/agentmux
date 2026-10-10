# Plan — every shortcut in the Help pane works on every platform, and agents can drive them

**Date:** 2026-10-10
**Status:** active — owner decisions recorded (§8); phase 2 (App API) in #4593, fixes from the first cross-platform run in #4603.
**Author:** AgentA@Area54 (Windows), with Masty@starpower and Maricon@charlie for the other platforms
**Builds on:**
- [../reports/REPORT_KEYBINDINGS_AUDIT_AND_CONSOLIDATION_2026_10_04.md](../reports/REPORT_KEYBINDINGS_AUDIT_AND_CONSOLIDATION_2026_10_04.md) (the audit, by reading code; phases 3–6 shipped in #4323–#4328)
- `frontend/app/keybindings/defaults.ts` (the one shortcut table the dispatcher, the Help pane, the host and the docs read)

## 1. Goals

1. **The Help pane is accurate on Windows, macOS and Linux.** Every key it lists does what its label says, on each platform, verified by pressing the key, not by reading code.
2. **Agents can drive every shortcut through the App API,** and the same API is how we verify the shortcuts: an agent can run a shortcut's command, press its keys, and check what happened.

## 2. What is true today

The Help pane's shortcut list is generated from the table (`keybindings/help.ts`), so it can't drift from the bindings. A shortcut can still fail to work:

| Kind of failure | Known or suspected cases |
|---|---|
| The OS or window manager takes the key first | Not measured by pressing yet. **macOS (read on starpower, §9):** ⌃Arrow (Mission Control, Spaces) is taken, and the hidden Shift variants may take ⌃⇧Arrow, our `pane:focus:*` on macOS; Magnet's defaults take ⌃⌥⌘←/→ (`pane:resize:left/right`); the F-row keys need fn at macOS's default setting. **GNOME 50 (read from gsettings on charlie):** Ctrl+Alt+Arrows (switch workspace) and **Ctrl+Shift+Alt+Arrows (move window to workspace), which is our `pane:swap:*` on every platform**; also Super+Alt/Shift+Arrows, Ctrl+Alt+Tab/Esc/D, Ctrl+Shift+Alt+R, Alt+Tab, Alt+\`, Alt+Esc, Alt+Space, Alt+F2/F4/F6/F7/F8/F10, Ctrl+Alt+F1–F12, Super+1–9 and Super+Ctrl+1–9, Super+Space and Super+A/V/M/S/N/H/D/P/Tab. GNOME does *not* bind Alt+Shift+Arrow (our Linux pane resize), Ctrl+Arrow or Ctrl+Shift+Arrow by default; KDE and others may. Windows: Ctrl+Shift+0 (keyboard-layout switch on some setups) |
| The host takes the key first | On macOS, AgentMux's own app menu takes ⌘H, ⌥⌘H and ⌘Q, and its standard edit items keep ⌘Z/⇧⌘Z/⌘X/⌘C/⌘V/⌘A, which overlap the Files pane's undo, cut, copy, paste and select-all. Minimize, Close Tab and Close Window have no key equivalent, so ⌘M (`pane:magnify`) and ⌘W (`pane:close`) reach the page. In a browser pane, Ctrl/⌘+Shift+N is consumed as `window:new` before the pane's "new incognito" can run |
| The key reaches the page but its row's context rule doesn't match | Not measured |
| Help shows something the table doesn't drive | The Help pane's own zoom keys are hardcoded (`helpview.tsx`) and ignore remaps; "Shift + drag" is hand-written; a meta key on Linux is labelled "Win" |
| A command that only works from its key | `term:copy`/`paste`/`clear` are no-ops in the global map (the terminal runs them itself; RunCommand reaches them through the terminal's own runner since phase 2); the command palette's registry diverges from the key handlers (`tab:close` skips the confirm dialog, `split:*` ignores `app:defaultnewblock`, most table ids aren't registered) |

**App API coverage:** about six table commands have an equivalent tool (`NewTab`, `SetActiveTab`, `SetName` for tab rename, `ClosePane`, partial splits through the open-pane tools). There is no tool that runs a shortcut's command or presses a key with modifiers: `BrowserDispatchKey` sends only Enter, Tab, Escape, Backspace, Space and the arrows, unmodified, into the agent's own pane. `FocusWindow` reorders the window list but appears not to raise the window (code trace only).

## 3. Three layers of verification

A shortcut works only if all three hold, and each needs a different tool:

| Layer | Question | How it is checked |
|---|---|---|
| **L1 Command** | Does the command do what the label says? | App API `RunCommand` (new, §4), then observe the result |
| **L2 Page key** | Does the key, once in the page, resolve to that command in that context? | App API `PressKeys` (new, §4): real key events with modifiers, injected through the DevTools protocol into the window |
| **L3 OS key** | Does the key reach AgentMux at all on this platform? | OS-level key synthesis on each host: `SendInput` (Windows); `osascript`/System Events (macOS); on Linux, `xdotool` on an X11 session only and `ydotool` on Wayland (kernel uinput, so the compositor's grabs apply; one-time root setup for uinput access). XWayland plus `xdotool` doesn't count: AgentMux is a native Wayland client there, and XTEST events skip the compositor's grabs. A short manual pass confirms the suspected list in §2 |

L1 and L2 run unattended from a script against a dev instance (§5). L3 is the per-platform part the partners own.

**L2 skips the host's menu.** A key injected through the DevTools protocol bypasses the macOS menu bar's key equivalents (and the host's own pre-key handling may differ), so a key the menu takes passes L2 and fails L3. The matrix records that as "taken by the host", not as a pass. On macOS, an editing key in a text field (copy, paste, select all) injected this way also needs CDP's `commands` field to act; the dispatcher's own key handlers don't.

**L3 safety, macOS (Masty, 2026-10-10), and the same rules on every host:**
- Keys go to whichever app is in front, and a host may run several AgentMux instances. The script brings the dev instance forward by process id (System Events: the process whose unix id is the dev PID; `wmctrl`/`xdotool --pid` on Linux; `SetForegroundWindow` on the dev PID's window on Windows), never by app name, and checks it is still in front before every key.
- Synthetic keys do trigger system shortcuts: a synthesized Ctrl+Arrow really switches Spaces. L3 needs the owner's go-ahead and an idle machine, and on macOS the Accessibility permission for the sending process, which only the owner can grant.
- The automated pass never sends ⌘Q, ⌘W, or any key that closes a tab, pane or window, or deletes files; those are checked by hand.

## 4. App API additions

Three tools on the agent App API (`crates/mcp`), served by srv and carried out by the frontend, following the path `UIClick` already uses (srv `ui_handlers.rs` → CEF host → page):

- **`ListShortcuts`** returns the effective table for this platform: command id, label, category, keys as the Help pane shows them, context rule, and whether the row is pane-local. Agents read what the user sees.
- **`RunCommand(command, target?)`** runs a table command by id in the agent's window: global commands through the dispatcher's own `runKeyCommand` (not the separate command-palette registry), pane commands (`doctab:*`, `editor:*`, `files:*`, `term:*`) through the target pane's handler. `target` picks the pane (default: the focused pane) or tab. It returns whether the command ran and a short reason when it didn't ("no terminal focused", "fewer than two terminals").
- **`PressKeys(keys, target?)`** presses a key combination given in the table's syntax (`ctrl+shift+d`, chords as two keys) as real key events in the agent's window, after focusing `target`. It returns which command, if any, the dispatcher resolved, so a verification script can tell "key resolved to the wrong command" from "command misbehaved". It also returns the physical modifiers it sent (on macOS the table's `meta` is ⌘, CDP's meta bit), so a Ctrl-for-⌘ mapping bug can't pass as a pass.

**Scope and safety** (owner decisions, §8):
- Act only in the window that holds the agent's own pane.
- Commands that destroy data or close what the user is working on (`files:trash`, `files:deletePermanently`, `tab:close`, `pane:close` of a pane the agent doesn't own) keep the confirmations a user sees, and closing someone else's pane keeps `ClosePane`'s 15-second undo.
- `PressKeys` only accepts combinations that appear in the table, so it can't type arbitrary shortcuts into other apps.

**As built (phase 2).** The frontend publishes `window.__agentmux_shortcuts` (`keybindings/app-api.ts`); the host's `list_shortcuts`, `run_command` and `press_keys` routes call it in the window that holds the agent's own pane, and srv resolves that pane from the agent's verified identity, so there is no `pane` argument. Each pane registers the function its own keys run (`registerPaneCommandRunner`), so `RunCommand` and the keys share one code path. `pane:close` and `files:deletePermanently` are refused outright, by srv and by the page: `pane:close` closes the focused pane at once, which may be someone else's, so agents use `ClosePane` (and its undo) or `QuitSelf`. Everything else that closes or destroys runs with the confirmation a user gets (`tab:close`, replacing a pane) or can be undone (`files:trash`, except on macOS, where restoring from the Trash isn't supported yet, so agents can't trash there either). `PressKeys` also refuses a key bound to a refused command anywhere in the table, whatever has focus, so ⌘W on macOS (`pane:close`, and `files:closeTab` in the Files pane) is a manual row. Its key events are `rawKeyDown`/`keyUp` with CDP's modifier bits and no text, so a shortcut never types its letter into a text box.

The command registry (palette) and the key handlers should run the same code for the same id. Part of this work routes the palette through `runKeyCommand` for every table id, so a fix lands in one place.

## 5. The verification script

`scripts/verify-shortcuts.mjs`, run against a dev instance (any platform):

1. List the rows for this platform (`ListShortcuts`' code path).
2. For each row, in the pane it needs (passed by block id: `--files`, `--editor`, `--media`, `--term`):
   - L1: run the command (`RunCommand`'s code path) and record whether it ran, or why not;
   - L2: press each listed key (`PressKeys`' code path) and check it resolved to that command.
3. Write the matrix (row × key × layer × result) as Markdown, or JSON with `--json`.

It talks to the page over the DevTools protocol directly, the same calls the host makes, so a partner can run it against a dev build without an agent in it. As built it checks resolution, not each command's visible effect; effect checks per row are a follow-up, and until then a row whose L1 "ran" is judged by eye when it matters. The grab reports, `scripts/gnome-grabs.mjs` (Maricon) and `scripts/mac-grabs.mjs` (Masty), are the read-only half of L3.

Rules for the script, since it runs on hosts where other agents and the owner are working:
- It talks to the dev instance whose DevTools port is in `AGENTMUX_CDP_PORT`; it never assumes the default port, which another agent's dev instance may hold.
- A dev build shows the owner's real agents, so it never launches or messages an agent. Terminal, editor, files, settings and help panes are fine.
- It skips the rows that close tabs, panes or windows or delete files, and lists them as "manual".

## 6. Cross-platform protocol

| Host | Platform | Owner |
|---|---|---|
| Area54 | Windows 10 | AgentA |
| starpower | macOS 26.5.2, Apple Silicon | Masty@starpower |
| charlie | Ubuntu 26.04.1, GNOME 50.1, Wayland (AgentMux as a native Wayland client); a VMware guest on a Windows host | Maricon@charlie |

On charlie, a manual press of a Ctrl+Alt or Super combination passes through VMware (Ctrl+Alt releases input) and the Windows host before GNOME, so those rows are flagged; `ydotool` runs inside the guest and isn't affected. `ydotool` is set up on charlie (2026-10-10, with the owner's approval).

**Injected L3 acts on the real desktop (Maricon, 2026-10-10).** `ydotool`, `xdotool`, `osascript` and `SendInput` send keys to whatever has focus, so on a key the OS takes, the OS acts for real: `pane:swap`'s Ctrl+Alt+Shift+Arrow would move the focused window to another workspace. Rules for the injected L3 pass (not written yet): skip by default the keys the grab report lists as taken, and record them as "taken by the OS" from that report; send them only with an explicit flag. Focus the dev instance first, refuse to send if that window doesn't have focus, restore the window and workspace afterwards, and run only when nobody is using the desktop.

For each PR that changes behaviour:
1. AgentA opens the PR with the Windows L1/L2/L3 results.
2. Each partner checks out the branch on its host, runs `task dev`, runs the script (L1, L2) and the OS-level pass (L3), and posts the matrix as a PR comment.
3. A row is done when it passes all three layers on all three platforms, or has a documented platform exception that the Help pane shows (a different key on that platform, or "not available").

## 7. Phases and PRs

1. **This plan.**
2. **App API:** `ListShortcuts`, `RunCommand`, `PressKeys`, with srv routes, the host and frontend handlers, and tests. Palette routed through `runKeyCommand`.
3. **Verification script** and a first full run on all three platforms. The results go into §9 of this plan.
4. **Fixes, from the results:**
   - rebind keys the OS takes, per platform;
   - the Help pane's zoom keys from the table; "Super" instead of "Win" on Linux (agreed by Maricon);
   - pane commands runnable outside their key (`term:*`);
   - the browser pane's Ctrl/⌘+Shift+N conflict;
   - the macOS menu overlap with the Files pane.
   Each fix re-runs the matrix on all three platforms.
5. **Docs:** regenerate the docs site's shortcut page from the table (`keybindings/doc.ts`) and note platform exceptions.

## 8. Decisions for the owner

**Decided 2026-10-10** (the owner: "proceed with best recommendations"):
1. `RunCommand` and `PressKeys` act only in the window that holds the agent's own pane.
2. Destructive commands keep the confirmation a user sees, and closing a pane the agent doesn't own keeps `ClosePane`'s 15-second undo. `files:deletePermanently` is not available through the App API at all (it can't be undone); `files:trash` is (it can).
3. When the OS takes a key, AgentMux changes its default on that platform. First case: `pane:swap:*` on Linux, with a replacement that avoids GNOME's grabs (§9).
4. The L3 setup (the Accessibility permission on starpower, `ydotool` with uinput access on charlie) is the owner's to do when a partner asks; L1 and L2 don't wait for it.

The questions as asked:

1. Agent scope for `RunCommand` and `PressKeys`: only the agent's own window (proposed), or any window of the instance.
2. Whether destructive commands are callable at all through `RunCommand`, or only with a confirmation the user answers.
3. When the OS takes a key: change our default on that platform (proposed), or keep it and mark it "taken by the OS" in the Help pane. The first known case: `pane:swap:*` (Ctrl+Alt+Shift+Arrow) is GNOME's "move window to workspace".
4. L3 setup that only the owner can do: the Accessibility permission for the sending process on starpower, and root setup of `ydotool` with uinput access on charlie. Both also need an idle machine, since injected keys act on the desktop.

## 9. Results

Filled in by phase 3. Early results:

**L3, taken by GNOME (charlie, Maricon, 2026-10-10, main @ `998175d3c`).** Every Linux key in the table (the first key of a chord) checked against the 150 accelerators GNOME 50 grabs on charlie: the window manager, mutter, the shell, media keys, and the enabled dash-to-dock and tiling-assistant extensions. **4 of 120 are taken, all `pane:swap:*`** (Ctrl+Alt+Shift+Arrow, `move-to-workspace-*`). The other 116 are free on stock GNOME, including every Alt+Shift+Arrow, Alt+Arrow and Ctrl+Alt+Shift+PageUp/PageDown row; the Linux table has no Super keys. A replacement for `pane:swap` on Linux should avoid Ctrl+Alt+Arrow and Ctrl+Alt+letter, which GNOME also takes; Alt+Shift+Arrow is free on GNOME but used by other desktops. This covers stock GNOME as configured on charlie, not KDE, Xfce or Sway, and not keys lost to an input method or VMware; injected L3 (`ydotool`) confirms it later. The check is a read-only script, `scripts/gnome-grabs.mjs`.

**L3, taken on macOS (starpower, Masty, 2026-10-10, main @ `998175d3c`).** Read-only: every macOS key in the table (117 keys over 98 rows, chords by their first key) against Apple's default system shortcuts with this Mac's overrides, AgentMux's menu bar, the window manager app running there (Magnet), and the F-row setting. **11 of 117 hit:**
- **The OS, suspected:** `pane:focus:*`, ⌃⇧Arrow. macOS has hidden Shift variants of the Mission Control and Spaces hotkeys (ids 34/35 for ⌃⇧↑/↓, 80/82 for ⌃⇧←/→; 80 and 82 are stored as enabled there). If they take the keys, focusing a pane by direction doesn't work on macOS at all, so this is the first key to press at L3. The plain ⌃Arrow keys are taken, but no macOS row uses them.
- **Magnet's defaults:** `pane:resize:left/right`, ⌃⌥⌘←/→ (next/previous display). ⌃⌥⌘↑/↓ are free. A Help-pane note rather than a rebind, since it's a third-party app.
- **The menu bar, as expected:** the Files pane's select-all, copy, cut, paste and undo (⌘A/C/X/V/Z) overlap the Edit menu. Chromium gives the page the key first, so the Files pane should win when it handles the key; L3 decides.
- **The F-row:** F1 (help), F2 (rename), F5 (refresh), F6/⇧F6 (next/previous pane) work there only because "Use F1, F2, etc. keys as standard function keys" is on. At macOS's default they need fn. Help has ⌘/ as well; rename, refresh and pane cycling have no macOS alternative yet.

Free on macOS: everything else, including ⌘M and ⌘W, ⌘T ⌘N ⌘D ⇧⌘D ⌘[ ⌘] ⌃Tab ⌃⇧Tab ⌘1–9, the ⌃⇧S chords, `pane:swap` (⌃⌥⇧Arrow) and zoom. Not covered: keys lost to input methods, and ⌃PageUp/PageDown and ⇧⌘PageUp/PageDown, which a keyboard without Page keys types as fn+Arrow; macOS 15+ window tiling owns fn⌃Arrow in the Window menu. Those need a manual press. The check is `scripts/mac-grabs.mjs`.

**L1/L2, first run on all three platforms (2026-10-10, #4593).** macOS (starpower, Masty), Linux (charlie, Maricon) and Windows (Area54, AgentA: global, Files and terminal rows; no editor pane). Every global row passes both layers on every platform, apart from rows whose context the script didn't set up: `agent:focusComposer` needs an agent pane, `pane:find`'s Ctrl+F goes to a focused terminal's shell by design (Ctrl+Shift+F is the terminal's find, and passes), and `app:escape` needs something open. The ⌃⇧Arrow focus and ⌃⌥⌘←/→ resize keys pass on macOS, as expected, because CDP skips the OS; those are L3 questions. The editor, document-tab and terminal rows pass. No platform-specific L1/L2 failure was found. Found and fixed in the follow-up:
- **Stale-read crash in the Files pane.** Both partners hit Solid's stale `<Show>` read from `runFilesCommand` → `setSelection` (`files:back` on macOS; `files:selectAll` after the full Files sequence on Linux), after which later updates kept failing until a reload. The rows read their entry through the `<Show>` callback accessor. A listing that drops a name in the same update as a selection change (the re-list prunes the selection in that batch) can re-run a row after its condition went false, and that read throws. Rows now read a memo that keeps the last entry. jsdom doesn't reproduce the ordering, and neither did the Windows run (back, forward and up pass L1 there), so the partners' rerun confirms it.
- **PressKeys left the caret behind.** `focus`/`plan` moved layout focus only, so a key went to the previously focused element: on Linux `editor:togglePreview`'s Ctrl+Shift+V reached the terminal. They now move the caret as a click does (`refocusNode`), and `plan` refuses if the target didn't take it. On Windows that guard also caught the script pressing keys into a Files tab that `files:newTabHere` had hidden behind a new one; that row now runs last.
- **`pane:refocus` (⌘I)** resolved nothing at L2 when a terminal had focus: it was the one pane row without `skipShell`, so the terminal sent ⌘I to the shell.
- **`files:trash` can't be undone on macOS** (restore isn't supported there yet), contrary to §8.2's reason for allowing it; it is now refused to agents on macOS, in the page and in srv.
- **The script** attached to a hidden pool window, ran pane rows after global rows had switched tabs, stopped at the first thrown row, timed L2 with a fixed 80 ms, and let `files:up` move the pane out of its folder before the rows that create and trash. On both partners' hosts that put two or three throwaway "New folder" entries above the scratch folder and then trashed them; nothing else changed. It now picks a shown window that holds the given panes, runs pane rows first, records a throw as that row's failure, polls for up to 500 ms, runs the disk-changing Files rows only with `--files-mutate` and before back/forward/up, creates a second document before the document-cycling rows, checks `app:escape` on a confirmation dialog, closes the windows `window:new` opens, and leaves `term:paste` (a line break in the clipboard would run in the shell), `files:openInNewTab` (opens the selected files in their apps) and `files:mention` (writes into an agent's message box) to a person. A dev build reloads its pages whenever a file in the checkout changes, so the script waits up to about 30 s for the shortcut API.
