# SPEC: A rich file browser pane (working title "Hangar")

**Status:** active. The v1 cut of Phases 0 and 1 is implemented (#4201, §12.1), Phase 2a (§12.2) git markers (§12.3) touched-by badges (§12.4) the grid view (§12.5) and tabs (§12.6, pane tabs); the rest of Phases 2–4 is not built. The §14 questions were settled with this spec's own recommendations, as the repo owner asked to take it to the end. Written against `main` @ `0408efa3a`.
**Date:** 2026-10-01
**Author:** korp
**Trigger:** Repo owner, 2026-10-01: *"we want to introduce a rich file browser pane inside of agentmux… I believe wave terminal had one (did it?) research best practices for an embedded file browser tab, also think up some good names. write spec to file."*
**Related:** `SPEC_EDITOR_FILE_TREE_2026-05-26.md` (the editor's tree, which this builds on), `SPEC_EDITOR_FILE_TREE_OPEN_ACTIONS_2026_07_12.md`, `SPEC_MEDIA_PANE_2026_07_26.md` (the directory watcher), `SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md` (how a view registers), `SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md` (drops into panes), `SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md` (attaching files to an agent), `ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md` §9 (the settled-content contract a new pane must follow).

> [!IMPORTANT]
> **2026-10-02, settled:** Hangar's tabs are **pane tabs** (§12.6, #4227). `SPEC_DOCUMENT_TABS_2026_10_02.md` moved them to document tabs (#4231), and the repo owner then kept document tabs for the Editor and Media only (*"which make semantic sense"*), so Hangar went back to pane tabs (that spec's §6.2).

---

## 1. Summary

AgentMux has no way to *browse* files except inside the Editor pane's side tree. That tree is a good start (lazy folders, context menu, rename, create, delete, live drives list), but it is welded to the editor: it opens files into CodeMirror, shows only names, and has no columns, sorting, history, selection model, preview or operations beyond single-file edits.

This spec proposes a **file browser pane**, a first-class view like Swarm or Media. It can be a pane, a pane tab or the content of a window tab, because all of those host any registered view. The recommendation:

1. **A details view as the default** (name, size, modified, kind; sortable; keyboard-first), plus a tree outline and a preview side panel. Miller columns and a thumbnail grid come later.
2. **One shared filesystem layer** (`frontend/app/fs/` and a generic `fs.*` RPC family in srv), extracted from the editor tree so the two never diverge. The editor keeps its tree until the new component can replace it.
3. **Streaming, cancellable listings and watcher-driven patches**, so a 100,000-entry folder or a slow network drive never freezes the pane.
4. **Trash by default, undo for what can be undone, and a job queue** for copy and move, with conflict handling and progress.
5. **What makes it AgentMux's, not just a clone:** agents can open it on a path and select files (`OpenFiles` MCP tool); a drag or a menu item attaches files to an agent; agent workspaces are first-class places; and later, files an agent touched are marked.

Wave Terminal had a file browser, so the idea is proven in this lineage (§2). What it lacked, and what the better file managers have, is covered in §3.

## 2. Did Wave Terminal have one?

**Yes.** In Wave it is the directory mode of the **Preview** widget, not a separate widget. Sources: the Wave docs' widget list (a navigation page; its Preview page returned only a table of contents when fetched, and `docs.waveterm.dev` rate-limited a second fetch), plus DeepWiki's summary of the Wave source ([File Preview and Editor Views](https://deepwiki.com/wavetermdev/waveterm/4.2-file-operations)) and several product write-ups. The details below are secondary-source and unverified against Wave's code.

- Opened with `wsh view .` from a Wave terminal, or from the widget bar. A directory renders as a Finder/Explorer-style table; the same widget shows images, Markdown, audio, video, HTML, CSV and code for files.
- Columns reported: name, size, permissions, modified time. It leans on `ls -alh` rather than icons.
- Double-click or Enter opens; a `..` row goes up; back and forward live in the widget header; the header menu has a filter.
- **Drag a file from one directory widget to another to copy it, including across connections** (SSH, WSL, S3). This is its best idea and the reason it was loved.
- Entries are reported to stream in chunks with a maximum count.

**Does AgentMux still have it?** No remnant. There is no directory or preview view under `frontend/app/view`, and `git log` finds no deleted `frontend/app/view/preview/`. Whatever came with the fork was removed before the history this repo keeps, or was never imported. The editor's tree (#1064, 2026-05-26) is the only browser, and it was written fresh.

## 3. What good file browsers do (research)

Condensed from the sources in §15. Each item becomes a requirement in §5–§9.

| Practice | Seen in | Why it matters here |
|---|---|---|
| Stream and cancel directory reads; never block the UI on one slow folder | Yazi (async by design, because Ranger's synchronous reads froze on big or NFS directories); Wave | Our `listeditordir` reads the whole directory before replying. |
| Separate **focus** from **selection**; Space toggles, Shift+arrows extend, Ctrl+A selects all | W3C ARIA Authoring Practices (tree and treegrid) | The editor tree has single-select only. |
| Type-ahead: typing moves to the next name with that prefix; multi-character buffer clearing after ~500 ms | ARIA APG; Windows and macOS file managers | The fastest way through a long folder. |
| Virtualized rows need `aria-level`, `aria-setsize`, `aria-posinset` | ARIA APG | Accessibility must survive windowing. |
| Don't watch everything. Exclude heavy folders; coalesce bursts; treat watcher overflow or a lagging consumer as "rescan this folder", never as "I saw every event" | VS Code (`files.watcherExclude`, inotify exhaustion on `node_modules`); `notify`/ReadDirectoryChangesW (16 KB buffer overflows under bursts; no guaranteed `Create` per file) | Our pool already says "wake signal, not a delivery log". |
| Delete moves to the OS Trash with no confirmation and an undo; permanent delete gets a specific, named confirmation | Nielsen's *user control and freedom*; Files app discussion; UX literature on destructive actions | Our `deleteeditorfile` is **permanent** (`remove_file` / `remove_dir_all`), scoped to home. Fine for a scratch file, wrong for a browser. |
| Preview pane / Quick Look on Space, async, so selecting a 50 MB file never stalls navigation | Finder, Files (Windows), Yazi, ForkLift | We already have the Editor and Media renderers. |
| Dual-pane or tabs for move/copy between places | Marta, ForkLift, Files, Total Commander | We get tabs for free (pane tabs, window tabs; §12.6) and two panes by tiling. Drag between two panes is Wave's winning feature. |
| Respect `.gitignore`, show git status | VS Code, JetBrains, Zed project panels; ripgrep's `ignore` crate | Developers browse repos far more than home folders. |
| Coding tools offer three routes from a file to the agent: an `@` mention, drag from the sidebar, a context-menu item. Drag is the most discoverable and the most fragile (it needs Shift in VS Code, regressed in Cursor on Windows) | Cursor, Claude Code (VS Code and Desktop), Kiro | We own both ends of this drag. Make it reliable and add the other two routes. |
| Windows: `\\?\` long paths, reserved names (`CON`, `NUL`…), stripped trailing dots and spaces, case-insensitive collisions, junction and symlink loops | Microsoft docs on file naming and paths | AgentMux's primary platform. §9. |

## 4. Goals and non-goals

**Goals**
1. Browse any folder the user can read: fast, keyboard-first, with columns, sorting, history, breadcrumbs and places.
2. Never freeze: bounded work per frame, cancellable reads, background previews.
3. Safe by default: trash not delete, undo where possible, conflicts resolved explicitly, destructive confirms that name the item.
4. Live: changes on disk appear within ~200 ms without a manual refresh, without watching whole drives.
5. First-class agent integration (§8): open-on-path from an agent, attach to an agent, agent workspaces as places.
6. Fits the product: registers like any view, persists its state in block meta, follows the settled-content contract, and survives tear-off and redock.

**Non-goals (v1)**
- Replacing the Editor's tree (it stays until §12 Phase 4).
- Remote roots (SSH, containers, S3). The design leaves a seam (§6.4); no implementation.
- Archive browsing (zip as a folder), tags, cloud sync, content search, bulk-rename patterns.
- Dragging files *out* to the OS desktop or Explorer (CEF gives no native drag source from the DOM). Dropping files *in* works through the existing OS file-drop path.
- Being a general-purpose OS file manager replacement. It is a browser for working folders.

## 5. UX

### 5.1 Layout

```
┌ Hangar ───────────────────────────────────────────────── ⟲ ⚙ ┐
│ ◀ ▶ ▲  C: › Users › asafe › .agentmux › agents › korp   🔍 ⌕ │  breadcrumb (click a segment; Ctrl+L = text path)
├────────┬──────────────────────────────────────┬──────────────┤
│ Places │ Name              Modified   Size    │  Preview     │
│ Home   │ ▸ agentmux/       2 min ago          │  (Editor /   │
│ Desktop│   docs/           yesterday          │   Media /    │
│ Agents │   package.json    3 h ago    2.1 KB  │   Markdown)  │
│  korp  │ ● README.md       just now   4.0 KB  │              │
│  clamk │                                      │              │
├────────┴──────────────────────────────────────┴──────────────┤
│ 3 selected · 2.4 MB · 143 items · git: main ↑1 ●2            │  status line
└──────────────────────────────────────────────────────────────┘
```

Sidebar and preview are toggleable and remember their state per pane; narrow panes hide the sidebar first, then the preview (container-query driven, like the agent picker's responsive tiers).

### 5.2 Views
- **Details** (default): fixed-height rows, resizable and reorderable columns (name, modified, size, kind; later permissions, git status), folders first (toggleable), natural sort (`file2` before `file10`), case-insensitive.
- **Tree** (outline): expand in place; the shared tree model from the editor. Also the sidebar of Phase 2.
- **Columns** (Miller): parent | current | preview. Phase 3.
- **Grid**: thumbnails for images and video, reusing the Media pane's thumbnail path. Phase 3.

### 5.3 Selection and keyboard
Follows the ARIA treegrid pattern; the key table is §5.3.1. Focus and selection are separate (`aria-multiselectable`, `aria-selected` on every selectable row). Virtualized rows carry `aria-level`, `aria-setsize` and `aria-posinset`. Focus is a roving `aria-activedescendant` so rows scrolled out of the DOM don't lose it.

#### 5.3.1 Keys
All bindings go through `keyutil` descriptors (note: in this app `Cmd` is **Alt** on Windows and Linux, so the table is in logical terms and the real bindings are chosen per platform in implementation).

| Action | Keys |
|---|---|
| Move focus | ↑ ↓, Home, End, PageUp, PageDown |
| Extend selection | Shift+↑/↓, Shift+Click (range), Ctrl+Click (toggle), Space (toggle focused), Ctrl+A |
| Open (file → default action, folder → enter) | Enter, double-click |
| Expand / collapse (tree view) | → / ←, `*` expands siblings |
| Up, back, forward | Alt+↑, Alt+←, Alt+→ (and Backspace = back on Windows) |
| Edit path | Ctrl+L (breadcrumb becomes a text box with path completion) |
| Rename | F2 (in-place, name part pre-selected without the extension) |
| New folder / new file | Ctrl+Shift+N / the menu |
| Trash / permanent delete | Delete / Shift+Delete |
| Copy / cut / paste | Ctrl+C / Ctrl+X / Ctrl+V |
| Quick look | Space on a single selection (when nothing is being type-ahead'd) |
| Filter in this folder | Ctrl+F (or just start typing: type-ahead jumps, `/` starts the filter) |
| Type-ahead | printable keys; buffer clears after 500 ms; wraps |
| Context menu | Menu key, Shift+F10, right-click |

### 5.4 Opening files
The default action for a file is by kind: text and code → Editor pane (as a pane tab beside the Files pane when there is room); images, audio, video → Media pane; Markdown → Editor in preview mode; everything else → the OS default application. The context menu always offers *Open with…* (Editor, Media, Terminal here, OS default) and *Reveal in OS file manager* (the host's existing `reveal_in_file_explorer`).

### 5.5 Places (sidebar)
Home, Desktop, Documents, Downloads, the drives list (reuse `geteditorroots`), pinned folders (user-editable, stored in settings), recents, and **Agents**: one entry per agent workspace (§8.3). Dragging a folder onto Places pins it.

## 6. Architecture

### 6.1 Reuse, don't fork
`frontend/app/view/editor/file-tree-model.ts` (226 lines) and `file-tree.tsx` (349 lines) hold the expand/collapse, lazy load, rename and create flows. Phase 0 extracts the model to `frontend/app/fs/` with no behaviour change. The new view and the editor tree both depend on it, so a fix lands once.

### 6.2 View registration
A `files` (working name) manifest in `block-registry.ts` / the pane-tab registry: name, aliases (`files`, `explorer`, `dir`), label, icon, `lifecycle: "keepAlive"` (so scroll and selection survive a pane-tab switch), and a `defwidget@files` entry in `widgets.json`. Per-pane state in block meta, in the existing namespaced style (`editor:tree_expanded`): `files:path`, `files:view`, `files:sort`, `files:sortdir`, `files:hidden`, `files:sidebar`, `files:preview`, `files:columns`. Nothing derivable is persisted (selection and scroll are not).

### 6.3 Backend: a generic `fs.*` family
Today's `listeditordir`, `watchmediadir` and the editor mutations are named for their first consumer and each has its own limits. Phase 0 adds generic equivalents beside them (the old names stay as thin aliases until nothing calls them):

| RPC | Purpose |
|---|---|
| `fs.list { path, cursor?, limit?, show_hidden? }` | **Cursor-paginated**, not one big reply. Returns `{ path (canonical), entries ≤ 1000, cursor \| null }`. The server holds the `ReadDir` iterator under a short TTL (30 s). Navigating away drops the cursor, which is the cancellation. The frontend paints each page as it arrives and sorts when `cursor` is null. A per-entry failure (permission denied, vanished) becomes an `error` on that entry, never a failed listing. |
| `fs.stat { paths[] }` | Batch stat for selection details and preview decisions. |
| `fs.watch { path } / fs.unwatch` | Non-recursive, ref-counted, block-scoped, on the existing `fs_watch` pool (`notify` 7; it already owns atomic-rename safety, backoff and health sweeps). Pushes `fs:changed { dir, upserts[], removes[], rescan }`. |
| `fs.places` | Home, known folders, drives (supersedes `geteditorroots`), plus the agent workspaces list. |
| `fs.op.start / resolve / cancel` | The job queue (§7). |

`DirEntry` (generated TS type) gains: `hidden` (the dot-prefix *or* the Windows hidden/system attribute), `readonly`, `link_target?`, `is_junction`, `error?`. `size`/`mtime` stay genuinely absent when unknown, per the existing comment on that type.

### 6.4 The root seam (not v1)
Every `fs.*` request takes an optional `root: { kind: "local" } ` that is always `local` in v1. Containers (agents with `agent_type: container`), SSH and WSL-over-SSH slot in later as other kinds without changing the frontend model. On Windows, WSL already works as `\\wsl$\…` UNC paths.

### 6.5 Rendering
- **Fixed-height rows, hand-windowed.** There is no virtualization library in `package.json`; for fixed-height rows a ~60-line windowing helper is enough and avoids a dependency. Variable-height views (grid, columns) can adopt a library in Phase 3 if needed.
- **No `transition: all`** (CI gate `check-no-transition-all.sh`): the pane lives inside window tabs hidden with inherited `visibility`, and an `all` transition animates it and pops the pane in a frame late.
- **Settled-content contract** (`ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md` §9): the view calls `trackPaneContent(blockId)` and holds `holdPaneContent` until the first page of the first listing has rendered, so a new window tab containing a Files pane appears complete in one frame rather than showing a loading state.
- **Dormancy:** in a hidden window or pane tab, watcher patches queue and apply on show; no work while hidden (`usePaneTabVisibility`).
- Icons come from the existing `fileKind` module and icon set; per-extension icon cache; no per-row async work.

### 6.6 Previews
Space or the preview panel renders by kind: text and code via a **read-only** CodeMirror (the Editor's lazy language packs), Markdown via the existing renderer, images/audio/video via the Media pane's components, folders as a count and size, and everything else as metadata. Previews are cancelled on selection change, capped (text first 256 KB with "open in editor" for the rest), and never run for more than one file at a time.

## 7. Operations

### 7.1 Job queue
Copy, move, trash and delete run as **srv jobs**, not request/response calls: `fs.op.start` returns an `op_id`; progress arrives as `fs:op { op_id, state, done_bytes, total_bytes, current_path, conflict? }` events; `fs.op.cancel` stops it between files. A small panel (or the status line) shows running ops. Large copies stream with bounded buffers; cross-volume move is copy then delete, with the delete only after the copy verified.

### 7.2 Conflicts
When the destination exists: **Replace, Skip, Keep both** (`name (2).ext`), with *apply to all*. `fs.op.resolve` answers a pending `conflict`. A directory merging into an existing directory merges, with the same prompt per file.

### 7.3 Delete and undo
- **Delete = move to the OS Trash**, no confirmation, with a toast and **Undo**. Uses the `trash` crate (Windows Recycle Bin, macOS Trash, freedesktop), on macOS with `NsFileManager` rather than the crate's Finder default, which prompts for Automation (§9.1.5). Its docs warn about undefined behaviour when called from multiple threads on Linux/FreeBSD, so all trash calls run on one dedicated worker.
- **Permanent delete** (Shift+Delete, or Trash unavailable such as network shares): a confirmation naming the item(s), the count and "this can't be undone". No generic "Are you sure?".
- **Undo** covers rename, move, new item and restore from trash. Windows and Linux use the `trash` crate's restore API. On macOS, `NSFileManager`'s `trashItemAtURL` returns the item's resulting URL in the Trash, so undo is a move back; whether the `trash` crate exposes that URL is to be checked, otherwise a direct `NSFileManager` call (a small new dependency) or the fallback of a toast offering "Reveal in Trash". Finder's "Put Back" is not offered for files trashed this way. The stack is per pane and in memory.

### 7.4 Rename and create
In-place, validated before the RPC: reserved device names, illegal characters, trailing dots and spaces (Windows silently strips them), empty names, and case-insensitive collisions. A **case-only rename** on a case-insensitive filesystem goes through a temporary name. New file and new folder create then enter rename mode.

## 8. Agent integration

### 8.1 Agents open it
A new MCP tool, `OpenFiles { path, select?: string[], view? }`, alongside `OpenEditor` and `OpenMedia`: opens (or reuses) a Files pane at `path` and selects the named entries. "Here is what I changed" becomes a pane, not a list of paths in the transcript. Read-only navigation: it creates no new capability, since the agent could already `OpenEditor` any path.

### 8.2 Files to agents (three routes, one result)
1. **Drag** a row (or the selection) onto an agent pane's composer. This reuses the attachments pipeline (`SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md`): the file is classified, processed and delivered the same as a dropped OS file. No modifier key required, with the composer showing a drop overlay while the drag is over it (the lesson from Claude Code and Kiro).
2. **Context menu:** *Attach to <agent>* (a submenu of the agents open in this window) and *Copy path for agent* (an `@path/relative/to/workspace` reference).
3. **Keyboard:** Alt+K (Claude Code's binding) inserts an `@path` mention into the focused agent composer.

Technically the drag needs host paths on an *in-app* drag. The drag-session already has a `files` kind for OS drags; Phase 2 adds `DragPayload.paths` so the file-drop controller treats a Hangar drag exactly like an OS file drop that happens to carry paths. Then the Editor, Media and agent hooks work unchanged, and dropping onto another Files pane becomes copy (Ctrl) or move (default on the same volume).

### 8.3 Agent workspaces as places
The **Agents** section of Places lists each agent's working directory from the agent definitions, with the agent's name and status. This answers "where did that agent put things?" with one click, and is why a general OS file manager can't match this pane.

### 8.4 Touched by (Phase 3)
Rows for files an agent wrote or edited in the last N minutes get a small badge with the agent's colour. The source is the agent's tool-call events (Write/Edit/NotebookEdit paths already pass through the transcript), **not** filesystem watching: a watcher can't say *who* changed a file. A user save in the Editor shows no badge.

## 9. Platform and safety

**Paths and names (Windows first):**
- Normalize with `\\?\` for >260 characters, and verify the manifest and `LongPathsEnabled` story (both are needed for APIs that don't take the prefix); the shell and other apps may still not open such a file, so show a warning badge.
- Reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1–9`, `LPT1–9`, with or without an extension), illegal characters `< > : " / \ | ? *`, trailing dots/spaces.
- Case-insensitive but case-preserving; drive letters case-insensitive; UNC and `\\wsl$` roots.
- Junctions and symlinks are different things: show a link badge and the target; **never recurse through a link** when sizing or copying; detect cycles by canonical-path set. Deleting a link removes the link, not the target (the existing editor code already does this; keep it).
- Hidden means the dot-prefix **or** the `HIDDEN`/`SYSTEM` attribute.
- macOS: bundles (`.app`) behave as files. The default filesystem (APFS) is case-insensitive and stores names in decomposed Unicode, so a name with an accent can fail to match the typed form: compare normalized, display as stored. Access prompts and denials are §9.1.

**Security.** Today's editor RPCs serve any absolute path the frontend sends, and the macOS root scoping comment in `editor_handlers.rs` says outright that it is "root scoping, not a sandbox". For a browser the policy needs to be explicit:
- *Reads:* any path the user can read, as today. The user is the principal; this is their machine.
- *Mutations:* absolute, canonicalized, re-validated at execution time (not only at request time), NUL-free, never a drive root or the home directory itself, and a short **protected list** (the OS directory, `Program Files`, `/usr`, `/etc`, the AgentMux install and data dirs) that needs a typed confirmation. The existing home-only scoping on `deleteeditorfile`/`renameeditorfile` is widened deliberately, in one place.
- *Agents don't mutate through this pane.* `OpenFiles` navigates and selects only. Agents already have their own file tools, which go through their own permission model.
- Operations log each mutation (paths, count, outcome) at info level for audit, like other host actions.

**Errors are per entry.** A permission-denied folder, a file locked by another process, a dangling link, and a vanished file are rows with an icon and a tooltip, not a failed pane.


### 9.1 macOS access prompts (warning UX)

> [!IMPORTANT]
> **Guiding rule (repo owner, 2026-10-01): minimize prompts, and ask only on need.** A prompt is allowed only as the direct result of the user opening something that needs it. Nothing the pane does on its own (restoring, listing Places, watching, searching, previewing, git status) may cause one. §9.1.5 turns this into a checklist and an acceptance test.

A file browser is the feature most likely to trip macOS's privacy system (TCC). The bad outcomes are a pane that freezes while a system prompt waits, and a denied folder that looks **empty**. This section separates what is established from what only a person with a Mac can settle.

#### 9.1.1 Established

From Apple's documentation and forums, and from this repo.

| # | Fact | Source |
|---|---|---|
| 1 | Three kinds of gate matter here. **Files & Folders** (since 10.15) asks per location: Desktop, Documents, Downloads, iCloud Drive, network volumes, removable volumes. **Full Disk Access** covers the rest (Mail, Messages, Safari, Time Machine…). **App bundle protection** (13+) guards other apps in `/Applications`. | [Apple Support](https://support.apple.com/guide/mac-help/control-access-to-files-and-folders-on-mac-mchld5a35146/mac); [Eclectic Light](https://eclecticlight.co/2026/04/08/privacy-files-folders-or-full-disk-access/) |
| 2 | A denial is **`EPERM`** (mandatory access control), not `EACCES` (ordinary permissions). | Apple DTS, [forum thread](https://developer.apple.com/forums/thread/126430) |
| 3 | The first access **blocks the calling thread** while the prompt is on screen. There is no way to ask for an error instead. | same thread |
| 4 | Child processes are attributed to the responsible app, so a CLI we spawn inside a protected folder prompts, or fails, in **AgentMux's** name. A spawned CLI can die at startup with `EPERM`. | [penguin-harness #846](https://github.com/Prism-Shadow/penguin-harness/pull/846); [t3code #5943](https://github.com/pingdotgg/t3code/issues/5943) |
| 5 | A denial is sticky. macOS does not ask again; the user must switch the app on in System Settings → Privacy & Security → Files & Folders. | Apple Support |
| 6 | A grant is keyed to the bundle id and the code-signing requirement. An ad-hoc signature makes the code hash the whole identity, so **every rebuild is a new app**. | [YARG #1695](https://github.com/YARC-Official/YARG/issues/1695) |
| 7 | Releases are Developer ID signed, notarized and hardened-runtime, but not sandboxed (`build/entitlements.mac.plist`), so TCC applies and no `files.*` entitlement is involved. | repo |
| 8 | The bundle id is `ai.agentmux.<channel>.<version>`. The owner **accepted** that every release is a distinct macOS app and that granted permissions reset once per version (`SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18.md` §4). Folder prompts therefore come back after every update, and Settings accumulates an entry per version. **The repo owner confirmed (2026-10-01) that the reset is intended: it means every release re-exercises the first-run permission flow.** | repo, owner |
| 9 | `Info.plist` (generated in `scripts/package-macos.sh`) declares only `NSMicrophoneUsageDescription`, `NSLocalNetworkUsageDescription` and `NSBonjourServices`. **No folder strings** (`NSDesktopFolderUsageDescription`, `…Documents…`, `…Downloads…`, `…RemovableVolumes…`, `…NetworkVolumes…`). | repo |
| 10 | The existing handlers call `std::fs::read_dir` inside an `async` block, which runs on a Tokio worker. A blocked prompt would hold that worker for as long as the user takes to answer. `fs.list` must use `spawn_blocking`. | repo (`editor_handlers.rs`) |
| 11 | Precedents to match: the status-bar **Keychain notice** shown on macOS before muxbus sign-in (`HostPopover.tsx`, `status-bar-popover-info-notice`); the opt-in LAN-discovery switch with its usage string; an Accessibility deep link in `SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24.md`. | repo |

#### 9.1.2 Not established: needs a person with a Mac

Sources disagree, or say nothing. Each is a test in §9.1.3.

| ID | Question | Why it matters |
|---|---|---|
| T1 | Does a prompt appear **at all** without the folder usage strings, and with what wording? A forum answer says the strings are optional; one project reports silent denial. | Decides whether we must add the five keys, and what the user sees. |
| T2 | **Whose name** is on the prompt and in the Settings list: AgentMux, `agentmux-srv`, the launcher, or a helper? Do `agentmux-srv`, the CEF host and spawned agent CLIs share one grant? | The warning has to name what the user will actually see. |
| T3 | After an update, does a folder granted in version N prompt again in N+1 (fact 8)? Does Settings show both? | The notice must say whether prompts repeat. |
| T4 | What does a denied `read_dir` return: `EPERM`, or an **empty** listing? Does `metadata()` on children in a protected folder behave the same? | An empty-looking denial is the worst failure. |
| T5 | Does starting a watcher (FSEvents through `notify`) on a protected folder prompt, or silently deliver no events? | Decides whether the browser may watch before the user has granted access. |
| T6 | Does today's Editor tree (which lists `$HOME`) prompt anything on startup? Does listing Places, or `stat` on `~/Desktop`, `~/Documents`, `~/Downloads`, prompt? | Defines "touch nothing eagerly". |
| T7 | Which URL opens Files & Folders, and Full Disk Access, on macOS 13, 14, 15 and 26? The legacy `…preference.security?Privacy_AllFiles` form is reported broken on newer releases; the `…PrivacySecurity.extension?…` form is the newer one; `Privacy_FilesAndFolders` is unconfirmed. | The "Open Privacy Settings" button. See [Ventura pane list](https://github.com/bvanpeski/SystemPreferences/blob/main/macos_preferencepanes-Ventura.md), [whosaid #34](https://github.com/sblattj/whosaid/issues/34). |
| T8 | Network volumes, removable volumes and iCloud Drive: separate prompts? Same behaviour as T4? | Covers the rest of Places. |
| T9 | How is a `task dev` build signed on macOS? If ad-hoc, every rebuild re-prompts (fact 6). | Developers testing this feature will hit it. |
| T10 | Do T1–T8 differ on macOS 13, 14, 15 and 26? | We support `LSMinimumSystemVersion` 11. |
| T11 | Does picking a folder through the native open panel, or dropping it from Finder onto the pane, grant access **without** a prompt (user intent)? Apple's sandbox model works that way; a non-sandboxed, Developer ID app may differ. | If yes, "Open folder…" and drop-from-Finder are prompt-free routes into a protected folder. |

#### 9.1.3 Test plan

Use a clean user account or a VM snapshot, and the **release** DMG (Developer ID signed), plus one `task dev` build for T9.
1. Record the identity first: `codesign -dv --verbose=4 AgentMux.app` and `codesign -d -r- AgentMux.app`.
2. Between runs: `tccutil reset All <bundle id>` (and `tccutil reset SystemPolicyDocumentsFolder <bundle id>` for a single service). Quit and relaunch; grants apply only to fresh processes.
3. Watch attribution live: `log stream --predicate 'subsystem == "com.apple.TCC"'`.
4. For each of Desktop, Documents, Downloads, an external drive, a network share and iCloud Drive, open it in the Editor tree (today's behaviour) and, once built, in the Files pane. Record: prompt shown or not, wording, the name in the prompt, whether the UI froze and for how long, and what the denied case shows.
5. Repeat after installing the next version over the first (T3), and on the oldest and newest macOS available.
6. Deny once, then check that the pane shows the denied state and that the Settings link lands on the right pane (T7).

Capture screenshots of each prompt: they become the reference for the in-app wording.

#### 9.1.4 Design, pending the tests (see also §9.1.5)

1. **Touch nothing eagerly** (the rule above). Places lists Desktop, Documents, Downloads, iCloud Drive and volumes by name without reading them. No thumbnails, git status, watchers, search or `stat` on their children until the user opens one (T5, T6).
2. **Explain just before the system prompt**, once per location, on macOS only: an inline notice in the pane in the style of the muxbus Keychain note. For example: *"macOS will ask to let AgentMux open your Documents folder. That's so it can show what's inside."* Whether to add *"You may be asked again after an update"* depends on T3. The "seen" flag is keyed by location **and bundle id**, because grants reset per version.
3. **Never freeze.** The listing runs on a blocking worker (fact 10). If no page has arrived after ~400 ms, the pane says *"Waiting for you to answer macOS's prompt…"*. Navigating away cancels.
4. **A distinct denied state.** `EPERM` under a known protected root on macOS is classified `access_denied_by_os`, separate from `permission_denied`. The pane shows *"macOS blocked AgentMux from reading Documents. Open Privacy Settings, turn AgentMux on under Files & Folders, then try again."*, with an **Open Privacy Settings** button (the URL form from T7, falling back to the base Privacy & Security pane) and **Try again**. A denied folder is never rendered as empty. If T4 shows macOS can return an empty listing instead of an error, add a heuristic: an empty result for a protected root that the user has not yet been prompted for is shown as "may be blocked".
5. **Full Disk Access is never requested by default.** Only a folder outside the Files & Folders set (such as `~/Library/Mail`) that fails shows the Full Disk Access explanation.
6. **Add the five folder usage strings** to `Info.plist` with the Phase 1 PR. They are optional but harmless and give the prompt an explanation (T1 confirms).
7. **Restore never prompts.** A pane restored at launch, redocked, or torn off with `files:path` inside a protected location does **not** list it. It shows a placeholder, *"Documents. Open to let macOS ask."*, and lists on the user's click. Otherwise an app launch would raise a prompt nobody asked for.
8. **Adjacent, separate follow-up:** an agent whose working folder is inside a protected location can hit the same prompt, or a startup `EPERM`, in AgentMux's name (fact 4). A notice at agent launch belongs to the agent flow, not this pane.


#### 9.1.5 Prompt budget

Every way the browser could cause a macOS prompt, and how each is avoided. A prompt is *allowed* only in the first row, and only after the user opens the location.

| Source | What triggers it | Avoided by |
|---|---|---|
| **Files & Folders** | Reading, listing, watching or `stat`-ing the children of Desktop, Documents, Downloads, iCloud Drive, network or removable volumes | **Allowed only on the user opening that location.** Never on restore, in Places, search, recents, previews, thumbnails, git status or watchers. The pane remembers locations it has already listed successfully, keyed by bundle id (so it resets with each release, as the permissions do), and treats only those as safe for background work; everything else is "unknown" and excluded from search and decoration. |
| **Full Disk Access** | Protected system data (`~/Library/Mail`, Safari, Messages, Time Machine) | Never requested. If a folder fails for this reason, explain; don't ask. |
| **Automation ("AgentMux wants to control Finder")** | The `trash` crate's macOS **default** deletes by asking Finder over AppleScript. A refusal then makes later deletes silently do nothing ([nuza #111](https://github.com/puang59/nuza/pull/111), [Gum #5565](https://github.com/vchelaru/Gum/pull/5565)). Dev builds hide it, because they inherit the terminal's permission. | Set the delete method to **`NsFileManager`** (`TrashContextExtMacos::set_delete_method`; check the crate docs) so no Apple Event is sent. Never script Finder. "Reveal" stays `open -R` (LaunchServices, no prompt; already how `openinshell` works). A check that macOS builds never gain `tell application "Finder"` outside the DMG packager. |
| **App Management** | Modifying files inside another app bundle (`/Applications/*.app`) | Treat `.app` bundles as files; disable rename/delete/paste *into* a bundle and say why. |
| **Removable and network volumes** | Opening a mounted drive or share | List mount points by name from `/Volumes`; read contents on click only. |
| **Local network** | Already opt-in (LAN discovery switch) | Don't browse network shares by Bonjour from this pane in v1. |
| **Opening a file** | The *target* app may prompt under its own name | Out of our control; use `open`, which adds nothing. |
| **User-intent routes** | (T11) Native open panel, drop from Finder | If T11 holds, offer **Open folder…** and accept Finder drops as the prompt-free way into a protected folder. |

The same rules fix the default root: a new pane starts in the active agent's workspace (under `~/.agentmux`, unprotected), or Home, never Documents or Desktop. Listing Home itself is expected to be safe (T6 confirms).

**Acceptance test (prompt budget).** On a fresh bundle id with all permissions reset: launch; open a Files pane at its default root; browse Home's top level; open a repo under an unprotected folder; search, preview, create, rename and trash files there; restore the window and tear a pane off and back. **Zero prompts.** Then open Documents: **exactly one**, and only after the click. Anything else is a bug.

## 10. Performance budgets

Measured with the repo's CDP trace method (per-frame screenshots plus main-thread task breakdown), on three real folders: `node_modules` (~30k entries), `C:\Windows\System32` (~5k), and a synthetic 200k-entry folder.

| Budget | Target |
|---|---|
| First rows on screen, ≤1k-entry folder | < 100 ms after navigate |
| First rows, 200k-entry folder | < 250 ms (first page), full sort < 1 s, input never blocked > 50 ms |
| Scroll | 60 fps, no frame > 16 ms from row rendering |
| Watcher patch applied | ≤ 200 ms after the disk change, one patch per 100 ms window |
| Selecting a file | preview starts only after 150 ms idle; cancels on the next selection |
| Watchers | only visible and expanded folders; hard cap (64 per window), LRU; never recursive |

## 11. Tests

- **Rust (srv):** cursor pagination (TTL, cancel by drop); per-entry errors; symlink and junction loops; hidden/system attributes; Windows long paths and reserved names; case-only rename; copy/move conflicts; cancel mid-copy leaves no half-file; trash worker serialization; watcher `Lagged` and overflow produce `rescan`.
- **Frontend (vitest):** selection model (range, toggle, extend, select-all); type-ahead buffer; sort (natural, folders-first); path/breadcrumb parsing for `C:\`, UNC, `~`; the windowing helper; keyboard table; undo stack.
- **Integration (CDP trace):** the §10 budgets as a script (`scripts/files-perf.mjs`), run before and after.
- **Manual matrix:** Windows (NTFS, a network share, `\\wsl$`), macOS (TCC folder, bundle), Linux (case-sensitive, `/mnt`).

## 12. Rollout

Each phase is independently shippable and reviewable.

- **Phase 0, shared foundation (no visible change).** Extract the tree model to `frontend/app/fs/`. Add generic `fs.list` (paginated), `fs.stat`, `fs.watch`, `fs.places`, with the old RPC names as aliases. Tests for the above.
- **Phase 1, the pane (MVP).** On macOS this includes the access-prompt handling of §9.1.4 (pre-prompt notice, waiting state, denied state, the `Info.plist` strings), settled by the §9.1.3 tests first. `files` view and `defwidget@files`; details view, breadcrumb, places, history, selection and keyboard, sort, hidden toggle, type-ahead, open-by-kind, context menu, watcher patches, rename and new item, **trash with undo**. `OpenFiles` MCP tool. Settled-content contract. No copy/move jobs yet.
- **Phase 2, operations and previews.** The job queue (copy/move/permanent delete, conflicts, progress), in-app drag between panes, OS file drop in, drag to the agent composer, *Attach to agent*, preview panel, tree view and sidebar, filter, git decorations (via `git status --porcelain=v2 -z`, `.gitignore`-aware dimming using the `ignore` crate's matcher).
- **Phase 3, agent-aware and extra views.** Agent workspaces in Places, touched-by badges, Miller columns, thumbnail grid, fuzzy find in the folder tree.
- **Phase 4, convergence.** The Editor's side tree becomes an instance of the shared component; the editor-specific tree code is deleted.
- **Later.** Remote roots (containers, SSH).

### 12.1 What v1 shipped (#4201)

- **srv:** `fs.list` (cursor-paginated, `spawn_blocking`, per-entry errors, hidden/system attributes, `os_blocked`), `fs.places`, `fs.watch`/`fs.unwatch` (`files:changed`, coalesced at 150 ms, 64 per block), `fs.rename` (never overwrites; case-only via a temp name), `fs.create`, `fs.trash`/`fs.restore` (one trash thread; `NsFileManager` on macOS; restore on Windows and Linux only), `fs.delete`, `fs.open`, `fs.reveal`. The mutation policy of §9 is enforced at execution time, with the protected list refused outright and agent workspaces under `~/.agentmux/agents/<name>/` allowed. `pane.open` with view `files` and `select`, layout export/import, `defwidget@files`, the `OpenFiles` MCP tool, `mux view <folder>/`.
- **Frontend:** the details view (hand-windowed rows keyed by name, natural sort, folders first), breadcrumb and Ctrl+L, history, Places with drives and agent workspaces, the §5.3.1 keys and type-ahead, open by kind beside the pane, the context menu, rename and new item, Trash with Undo, Shift+Delete with a named confirmation, live re-list (deferred while hidden), the settled-content hold, and the macOS rules of §9.1.4: a protected folder is never listed until the user clicks Open on the pane's explanation, once per release; a denial says where to turn access on. `Info.plist` has the five folder usage strings.
- **Not built from Phase 0:** the editor tree model was not extracted, `fs.stat` was not added, and the editor RPCs keep their names.
- **Not yet verified:** nothing has been run on macOS (the §9.1.3 test plan stands), and the §10 budgets have not been traced.

### 12.2 Phase 2a: transfers, preview, filter, files to agents

- **Copy and move jobs (§7.1, §7.2):** `fs.op.start` / `fs.op.resolve` / `fs.op.cancel`, with `files:op` progress events scoped to the pane. Conflicts ask Replace, Skip or Keep both, optionally for every conflict; a folder onto a folder merges. A move within a drive is a rename; across drives it is a copy, and each source is deleted only after its copy finished. Cancel stops between files and between chunks, and removes a partial file. Links are copied as links, never followed.
- **Clipboard:** Ctrl+X / Ctrl+C / Ctrl+V (and Cut, Copy, Paste in the menus), shared by every Hangar pane in the window. Ctrl+C also puts the paths on the system clipboard. A cut is pasted once.
- **Drag (§8.2, route 1):** a row (or the selection) drags as an in-app *path drag* (`beginPathDrag` in `app/drag/file-drop.ts`). The drop controller treats it like an OS file drop that carries paths, so the agent composer, the Editor, Media and terminal panes take it unchanged. Dropped on another Hangar pane it moves (same drive) or copies; OS files dropped on Hangar are copied in.
- **Attach to agent (§8.2, route 2):** one menu item per agent pane on screen, which hands the paths to that pane's own drop hook (`dropPathsOnto`), so it ends exactly where a drag would.
- **Preview panel (§6.6):** Space toggles it (Ctrl+Space toggles the focused row's selection instead). Code is highlighted, Markdown rendered, images shown; text reads only the first 256 KB, through a ranged request; binary files and video/audio get a card. A preview starts 150 ms after the selection settles and is abandoned when it moves.
- **Filter:** Ctrl+F or `/`, case-insensitive, per folder.
- Dropping onto a folder row puts the files in that folder (the row is outlined while the drag is over it), including a drag within the same pane; dropped anywhere else in the pane they go into the folder shown, and a same-pane drop there does nothing.
- **Mention (§8.2, route 3):** Alt+K (or *Mention in agent* in the row menu) splices an `@path` per selected entry into the message box of the agent the user last worked in (the only agent pane, if there is one), relative to that agent's working folder when inside it, quoted when it has a space.
- Not built: the tree view.

### 12.3 Git markers

`fs.git_status` runs `git status --porcelain=v2 -z --branch --untracked-files=normal --ignored=matching -- .` in the folder shown, 300 ms after a listing settles, and folds the result into one state per entry: a letter beside the name (M, A, D, R, U, ! for a conflict; a folder shows the most pressing state inside it), ignored entries dimmed, and the branch with ahead/behind and the number of changes under the folder in the status line. Browsing a repository must not run code it chose. A repository's own config can name programs that `git status` executes: `core.fsmonitor`, and a `filter.<name>.clean` / `.process` applied through its `.gitattributes` whenever a file's stat data differs from the index (ReAgent found the second on #4223). Every run passes `-c core.fsmonitor=false`, blanks each filter the repository's own config defines (`git config --show-scope` tells them from the user's system and global ones, such as Git LFS, which are kept; an older git without `--show-scope` gets every filter blanked; a repository with a filter whose name contains `=`, which `-c` can't express, gets no git run at all), does not enter submodules (`--ignore-submodules=all`, as each has its own config), passes `--no-optional-locks`, inherits none of the instance's `AGENTMUX_*` environment, never prompts, and is killed after 4 s. Tests with real repositories prove both overrides matter: without them, git runs the repository's program.

### 12.4 Touched by an agent (§8.4)

A coloured dot before a row's name when an agent wrote or edited it in the last 30 minutes, in the agent's own colour (its pane border); a folder shows one when something inside it was changed. The tooltip says who, when and with which tool. The source is each agent pane's live stream: a `Write`, `Edit`, `MultiEdit` or `NotebookEdit` call is remembered by its id, and counts once its result reports success (`app/store/touched-files.ts`). Nothing watches the filesystem for this, so a user's own save shows no badge. Kept in memory for the window: up to 2,000 paths, each for 30 minutes. History replay doesn't feed it, so a reload starts empty.

### 12.5 Grid view (§5.2)

A toolbar toggle switches the folder between the details list and a grid of tiles (`files:view = grid`, kept in the block). Images get a thumbnail: read once, scaled to 192 px on the longer side with `createImageBitmap`, and kept as a small object URL (at most 300, least recently used released; 4 decoded at a time; files over the inline image cap get an icon). Everything else gets a large icon. The grid is windowed by rows like the list, and shares its selection, keyboard (left and right move a tile, up and down a row), drag, drop, menus, rename, git markers and touched-by badges.

### 12.6 Tabs: Hangar on pane tabs

> **Current (2026-10-02).** For a few hours these tabs were document tabs (#4231, `SPEC_DOCUMENT_TABS_2026_10_02.md` §6.2); the repo owner then kept document tabs for the Editor and Media only, and Hangar is back on pane tabs, as below. The paragraph below predates that spec: the Editor's files did *not* move onto pane tabs (they are document tabs), but Hangar's folders are pane tabs.

AgentMux has three kinds of tab: window tabs, pane tabs (several blocks stacked in one pane, `blockStack`), and inner-pane tabs (the Editor's open files, its own store). Pane tabs and the Editor's file tabs draw with the same `PaneTabStrip`; their state is separate, and SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md (resolution 3) has decided the Editor's should move onto pane tabs. Hangar therefore uses **pane tabs**, not tabs of its own: each tab is a whole Hangar with its own folder, history, selection and view, and gets reordering, dragging between panes, tear-off and layout persistence for free.

- **Open in new tab** for a folder: Ctrl+Enter, middle-click, or the row menu. It opens beside the current tab in the same pane.
- **Ctrl+T** (or *New tab here* in the folder menu) opens the folder shown in a new tab; the pane's own **+** → Hangar starts in that folder too (the manifest's `chrome().newTabMeta`).
- **Ctrl+W** closes this Hangar tab, but never the pane's last one (the pane's × does that). These keys are Hangar's own: app-wide pane-tab shortcuts are still unbuilt.

## 13. Names

Constraints: short, a single word that reads on a widget bar, doesn't collide with an existing widget (Agent, Swarm, Browser, Editor, Terminal, Sysinfo, Drone, Help, Warden, Media, Toolchain, Armory, Settings), and says "files". **"Browser" is out**: the web Browser widget owns it. The product's own names lean on a fleet/operations theme (Armory, Swarm, Warden, Drone) and a `mux` family (muxbus, muxlog, muxspect).

| Name | Why it works | Against |
|---|---|---|
| **Hangar** | Where a fleet's craft are kept and serviced. Sits naturally beside Armory, Drone and Swarm. Zero hits in the repo. Easy to say; icon: a folder-open or a hangar silhouette. | Needs the "Files" alias for discovery. |
| **Muxplorer** | The `mux` family plus an obvious pun on *explorer*. Self-explanatory. Zero hits. | A pun can wear thin; long for a widget label. |
| **Depot** | Plain storage metaphor; reads as "stuff is kept here". | Overloaded (build depots, Steam depots). |
| **Cairn** | A trail marker: a pile of stones that says *the path goes this way*. Short, distinctive. | Obscure; doesn't say "files" at all. |
| **Atlas** | A map of everything. Familiar. | Generic and heavily used. |
| **Quartermaster** | The officer who issues supplies; fits Armory/Warden. | Long. One doc already mentions it. |
| **Hive / Comb** | The Swarm widget's icon is a bee; a hive is where the swarm's work lives. | Blurs with Swarm, which is about agents. |
| **Locker** | Personal storage. | Suggests private/secure, which it isn't. |
| **Files** | Honest and discoverable, like Editor, Media and Terminal. | No personality; the app's own files vs. the user's files is ambiguous. |
| **Explorer** | What users already call it. | Windows' trademark of the idea; confusing next to "reveal in Explorer". |

**Recommendation: *Hangar*, registered with the aliases `files`, `explorer` and `dir`** so the widget bar search and `OpenFiles` both find it. Runner-up: *Muxplorer* if the owner wants the pun. The internal view type can be `files` either way, since it is what the code and block meta use (`files:path`); only the label, icon and docs carry the brand.

## 14. Decisions

The repo owner asked (2026-10-01) to implement the browser and "take it to the end" without stopping for each question, so v1 takes this spec's recommendation on each. Any can be revisited.


1. **Name:** **Hangar.** The view type is `files`, the label and widget say Hangar, and the command palette has *Open Hangar (Files)*. No `explorer`/`dir` aliases: in this registry an alias is a migration path for an old `meta.view`, not a search keyword.
2. **Editor tree:** **kept until Phase 4.** v1 did not extract the tree model (§12.1).
3. **Delete policy:** **yes.** Delete moves to the Trash with no confirmation and offers Undo; Shift+Delete asks with the item's name.
4. **Mutation scope:** **any path, minus the protected list in §9**, which is refused outright in v1 rather than behind a typed confirmation.
5. **Default root:** **Home** for a pane opened from the widget bar; `OpenFiles` opens wherever the agent says.
6. **Undo across restarts:** **in memory, per pane.**
7. **macOS Trash undo:** **not in v1.** Undo of a trash says it isn't supported on macOS yet; Windows and Linux restore through the `trash` crate.
8. **Remote roots:** **open.** Not needed for v1.
9. **Dragging out to the OS:** **Copy path and Reveal for v1.**
10. ~~Per-version bundle id~~ **Decided 2026-10-01: keep.** The reset on each release is intended, so every release re-tests the permission flow.

## 15. Sources

- Wave Terminal: [File Preview and Editor Views (DeepWiki)](https://deepwiki.com/wavetermdev/waveterm/4.2-file-operations); [Wave docs, widgets](https://docs.waveterm.dev/widgets); [Introducing Wave v0.8](https://blog.waveterm.dev/introducing-the-new-wave-terminal-v08).
- Accessibility: [W3C ARIA APG, Tree View](https://w3.org/WAI/ARIA/apg/patterns/treeview); [MDN, `treeitem`](https://developer.mozilla.org/en-US/docs/Web/Accessibility/ARIA/Roles/Treeitem_Role); [Aria UI treegrid](https://www.ariaui.dev/docs/components/treegrid).
- Watching and performance: [VS Code issue #61393](https://github.com/microsoft/vscode/issues/61393) and [#142673](https://github.com/microsoft/vscode/issues/142673) (watcher excludes and large workspaces); [`notify`](https://github.com/notify-rs/notify); [`windows-file-watcher` (ReadDirectoryChangesW overflow)](https://docs.rs/windows-file-watcher/latest/windows_file_watcher/); [nicti #139 (burst and rescan behaviour)](https://github.com/jordanfelle/nicti/issues/139); [Yazi guide (async Miller columns)](https://blog.starmorph.com/blog/yazi-terminal-file-manager-guide).
- Destructive actions: [A UX guide to destructive actions](https://medium.com/design-bootcamp/a-ux-guide-to-destructive-actions-their-use-cases-and-best-practices-f1d8a9478d03); [Files app PR #11185 (configurable delete confirmation)](https://github.com/files-community/Files/pull/11185); [`trash` crate](https://docs.rs/trash); [`ignore` crate](https://docs.rs/ignore).
- Windows paths: [Naming Files, Paths, and Namespaces (Microsoft)](https://learn.microsoft.com/nl-nl/windows/win32/fileio/naming-a-file); [Maximum Path Length Limitation (Microsoft)](https://learn.microsoft.com/cs-cz/windows/win32/fileio/maximum-file-path-limitation).
- macOS permissions: [Apple Support, files and folders](https://support.apple.com/guide/mac-help/control-access-to-files-and-folders-on-mac-mchld5a35146/mac); [Apple DTS forum thread 126430](https://developer.apple.com/forums/thread/126430); [Eclectic Light, Files & Folders vs Full Disk Access](https://eclecticlight.co/2026/04/08/privacy-files-folders-or-full-disk-access/); [Full Disk Access inheritance](https://lapcatsoftware.com/articles/FullDiskAccess.html); [penguin-harness #846](https://github.com/Prism-Shadow/penguin-harness/pull/846); [t3code #5943](https://github.com/pingdotgg/t3code/issues/5943); [YARG #1695 (ad-hoc re-prompts)](https://github.com/YARC-Official/YARG/issues/1695).
- Prior art: [Files (Windows) and Spacedrive on AlternativeTo](https://alternativeto.net/software/windows-explorer/?license=opensource); [Spacedrive](https://github.com/spacedriveapp/spacedrive); [Marta docs](https://marta.sh/docs/).
- Files to agents: [Claude Code in VS Code](https://code.claude.com/docs/en/vs-code); [Claude Code Desktop](https://code.claude.com/docs/en/desktop); [claude-code #51027](https://github.com/anthropics/claude-code/issues/51027); [Cursor regression: drag from Explorer to chat](https://forum.cursor.com/t/regression-drag-and-drop-from-cursor-explorer-to-chat-composer-stopped-working-windows/157586); [Kiro #3987](https://github.com/kirodotdev/Kiro/issues/3987).
