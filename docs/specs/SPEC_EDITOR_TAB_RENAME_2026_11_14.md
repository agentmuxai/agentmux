# SPEC: Rename a file from its Editor document tab

**Status:** approved 2026-11-14. The operator, on choices A and B with their recommendations and the cancel-on-leaving default, as relayed by AgentY: "looks good, lets do it". So: A is the recommended folder order, B is refused, and leaving the box cancels. Sections 11 (edge cases) and 12 (checked against established tools) were added the same day at the operator's request; they change none of the three.
**Date:** 2026-11-14
**Author:** Camper@narko
**Tasks:** #47621 (this spec), #47622 (the build)
**Trigger:** the operator, 2026-11-14: "lets spec and implement file rename when dbl clicking on a document tab. if Untitled, it would save the tile as the new name" ("tile" means file).
**Read against:** `origin/main` at `3afa616e9`.

## 1. What it does

Double-click a document tab in the Editor and its name becomes a text box, in place.

- **A saved file** is renamed on disk, in its folder. The tab and everything else showing that file follow.
- **An Untitled document** is saved as a real file under the typed name, and the tab becomes that file.

Enter commits. Escape cancels. There is no dialog: a hint and any error appear on a line attached under the box.

## 2. Two choices for the operator

| # | Choice | Recommended | The alternative |
|---|---|---|---|
| A | Where an Untitled document is saved when only a name is typed | The folder of the file that pane showed last. If it has shown none: the folder the last Save As or tab rename saved into. If neither: the home folder, as Save As starts today. | Always the home folder (predictable, but files pile up there), or always the last save folder. |
| B | A saved file, and the typed name contains `/` or `\` | Refused: "A name can't contain / or \\." A rename stays in the folder, as in the file tree and the Files pane. | Treat it as a path and move the file. More cases (another drive, a missing folder, a name taken there); better as its own task. |

One smaller decision is made in §3 and can be flipped: **leaving the box cancels**.

## 3. The gesture

- **Double-click a tab** (anywhere but its close button) opens the box with the name selected up to its last dot: `notes` in `notes.md`, `notes.test` in `notes.test.ts`, all of `.gitignore`.
- **Enter** commits. **Escape** cancels. Enter with an empty or unchanged name cancels.
- **Leaving the box (a click elsewhere, a tab switch) cancels.** Nothing on disk changes without Enter. This matches the Save As box in the same tab; the file tree's rename box commits on blur, but there a wrong click costs less than an Untitled document saved into a folder nobody chose.
- **A refused name keeps the box open** with the typed text, and the reason on the line under it. The reason is srv's own sentence, so the rules live in one place.
- The tab can't be dragged while its box is open (the Save As box already works this way). Closing it cancels the rename first.

**What double-click does today, and how the two live together.** Today a double-click on a preview tab (italic, opened by a single click in the tree) keeps it open; on any other tab it does nothing. That stays: a double-click on a preview tab keeps it and does not start a rename; a double-click on a kept tab renames. Rename from the menu or F2 on a preview tab keeps it too.

## 4. A saved file

The rename goes through `fs.rename`, the route the Files pane uses, not the tree's `renameeditorfile`. `fs.rename` already never overwrites, handles a change of case only, has the per-platform name rules with sentences meant for the user, and works on an SSH host. The file tree's rename moves to the same route in this build, so the two can't differ. `fs.rename` checks the pane that calls it (`files_root::refuse_change`); the build confirms an Editor pane passes that check.

What follows the new name, with the tab id unchanged:

- the tab's title, tooltip, icon and path, and the pane title where it shows the file;
- the language mode, when the extension changed (the store's `RenameFile` updates the path but not the language today), and the language server: the old document is closed and the new one opened;
- the file watch (already re-pointed for a tree rename);
- **every other Editor tab on that file, in any pane or window.** Today only the pane that did the rename follows. srv publishes one event per successful rename (old path, new path, connection), and every Editor follows it. That also covers a rename done in the Files pane, which leaves Editor tabs on the old path today.

**Unsaved edits stay in the buffer and stay unsaved.** The rename touches the name only; the text, undo history, selection and encoding stay with the tab.

One message each, on the line under the box:

| Case | What happens |
|---|---|
| The name exists in that folder | Refused: "Something named “x” already exists here." Never overwritten. |
| Invalid on this platform (Windows: `< > : " | ? *`, a reserved name such as `nul`, a trailing dot or space; anywhere: empty, `.`, `..`, too long) | Refused with the existing sentence for that rule. |
| A change of case only (`readme.md` to `README.md`) | Goes through, also on a case-insensitive disk. The tree refuses this today. |
| A changed extension | Goes through; the language mode follows. No warning. |
| No permission (a read-only folder, an access rule) | "You don't have permission to rename it." The read-only mark on a tab does not block the attempt: the OS decides. |
| The file is gone | "It doesn't exist anymore." The tab and its text stay as they are. |
| The file changed on disk meanwhile | The rename goes through. A clean tab reloads as it does today; a tab with unsaved edits keeps them. |
| A save is in flight for that tab | "It is still being saved. Try again in a moment." |
| A path AgentMux protects (a drive root, its own data) | Refused with the Files pane's sentence. |

## 5. An Untitled document

Committing a name saves the buffer to a real file and the tab becomes that file, clean. It runs the existing Save As path (write the buffer, `movescratchfile`, `PromoteScratch`), so nothing new touches the disk.

- **The destination is shown before the commit.** The line under the box reads "Saves to ~/projects/site/notes.md" and updates as the user types.
- **A name alone** goes to the default folder (choice A).
- **A typed path chooses another folder:** `drafts/notes.md` is relative to the default folder; `~/x/notes.md` or `C:\x\notes.md` is used as given. Missing folders are created, as Save As does today. `..` is refused.
- The name is saved as typed. No extension is added.
- The same refusals apply: an existing file is never overwritten, the name rules of §4 apply to the last part (`movescratchfile` does not check them today), and a destination outside the home folder is refused with "The Editor saves only inside your home folder", which is the Editor's existing limit.
- Save As (Ctrl+S or Ctrl+Shift+S on an Untitled tab) stays as it is.

## 6. Other ways in

- **The tab's context menu:** "Rename…" on a saved file, "Save As…" on an Untitled document, above "Move to New Pane".
- **Keyboard:** a new command `editor:renameFile` ("Rename file"), default **F2**, acting on the active document. It is a row of the shortcut table, so Help lists it and it can be rebound. With the file tree focused, F2 keeps renaming the tree's row. F2 with the text focused does nothing today (the window tab's rename is off while a text field has focus), so nothing is displaced. As built, the command is not in the command palette: the palette lists app-wide commands only, and no pane's own commands (Save, Find) are in it. It runs by its key, from the tab's menu, and by id through MuxAPI's `UI command`.

## 7. Files that aren't on this machine

| Where | Result |
|---|---|
| An SSH host | Works. `fs.rename` runs on the host through the helper that already reads and saves the file. The host's own refusal is shown as its sentence. Other tabs on the same connection follow. |
| A WSL distro | Works. The Editor opens these through the distro's share as local files. |
| A container agent's workspace | Works. It is a host folder mounted into the container, so the Editor renames it as a local file and the container sees the new name at once. |
| An Untitled document on a host | Can't occur: an Editor on a host has no Untitled documents today. |
| A dropped item with no file behind it | The box doesn't open; the menu item is disabled with "This document has no file on disk." |

## 8. The agent side

An agent can't rename the file behind an Editor tab through a tool today. `Pane rename` covers a window, tab, pane or workspace; `UI command` reaches only the agent's own pane. An agent can rename the file in its own shell, but then the tab stays on the old path.

The smallest API for parity: **`Pane rename` with `target: "file"`** (`path`, `name`, optional `connection`), calling the same `fs.rename` route, so every open tab follows through the event of §4. It is open to container agents by default, inside their own mounts; a path outside them is refused with a sentence saying so. This is a follow-up task, filed when this spec is approved, not part of #47622. The event is part of #47622, so the follow-up is one small route.

## 9. Tests, and what the operator tries

**Unit tests (the build):**

- srv: the rename event is published once per success and not on a refusal; `movescratchfile` applies the name rules and refuses a destination outside home. The name rules and case-only rename are already covered in `fs_ops`.
- The store: `RenameFile` keeps the tab id, the unsaved mark and the text, and updates key, title and language; a second pane's tab follows the event; a folder rename moves the tabs under it.
- A pure function for the Untitled destination: a name, a relative path, `~/`, an absolute path, a Windows drive path, `..` refused, each default-folder fallback.
- The strip: a double-click on a kept tab opens the box with the right selection; on a preview tab it only keeps the tab; Enter, Escape and blur; a refusal keeps the box open with its message; the menu items and F2.

**Look checks:** macOS and Linux testers check the box and the line under it before merge, named in the PR.

**What the operator tries**, on a task dev build from current main on narko:

1. Double-click a saved file's tab, type a new name, Enter. The tab, the file tree and the Files pane show the new name.
2. Open the same file in a second Editor pane, rename it in the first. The second follows.
3. Type some text without saving, then rename. The text and the unsaved mark stay.
4. Rename to a name that exists, then to `nul`. Each shows its reason under the box, and nothing changes on disk.
5. New Untitled tab, type text, double-click, type `notes.md`. The line shows where it will go; Enter saves it there.
6. Same, typing `drafts/notes.md`. The folder is created and the file saved in it.

## 10. Out of scope

Moving a saved file to another folder (choice B). Renaming a folder from a tab. Save As for a file that already has a name. The agent API of §8.

## 11. Edge cases (added 2026-11-14)

Added at the operator's request, relayed by AgentY: "lets also capture edge cases for robust, like if the file is non-permission, or anything else. also do a quick research into best practice in case we missed anything". The three approved choices (A, B, leaving the box cancels) are unchanged.

Each row was checked against `origin/main` at `3afa616e9`. "Today" is what the `fs.rename` route (or, for an Untitled document, the Save As route) does now. "Sees" is always one line under the box, with the box kept open and the typed text kept, unless it says otherwise. "Build" marks what #47622 adds. Tests are unit tests unless marked manual.

### 11.1 Permission and locks

| # | Case | Today | The person sees | Test |
|---|---|---|---|---|
| 1 | The folder isn't writable (Linux, macOS), or an access rule denies it (Windows) | Refused by the OS, mapped to a sentence | "You don't have permission to rename it." | srv, Unix: a read-only folder |
| 2 | The file has the Windows read-only attribute | The rename goes through: the attribute doesn't block a rename | The new name. The tab's read-only mark stays | srv, Windows |
| 3 | A protected place: a drive root, the home folder itself, AgentMux's data or installation, a system folder | Refused before anything is tried | The Files pane's sentence for that place, such as "This is part of the AgentMux installation; AgentMux won't change it." | Exists in `fs_ops` |
| 4 | Another program has the file open or locked (Windows sharing violation) | Fails with the raw OS text and "(os error 32)" | **Build:** one retry after 200 ms, for a scan or an indexer passing by; then "Another program is using this file. Close it there and try again." | srv, Windows: a handle held open |
| 5 | A pane confined to an agent's workspace, shown read-only | Refused by `files_root::refuse_change` | "This folder is shown read-only from an agent's workspace: nothing in it can be changed from this pane." | srv: the Editor's block id is passed |
| 6 | Untitled: the destination folder is missing | Created, as Save As does | The file saved there | srv |
| 7 | Untitled: the destination folder isn't writable | Fails with a raw "copy: … (os error 13)" | **Build:** "You don't have permission to save there." The tab stays Untitled with its text | srv: the sentence for a refused write |

### 11.2 The name already exists

| # | Case | Today | The person sees | Test |
|---|---|---|---|---|
| 8 | A file or a folder of that name exists | Refused, never replaced | "Something named “x” already exists here." | Exists in `fs_ops`; the strip keeps the box open |
| 9 | Only the case differs, on a case-insensitive disk | Goes through, by way of a temporary name | The new name | Exists in `fs_ops`; the store updates the title when only case changed |
| 10 | Only the case differs, on a case-sensitive disk, and that other name is a different file | Refused | As 8 | Exists in `fs_ops` |
| 11 | The name is free at the check and taken at the rename | Can't happen on this machine: the rename itself refuses to replace (`MoveFileExW` without replace, `renameat2` no-replace, `renamex_np` exclusive). A filesystem without that falls back to check-then-rename | As 8 | Exists (`rename_no_replace_refuses_an_existing_destination`) |
| 12 | Two tabs commit the same new name at nearly the same time | The second loses to 11 | As 8, in the second | Covered by 11 |
| 13 | Untitled: the name is free at the check and taken at the write | **A gap today:** `movescratchfile` checks, then copies, and a copy replaces | **Build:** the destination is created exclusively, so an existing file is never replaced. As 8 | srv: an existing destination is untouched |
| 14 | The new name is already an open tab in this pane (a stale tab of a file since deleted) | Not considered | **Build:** refused before srv is asked: "“x” is already open in a tab here. Close that tab first." | Store |

### 11.3 The file changed under the tab

| # | Case | Today | The person sees | Test |
|---|---|---|---|---|
| 15 | Renamed, moved or deleted by something else, before or while the box is open | Refused | "It doesn't exist anymore." The tab and its text stay | Exists in `fs_ops`; the strip shows it |
| 16 | The content changed on disk meanwhile | The rename goes through | The new name. A clean tab reloads as it does today; a tab with unsaved edits keeps them | Model: an unsaved tab keeps its text |
| 17 | Unsaved edits | Untouched: the rename changes only the name | The text and the unsaved mark, under the new name | Store |
| 18 | The Editor's own watcher reports the rename as "changed" or "deleted" | Can't happen: the watcher publishes only for a file that still exists, and the tab's watch moves to the new path | Nothing | Exists (`test_removed_event_does_not_trigger_publish`) |
| 19 | Something writes to the old path after the rename and recreates it | Two files. The tab is on the new one and no longer watches the old | Nothing in the tab; the file tree shows both | None: nothing to do |
| 20 | The tab's file never loaded (an error panel) | The rename is tried all the same | The outcome of the rename; usually 15 | Covered by 15 |

### 11.4 Who else holds the path

| # | Case | Today | The person sees | Test |
|---|---|---|---|---|
| 21 | Other tabs on the file, in other panes and windows | Not followed | **Build:** they follow, by the rename event | Model: a second pane follows |
| 22 | The same file under another spelling in another pane (`~/x`, a drive letter in another case) | Would not match | **Build:** paths are compared in canonical form, ignoring case for Windows paths | Pure function |
| 23 | A tab on a host: a rename on the same host from another window | Not followed | **Build:** the event carries the connection; only tabs on that connection follow | Model |
| 24 | A folder renamed in the tree or the Files pane, with open tabs under it | Followed only by the pane whose tree did it | **Build:** every pane's tabs under it follow | Model |
| 25 | The Files pane | It re-lists by its own watcher | The new name | None: unchanged |
| 26 | The saved session | Tabs are stored with their path | After a restart the tab is on the new path | Store: the stored tab has the new path |
| 27 | The reopen-closed list (Ctrl+Shift+T) holds the old path | Would reopen a dead path | **Build:** closed entries follow too | Store |
| 28 | The language server | Told only when the tab is next shown | **Build:** the old document is closed and the new one opened at once, so diagnostics move | LSP session |
| 29 | The language mode in the live editor, after a change of extension | Not updated: CodeMirror is rebuilt on a tab change, not a path change | **Build:** the language is swapped in place, keeping the undo history | Session |
| 30 | Undo history, cursor, scroll and folds | Kept: the tab id and its CodeMirror state don't change | The same | Session: the state object survives |
| 31 | A pinned tab, a viewer choice, follow-folder | Kept with the tab | The same | Store |
| 32 | The other pane already has a tab at the new path (as 14) | Not considered | **Build:** that pane doesn't follow; its tab stays on the old path, as it would today | Model |

### 11.5 Names

| # | Case | Today | The person sees | Test |
|---|---|---|---|---|
| 33 | Empty, spaces only, or unchanged | (the box decides) | **Build:** the box closes. Nothing happens, no error | Strip |
| 34 | Spaces at either end | (the box decides) | **Build:** trimmed, as in the tree's box | Strip |
| 35 | A trailing dot or space, on Windows | Refused | "Windows doesn't allow a name to end with a dot or a space." | Exists in `fs_ops` |
| 36 | A Windows reserved name, with or without an extension; `< > : " \| ? *` | Refused | "“nul.txt” is reserved by Windows. Pick another name." or "Windows doesn't allow : in a name." | Exists in `fs_ops` |
| 37 | `.` or `..`, a NUL, `/` or `\` | Refused | "A name can't contain / or \\." and its siblings | Exists in `fs_ops` |
| 38 | Longer than 255 bytes or UTF-16 units | Refused | "That name is too long." | Exists in `fs_ops` |
| 39 | The whole path passes 260 characters, on Windows | Goes through: srv works on extended-length paths | The new name | srv, Windows: a path over 260 |
| 40 | Pasted text with line breaks | A single-line box drops them | The text on one line | None: the browser's behaviour |
| 41 | A tab or another control character | Refused on Windows only | **Build:** refused everywhere by the box: "A name can't contain control characters." | Pure function |
| 42 | A typed right-to-left override or another direction control | Accepted | **Build:** refused by the box: "A name can't contain hidden direction characters." | Pure function |
| 43 | What is selected first | (the box decides) | **Build:** up to the last dot: `archive.tar` in `archive.tar.gz`; all of `.env`; all of a name with no dot | Pure function |
| 44 | The extension changed or removed | Goes through | The new name; the language follows (29). No warning | Store |
| 45 | Enter that confirms an input-method composition (Japanese, Chinese, Korean, accents) | The Editor's existing boxes commit on it | **Build:** it doesn't commit (`isComposing`, or key code 229). Also fixed in the tree's box, the Save As box and the pane-tab rename box | Strip: a composing Enter is ignored |
| 46 | A host: a name the host's system refuses | The host's refusal | "That name can't be used." or the host's own text | Exists for the remote route |

### 11.6 An Untitled document

| # | Case | Today | The person sees | Test |
|---|---|---|---|---|
| 47 | No extension typed | Saved as typed | The destination line shows exactly the name that will be written | Pure function |
| 48 | The write fails part-way (disk full, a quota) | A partial file can be left at the destination | **Build:** the partial file is removed; the scratch copy is removed only after the real file is complete. "There isn't enough space to save it there." The tab stays Untitled with its text | srv: a failed write leaves the scratch file and no destination |
| 49 | A typed path: relative, `~`, absolute | Save As takes any of them | The destination line shows the resolved path as it is typed | Pure function |
| 50 | A typed path with `..` | Refused | "A path can't contain “..”." | Pure function; srv exists |
| 51 | A destination outside the home folder, a network path included | Refused: the Editor's existing limit | "The Editor saves only inside your home folder." | srv exists; the sentence is new |
| 52 | The last part of the path breaks a name rule | Not checked today | **Build:** the same sentences as 35 to 38 | srv |
| 53 | Encoding and line endings | The buffer's own: UTF-8, as a Save As writes it | The same | None: unchanged |
| 54 | A buffer over 10 MB | Refused: the Editor's existing limit | "Content too large (>10MB)". The tab stays Untitled | Exists |
| 55 | An empty buffer | Saved as an empty file | The file | Covered by 6 |

### 11.7 Where the file lives

| # | Case | Today | The person sees | Test |
|---|---|---|---|---|
| 56 | A host: slow | The call waits | **Build:** the box shows "Renaming…", a second Enter is ignored, and leaving the box no longer cancels: the outcome is applied when it is known | Strip: a pending commit ignores Enter and blur |
| 57 | A host: the connection drops before the rename | Fails | "Couldn't rename it: " and the connection's error | Exists for the remote route |
| 58 | A host: the name is free at the check and taken at the rename | **A gap:** the host's helper checks, then renames | Not fixed here (row 80) | None |
| 59 | A WSL distro, a container agent's workspace | Local files | As any local file | None: the same route |
| 60 | A network share | The server's own case rules and locks; no-replace falls back to check-then-rename where the share lacks it | The outcome, or the OS's refusal as a sentence | None: not reproducible in unit tests |
| 61 | A symbolic link | The link is renamed, not its target. (The tree's old route renamed the target.) | The new name | srv: the target keeps its name |
| 62 | A hard link, a junction | Nothing special: each is a name like any other | The new name | None |
| 63 | A file tracked by git | Shows as a delete and an add until staged. AgentMux runs no git | Nothing | None |

### 11.8 Tabs that aren't a plain file

| # | Case | Today | The person sees | Test |
|---|---|---|---|---|
| 64 | An image, PDF, video or audio tab | A file like any other | The rename works; the viewer keeps showing it | Store: the viewer choice survives |
| 65 | A dropped item with no file behind it | Nothing to rename | No box opens. The menu item is off, with "This document has no file on disk." | Strip |

### 11.9 The gesture and the box

| # | Case | Today | The person sees | Test |
|---|---|---|---|---|
| 66 | A double-click that was the start of a drag, or two Ctrl-clicks | The strip already tells these apart | No box | Exists (`pane-tab-pill`) |
| 67 | A double-click on the close button; a middle-click | The first click closes the tab, and the second lands on the tab that slid under the pointer; a middle-click never double-clicks | **Build:** the tab closes, and for half a second after a tab closes a double-click renames nothing | Model |
| 68 | A double-click on the empty part of the bar | Unchanged | Nothing new | None |
| 69 | A touch or pen double-tap | Arrives as a double-click | The box | Manual |
| 70 | A tab too narrow for the box, or scrolled partly out of view | (the box decides) | **Build:** the tab widens while the box is open, as for Save As, and scrolls into view | Manual, in the look checks |
| 71 | Focus goes elsewhere in the app | Leaving cancels | The box closes. Nothing was sent before Enter, so nothing is half-applied | Strip |
| 72 | The whole window loses focus (another app, an input-source switch) | (the box decides) | **Build:** the box stays with its text. Only focus moving elsewhere inside AgentMux counts as leaving | Strip |
| 73 | Keys inside the box | (the box decides) | **Build:** Enter, Escape; undo, select-all, copy and paste are the box's own and never reach the editor or the app; Tab leaves, so it cancels; F2 moves the selection from the name, to the whole name, to the extension | Strip |
| 74 | A screen reader | (the box decides) | **Build:** the box is labelled "File name"; the line under it is tied to the box; an error is announced (`role="alert"`) and marks the box invalid; it has an icon as well as a colour; focus returns to the document after a commit or a cancel | Strip: the attributes and the focus |
| 75 | A preview tab renamed by F2 or the menu | (new) | **Build:** it becomes a kept tab | Strip |
| 76 | Enter pressed twice | (new) | **Build:** the second is ignored while the first is pending | Covered by 56 |
| 77 | The tab leaves the pane while the box is open (a merge of panes) | (new) | The box closes with the tab. A commit already sent still applies, through the event | Model |
| 78 | A save is in flight for the tab | (new) | "It is still being saved. Try again in a moment." | Model |

### 11.10 Decided not to handle, and why

| # | Case | Why |
|---|---|---|
| 79 | The name differs only in Unicode form (composed and decomposed accents), on macOS | Refused as "already exists": the disk treats them as one name. Nothing is lost. The fix is a same-file test in `fs_ops`, shared with the Files pane; worth doing if anyone meets it |
| 80 | The host-side race of 58 | It is in the helper that runs on the host, outside this seat, and needs a helper release. Reported to AgentY for the owning seat |
| 81 | A host renames, and the connection drops before it answers | The outcome is unknown here. The box shows the connection error; the tab is on the old path, and reopening the file shows the truth. Rare |
| 82 | Undo of a rename | Explorer has it. Here the way back is the same gesture, and the old name is in no danger. A transient "Undo" can be a follow-up |
| 83 | A warning on a change of extension | Explorer warns. People renaming in a code editor change extensions on purpose, and the language following shows what happened |
| 84 | Updating imports in other files | VS Code does it through the language server. The Editor's language client has no rename support |
| 85 | A record in the Audit view | srv logs every rename (one line: from, to, outcome). The Audit view lists no pane's file renames today; adding them is the Audit owner's, and matters most for the agent API of §8 |
| 86 | A folder hint when two tabs share a name | Already true of any two files with one name. The tooltip has the path |
| 87 | Direction controls in a name that already exists | Shown as every file name in the app is shown. An app-wide question, not this tab's |
| 88 | Online-only files in a cloud-synced folder | Not tested. A rename changes only the name, and what the sync client refuses comes back as the OS's sentence |

### 11.11 Cannot occur

- **A crash-recovery or backup copy keyed by path.** The Editor keeps none for a named file; only Untitled documents are backed by a file.
- **A recent-files list.** The Editor has none.
- **A diff, a history version, settings or the Memory editor as a document tab.** The Editor's document tabs are files in one of five viewers (text, image, media, PDF, info), or a dropped item (row 65).
- **An Untitled document on a host.** An Editor on a host has none.
- **Bookmarks.** The Editor has none.

## 12. Checked against established tools (added 2026-11-14)

A quick web lookup, under an hour. "Confirmed" means the source says it; the rest is marked.

| Tool | What it does | Source |
|---|---|---|
| VS Code | No rename from the tab; it has been asked for several times and left in the backlog. Rename is F2 in the Explorer, inline. F2 again moves the selection: name, whole name, extension (confirmed by a tips page). The Explorer commits a valid name when the box loses focus (confirmed by a recent fix). A double-click in the Explorer keeps a preview tab (confirmed) | [#21919](https://github.com/microsoft/vscode/issues/21919), [#63892](https://github.com/microsoft/vscode/issues/63892), [F2 cycling](https://code2020.dev/f2-cycle-renaming/), [PR #338126](https://github.com/microsoft/vscode/pull/338126), [user interface docs](https://code.visualstudio.com/docs/getstarted/userinterface) |
| VS Code, pitfalls | The Explorer once wrote half-typed names to disk when focus left (from the issue's title). On GNOME Wayland, switching the input source closed the rename box | [#112503](https://github.com/microsoft/vscode/issues/112503), [#338125](https://github.com/microsoft/vscode/issues/338125) |
| JetBrains | Rename is a refactoring: Shift+F6 opens a dialog with a preview. Nothing found for the editor tab | [Rename refactorings](https://www.jetbrains.com/help/idea/rename-refactorings.html) |
| Sublime Text | A "Rename File" command in the palette. Tab-menu rename comes from plugins, and those break on unsaved buffers | [changelog](https://www.sublimetext.com/3), [FileTabRename](https://github.com/LDprg/FileTabRename/issues/2) |
| macOS title bar | Clicking a document's title opens a small panel with Name, Tags and Where. Naming an unsaved document there saves it (secondary sources, not Apple) | [How-To Geek](https://www.howtogeek.com/266661/quick-tip-you-can-move-and-rename-macos-documents-from-the-title-bar/), [MacMost](https://macmost.com/save-rename-and-move-files-with-the-title-bar.html) |
| Explorer | Ctrl+Z undoes a rename (confirmed). It asks before a change of extension (a forum, not Microsoft). What is selected and what leaving the box does were not confirmed by a source | [Microsoft Q&A](https://learn.microsoft.com/en-us/answers/questions/2481071/is-it-possible-to-undo-renaming-a-file), [TenForums](https://www.tenforums.com/general-support/189607-rename-warning-if-you-change-file-name-extension.html) |
| Finder | Nothing confirmed by a source | none |
| WCAG | An error is given in text, never by colour alone (3.3.1). A status or an error is announced without moving focus, with `role="status"` or `role="alert"` (4.1.3). Focus returns somewhere sensible (2.4.3; the page speaks of dialogs, so applying it here is our reading) | [3.3.1](https://www.w3.org/WAI/WCAG22/Understanding/error-identification.html), [4.1.3](https://www.w3.org/WAI/WCAG22/Understanding/status-messages.html), [2.4.3](https://www.w3.org/WAI/WCAG22/Understanding/focus-order.html) |
| Input methods | `isComposing` is true during a composition. Safari ends the composition before the confirming key arrives, so key code 229 is checked too (the Safari part is from write-ups and the bug report) | [MDN](https://developer.mozilla.org/en-US/docs/Web/API/KeyboardEvent/isComposing), [WebKit 165004](https://bugs.webkit.org/show_bug.cgi?id=165004) |

**Where we match.** The first selection stops at the last dot (VS Code). Enter commits and Escape cancels. An existing name is refused, never replaced. Naming an Untitled document saves it (the macOS title bar). Errors are text under the box.

**Where we differ on purpose.**

- **Leaving the box cancels; VS Code's Explorer commits.** This is the largest difference from the established tools, and the Editor's own tree box commits too. It was approved as a choice, and the lookup gives one more reason for it: commit-on-leaving is the design that once wrote half-typed names to disk in VS Code. It can still be flipped; row 71 is the only one that changes.
- **No warning on a change of extension** (row 83) and **no undo** (row 82).
- **Rename from the tab at all.** VS Code hasn't built it, though it is asked for; JetBrains and Sublime don't have it built in.

**What the lookup added.** The input-method guard (row 45), the window losing focus (row 72), F2 moving the selection (row 73), and the screen-reader items (row 74).

## 13. As built (#47622)

Built as specified, with these notes.

- **Where the code is.** `editor-tab-rename.ts` (the rename on disk, the tabs that follow, the name box's state), `editor-rename-names.ts` (the pure rules), `editor-tab-name-input.tsx` (the box and its line). srv: `fs.rename` publishes `fs:renamed`; `movescratchfile` creates its file exclusively and answers in sentences.
- **The line under the box** is a row under the tab bar that starts under the box, not a popup: nothing floats over the text.
- **"Save As…" in an Untitled tab's menu** opens the name box of §5, not the path box that Ctrl+S opens. Both stay.
- **The tree's old route, `renameeditorfile`, is removed.** The tree renames through `fs.rename` like the tab. Two things change for the tree with it: a change of case only now works, and a symbolic link is renamed itself, not its target.
- **The whole tree is listed again after a rename,** not one folder: the tree spells paths in the system's own way, which a tab's path may not match.
- **Not in the command palette** (§6): the palette lists app-wide commands only.
- **Found on the way and fixed:** creating a file from the tree checked that the name was free and then wrote it, which empties a file made in between. It is created exclusively now.
