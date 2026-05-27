# Spec: Editor Pane — Multi-File Tabs

**Branch:** TBD (`agenty/editor-tabs-spec` for the spec PR; implementation branches per phase)
**Status:** Draft — design
**Date:** 2026-05-26
**Author:** AgentY

---

## TL;DR

The editor pane currently holds **one file at a time**. Opening a new file in the same pane replaces the previous one — there's no way to keep two files open side-by-side without splitting the pane. Add a **tab strip** above CodeMirror that mirrors what VS Code does: opened files become tabs; previously-opened files stay open until explicitly closed; the set of tabs and the active tab persist per pane (and, via a new global setting, survive across pane re-opens).

Phase 1 ships the strip + basic open/close/switch + persistence. Preview tabs (single-click temporary, double-click pins), drag-to-reorder, split-as-tab-group, and tab-overflow chips land in follow-up phases — each individually a small change once the data model is right.

---

## Current state

`EditorViewModel` (`frontend/app/view/editor/editor-model.ts`) owns one file:

| Signal | Type | Persistence |
|---|---|---|
| `filePathAtom` | `string` | block meta `editor:file_path` (restored on pane reopen) |
| `contentAtom` | `string` | RAM only — re-fetched on `openFile` |
| `languageAtom` | `string` | derived from extension |
| `dirtyAtom` | `boolean` | RAM only |
| `readOnlyAtom` | `boolean` | RAM only |
| `errorAtom` | `string \| null` | RAM only |

`openFile(path)` is one-shot: it discards the existing CodeMirror state and replaces all six signals. The file-tree highlights the row that matches `filePathAtom()`. The LSP layer (PR #1074) keys diagnostics off `filePathAtom()` + `languageAtom()`.

Per-pane meta keys already in use (see `editor-model.ts`):

- `editor:file_path` — current file
- `editor:tree_expanded` — file-tree column open/closed
- `editor:show_hidden` — hidden-file toggle
- `editor:tree_width` — resize-handle position
- `term:zoom` — per-pane font scale (PR #1084)
- `editor:lsp.enabled` — LSP master switch

---

## Goals

1. **Open multiple files in one pane.** Click a file in the tree; if it's already open it activates that tab, otherwise a new tab is appended.
2. **Switching tabs is instant.** Each tab keeps its CodeMirror state alive — scroll position, selection, undo history, dirty buffer — so switching back to a tab is a true return, not a reload.
3. **Tabs survive pane reopen.** Closing and reopening the editor pane restores the same set of tabs and the active tab.
4. **Tabs survive editor-on-editor reuse.** Opening a new editor pane (fresh from the widget bar) optionally pre-populates from a global setting — the "last set of tabs you had open in any editor pane" — so a freshly-spawned pane lands on something useful instead of empty.
5. **Dirty state per tab.** A `*` on the tab label when unsaved; closing a dirty tab prompts to save / discard / cancel.
6. **Keyboard shortcuts that match VS Code.** Ctrl+W close, Ctrl+T (or Ctrl+Tab) cycle, Ctrl+1..9 jump-by-index, Ctrl+Shift+T reopen last closed.

## Non-goals (Phase 1)

- **Drag-to-reorder** — landing Phase 2. The data model supports reorder; we just don't ship the drag handle yet.
- **Drag-tab-out-of-pane** — coupling tabs to the floating-pane / tear-off system (PR #1073) is a separate design conversation.
- **Pinned tabs** — out of scope until preview tabs land; pin-vs-preview is a coupled UX.
- **Diff view tabs** — open `a.ts ↔ b.ts` as one tab. Future, very useful for code review; not in this scope.
- **Persistence of CodeMirror state across pane reopen** — scroll position, undo history, dirty buffer are RAM-only. Only the **set of file paths** and the **active path** persist; reopening rehydrates each tab's content fresh from disk (which means unsaved changes are lost on pane close, same as today — but now with a warning prompt).

---

## Design

### 1. Data model

`EditorViewModel` becomes a thin wrapper that owns an ordered list of **tabs** and an **active index**.

```ts
interface EditorTab {
    id: string;                     // stable id (uuid) so reorders survive
    filePath: string;               // absolute path
    // The next four are RAM-only — rehydrated on pane reopen.
    content: string;
    language: string;
    readOnly: boolean;
    dirty: boolean;
    error: string | null;
    // CodeMirror state, kept alive across tab switches:
    cmState: EditorState | null;    // null until first activate
}

class EditorViewModel {
    private _tabs = createSignal<EditorTab[]>([]);
    tabsAtom = this._tabs[0];

    private _activeId = createSignal<string | null>(null);
    activeIdAtom = this._activeId[0];

    activeTabAtom = createMemo(() =>
        this.tabsAtom().find((t) => t.id === this.activeIdAtom()) ?? null
    );
}
```

The existing `filePathAtom`, `contentAtom`, etc. are **derived from the active tab**:

```ts
filePathAtom = () => this.activeTabAtom()?.filePath ?? "";
contentAtom = () => this.activeTabAtom()?.content ?? "";
// ...etc.
```

So every existing consumer (the LSP layer, the file-tree active-row highlight, the pane title) keeps working without code changes — they just track whichever tab is active.

### 2. Persistence

Two layers:

#### Per-pane (block meta)

```jsonc
{
    "editor:tabs": [
        { "id": "<uuid>", "filePath": "C:/repo/index.ts" },
        { "id": "<uuid>", "filePath": "C:/repo/lib/util.ts" }
    ],
    "editor:active_tab_id": "<uuid>"
}
```

Replaces the existing `editor:file_path`. On pane mount:

1. If `editor:tabs` is present, restore tab order; rehydrate each tab's content via `RpcApi.GetFileContents`. The active tab's content is loaded first (block on it); other tabs load lazily on first activation to keep pane-open fast.
2. If only the legacy `editor:file_path` is present (pre-migration), build a single-tab list from it. Write `editor:tabs` once; the legacy key is left for one minor version then dropped.
3. If both are missing, fall through to the global setting (next layer).

#### Global (settings)

```jsonc
// settings.json
{
    "editor:default_tabs": [
        { "filePath": "C:/repo/recent-1.ts" },
        { "filePath": "C:/repo/recent-2.ts" }
    ],
    "editor:default_active_path": "C:/repo/recent-1.ts"
}
```

Updated *atomically* on every tab list change in any editor pane (debounced 500 ms). On a fresh editor pane (no per-pane meta), this is the fallback: the user gets the last tab set they had open, in any pane.

**Trade-off:** if you have two editor panes open with different tab sets, the last-write-wins behavior could be surprising. Acceptable: this is for the "I just opened a new editor pane and don't want a blank slate" case. Power users with multiple long-lived panes have per-pane persistence, which is stricter.

Capped at **10 paths** to keep settings small; the cap is enforced when writing.

### 3. Tab strip UI

Above CodeMirror, below the pane header. Roughly:

```
┌──────────────────────────────────────────────────────────────────┐
│ index.ts × │ util.ts * × │ README.md × │ + │           overflow ⌄ │
└──────────────────────────────────────────────────────────────────┘
│                       CodeMirror                                  │
```

| Element | Behavior |
|---|---|
| Tab body | Click to activate. Middle-click to close. Right-click for context menu (close, close others, close to the right, copy path, reveal in tree) |
| Filename | The basename (e.g. `util.ts`). Hover tooltip shows the full path |
| `*` indicator | Visible when `dirty === true` |
| `×` close button | Visible on hover (and always for dirty tabs, since closing requires confirmation) |
| `+` button at end | Focuses the file-tree path-input affordance ("open by path"). Cheaper than reaching for the tree on a fresh pane |
| Overflow chip | When tabs overflow horizontally, a `⌄` button at the right reveals a dropdown of hidden tabs (filtered by active-substring) |

Sizing: tabs auto-flex from a min of **80px** (just enough for a short filename + `×`) to a preferred of **140px**. Once total content width exceeds the strip width, tabs shrink to min; once even min doesn't fit, the rightmost tabs collapse into the overflow chip.

A subtle bottom border on the active tab connects it visually to the CodeMirror area below; the dirty `*` is colored with `--accent-color` so it draws the eye.

### 4. Lifecycle

#### Open a file

`model.openFile(path)`:

1. If a tab with `filePath === path` exists, set `activeIdAtom` to its id. Done. **No reload.** This is the part that makes switching feel instant — we never re-fetch a file that's already a tab.
2. Otherwise: fetch content via `RpcApi.GetFileContents(path)`. Create a new tab record. Append to `tabsAtom`. Set `activeIdAtom` to the new tab's id. Persist (per-pane + global).

#### Switch tabs

Click on a tab body, or Ctrl+Tab cycle, or Ctrl+1..9 by index, or programmatic `model.activateTab(id)`.

Internals:

1. **Snapshot** the outgoing tab's CodeMirror state into `outgoing.cmState = view.state` — preserves cursor, selection, scroll, undo history, dirty buffer.
2. **Restore** the incoming tab: if `incoming.cmState` is non-null, `view.setState(incoming.cmState)`; otherwise `view.setState(EditorState.create({ doc: incoming.content, extensions: ... }))` (first-time activate).
3. Update `activeIdAtom`. Re-evaluate LSP `didOpen` / `didClose` lifecycle (the LSP layer already handles this via `startLspIfSupported` in `editor-view.tsx`).
4. Persist `editor:active_tab_id`.

CodeMirror state objects are immutable — keeping them around is cheap (no DOM, no listeners).

#### Close a tab

Click `×`, middle-click the tab, or Ctrl+W:

1. If `dirty === false`: drop the tab from `tabsAtom`. Persist.
2. If `dirty === true`: open a `Save? / Discard? / Cancel` confirm modal (the same surface used for unsaved-changes today, just routed through the tab closer).
3. If the closed tab was active and there are siblings: activate the **right neighbor** (matches VS Code; falls back to the left neighbor when closing the rightmost tab).
4. If the closed tab was the only one: leave a blank state with the empty-state path input visible (the existing zero-tab UI).

#### Reopen last closed

Per-pane stack of recently closed tab paths (RAM, max 10). Ctrl+Shift+T pops and re-opens. Cleared when the pane unmounts.

### 5. Keyboard shortcuts

Editor-pane-focused only — same convention as the rest of AgentMux:

| Shortcut | Action |
|---|---|
| Ctrl+W | Close active tab (or close pane if no tabs) |
| Ctrl+Tab | Activate next tab (wraps) |
| Ctrl+Shift+Tab | Activate previous tab |
| Ctrl+1 … Ctrl+9 | Activate Nth tab by index (1-based; clamps to last tab if N > tab count) |
| Ctrl+Shift+T | Reopen last closed tab |
| Ctrl+PageDown / Ctrl+PageUp | Same as Ctrl+Tab / Ctrl+Shift+Tab — VS Code accepts both |

Bindings registered in `keymodel.ts` under a new `editor-tabs` group, gated on `viewType === "editor"` so they don't conflict with terminal/agent shortcuts.

### 6. Interaction with existing systems

#### File tree

The active-row highlight already keys off `filePathAtom()`, which is now derived from the active tab. No change.

Clicking a tree row routes through `model.openFile(path)`, which now does the "activate existing tab if one exists" check. So clicking around the tree feels lighter — you don't reload files you've already opened.

#### LSP (PR #1074)

`startLspIfSupported(filePath, language, content)` is called in a `createEffect` watching `filePathAtom()`. The same effect fires when you switch tabs (because `filePathAtom` is derived from active tab). The existing same-workspace reuse path in `startLspIfSupported` already does `didClose(prevUri)` → `didOpen(newUri)` — that gives us tab switching for free on the LSP side. No changes.

#### Pane title

`viewName` already shows the full file path with `*` for dirty (PR #1071). Tracks the active tab via the same derivation. No change.

#### Header icon (file-tree toggle)

Per PR #1084, the icon expands/collapses the file-tree column. Unchanged.

#### Zoom (PR #1084)

`term:zoom` applies to the whole editor (CodeMirror + tree). Tab strip itself is **not** zoomed — it's chrome, like the toolbar. Tab labels stay at 11px regardless of zoom factor.

#### Magnify

Double-clicking the header to magnify still works (tab strip stops dblclick from propagating, like the icon does).

#### Split view (future)

Out of scope for Phase 1, but the data model leaves room: a future `EditorSplit` wrapper can hold N `EditorViewModel`s and the tab strip is per-`EditorViewModel`. The user already gets layout-level pane splits via the existing tile layout; this would be a *within-pane* split (left/right or top/bottom of CodeMirror, sharing one tab strip), which is a different feature.

---

## Phased rollout

### Phase 1 — Core tab strip (shippable)

- [ ] `EditorTab` type + tab list + active-id signals in `EditorViewModel`
- [ ] Derive `filePathAtom`/`contentAtom`/etc. from active tab
- [ ] `openFile`: activate-if-exists, else append
- [ ] Tab switch: snapshot/restore CodeMirror state
- [ ] Close tab + dirty-confirm modal
- [ ] Reopen last closed (Ctrl+Shift+T) — per-pane RAM stack
- [ ] Per-pane persistence (`editor:tabs`, `editor:active_tab_id`)
- [ ] Global persistence (`editor:default_tabs`, `editor:default_active_path`)
- [ ] Migration from legacy `editor:file_path`
- [ ] Tab strip render (no overflow chip yet — tabs just compress to min width)
- [ ] Keyboard shortcuts (Ctrl+W, Ctrl+Tab, Ctrl+1..9, Ctrl+Shift+T)
- [ ] Right-click context menu (close, close others, close to the right, copy path)

### Phase 2 — Polish

- [ ] Overflow chip + hidden-tabs dropdown
- [ ] Drag-to-reorder within the strip
- [ ] Preview tabs (single-click in tree = preview; double-click or edit = pin)
- [ ] Pinned tabs render compacted on the left
- [ ] Middle-click to close

### Phase 3 — Future

- [ ] Drag tab between editor panes
- [ ] Drag tab out → spawns a new editor pane (couples to floating-pane / tear-off)
- [ ] Diff view tabs (`a.ts ↔ b.ts`)
- [ ] Within-pane split (one tab strip, two CodeMirror viewports)

---

## Open questions

1. **Tab close on pane close vs. dirty prompt.** Today, closing an editor pane with unsaved changes silently discards them. With tabs, closing the pane is closing N tabs. Single big prompt with a list ("Save changes to 3 files?") or N prompts in sequence? VS Code does the former. Recommend that.
2. **Global `editor:default_tabs` write contention.** Two panes editing different tab sets will fight over the global setting (last-write-wins). Acceptable? Or should the global only update for the *focused* pane? Recommend focused-only — same semantics as the active-tab indicator.
3. **What counts as "the same file" for activate-if-exists?** Just path-string equality, or canonicalized path (symlinks resolved)? Recommend canonicalized — opening `C:/repo/a.ts` and `C:\repo\a.ts` should hit the same tab.
4. **LSP `didOpen` for inactive tabs?** Today the LSP only knows about the active file. Should we `didOpen` all tabs at pane mount so the server has the whole open set in its workspace? VS Code does. Recommend deferring — the LSP layer is Phase-1-shipped at "single open file"; extending it to "set of open files" is a separate small change that doesn't block tabs.

---

## Open implementation risks

- **CodeMirror state immutability assumption.** We're relying on `EditorState` snapshots being safe to hold across DOM lifecycle. They are — CodeMirror 6 explicitly designs for this — but worth a test that verifies a snapshotted-then-restored state preserves cursor + undo correctly.
- **Settings write fan-out.** Every tab open/close fires a settings write. Debounce 500 ms; settings layer should already be debounced but worth checking. If not, this lands new pressure on the settings store.
- **Memory.** N tabs × ~1 MB doc cap = N MB resident in the worst case. The 10 MB doc cap is enforced per file at read time; we don't enforce a total-tab-memory cap. Probably fine. If it becomes a problem, evict the inactive tab content (keep just the path) and re-fetch on activate — a small UX cost (~50 ms file read) for a 90% memory win.

---

## Why not just use VS Code's exact behavior?

Two reasons:

1. We're in CodeMirror, not Monaco. CodeMirror's state model gives us snapshot/restore in a way VS Code's TextDocument-and-buffer-pool model doesn't translate to directly. Our design is "VS Code-like for users, CodeMirror-native for the implementation."
2. AgentMux is pane-first. Pane open/close is a real cycle (unlike VS Code's window) and per-pane persistence matters more. The global fallback is the AgentMux-specific bit that wouldn't exist in VS Code.

References:

- VS Code "Tabs" docs — open files concept, preview vs. pinned, keyboard shortcuts: https://code.visualstudio.com/docs/getstarted/userinterface#_tabs
- VS Code source: workbench/contrib/files/browser/views/openEditorsView (open-editors panel, the canonical tab list)
- CodeMirror 6 EditorState immutability + transactional updates: https://codemirror.net/docs/guide/#state-and-updates
