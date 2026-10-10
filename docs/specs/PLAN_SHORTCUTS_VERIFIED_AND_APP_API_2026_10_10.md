# Plan — every shortcut in the Help pane works on every platform, and agents can drive them

**Date:** 2026-10-10
**Status:** proposed — nothing built yet.
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
| The OS or window manager takes the key first | Not measured. Likely candidates: ⌘M (minimize) and ⌘H (hide) on macOS; Ctrl+Arrow and Ctrl+Shift+Arrow (Mission Control, desktop switching); Ctrl+Alt+Arrow and Alt+Shift+Arrow on Linux desktops; Ctrl+Shift+0 (keyboard-layout switch on some Windows setups) |
| The host takes the key first | macOS standard menu items keep ⌘Z/⇧⌘Z/⌘X/⌘C/⌘V/⌘A, which overlap the Files pane's undo, cut, copy, paste and select-all. In a browser pane, Ctrl/⌘+Shift+N is consumed as `window:new` before the pane's "new incognito" can run |
| The key reaches the page but its row's context rule doesn't match | Not measured |
| Help shows something the table doesn't drive | The Help pane's own zoom keys are hardcoded (`helpview.tsx`) and ignore remaps; "Shift + drag" is hand-written; a meta key on Linux is labelled "Win" |
| A command that only works from its key | `term:copy`/`paste`/`clear` are no-ops in the global map (the terminal runs them itself); the command palette's registry diverges from the key handlers (`tab:close` skips the confirm dialog, `split:*` ignores `app:defaultnewblock`, most table ids aren't registered) |

**App API coverage:** about six table commands have an equivalent tool (`NewTab`, `SetActiveTab`, `SetName` for tab rename, `ClosePane`, partial splits through the open-pane tools). There is no tool that runs a shortcut's command or presses a key with modifiers: `BrowserDispatchKey` sends only Enter, Tab, Escape, Backspace, Space and the arrows, unmodified, into the agent's own pane. `FocusWindow` reorders the window list but appears not to raise the window (code trace only).

## 3. Three layers of verification

A shortcut works only if all three hold, and each needs a different tool:

| Layer | Question | How it is checked |
|---|---|---|
| **L1 Command** | Does the command do what the label says? | App API `RunCommand` (new, §4), then observe the result |
| **L2 Page key** | Does the key, once in the page, resolve to that command in that context? | App API `PressKeys` (new, §4): real key events with modifiers, injected through the DevTools protocol into the window |
| **L3 OS key** | Does the key reach AgentMux at all on this platform? | OS-level key synthesis on each partner's host: `SendInput` (Windows), `xdotool` (Linux, X11; `ydotool` on Wayland), `osascript`/System Events (macOS). Injected keys bypass some OS shortcuts, so a short manual pass by a human or partner confirms the suspected list in §2 |

L1 and L2 run unattended from a script against a dev instance (§5). L3 is the per-platform part the partners own.

## 4. App API additions

Three tools on the agent App API (`crates/mcp`), served by srv and carried out by the frontend, following the path `UIClick` already uses (srv `ui_handlers.rs` → CEF host → page):

- **`ListShortcuts`** returns the effective table for this platform: command id, label, category, keys as the Help pane shows them, context rule, and whether the row is pane-local. Agents read what the user sees.
- **`RunCommand(command, target?)`** runs a table command by id in the agent's window: global commands through the dispatcher's own `runKeyCommand` (not the separate command-palette registry), pane commands (`doctab:*`, `editor:*`, `files:*`, `term:*`) through the target pane's handler. `target` picks the pane (default: the focused pane) or tab. It returns whether the command ran and a short reason when it didn't ("no terminal focused", "fewer than two terminals").
- **`PressKeys(keys, target?)`** presses a key combination given in the table's syntax (`ctrl+shift+d`, chords as two keys) as real key events in the agent's window, after focusing `target`. It returns which command, if any, the dispatcher resolved, so a verification script can tell "key resolved to the wrong command" from "command misbehaved".

**Scope and safety** (owner decisions, §8):
- Act only in the window that holds the agent's own pane.
- Commands that destroy data or close what the user is working on (`files:trash`, `files:deletePermanently`, `tab:close`, `pane:close` of a pane the agent doesn't own) keep the confirmations a user sees, and closing someone else's pane keeps `ClosePane`'s 15-second undo.
- `PressKeys` only accepts combinations that appear in the table, plus plain text keys, so it can't type arbitrary shortcuts into other apps.

The command registry (palette) and the key handlers should run the same code for the same id. Part of this work routes the palette through `runKeyCommand` for every table id, so a fix lands in one place.

## 5. The verification script

`scripts/verify-shortcuts.mjs`, run against a dev instance (any platform):

1. `ListShortcuts` to get the rows for this platform.
2. For each row: set up its context (open the pane type a pane-local row needs, focus a terminal for `terminalFocus` rows, two terminals for `term:multiInput`), then:
   - L1: `RunCommand`, observe the effect through `Layout` / `UIQuery` (tab count, focused pane, zoom level, open modal, pane type);
   - L2: undo, then `PressKeys` with each listed key and check the same effect and the resolved command id.
3. Write a matrix (row × key × layer × result) as JSON and Markdown.

Each row's expected effect lives next to the script as a small check function. Rows whose effect can't be observed from the App API are listed as "manual" rather than passed.

## 6. Cross-platform protocol

| Host | Platform | Owner |
|---|---|---|
| Area54 | Windows 10 | AgentA |
| starpower | to be confirmed by Masty | Masty@starpower |
| charlie | to be confirmed by Maricon | Maricon@charlie |

We need macOS and Linux covered; if both partner hosts run the same OS, the owner picks a third.

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
   - the Help pane's zoom keys from the table; "Super" instead of "Win" on Linux;
   - pane commands runnable outside their key (`term:*`);
   - the browser pane's Ctrl/⌘+Shift+N conflict;
   - the macOS menu overlap with the Files pane.
   Each fix re-runs the matrix on all three platforms.
5. **Docs:** regenerate the docs site's shortcut page from the table (`keybindings/doc.ts`) and note platform exceptions.

## 8. Decisions for the owner

1. Agent scope for `RunCommand` and `PressKeys`: only the agent's own window (proposed), or any window of the instance.
2. Whether destructive commands are callable at all through `RunCommand`, or only with a confirmation the user answers.
3. When the OS takes a key: change our default on that platform (proposed), or keep it and mark it "taken by the OS" in the Help pane.

## 9. Results

To be filled in by phase 3.
