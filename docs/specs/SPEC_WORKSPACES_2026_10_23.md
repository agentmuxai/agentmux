# SPEC: Workspaces — a named set of windows, saved as you go

**Date:** 2026-10-23
**Status:** active — a running spec, refined with the operator. PR 1 (§3, §3.1) shipped in #5357; PR 2 (the saved Default Workspace, §2, §4, §5 header, §8) in #5358; PRs 3–5 not started. Supersedes the user-facing parts of `SPEC_OFFLINE_RESILIENCE_2026_10_20.md` §4.7 as built in PR #5356 (unmerged), and the naming decision of `SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13.md` §3.
**Trigger:** Operator, 2026-10-23, reviewing #5356 ("closing a window keeps its workspace"): *"remove the entries from the hamburger that already have widgets … change the "layouts" to "Workspaces". A workspace to the user (lets define it in the glossary) is a set of windows/panes. Opening a workspace necessarily means closing the existing one. … Start with Default Workspace and it is a running file. Users can also create New Workspace. … if he closes that window, it becomes "greyed" out in the status panel. We need a reopen and delete buttons …"*
**Related:** `SPEC_LAYOUT_FILES_2026_09_25.md` (the `agentmux.layout` file this extends), `SPEC_CONTINUOUS_SESSION_PERSISTENCE_2026_09_08.md` (proposed, unbuilt: continuous session record), `SPEC_OFFLINE_RESILIENCE_2026_10_20.md` G8 ("closing is not deleting"), `SPEC_VERSION_INSTANCE_PANEL_2026_04_25.md` (the status bar panel).

## 0. TL;DR

- **A Workspace is a named set of windows**, each with its tabs and panes. Exactly one is open at a time. Everyone starts in **Default Workspace**.
- **It saves itself.** Every change to its windows, tabs, panes and their arrangement is saved inside AgentMux about a second after it happens (§4). There is no Save button. **Export…** writes a workspace to a file to keep or share; **Import…** opens one.
- **Closing a window doesn't remove it.** The window stays in the workspace, greyed out in the status bar panel, with a **reopen** button. An open window's row has a **delete** button that removes it from the workspace. One slot per row (§5).
- **Opening or creating a workspace closes the current one** (its agents stop, everything is saved), then opens the other one's windows (§6).
- **☰ → Layouts becomes ☰ → Workspaces**, and Settings, Connectors, Memory and Toolchain leave the ☰ menu (§3).
- One mechanism replaces the four that overlap today: the session record (`session:last_topology`), #5356's closed-window list, the other-build offer (#5236) and layout files (§2).

## 1. Today (measured on `main` @ `570503c4b`)

### 1.1 Four overlapping ways panes come back

| Mechanism | What it keeps | When it's written | How you get it back |
|---|---|---|---|
| Session record, `Client.meta["session:last_topology"]` (#2560) | Every window open at a quit, or the last window closed | Only at quit, last-window close, and OS shutdown (`saved_for`: `quit`, `last-close`). Never while running | Automatically at the next launch, rebuilt with new pane ids |
| Closed-window list, `session:closed_windows` (#5356, **unmerged**) | Up to 10 closed windows' workspaces | At each close | ☰ → Reopen closed window |
| Other-build offer (#5236, shipped) | Another build's session record | — (reads other builds' databases) | A notice on first launch; ☰ → Layouts → Restore layout from another build… |
| Layout file, `*.agentmux-layout.json` (#3787, #3825, #3849) | One window, saved on purpose | ☰ → Layouts → Save layout… | ☰ → Layouts → Open layout… |

On `main`, closing a window that isn't the last deletes its contents for good (`window_close.rs` → `delete_workspace`). That is the bug G8 was written for.

`SPEC_CONTINUOUS_SESSION_PERSISTENCE_2026_09_08.md` proposed writing the session record continuously (1 s debounce, 30 s ceiling). It is still `Proposed`: only the layout tree autosaves (100 ms debounce, `frontend/layout/lib/layoutPersistence.ts`), and only into the live database.

### 1.2 "Workspace" already means four things

| Where | Meaning |
|---|---|
| srv object model: `Window → Workspace → Tab → Block` | One window's contents. One per window; the window title and the status panel fall back to its `name` |
| An agent's working folder (`AGENTMUX_AGENT_WORKDIR`, the agent pane's context cards: "Outside the workspace …") | The agent's folder under `~/.agentmux/agents/` |
| Global Memory entries | The `# [Workspace] <name>` heading AgentMux composes them under |
| Codex sandbox mode | "Workspace write" |

The 2026-08-13 spec ruled "Workspace" out as a user-facing word for this reason (§3 there). The operator has now chosen it (2026-10-23); §7 says how the other meanings stop competing with it.

### 1.3 The ☰ menu today

New Tab · New Window · *(Reopen closed window, #5356)* · Theme · Opacity · **Layouts** (Save layout…, Open layout…, Restore layout from another build…) · Settings · Command Palette · **Connectors** · **Memory** · **Toolchain** · DevTools · Online Docs · Exit.

Widget bar defaults (`crates/srv/src/config/widgets.json`): Connectors and Memory are pinned; **Toolchain and Settings are not pinned** (they are in the widget bar's + menu). Settings also has `Ctrl+,` / `Cmd+,` and the palette.

## 2. Model

```
Workspace "Default Workspace"            ← one open at a time, per instance
├── Window "Main"        open            ← tabs, panes, placement, name, appearance
├── Window "Reviews"     open
└── Window "Scratch"     closed (greyed)  ← kept, agents stopped, reopenable
```

- **Workspace:** id, name, its windows, which window was focused, timestamps. Stored inside the instance, in its own database (§4.2).
- **Window in a workspace:** the window's tabs, split trees and panes (with their ids and per-view settings), placement, name, appearance, and a state: `open` or `closed`.
  - **open:** shown now (or, after a quit, reopened at the next launch).
  - **closed:** the user closed it. Its agents and shells were stopped; its panes are kept with the same ids, so reopening resumes each agent's conversation.
  - **deleted:** gone from the workspace and its saved record. Its agents are not deleted (they stay in My Agents); only the panes go.
- **Quit is not close.** Quitting leaves every window's state as it was: what was open reopens at the next launch. Closing the last open window quits, and that window also stays `open`.
- **Crash:** the saved record is at most a moment old (§4), so the next launch reopens the workspace from it, every window included (today secondary windows come back as fresh windows after a crash).

### 2.1 Mapping onto the code

The srv per-window `Workspace` object stays as the runtime container of one window's tabs (renaming it in code is §9 Q6). A user Workspace is new: a record listing the windows it holds. While it is open, its open windows are live srv objects; its closed windows are kept (as #5356 already keeps them: same tab, block and layout rows, agents stopped). The workspace record is saved from those objects (§4); the next launch or a crash recovery rebuilds from the record when the rows aren't there, and a new version's first launch copies the records from the previous version (§4.2).

## 3. The ☰ menu

**Remove:** Settings, Connectors, Memory, Toolchain. Each stays reachable from the widget bar, the command palette, and (Settings) its shortcut.

### 3.1 Default pinned widgets

Operator, 2026-10-23: the widget bar's default pins become **Agent, Swarm, Memory, Connectors, Hangar, Browser, Settings, Help**, in that order (`display:pinned` and `display:order` in `crates/srv/src/config/widgets.json`).

| Widget | Today | New |
|---|---|---|
| Agent, Swarm, Memory, Hangar, Connectors, Browser, Help | pinned | pinned (reordered) |
| Settings | unpinned (order 99) | **pinned**, before Help |
| Kanban, Terminal, Editor, Messengers, Sysinfo | pinned | **unpinned**: in the widget bar's + menu |
| Drone, Warden, Vault, Toolchain, Tower, Discord, Slack, Telegram, WhatsApp, Teams | unpinned | unpinned |

Only the defaults change: a user who has customized the bar (`widget:pinned` in settings) keeps their choice; Settings → Widgets → Reset brings the new defaults. The docs that say Kanban is "pinned to the widget bar by default" (glossary `kanban`, the Kanban page) change with it.

### 3.2 Workspaces submenu

**Rename Layouts → Workspaces**, with:

| Item | Does |
|---|---|
| **New Workspace** | Asks for a name (prefilled "Workspace 2", …), closes the current workspace (§6), opens one window with the default panes |
| **Open Workspace** › *(list, most recent first, current one checked)* | Closes the current workspace and opens the chosen one |
| **Import…** | Opens an exported workspace file, or an old layout file, as a new workspace (§8) |
| **Export…** | Writes the current workspace to a file you choose (what Save layout… does today, but every window). A snapshot: later changes aren't written to it |
| **Rename workspace…** | Also by double-clicking the name in the status panel |
| **Delete workspace…** | Confirms, then deletes another workspace's file. The open one can't be deleted; switch away first |

"Restore layout from another build…" goes away for upgrades: a new version copies the previous version's workspaces on its first launch (§4.2). Bringing a workspace over from another channel's build stays possible through #5236's offer and through Export… / Import…. #5356's "Reopen closed window" and "Close workspace…" don't ship; the status panel does both jobs (§5).

## 4. Saving

### 4.1 When

**What comparable apps do** (researched 2026-10-23; sources at the end of this section). Every mature tool saves workspace state continuously, in the background, without a Save action. The ones that rely on long timers or on the quit alone are where people lose work.

| Product | Trigger | Where |
|---|---|---|
| Firefox session restore | On change, at most every 15 s (`browser.sessionstore.interval`) | `recovery.jsonlz4`: temp file + rename, plus `recovery.baklz4`, `previous.jsonlz4` and upgrade backups; closed windows kept in the same file |
| Chrome | Continuous append-only log | `Sessions/`; 25 recently closed windows/tabs |
| VS Code | Storage batches writes for 100 ms, flushes when idle (~60 s) and on shutdown; hot exit on quit | `.code-workspace` (folders and settings only) + an internal per-workspace DB with a `.backup` |
| JetBrains | On idle, on focus loss, on run/VCS/close/quit | `workspace.xml` in the project, written by back up → write → fsync |
| Windows Terminal | Continuously "to assist with restoration from crashes", and on quit | `state.json`; saving buffer contents too has hung startup (microsoft/terminal#20765) |
| Zellij | Every 60 s by default | A readable layout in the cache folder; restored commands wait behind "Press ENTER to run" |
| tmux-continuum | Every 15 min | Snapshot files; the slow interval is its main complaint |
| Apple HIG / GNOME HIG | "Avoid making people take an explicit action to save"; "if something can be done automatically, do it automatically" | — |

**Decision: continuous, debounced, flushed at every boundary.**
- Any change to the workspace's windows, tabs, panes, arrangement, placement or names marks it dirty. It is written **1 s after the last change**, and **at least every 15 s** while changes keep coming (a long drag-resize), Firefox's ceiling.
- Written at once, without waiting, on: window close, workspace switch, quit, OS shutdown, and the app losing focus.
- A write with nothing changed is skipped. Writes are serialized: one in flight, the next one queued.
- The quit-time write is a last flush, never the only one: on a crash, a forced quit or a fast macOS quit it doesn't run.

**As built (#5358).**
- Changes are counted by the store's own writes to windows, a window's contents, tabs, layouts and blocks (`LayoutWrites`), so no writer is missed; the saver looks every 250 ms.
- A change to blocks' settings alone (an agent pane writes them as it works) is saved 30 s after the first instead of 1 s after the last; any change to the arrangement keeps the 1 s / 15 s rule. Close, quit, OS shutdown and focus loss (`workspace.Flush`) save at once either way. A save that would store what is stored already is skipped.
- While a quit is closing its windows one by one, saves wait, so the quit's record of every window isn't replaced by the ones still open.
- Windows a cold start restored but the main window hasn't opened yet (`session:restored_windows`) are part of every save.
- The quit record (`session:last_topology`) is still written as before, for one release; a cold start restores the newer of it and the workspace record.

**What is saved:** the arrangement and each pane's settings, plus each agent pane's agent and session id, so it resumes. **Not saved:** running processes, terminal scrollback (Windows Terminal's experience), an agent's conversation (it lives in the agent's own history).

**Restoring doesn't run anything silently.** Agent panes resume their own sessions, as a crash recovery does today. A terminal pane comes back as a shell in its folder; a command it was started with is shown, not run, until the user presses Enter (Zellij's rule, and what Open layout… already offers).

Sources: wiki.mozilla.org/Firefox/session_restore; code.visualstudio.com/docs/editing/workspaces/workspaces; jetbrains.com/help/idea/system-settings.html; learn.microsoft.com/windows/terminal/customize-settings/startup; zellij.dev/documentation/session-resurrection; github.com/tmux-plugins/tmux-continuum; developer.apple.com/design/human-interface-guidelines/patterns/file-management; developer.gnome.org/hig/principles.html.

### 4.2 Where and how

**Decision (operator, 2026-10-23): internal store, file only as export.** No running file: the live state isn't a file the user sees or edits, as in VS Code, Sublime and JetBrains, which keep it internal and offer a file only to export or share.

- **Where:** a `db_workspaces` table in the instance's own `objects.db` (`channels/<channel>/versions/<v>/data/db/`). **Instances stay independent** (operator, 2026-10-23): no workspace state is shared between two running AgentMux instances. Today the same channel and version is already one instance (a second launch opens a window in the first, `crates/launcher/src/hash.rs`); two versions of one channel can run side by side, and each keeps its own workspaces.
- **Upgrades:** the first launch of a new version copies the workspaces of the channel's most recent other version, once, reading that database only (as #5236 already reads another build's session record). After that the two versions' workspaces are separate.
- **What a row holds:** the workspace's id and name, and its windows in the session record's shape (`session_windows::snapshot_session`: one entry per window, its tabs, panes with their settings, split trees, placement, name and appearance), so the restore that already brings back a quit's windows (`restore_all`) restores it. Export (PR 4) writes it as an `agentmux.layout` document (`SPEC_LAYOUT_FILES_2026_09_25.md` §3) with each window's `state` (`open`/`closed`).
- **One transaction per save** (SQLite WAL), so a save is all or nothing.
- **Backups:** the last record that loaded cleanly is kept as the row's `previous`, replaced only after a clean load, so a bad save never overwrites the good copy. A copy is also kept before each upgrade.
- **Loading falls back:** the record, then `previous`, then the pre-upgrade copy, then a fresh default window. When a fallback was used, a notice says so and which copy was used.
- **Single writer:** the instance's srv saves it (from the live objects, as `layout_file::export_window` reads them). No other instance writes there.
- **Export…** writes the document to a `*.agentmux-workspace.json` file, without the machine-local `resume` references unless asked (as Save layout… does now). **Import…** reads one, or an old `*.agentmux-layout.json`.

## 5. The status bar panel

The version chip's panel (`frontend/app/statusbar/InstancePanel.tsx`) heads its window list with "This process — N windows". It becomes the workspace's:

```
Default Workspace — 3 windows                    ✎
● Main            this        [opacity]   🗑
○ Reviews                     [opacity]   🗑
◌ Scratch         closed 14:02             ↺      ← greyed
+ Open another window
```

- **Header:** the workspace name and its window count. Double-click (or ✎) renames the workspace.
- **Rows:** every window in the workspace. Open windows as today (click focuses, double-click renames, opacity). Closed windows greyed, with when they were closed; clicking a closed row reopens it.
- **One action slot per row:**
  - **open → delete** (trash icon, "Remove from workspace"). Closes the window and removes it from the workspace. If an agent in it is mid-turn it asks first. The only open window can't be deleted (the slot is disabled, "The last window; use New Workspace to start over").
  - **closed → reopen** (rotate-left icon, "Reopen"). Opens it where it was; its agents resume. Right-click → **Remove from workspace** deletes it without reopening it (§9.1).
- Floating panes and Maintenance stay as they are.

## 6. Switching workspaces

Opening another workspace (or creating one):
1. If an agent in the current workspace is mid-turn, ask first, naming them ("Korp and Loap are working. Close Default Workspace?").
2. Save the current workspace, stop its agents and shells, and close its windows. Nothing is deleted.
3. Open the other workspace's `open` windows. The window the user acted from is reused for its first window, so the screen doesn't flash empty.
4. Remember it as the last open workspace; the next launch opens it.

## 7. Terminology and the glossary

**User-facing:** "Workspace" means only §2's set of windows. "Layout" stops being a user-facing noun (layout files become exported workspaces; "layout" stays for the arrangement of panes inside a tab, as the glossary already uses it).

**The other meanings:**
- Agent folders: say "the agent's folder" (or "working directory") in UI text, not "workspace".
- Global Memory headings: `# [Workspace] <name>` reads as "this AgentMux workspace's memory". Rename to `# [Global] <name>` in a separate change (it touches every agent's startup file and the `Memory` tool docs). §9 Q7.
- The srv `Workspace` object: in docs, "a window's contents". §9 Q6.
- Codex's "Workspace write" is Codex's own term; unchanged.

**Glossary entries** (agentmux-docs `glossary.md`, shipped with the implementation, since the glossary describes the released app):

> **workspace** — A named set of [windows](#window), each with its tabs and panes, that AgentMux saves as you work. One workspace is open at a time; you start in **Default Workspace**. Closing a window keeps it in the workspace (greyed out in the version chip's panel, ready to reopen); deleting it removes it. Opening another workspace closes the current one first. Managed from ☰ → Workspaces. Not to be confused with an agent's folder.

> **workspace export** — A workspace written to a file, `*.agentmux-workspace.json`, with ☰ → Workspaces → **Export…**, to keep or share. A snapshot: the workspace itself is saved inside AgentMux as you work, and later changes aren't written to the file. **Import…** opens one as a new workspace, as it does an older `*.agentmux-layout.json`.

And `window` gains: "Belongs to one [workspace](#workspace); closing it keeps it there."

## 8. Migration

- **First launch with this:** the windows open now (or the session record's, at a cold start) become **Default Workspace**, saved at once.
- **#5356's closed windows** (if it ever shipped): become closed windows of Default Workspace.
- **Old layout files** open through **Import…** as a new workspace (template mode: fresh agents, as today).
- **Older builds' session records** (#5236): if this channel has no workspace yet, the offer stays, and accepting it creates Default Workspace from it.
- **Removed:** `session:last_topology` writes (the workspace record replaces them; kept readable for one release), `session:closed_windows`, the Layouts submenu items.

## 9. Decisions (were open questions)

All decided by the operator on 2026-10-23.

1. **Deleting a closed window:** its row's slot shows reopen; a right-click "Remove from workspace" removes it without reopening it.
2. **New Window's contents:** the default panes, as now.
3. **Closed windows' lifetime:** kept until the user deletes them. No cap.
4. **Instances:** stay independent and never share workspaces (§4.2).
5. **Widget bar:** Settings is pinned; Toolchain stays in the + menu (§3.1).
6. **The srv `Workspace` object:** keeps its name in code for now; docs call it "a window's contents". Rename later if it causes bugs.
7. **Global Memory's `[Workspace]` heading:** renamed to `[Global]`, in a separate change (it touches every agent's startup file and the `Memory` tool docs).
8. **What follows a workspace:** only its windows (tabs, panes, placement, names, appearance) in v1. Theme and the like stay app-wide.
9. **Agents when a workspace closes:** they stop, after the warning in §6 if one is mid-turn, and resume when the workspace reopens. They don't keep running detached.

## 10. Rollout

| PR | Scope |
|---|---|
| 1 | ☰ cleanup: remove Settings, Connectors, Memory, Toolchain; new default pinned widgets (§3, §3.1). Independent, small |
| 2 | Workspace record + continuous save + Default Workspace migration; status panel header shows its name (§2, §4, §5 header, §8) |
| 3 | Closed windows: a close keeps the window in the workspace; status panel greyed rows with reopen/delete (§5). Reuses #5356's srv work (keep on close, reopen with same ids); drops its closed-window list, Close workspace… and ☰ item |
| 4 | New / Open / Rename / Delete workspace, Export…, Import… (§3, §6) |
| 5 | Glossary, `internals/persistence.md`, Workspaces docs page (agentmux-docs) |

PR #5356 stops here: close it, or turn it into PR 3.

**Follow-up, separate spec (operator, 2026-10-23):** instances stay independent beyond workspaces too: two versions of one channel running side by side should stop sharing agent definitions and settings, which they share today by design. Not in this spec's scope.
