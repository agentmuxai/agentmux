# Plan — ironing out the kinks found while verifying the shortcuts

**Date:** 2026-10-10
**Status:** active — work split across three platforms (#4607); owner decisions in §5.
**Author:** AgentA@Area54 (Windows), for Masty@starpower (macOS) and Maricon@charlie (Linux)
**Builds on:** [PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md](PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md) (#4591, #4593, #4603). That plan's phase 4 (per-platform rebinds and Help fixes) stays there; this one covers what went wrong *around* it.

## 1. Why

The first cross-platform run of `scripts/verify-shortcuts.mjs` found bugs, which #4603 fixed. It also turned up kinks that aren't single bugs:
- App API behaviour that surprises an agent;
- a macOS gap users share;
- a crash whose cause isn't confirmed;
- dev-environment friction that cost each of us time.

This plan turns them into work packages, one owner each, sized for one PR apiece.

## 2. The kinks

| # | Kink | Kind |
|---|---|---|
| K1 | A pane with tabs has two block ids (the layout node's and each tab's). `target` can name a hidden tab: RunCommand runs there, PressKeys is refused | App API |
| K2 | RunCommand/PressKeys reach only panes in the window's active tab; switching tabs means guessing `tab:goto:N` | App API |
| K3 | Three table commands act beyond their pane: `files:openInNewTab` (a PDF opens in its OS app unasked; programs ask first), `term:paste` (a line break in the clipboard runs in the shell), `files:mention` (writes into an agent's message box) | App API safety |
| K4 | `files:trash` can't be undone on macOS (restore isn't implemented), for users too. #4603 refuses it to agents there | macOS gap |
| K5 | The Files pane's stale `<Show>` crash: fixed in #4603 by reading a memo instead of the `<Show>` accessor. Confirmed gone on Linux (Maricon's rerun, 3 of 3 rounds); macOS rerun pending. It never reproduced in jsdom or on Windows | Root cause inferred |
| K6 | Dialogs handle Escape themselves, so `app:escape` rarely resolves; PressKeys can't report it | App API reporting |
| K7 | Ctrl+F in a focused terminal goes to the shell on Windows/Linux, by design; the Help pane doesn't say so | Help accuracy |
| K8 | Ctrl+Tab in an editor with one document falls through to the global `tab:next` | Behaviour (by design) |
| K9 | `window:new` windows persist and are restored on the next launch, with every pane a run created | Test hygiene |
| K10 | Every new tab's default layout includes the agent picker, often focused: one Enter away from launching an agent | UX / agent safety |
| K11 | Pre-warmed pool windows look like app windows over DevTools ("AgentMux"; visible size on Windows) | Tooling |
| K12 | A dev instance launched by an agent stops when the shell job that launched it does; the MCP `Shell` launch of `dev-agent.cmd` exited 200 at once (TITLE with spaces and `#`, unverified) | Dev env |
| K13 | Dev pages reloaded mid-run, twice right after terminals were created; Vite already ignores `dist/`, `target/`, `*.md`, `*.json`, so the trigger is unknown | Dev env |
| K14 | `npm install` in `task dev` rewrites `package-lock.json` with another npm's flags; it slipped into #4603 | Dev env |
| K15 | The verification script changed files outside its scratch folder on the first runs (folders created and trashed a few levels up) | Process |
| K16 | `editor:saveAs` notes the key as resolved before finding that Save As doesn't apply (non-scratch tab), so PressKeys reports a false pass | App API reporting |
| K17 | One `--editor` pane can't cover every editor row: a Markdown file opens in preview, where save, Save As and find don't apply, but `editor:togglePreview` needs Markdown | Script setup |

Agent-side tooling notes (heredoc quoting in the shell wrapper, Node's WebSocket exit assertion on Windows, `gh pr merge --delete-branch` switching the checkout, cargo and a dev build sharing the target lock) aren't AgentMux bugs. They go into each agent's own notes, not this plan.

## 3. Work packages

### Masty@starpower (macOS)

- **M1. Restore from the Trash on macOS** (K4).
  - `crates/srv/src/backend/fs_ops/trash_worker.rs`: the macOS `restore_paths` returns "isn't supported".
  - Trash through `NSFileManager.trashItemAtURL`, which returns the item's new URL, and keep that per operation. Then restore moves it back, refusing on a name collision like the other platforms.
  - `files:undo` after `files:newFolder` also moves the folder to the Trash (Masty's rerun), so it hits the same gap; restore covers it.
  - When it lands, lift #4603's macOS refusal of `files:trash` (`frontend/app/keybindings/app-api.ts` `API_REFUSED_MAC`, `crates/srv/src/server/ui_shortcuts.rs`).
  - Tests: trash then undo in a temp dir, run on macOS.
- **M2. L3 on macOS**, once the owner grants Accessibility.
  - Press ⌃⇧Arrow (`pane:focus:*`; Mission Control ids 34/35/80/82), the Files pane's ⌘A/C/X/V/Z against the Edit menu, and the fn+arrow Page keys.
  - Results go into the shortcuts plan §9. Rebinds that follow belong to that plan's phase 4.

### Maricon@charlie (Linux)

- **L1. Stale `<Show>` crash** (K5). Confirmed gone on Linux after #4603.
  - Done on Linux; Masty's macOS rerun closes it.
  - If it comes back anywhere: log the stack with Solid's dev build, and bisect which `<Show>` accessor is read after unmount (`RenameInput`, the confirm and context-menu `<Show>`s, `GridTile`).
  - Fix at the reader, with a jsdom test that reproduces the order of updates.
- **L2. Temp-tree guard for the script** (K15).
  - `verify-shortcuts.mjs --files-mutate` creates its own temp tree, at least three levels deep, and points the Files pane there.
  - It refuses to run disk-changing rows unless the pane's path is under that tree, rechecking before each one.
  - Script robustness (Masty's rerun): give every DevTools call a timeout, and fire `closeWindow` without awaiting its reply; a full macOS run hung 7+ min waiting on a window that closed first. Say in the header that `files:copy`/`cut` write the system clipboard, or gate them like the disk rows.
  - Editor rows (K17): take a second editor pane on a plain-text file (`--editor-text`) for save, Save As and find, and keep `--editor` on a Markdown file for the preview toggle.
- **L3. Injected L3 pass on Linux** (`ydotool`), following the safety rules in the shortcuts plan §6:
  - skip keys the grab report lists as taken;
  - focus the dev instance first;
  - restore the window and workspace afterwards.

### AgentA@Area54 (Windows, App API)

- **A1. `target` names a pane, whichever tab it shows** (K1).
  - `focus(target)` resolves a pane tab's id to its layout node and makes that tab the visible one.
  - ListShortcuts, Layout and WhoAmI document which id to pass.
  - Tests in `app-api.test.ts`, plus a Files pane tab case.
  - On macOS every Files key failed L2 with "didn't take keyboard focus" (Masty's #4603 rerun). `giveBlockFocus` lands the caret a few frames late, and `plan` checked at once, so `plan` now waits up to 500 ms for it.
- **A2. Reach a pane in another tab of the agent's window** (K2), if the owner agrees (§5, D1): RunCommand/PressKeys switch to the tab that holds `target`, still in the agent's own window.
- **A3. Commands that act beyond their pane** (K3): applies the owner's choice per command (§5, D2), in the page and srv, with tests and the tool descriptions updated.
- **A4. Help pane notes** (K7):
  - a short "not while a terminal has focus" on `pane:find`'s Ctrl+F row;
  - PressKeys reports when a dialog handled the key itself (K6), so the result isn't "resolved nothing".
  - The editor's Save keys note the key only when the command applied (K16).
- **A5. Pool windows identify themselves** (K11): pool pages set a distinct document title or a marker DevTools clients can filter on, and the script uses it.
- **A6. Dev-environment fixes** (K12–K14), one small PR each:
  - `npm ci` instead of `npm install` in the dev path when `package-lock.json` hasn't changed;
  - log Vite's full-reload reason in dev;
  - a supported way for an agent to run a long-lived dev instance (and the TITLE quoting in `dev-agent.cmd`);
  - terminals start in the user's home, not `dist/cef-dev*`.
- **A7. New-tab default** (K10): per the owner's choice (§5, D3).

K8 (Ctrl+Tab fall-through) and K9 (restored windows) need no product change: the script now closes the windows it opens, and K8 is documented behaviour.

## 4. Order and protocol

1. **Unblocked now:** M1, L1, L2, A1, A4, A5, A6.
2. **After the owner's decisions:** A2, A3, A7.
3. **After the owner's L3 setup:** M2 and L3.

Each PR:
- names its work package in the title (`M1`, `L2` …);
- links this plan;
- reruns `verify-shortcuts.mjs` on its own platform and posts the matrix.

Changes to the App API or the shortcut table also get a run on the other two platforms before merge, as in the shortcuts plan §6. When a package lands, its owner ticks it here in a small docs commit (§6).

## 5. Decisions for the owner

**Decided 2026-10-10.** The owner: "we want to give agents powerful tools, they are first class owners in agentmux, so they have widespread perms, but with protections". So the rule is protections, not refusals; refuse only what has no protection.
- **D1: yes.** RunCommand/PressKeys may switch tabs within the agent's own window to reach a pane. The window stays the boundary.
- **D2: allow, with protections.** All three commands run for agents:
  - `term:paste` is refused only when the clipboard has a line break and the shell hasn't turned on bracketed paste, because then the line would run. PressKeys' paste key gets the same guard.
  - `files:openInNewTab` and `files:mention` are allowed, and their tool descriptions say what they do.
  - Going beyond the question: RunCommand's `pane:close` no longer refuses. It needs `target` and closes that pane the way ClosePane does: at once if the agent is in it; otherwise after the 15 seconds its user has to keep it.
  - `files:deletePermanently` stays refused. It can't be undone, and an agent could click its own confirmation.
- **D3: yes.** A new tab keeps its layout but doesn't give the agent picker keyboard focus.

The questions as asked:

- **D1 (K2).** May RunCommand/PressKeys switch tabs within the agent's own window to reach a pane? Proposed: yes. It stays inside the window, as decided before (shortcuts plan §8.1).
- **D2 (K3).** For `files:openInNewTab`, `term:paste` and `files:mention`: refuse them to agents, or allow them and say what they do in the tool description?
  - Proposed: refuse `term:paste`. Its effect depends on whatever is on the clipboard.
  - Proposed: allow the other two and describe them. Programs already ask before they run, and a mention is visible in the message box before anything is sent.
- **D3 (K10).** What should a new tab show? Proposed: keep the layout, but don't give the agent picker keyboard focus when a tab opens.

## 6. Status

| Package | Owner | Status |
|---|---|---|
| M1 Trash restore (macOS) | Masty@starpower | done (#4610) |
| M2 L3 macOS | Masty@starpower | waiting for Accessibility |
| L1 Stale `<Show>` | Maricon@charlie | done: gone on Linux and macOS (#4603 reruns) |
| L2 Temp-tree guard, script timeouts | Maricon@charlie | done (#4612) |
| L3 Injected L3 Linux | Maricon@charlie | done (#4624): on GNOME 50 Wayland all 92 keys pressed reached the app; the 4 `pane:swap:*` keys are taken by GNOME (move-to-workspace) and weren't pressed |
| A1 Pane-tab targets, caret wait | AgentA@Area54 | done (#4609) |
| A2 Other tabs | AgentA@Area54 | in review (#4634) |
| A3 Paste guard, `pane:close` via ClosePane | AgentA@Area54 | in review (#4626) |
| A4 Save-key reporting, dialog Escape | AgentA@Area54 | done (#4613); the Ctrl+F Help note moved to SPEC_HELP_HIDDEN_TIPS_2026_10_10 (#4616) |
| A5 Pool window marker | AgentA@Area54 | done (#4618) |
| A6 Dev environment | AgentA@Area54 | terminals start at home (#4621), full-reload reason logged (#4623); `npm ci` and launching a dev instance from an agent still open |
| A7 New-tab focus | AgentA@Area54 | in review (#4635) |
