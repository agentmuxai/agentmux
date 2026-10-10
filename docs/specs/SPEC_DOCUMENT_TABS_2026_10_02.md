# SPEC: Document tabs: one shared tab system for the documents inside a pane

**Status:** active. Phase 1 is built (#4231): the shared layer, first with Hangar on it; Hangar has since gone back to pane tabs (§6.2). Phase 2 is built: the Editor on it (#4233, §6.1). Phase 3 is built for Media (§6.3); Phase 5 is not; Phase 4 (Browser) is dropped (§6.4). Where the build differs from the design below, the section says so. The repo owner settled the three-layer model and the name on 2026-10-02 (§1). Written against `main` @ `cede0f2b8`.
**Date:** 2026-10-02
**Author:** korp
**Trigger:** Repo owner, 2026-10-02: *"there are actually 3 types: Window tabs, Pane tabs, and inner-pane tabs"*; *"The media pane tabs would also be in-pane tabs"*; *"ok document tabs. so u will create 1 document tab system that editor, hangar, media, (does browser have it too?) and whatever types"*; and *"lets also backreference old docs to this, so old stuff like that idea you found is squashed"*. Later the same day the repo owner narrowed it: *"only Editor and Media really need document tabs. Terminal and Browser do not"*, then *"I think only editor and media need the document tabs .. which make semantic sense in the end"* (§6.2, §6.4).

**Supersedes, in part** (each of these now carries a note pointing here):
- `SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md` §7 resolution 3 ("Editor files-tabs migration onto `blockStack`: yes"), the §2.4 bullet that sets it up, §2.2's tab taxonomy (which has no document layer), and the `Ctrl:Shift:T` row of §4.9.
- `PLAN_PANE_TABS_UNIVERSAL_IMPLEMENTATION_2026_09_17.md`: the deferred "Editor's own files-tabs migration onto real `blockStack` semantics" item.
- `SPEC_BROWSER_AND_EDITOR_PANES_2026_04_16.md`: as to the Editor's files (§6.1). Its non-goal "Tab management in browser pane" **stands**: the repo owner dropped Browser tabs on 2026-10-02 (§6.4).
- `SPEC_FILE_BROWSER_PANE_2026_10_01.md` §12.6 (Hangar on pane tabs) and the §3 line "We get tabs for free (pane tabs, window tabs)".
- `SPEC_MEDIA_PANE_2026_07_26.md`: one file per Media pane.

**Related, unchanged:** `SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md` (origin of the shared `PaneTabStrip` component this reuses), `SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md` (how a view registers; document tabs are a capability a view opts into, §5.6), `SPEC_EDITOR_MCP_OPEN_BLANK_PREVIEW_AND_PANE_REUSE_2026_08_03.md` (the editor's pending-open queue, which §5.7 generalizes).

---

## 1. The three layers of tab

| Layer | What one tab is | Lives in | Strip | Examples |
|---|---|---|---|---|
| **Window tabs** | a whole layout of panes | the workspace | top of the window (`tabbar.tsx`) | Tab 1, Tab 2 |
| **Pane tabs** | one block (one view instance) stacked in a pane slot | the layout tree (`blockStack`) | the pane header (`PaneChrome` → `PaneTabStrip`) | an Agent and a Terminal sharing a slot |
| **Document tabs** | one *thing a pane shows*, inside one block | the block (its meta, §5.3) | under the pane header (`DocTabStrip` → `PaneTabStrip`) | Editor files, Media files |

The rule that decides the layer: **a pane tab is a different pane; a document tab is a different document in the same pane.** An Agent and a Terminal are different panes; two files are two documents of one Editor. Agent and Terminal have no document tabs: each instance *is* its one session.

A document is a file the pane shows. Folders (Hangar) and pages (Browser) are not documents in this sense: those panes stay one thing each, and more of them are more panes or pane tabs (§6.2, §6.4). **Doc tab** is the short form; code uses `doc-tabs` / `DocTab*`.

### 1.1 What exists today
- Window tabs and pane tabs: built, separate state, as above.
- **Editor:** document tabs in all but name. Its file tabs draw with the shared `PaneTabStrip` (`editor-tab-strip.tsx`) but keep their state in an editor-only store (`editor-pane-state-store.ts`: tabs, active, preview, scratch, recently closed). They are not persisted: only the legacy single `file` meta key restores one tab (the planned `editor:tabs` key, "Phase 1C" in `editor-model.ts`, was never built). No keyboard shortcuts; `reopenLastClosed` has no caller; drag-reorder is not wired.
- **Browser:** its reducer is already multi-tab (`store/browser-pane-state/types.ts`: `tabs[]`, `activeTabId`, `ReorderTab`), but no strip is drawn and the model only ever opens one tab.
- **Media:** one file per pane.
- **Hangar:** #4227 put its tabs on *pane tabs* (each folder its own block). This spec first moved them (§6.2); the repo owner then kept Hangar on pane tabs, where it is now.

## 2. Goals and non-goals

**Goals**
1. One document-tab system: one model, one strip, one key set, one persistence format, used by every pane type that shows documents. A new type opts in by declaring what its documents are (§5.6).
2. Feel like a browser or an IDE: open, close, switch, reorder, reopen closed, pin, preview tabs.
3. Survive restart: a pane's document tabs come back after the app restarts.
4. Agents add documents to an open pane instead of piling up panes (§5.7).

**Non-goals (v1)**
- Changing window tabs or pane tabs.
- Moving a document tab between panes of *different* types, or between windows (tear-off); §5.8 keeps the seam.
- Grouping, colouring or nesting document tabs.

## 3. Prior art

| | Layers | Notes |
|---|---|---|
| VS Code | window → editor group → editor tab | An editor group is our pane; its tabs are document tabs. Preview tabs (italic, replaced by the next single-click), pinned tabs, Ctrl+Tab MRU, Ctrl+Shift+T reopen, dirty dot. |
| JetBrains | window → split → editor tab | Same shape; tab limit with oldest-closed eviction. |
| Browsers | window → tab | Tab discarding: hidden tabs are unloaded past a memory budget and reloaded on activation (§6.4). |
| cmux | workspace → pane → surface | Our pane tabs. It has no per-pane document layer, which is why §7 resolution 3 of the pane-tab spec followed it; this spec adds the layer back. |

## 4. UX

### 4.1 The strip
- Drawn **under the pane header**, above the pane's content, by `DocTabStrip` (a thin wrapper around `PaneTabStrip`, as `EditorTabStrip` is today).
- Shown when a pane has **two or more** document tabs. A type can ask to show it always (`alwaysShowStrip`): the Editor does, matching today, and so does Media (§10 question 2).
- Each tab: icon, title, close ×, and per-type marks: **dirty** (a dot that the × replaces on hover), **preview** (italic title), **pinned** (no ×, kept left).
- "+" at the end: a new document tab (§4.3, `Ctrl+T`).
- Reorder: `Ctrl+Shift+PageUp` / `PageDown` (§4.3). Pinned tabs stay before unpinned ones. **Drag to reorder is deferred to Phase 5** (as built in Phase 1): `PaneTabStrip`'s drag is the pane-tab drag (its own drag kind, with tear-off into a new window), so a document tab dragged with it would be taken for a pane tab. Document tabs get their own drag kind with §5.8's between-pane drag.
- The **pane tab's label** in the header is the active document tab's title (through the view's `liveTitle`), so a pane reads "README.md" or "docs", not "Editor".

### 4.2 Preview tabs
A single-click open (Hangar row, Editor tree, a Media file stepped through) opens a **preview tab**: there is at most one per pane, and the next preview replaces it. Editing, double-clicking the tab, or a double-click open makes it a normal tab. Types without previews (Browser) never make one.

### 4.3 Keys
These act only while focus is inside a pane that has document tabs, and take precedence there. Panes without document tabs (Terminal, Agent) are unaffected, so the shell keeps `Ctrl+W` and `Ctrl+T`.

| Action | Keys |
|---|---|
| New document tab (§5.6 `newDocument`) | `Ctrl+T` |
| Close the active document tab | `Ctrl+W`, middle-click a tab |
| Next / previous tab | `Ctrl+Tab` / `Ctrl+Shift+Tab`, `Ctrl+PageDown` / `Ctrl+PageUp` |
| Reopen the last closed tab | `Ctrl+Shift+T` |
| Move the active tab left / right | `Ctrl+Shift+PageUp` / `Ctrl+Shift+PageDown` |
| Pin / unpin | tab menu |

`Ctrl` is literal on every platform, matching the app's convention that `Cmd:` chords are window-level and `Ctrl:` chords pane-level and below (pane-tab spec §4.9). **`Ctrl+Shift+T`** was reserved by that spec's §4.9 for "new pane tab", a binding that was never built: that reservation is withdrawn, because every browser and IDE uses it to reopen a closed tab. A new pane tab stays on the pane's "+" until pane-tab shortcuts are built and pick another chord.

### 4.4 Closing
- Closing a dirty tab asks first (the Editor's existing save/discard/cancel).
- **Closing a document tab never closes the pane.** With no tabs left the pane shows its empty state (Editor, Media, Browser start page). A type can require at least one tab (Hangar: a Hangar always shows a folder): then `Ctrl+W` on the last tab does nothing and says so. The pane's own × (or `Cmd+W`, per the pane-tab spec) closes the pane.
- Closed tabs go on a per-pane **recently closed** list (10), which `Ctrl+Shift+T` and the strip's menu reopen.

### 4.5 Tab menu
Close · Close others · Close to the right · Pin/Unpin · Copy path (types with paths) · Reveal in Hangar (types with paths) · **Move to new pane** (Phase 5): the document becomes its own pane tab, beside this one.

## 5. The shared layer

### 5.1 Where it lives
`frontend/app/doc-tabs/`: the pure model, its commands and persistence (`doc-tabs.ts`); a controller holding one pane's state as a signal, saving it debounced, plus the keys (`doc-tabs-controller.ts`); the strip (`DocTabStrip.tsx`). No view-specific code. Tests: `doc-tabs.test.ts`.

### 5.2 Model
```ts
interface DocTab<P> {
    id: string;            // stable for the tab's life
    key: string;           // identity of the document (a path, a URL): opening an
                           // already-open key activates that tab instead of adding one
    title: string;
    icon?: string;
    preview: boolean;
    pinned: boolean;
    dirty?: boolean;       // from the type (Editor buffers)
    payload: P;            // the type's own data: what to show, and how
}
interface DocTabsState<P> {
    tabs: DocTab<P>[];     // display order; pinned first
    activeId: string | null;
    mru: string[];         // most recently active first (Ctrl+Tab order)
    closed: DocTab<P>[];   // most recent first, at most 10
}
```
Commands (one reducer, pure, tested once for every type): `Open { key, payload, title, preview?, activate?, at? }` (an open key activates; a preview replaces the pane's preview tab), `Activate`, `Close { id }` (the caller has confirmed a dirty one), `CloseOthers`, `CloseToRight`, `Reorder`, `Pin`/`Unpin`, `Promote` (preview → normal), `ReopenClosed`, `Update { id, title?, icon?, dirty?, payload? }`, `Hydrate`.

### 5.3 Persistence
One block-meta key, `doctabs`, written debounced (300 ms) on any change, never containing transient state:
```json
{ "v": 1, "active": 1, "tabs": [ { "key": "C:\\repo\\a.ts", "title": "a.ts", "icon": "file", "pinned": true, "state": { … } } ] }
```
As built, `active` is an index into `tabs` and tabs carry no id: ids are minted fresh on restore, so nothing depends on them surviving a restart.
`state` is what the type's `serialize(payload)` returns (a path, a URL, a scroll line): small and JSON-safe. Preview tabs and the closed list are not persisted. On restore, `deserialize` rebuilds each payload; a document that's gone (a deleted file) restores as a tab that says so. This replaces the Editor's unbuilt `editor:tabs` and its legacy single-`file` restore (kept read-only for one release, for a downgrade).

### 5.4 Content per tab
The layer manages tabs, not content. Each type decides how inactive documents are kept:
- **Keep state, mount one** (Media): each tab's state object lives in memory; only the active tab's view is mounted.
- **Keep mounted, show one** (Hangar while it had document tabs, #4231; no current user): each tab's view stays mounted and hidden, so its scroll, filter and an open rename survive a switch. Hangar's views are plain DOM, so this costs little; only the active one listens for drops, holds settled content or applies an OpenFiles selection.
- **Keep instances** (Editor): one CodeMirror `EditorState` per tab, swapped into one view, which is how the editor already works.
- **Keep a budget** (Browser): §6.4.

### 5.5 Focus, the pane title, the settled-content contract
- Switching a document tab keeps focus in the pane and moves it into the new document.
- The view's `liveTitle` is the active tab's title.
- A pane holds its settled-content hold (`ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md` §9) until the **active** document has painted; inactive documents never hold it.

### 5.6 How a pane type opts in
In its pane-tab manifest (`SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md`), a new optional field:
```ts
docTabs?: {
    serialize(payload): unknown; deserialize(state): Payload | null;
    titleOf(payload): string; iconOf?(payload): string; keyOf(payload): string;
    newDocument?(ctx, activePayload?): Payload | null;   // Ctrl+T, "+"
    keepOne?: boolean;            // §4.4 (Hangar)
    alwaysShowStrip?: boolean;    // §4.1 (Editor)
    supportsPreview?: boolean;    // §4.2
}
```
The host renders the strip, owns the keys and the persistence, and hands the view its active document. The view renders that document.

**As built in Phase 1** the manifest field is not there yet: a view that wants document tabs makes a `DocTabsController` with a `DocTabsSpec` (the fields above) and renders `DocTabStrip` itself, as Media's `media-pane.tsx` does (and Hangar's did). Moving that into the host is for when a second type adopts the layer (Phase 2), so its shape is settled by two users, not one.

### 5.7 Agents and other panes opening documents
`OpenEditor`, `OpenFiles`, `OpenMedia` (and a future `OpenBrowser`), and in-app opens (Hangar's open-by-kind, a Read row's image), **add a document tab to an existing pane of that type in the window tab on screen**, preferring the focused one, then the most recently focused. They create a new pane only when none exists or the caller asks (`new_pane: true`). This generalizes the Editor's `editor:pending_open_files` queue (`SPEC_EDITOR_MCP_OPEN_BLANK_PREVIEW_AND_PANE_REUSE_2026_08_03.md`) to a `doctabs:pending` meta queue drained by the host, so it works for every type.

### 5.8 Between panes (Phase 5)
Dragging a document tab onto another pane's strip of the **same type** moves it there; onto empty layout space, it becomes a new pane. **Move to new pane** does the same from the menu. The payload's `serialize` form is what moves, so it works across windows later.

## 6. Pane types

### 6.1 Editor (Phase 2)
**As built:** `editor-pane-state-store.ts` keeps its commands, events and audit ring, but its state is the shared model (`doc`, a `DocTabsState` whose payload is the Editor's `EditorBuffer`: path, language, load state, scratch identity), changed only through `doc-tabs.ts`'s functions; `tabs`, `activeTabId` and `recentlyClosed` are a projection of it in the shape the editor already read, reusing unchanged tabs' objects. So new tabs open after the one in front, closing the tab in front activates the one used before it, and the reopen list is the shared one (a reopened tab reads its file again; a scratch buffer comes back as one). Persistence is the `doctabs` record (§5.3) with each tab's path, language and scratch identity; a restored tab reads its file when first shown; `file` is still written for a downgrade and cleared when the last tab closes. The keys (§4.3) are handled by the editor view before CodeMirror sees them, except inside a text box. Closing a tab with unsaved changes now asks (*Discard changes*); before, the × lost them silently. The strip stays `EditorTabStrip` (`PaneTabStrip` with the Save As box inline), always shown. Not built: drag-reorder (Phase 5, §4.1); pinning to the left (no tab menu yet).

The design, as written before the build:
- Its store's tab state moves onto the shared model. What's left in `editor-pane-state-store.ts` is buffer state per tab: content hash, encoding, line endings, LSP, scratch identity. `EditorTab` becomes the payload: `{ kind: "file", path } | { kind: "scratch", scratchId }` plus language and read-only.
- Kept exactly: preview tabs (tree single-click), pin on double-click, dirty dot, save-as for a scratch, reopen closed (finally wired, `Ctrl+Shift+T`), the strip always shown.
- New for the Editor: persistence (§5.3, the never-built Phase 1C), the keys (§4.3), drag-reorder (its reducer had `ReorderTab`, never dispatched).
- Risk: highest of the four (buffers, dirty state, LSP, the file watcher). It gets its own PR, a live check of save, close-dirty, reopen and restart, and nothing else bundled in.

### 6.2 Hangar (Phase 1, first adopter): back on pane tabs
**Withdrawn by the repo owner, 2026-10-02:** *"I think only editor and media need the document tabs .. which make semantic sense in the end."* A folder is a place, not a document. #4231 built what follows; Hangar is back on pane tabs exactly as #4227 had it (`SPEC_FILE_BROWSER_PANE_2026_10_01.md` §12.6): each folder tab is its own Hangar block in the pane's stack; Ctrl+T, Ctrl+W, Ctrl+Enter and middle-click act on pane tabs. Kept from the document-tab work: Hangar's media opens join the Media pane on screen (§6.3). The design as built in #4231, for the record:
- Payload `{ path }`; key = the path. Each tab has its own `FilesModel` (history, selection, sort, scroll), made in its own reactive root, and its own mounted view (§5.4). The tabs share one queue of copy and move jobs; when one finishes, the tab in front reports it and every tab re-lists. The block's `files:path` follows the tab in front, so the pane's "+", OpenFiles and layout export keep working, and a Hangar from before document tabs opens with one tab on it. Built in `frontend/app/view/files/files-pane.tsx`.
- `Ctrl+T` and "+" open the folder shown; `Ctrl+Enter`, middle-click and **Open in new tab** on a folder open it as a document tab. These replace #4227's pane-tab behaviour. **Open in new pane** stays in the row menu for a folder you want beside this one.
- `keepOne: true`: Hangar always shows a folder; `Ctrl+W` on the last tab says to use the pane's ×.
- Single-click on a folder row in Hangar navigates the current tab (as now); there are no preview tabs.

### 6.3 Media (Phase 3)
**As built:** `frontend/app/view/media/media-pane.tsx` holds a `DocTabsController` (payload `{ path }`; a tab with no file yet is its own document). Only the tab in front is mounted (§5.4 "keep state, mount one"), so a video in a tab behind it doesn't keep playing; switching back fetches the file again. The pane always has a tab: closing the last one leaves an empty one ("Click to load media"). Ctrl+T adds an empty tab; a file dropped on a tab showing one opens a new tab, onto an empty tab it shows there; *Pick a different file* replaces the file in the tab. The tab in front watches its folder and follows newer renders, and its tab and the pane's `media:path` follow it; a tab behind doesn't watch (it shows its last file when brought back). §5.7, frontend only: a click on an image or video in an agent's reply, and Hangar's open-by-kind, add a tab to the Media pane on screen (the focused one, else the first) through a `media:open` meta queue it drains, and open a new pane only when there is none. The `OpenMedia` tool still opens a new pane: routing it is backend work (`pane.rs`, like the Editor's opt-in `reuse_editor_pane`) left for later. Not built: preview tabs for stepping through files.

The design, as written before the build:
- Payload `{ path }`. Opening a media file when a Media pane is on screen adds a tab there (§5.7). Stepping through files from Hangar's preview or a Read image opens **preview** tabs, so browsing a folder doesn't pile up tabs.
- Its directory watcher (`media_file_watcher`) watches each open tab's file, not one path.

### 6.4 Browser (Phase 4): dropped
**Dropped by the repo owner, 2026-10-02:** *"only Editor and Media really need document tabs. Terminal and Browser do not"* (Hangar went back to pane tabs the same day, §6.2). A Browser pane stays one page; more pages are more panes, as `SPEC_BROWSER_AND_EDITOR_PANES_2026_04_16.md` has it. The design below is kept for the record only.
- Wires the existing multi-tab reducer to the shared model; payload `{ url, title, favicon }`; key = a generated id (two tabs may show one URL).
- Each tab is a native CEF browser view (`nativeSurface`). Inactive views are hidden, not destroyed, up to a budget (4 live per pane by default). Past it, the least recently used is **discarded** (its URL kept) and reloaded on activation, as browsers do.
- `Ctrl+T` opens the start page; links that ask for a new tab (`target=_blank`, middle-click) open a document tab instead of a new pane.
- Closes the gap noted on the origin spec: `SPEC_BROWSER_AND_EDITOR_PANES_2026_04_16.md` listed "tab management in browser pane" as a non-goal.

### 6.5 Later types
Any view whose instance shows one of several documents: a diff viewer (one tab per file pair), Drone canvases, history transcripts, a log viewer. Each declares §5.6 and gets the rest.

## 7. Relationship to pane tabs (what changes in that spec)
- Pane tabs stay exactly as built for **different panes**. Document tabs never appear in the pane header, and pane tabs never appear in a document strip.
- The pane-tab spec's resolution 3 is **reversed**: Editor files stay inside the Editor's block, as document tabs. Its argument ("some widget types work one way, others another") is met by this spec's single shared layer, which every document-showing type uses.
- The pane header still shows only pane tabs. For a pane with one pane tab, the header shows that pane's title, which is now the active document's title (§4.1).
- Keys: `Ctrl+Shift+T` moves to document tabs (§4.3); the other pane-tab chords in that spec's §4.9 are unaffected.

## 8. Tests
- Reducer (`doc-tabs.test.ts`): every command, preview replacement, key de-duplication, MRU order, the closed list (cap, reopen), pinned ordering, `keepOne`.
- Persistence: round-trip, unknown version ignored, a missing document restored as a marked tab, preview and closed not written.
- Strip and keys: visibility threshold, marks, reorder by drag and by keys, the keys acting only inside a doc-tab pane (a terminal keeps `Ctrl+W`).
- Per type: the Editor's existing tab tests ported onto the shared layer, unchanged in what they assert; Hangar's #4227 tests were rewritten for document tabs in #4231 and restored when Hangar went back to pane tabs.
- Live checks on a dev build for each phase, the Editor's with save and close-dirty.

## 9. Rollout

| Phase | What | Notes |
|---|---|---|
| 0 | This spec, and pointers in the superseded docs | this PR |
| 1 | The shared layer (§5.1–§5.6) and **Hangar** on it (§6.2) | **built** (#4231); Hangar then went back to #4227's pane tabs (§6.2), the layer stays for the Editor and Media. Drag-reorder moved to Phase 5 (§4.1); the manifest field to Phase 2 (§5.6) |
| 2 | **Editor** on it (§6.1) | **built.** Its own PR. The slice keeps its commands on the shared model rather than being deleted (§6.1) |
| 3 | **Media** tabs (§6.3) and §5.7's open-into-existing-pane for Editor, Hangar, Media | **built for Media**, from the frontend (§6.3); `OpenMedia` and Hangar's `OpenFiles` still open new panes |
| 4 | ~~**Browser** tabs (§6.4)~~ | **dropped** (§6.4) |
| 5 | Between panes (§5.8): drag to another pane, Move to new pane; drag to reorder (§4.1) | a drag kind of its own |

## 10. Open questions for the repo owner
1. **macOS keys.** Mac users expect `Cmd+W`, `Cmd+T` and `Cmd+Shift+T` for document tabs (Safari, VS Code), but in this app `Cmd:` chords are window-level (`Cmd:t` is a new window tab). Proposed: `Ctrl` on every platform (§4.3), revisited after use. The alternative is `Cmd` on macOS only for document tabs, which means moving the window-tab chords there.
2. ~~**Strip from one tab or from two?**~~ Settled 2026-10-10: from one, for the Editor and Media. Built from two for Media first, so a pane with one document kept its row; the repo owner, testing tab drag between panes, found Media had no tab bar.
3. ~~**Browser live-view budget.**~~ Moot: Browser tabs are dropped (§6.4).
