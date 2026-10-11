# Plan — the Editor's document tabs on the shared surfaces

**Date:** 2026-10-10
**Status:** proposed — built on branch `agent2/editor-on-doc-tabs`, not merged yet; this line gets the PR number when it is.
**Author:** Agent2@narko
**Builds on:**
- [SPEC_DOCUMENT_TABS_2026_10_02.md](SPEC_DOCUMENT_TABS_2026_10_02.md) §6.1 (the Editor on document tabs) and §6.3 (Media)
- [../reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md](../reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md) §7 (DRY notes)

## 1. Why

The repo owner asked how DRY the document tabs are. The core is shared: one pure reducer (`doc-tabs.ts`) holds both the Editor's and Media's tab state, one `PaneTabStrip` draws every tab layer, and one move path (`moveDocTab`) carries a tab between panes. But around that core the Editor kept a parallel copy of every surface Media gets from the shared layer:

| Surface | Media | Editor |
|---|---|---|
| Tab strip | `DocTabStrip` | its own `editor-tab-strip.tsx` |
| Tab keys (Ctrl+T, Ctrl+W, …) | `handleDocTabKey`, `runDocTabAction` | its own action switch in `editor-view.tsx` |
| Saving tabs to the block | `DocTabsController`'s timer | its own timer in `editor-model.ts` |
| Moving between panes | `controllerHost` | its own `editorDocTabHost` |

So each document-tab feature was built twice, and the copies had started to drift (a restored tab's title and icon fix reached Media only).

## 2. What stays

The Editor keeps its store (`editor-pane-state-store.ts`). It already runs on the shared reducer, and what it adds is the Editor's own: the dirty-close confirmation, scratch buffers, file load state, its events (the view's CodeMirror cache, file watches and LSP listen to them) and the audit ring. Folding those into `DocTabsController` would move them, not remove them.

## 3. What changes

1. **`DocTabsModel<P>`** (`doc-tabs-controller.ts`): the tab list, the active tab and the tab actions (activate, close, cycle, move, reopen, new, pin, detach, attach). `DocTabsController` is one. The Editor's store is another, through `editorDocTabs(model)` (`editor-doc-tabs.ts`), which runs each action as the store's command.
2. **One strip.** `DocTabStrip` takes a `DocTabsModel`. The Editor's extras are props: a leading element (the file tree toggle), a label override (the Save As box) and extra tab classes. `editor-tab-strip.tsx` keeps only the Save As box. The Editor's row styles move to `.doc-tab-strip`, so both panes draw the strip the same way.
3. **One key path.** `handleDocTabKey` and `runDocTabAction` take a `DocTabsModel`; the Editor's own switch goes.
4. **One saver.** `DocTabsSaver` (`doc-tabs-controller.ts`) writes a pane's tabs to its block after a pause, skips a write that changes nothing, and writes at once on dispose. The controller and the Editor model both use it.
5. **One host.** `docTabsHost(model, docType, hooks)` (`doc-tab-hosts.ts`) replaces `controllerHost` and the Editor's own host. The Editor passes hooks for what it alone carries: its unsaved text, encoding, mode and undo history (taken before the tab leaves, put in place before it arrives), the scope (its connection), and the refusal while a save is running.

## 4. Not in this change

Phase 4 of the drag report (a tab dropped on layout space, or moved to a new pane) builds on this and follows in its own change.
