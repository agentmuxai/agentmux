# Report: dragging document tabs between panes (Editor to Editor, Media to Media)

**Status:** active. Phase 1 (drag to reorder) and Phase 2 (Media ↔ Media) are built; Phase 3 (Editor ↔ Editor) is next.
**Date:** 2026-10-09 · **Author:** agent2
**Trigger:** the repo owner: *"we want to implement dnd for document panes. Editor and Media each have document panes, but they each are not compatible with eachother. we want to implement dnd so a user can drag a doc pane from editor to another, or from one media to another, i believe it is already DRY and we want to keep it that way."*
**Written against:** `main` @ `4afd8c527`. **Builds on:** `SPEC_DOCUMENT_TABS_2026_10_02.md` (this is its Phase 5, §5.8 and §4.1), `SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md` (the pane-tab drag this reuses), `SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md` (the drag session).

## 1. Verdict

The document-tab **model** is DRY, as believed: Editor and Media tabs are one pure reducer (`frontend/app/doc-tabs/doc-tabs.ts`), drawn by one strip component (`PaneTabStrip`), persisted in one format (`doctabs`). The feature fits the existing design: `SPEC_DOCUMENT_TABS` already reserves it as Phase 5 ("a document tab dragged onto another pane's strip of the same type moves it there"), and already rules out cross-type moves, as the request does.

Three things stand in the way, and each has a DRY fix:

1. **The strip's drag is hard-wired to pane tabs.** `PaneTabStrip`'s drag code (§3.1) tags every drag `PANE_TAB_ITEM`, opens a `"pane-tab"` drag session, sets up tear-off into a floating pane, and treats the whole surrounding pane as the drop zone. That is why `DocTabStrip.tsx` and the spec deliberately left document drag unwired. **Fix:** make the drag kind a property of the strip, not a constant, so pane tabs and document tabs share every line of it.
2. **Nothing lets one pane hand a tab to another.** Each pane's tabs live in its own model, and there is no way for a drop target in pane B to take a tab out of pane A. **Fix:** one small per-block registry of document-tab hosts with `give` and `take`, implemented once for any pane on `DocTabsController` (Media), and once for the Editor.
3. **The reducer has no commands for leaving or joining a pane.** `closeDoc` puts a tab on the reopen list, and `openDoc` builds a new tab from scratch. Neither is a move. **Fix:** two pure functions, plus one for drag reorder, tested once for both types.

The one part that is not mechanical is the **Editor's in-memory state**. Unsaved text, encoding and undo history live outside the reducer, so moving a dirty Editor tab needs a hand-off of live state, not just the serialized form (§3.4).

## 2. What exists

| Piece | Where | State |
|---|---|---|
| Tab model and commands | `doc-tabs/doc-tabs.ts` | Pure, tested. `openDoc`, `closeDoc`, `moveDoc(delta)`, `updateDoc`, persistence. No detach/attach, no move-to-position. |
| Controller (signal, debounced save, keys) | `doc-tabs/doc-tabs-controller.ts` | Used by **Media** (`media-pane.tsx`). |
| Editor's driver of the same reducer | `store/editor-pane-state-store.ts` | Its own command vocabulary over `doc-tabs.ts` (`OpenFile`, `CloseTab`, `ReorderTab{toIndex}` → `moveDoc`, ...). `ReorderTab` exists but nothing dispatches it from a drag. |
| Strips | `DocTabStrip.tsx` (Media), `editor-tab-strip.tsx` (Editor) | Both thin wrappers over `element/PaneTabStrip.tsx`. Neither passes `onReorder`, so no drag is registered on their pills. |
| Drag machinery | `PaneTabStrip.tsx`, `drag/pane-tab-drag.ts`, `drag/drag-session.ts`, `drag/drag-types.ts` | pragmatic-dnd `draggable()` per pill and `dropTargetForElements()` per pill (reorder) and per pane (receive). Every tag, session kind and tear-off path is pane-tab specific. |
| Per-block model lookup | `store/block-component-registry.ts` | Exists for block components; there is no lookup for a block's Editor or Media model. |
| File drops (OS files) | `drag/file-drop.ts`, `editor-drop.ts`, `media-drop.ts` | Separate channel (native `Files` drags); unaffected by element drags. |

## 3. Design

### 3.1 One drag implementation for both tab layers

`PaneTabStrip` gains one prop that says which kind of tab it carries. Pane-tab strips pass nothing and behave exactly as today.

```ts
/** Which drag this strip's pills start, and which they accept. Omitted: pane tabs (today's behaviour). */
docDrag?: { docType: "editor" | "media"; hostBlockId: string };
```

With `docDrag` set, the same code paths run with different constants:

| | Pane tab (today) | Document tab |
|---|---|---|
| pragmatic tag | `PANE_TAB_ITEM` | `DOC_TAB_ITEM` (new, `drag-types.ts`) |
| drag-session kind | `"pane-tab"` | `"doc-tab"` (new `DragKind`) |
| drag data | `{ blockId, sourceNodeId }` | `{ tabId, docType, sourceBlockId }` |
| tear-off on drag-out | yes (`sourceTabId`) | **no** (v1, §5); no cross-window payload is set |
| reorder within the strip | `onReorder` | `onReorder` |
| receive from another pane | whole pane (`foreignDropRootFor`) | the document pane's root, marked `data-doc-tab-host` (§3.3) |
| accepts | other pane's pane tabs | same `docType`, other block |

`pane-tab-drag.ts` (`startPaneTabDrag`, `isDraggedPaneTab`) becomes a tab-drag helper parameterized by kind, so the "this pill is being dragged" dimming and the landing bounce (`markLanded`) work for both without a second copy. The pane-tab path keeps its exact behaviour; its tests (`PaneTabStrip.test.tsx`, `.dropTarget.test.tsx`, `pane-tab-drag.test.ts`, `pane-tab-tearoff.test.ts`) are the regression guard.

### 3.2 Pure commands in `doc-tabs.ts`

```ts
/** Move `id` to before/after `targetId`, staying in its pinned or unpinned group (drag reorder). */
moveDocTo(s, id, targetId, position): DocTabsState<P>
/** Take a tab out of this pane: like closeDoc, but not onto the reopen list, and returned. */
detachDoc(s, id): { state: DocTabsState<P>; tab: DocTab<P> } | null
/** Put a moved tab in: at a position or after the active tab; if its key is already open, that tab is activated instead. */
attachDoc(s, tab, at?: { targetId: string; position: "before" | "after" }): DocTabsState<P>
```

A preview tab arriving in another pane becomes a normal tab (dropping it was a deliberate act). Pinned stays pinned. `DocTabsController` and the Editor store each get thin wrappers. The Editor store also needs `DetachTab` and `AttachTab` commands so its audit ring sees the move. Its existing `ReorderTab` can take `moveDocTo`'s target form.

### 3.3 One registry of document-tab hosts

New `doc-tabs/doc-tab-hosts.ts`, keyed by block id. Each document pane registers itself on create and unregisters on dispose. Owner-checked, like `block-component-registry.ts`, because a keep-alive block can be mounted twice during a move.

```ts
interface DocTransfer {
    docType: "editor" | "media";
    tab: DocTab<unknown>;     // key, title, icon, pinned, payload (in-memory, not serialized)
    live?: unknown;           // type-specific state that isn't in the payload (§3.4)
}
interface DocTabHost {
    docType: "editor" | "media";
    /** Why this tab can't leave (a dirty conflict, a keepOne pane's last tab), or null. */
    refuseGive(tabId: string): string | null;
    /** Why this transfer can't come in (a remote Editor and a local one), or null. */
    refuseTake(t: DocTransfer): string | null;
    give(tabId: string): DocTransfer | null;
    take(t: DocTransfer, at?: { targetId: string; position: "before" | "after" }): void;
}
```

- **Media** gets its host from one generic adapter, `docTabsControllerHost(ctl, docType)`, which any future `DocTabsController` pane also gets for free. A Media tab's payload already holds everything, including dropped bytes with no path (`MediaDoc.file`), so `live` is empty.
- **Editor** implements the four methods on `EditorViewModel` over its store, plus the live hand-off below.

The drop target in pane B reads the source block from the drag data, finds both hosts, asks `refuseGive` and `refuseTake` (also during hover, to show a "no" cursor rather than a silent failure), then calls `give` on A and `take` on B. As with pane tabs, the commit runs one task after the drop, because it unmounts the dragged pill, which is the live drag source (`SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md` §4.2).

**Drop zone.** pragmatic-dnd allows one drop target per element, and the pane-tab receiver already holds the pane root (`.pane-stack`). The document receiver therefore registers on the document pane's own root inside it (`.editor-view`, `.media-pane`, marked `data-doc-tab-host`). It accepts only document drags, so a pane-tab drag still bubbles up to the pane-tab receiver. Feedback goes on the document strip, where the tab will land.

### 3.4 The Editor's live state

`EditorBuffer` (the payload) holds path, language, scratch identity and load state, but deliberately not the text. The rest is per tab, outside the reducer:

| State | Lives in | On move |
|---|---|---|
| Unsaved text | `EditorViewModel._contentByTab` | carried in `live`; target marks the tab dirty |
| Encoding, BOM, line endings | `_encodingByTab` | carried, so a save after the move writes the same bytes |
| File watch | `_watchedPathByTab` | source stops watching, target starts (the existing `_syncWatch`) |
| CodeMirror state (undo and redo history, cursor, selection, scroll) | `cmStates` in `editor-view.tsx`: one `EditorState` per tab, saved on switching away and restored on switching back; the active tab's is the live view's | **undo history and selection carried as JSON** (`state.toJSON({ history: historyField })`), rebuilt in the target with its own extensions (`EditorState.fromJSON`). The `EditorState` object itself can't move: its extensions include the source view's `updateListener`, which reports every edit to the source pane's model. Scroll position starts at the top. `cmStates` moves from the view's closure onto the model so the host can reach it |
| LSP document | opened for the active tab only | nothing to do: activation in the target sends `didOpen`, the source's switch sends `didClose` |
| Scratch buffer | cache file named by `scratchId` | moves with the payload; same cache file |

Rules the Editor host enforces:
- **Same connection only.** An Editor can be on another host (`connection()`); its paths are that host's. A tab never moves between Editors on different connections (local ↔ remote, host A ↔ host B).
- **Same file already open in the target.** Refused with a message saying so, clean or dirty, for every type (`ALREADY_OPEN_THERE` in `doc-tab-hosts.ts`). Moved in, the tab would merge into the target's and vanish from where it was: the repo owner, testing a first build that did that for a clean Media tab, read it as a lost file.
- A tab still loading moves as a not-yet-loaded tab; the target reads it when it is shown (the existing `_ensureActiveLoaded`).

Media's own rules:
- An empty tab ("Click to load media") drags like any other: to reorder it, or to move it and load a file there. A file moved onto a pane showing only its empty tab takes that tab's place; an empty tab moved there joins it. (A first build refused to drag an empty tab; the repo owner, testing, expected it to move.)
- The source pane keeps its "always one tab" rule: moving its last tab leaves an empty one behind, as closing does today.

### 3.5 Reorder within a pane comes free

With §3.1 and `moveDocTo`, passing `onReorder` to both strips gives drag-to-reorder inside one pane. `SPEC_DOCUMENT_TABS` §4.1 deferred this only for want of a document drag kind. It needs no host and no registry, so it is the right first step.

## 4. Plan

| Phase | What | Risk |
|---|---|---|
| 1 | §3.1 drag kind in `PaneTabStrip` (no change for pane tabs), §3.2 `moveDocTo`, and drag-to-reorder in both strips (§3.5) | Low. Pane-tab tests guard the shared path. |
| 2 | §3.2 detach/attach, §3.3 registry and drop zone, **Media ↔ Media** | Low. The payload is self-contained. |
| 3 | **Editor ↔ Editor** with §3.4's live hand-off and rules | Medium: dirty buffers, watches, connections. A live check of move-dirty, then save in the target; move a scratch buffer; move to an Editor with that file open; local ↔ remote refused. |
| 4 (later) | Drop on empty layout space, or **Move to new pane** from a tab menu: the receiving pane is created first, then `take` runs once its host registers. Cross-window via the `serialize` form, as `SPEC_DOCUMENT_TABS` §5.8 foresees. | Needs a tab menu, which doesn't exist yet. Cross-window touches the per-platform `CrossWindowDragMonitor`s. |

Tests, per phase: reducer cases in `doc-tabs.test.ts` (`moveDocTo` group bounds; `detachDoc` not on the reopen list; `attachDoc` key de-duplication and preview promotion); `PaneTabStrip` with `docDrag` registering a document tag and no tear-off; registry owner-checks; per-type host tests, including the Editor's carried text, dirty mark and encoding, and its refusals.

## 5. Non-goals

- Cross-type moves (Editor ↔ Media), as asked and as `SPEC_DOCUMENT_TABS` §2 rules out.
- Tearing a document tab off into its own window, and cross-window moves (Phase 4 at the earliest).
- Dragging a file from Hangar onto a document pane. That is a different drag: a native path drag (`beginPathDrag` in `files-view.tsx`) handled by the file-drop facility, not a document-tab move.

## 6. Open questions for the repo owner

1. **Dropping onto a pane's body, or only its tab strip?** Proposed: anywhere on the document pane, matching how pane tabs join a pane, with the feedback on its document strip.
2. **Undo history across a move.** Each Editor tab keeps its own undo history today (`cmStates`, kept across tab switches, dropped on close, not kept across a restart). Proposed: carry it, as JSON, per §3.4. The cost is small, and losing undo by dragging a tab would read as a bug. The alternative is carrying only the text in v1.
3. **File already open in the target.** Settled: refuse with a message, clean or dirty (§3.4).

## 7. DRY notes found on the way (not part of this work)

- The cross-window drag payload type, `DragItemPayload`, is declared three times, once in each of `CrossWindowDragMonitor.darwin.tsx`, `.linux.tsx` and `.win32.tsx`. Phase 4 would have to add its document variant three times; moving the type into `drag-types.ts` first would fix that.
- Two drivers of one reducer: Media uses `DocTabsController`; the Editor drives `doc-tabs.ts` through its own store. That's why §3.3 needs two host implementations instead of one. Converging the Editor onto the controller is a larger change (its store also carries dirty confirmation, the audit ring and command sources), and this work doesn't need it.
- `SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md` still says *"nothing in this spec is implemented"*, but its §5.1 drag session (`drag-session.ts`) and §5.3 file-drop facility (`file-drop.ts`) are in the tree and cite it.
