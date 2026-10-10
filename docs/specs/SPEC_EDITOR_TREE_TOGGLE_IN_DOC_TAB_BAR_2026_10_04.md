# SPEC: The Editor's file-tree toggle moves into its document tab bar

**Status:** proposed, not built.
**Date:** 2026-10-04
**Author:** agent2
**Trigger:** Repo owner, 2026-10-04: the show/hide file tree toggle is part of the pane tab; it belongs in the editor's document tab bar.

**Amends:** `SPEC_EDITOR_FILE_TREE_2026-05-26.md` ("Header toggle") and `SPEC_DOCUMENT_TABS_2026_10_02.md` §6.1 (the Editor's strip is "always shown").
**Related, unchanged:** `SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md` and `SPEC_OPENEDITOR_FLOATING_AND_COLLAPSED_TREE_2026_06_16.md` (the tree starts collapsed for scratch files, Markdown and `collapse_tree`; none of that changes).

---

## 1. Problem

The file-tree toggle lives in the **pane header**, to the left of the pane-tab pills. It is built by a special path:

- `EditorViewModel.viewIcon` (`editor-model.ts`, "Pane icon doubles as the file-tree expand/collapse toggle") returns an `IconButtonDecl` (`folder-tree` / `folder`, titles "Hide file tree" / "Show file tree").
- `blockframe.tsx` `viewIconControlElem` renders it as `.block-frame-view-icon-control` when the header has a leading tab strip. Its own comment names it: "keep it only when it's a real control (the editor's file-tree toggle)". `mixins.scss` gives it a 10px inset so it doesn't touch the pane edge.
- The editor is the **only** user of that path. Every other `viewIcon` is a string.

Why that is the wrong place, using the three layers of tab from `SPEC_DOCUMENT_TABS_2026_10_02.md` §1:

| # | Problem | Detail |
|---|---|---|
| 1 | **Wrong layer** | The header is the *pane* layer (window tab, pane tab). The tree is a part of what the Editor shows *inside* the pane, next to its document tabs. A control for the inside lives with the document tab bar. |
| 2 | **The header jumps** | Pane tabs can stack, for example an Editor and an Agent in one slot. The control exists only while the Editor is the active pane tab, so switching pane tabs adds or removes a control and a 10px inset, and shifts every pill to its right. |
| 3 | **It is not on the pill, but reads like it** | The pill's own icon comes from block meta and the manifest (`file-lines`), so it is stable. But the toggle sits in front of the pills and takes the "Pane icon" slot, so it reads as the editor's identity icon changing between two folder glyphs. |
| 4 | **Dead on a remote host** | `treeExpandedAtom` is forced `false` when `connection()` is set (the tree lists this computer's folders). The header icon still renders and still writes `editor:tree_expanded`, but nothing changes on screen. |
| 5 | **Unreachable with no document** | The doc tab row renders only when there are tabs (`editor-view.tsx`, `Show when={model.tabsAtom().length > 0}`), so the new location must not inherit that. §3.2. |

## 2. Goals and non-goals

**Goals**
1. The toggle is the first element of the Editor's document tab bar, and it is gone from the pane header.
2. Its position **does not move** when the tree opens or closes, so it can be clicked repeatedly.
3. It works with zero open documents, and is not rendered where there is no tree (remote host).
4. The header no longer special-cases the Editor.

**Non-goals**
- Changing what the tree does, its width, its persistence, or its context menu.
- A new shortcut (see §7).
- Changing document-tab behaviour, the Markdown mode toolbar, or the pane tabs.
- Moving the tree *toggle* of other panes. There are none; Hangar and Media have no tree.

## 3. Design

### 3.1 Layout: the tab bar spans the whole editor

Today the tab row sits inside `.editor-main-column`, to the right of the tree. A toggle placed at the left edge of *that* row would move by the tree's width every time it is clicked (240px by default). So the bar becomes a **full-width top row**, and the tree moves down a level:

```
today                                         proposed
┌ pane header: [toggle][ pill ][ pill ] ─┐    ┌ pane header: [ pill ][ pill ] ─────────┐
├────────────┬───────────────────────────┤    ├────┬───────────────────────────────────┤
│ file tree  │ [tab][tab][+]             │    │ ⎘  │ [tab][tab][+]                     │  <- document tab bar
│            ├───────────────────────────┤    ├────┴──────────┬────────────────────────┤
│            │ editor body               │    │ file tree     │ editor body            │
└────────────┴───────────────────────────┘    └───────────────┴────────────────────────┘
```

DOM in `editor-view.tsx`:

```
.editor-view            (flex column)
 ├ .editor-tab-bar       NEW: flex row, full width, border-bottom
 │   ├ TreeToggle         28×28, fixed at the left edge
 │   └ EditorTabStrip     unchanged component, flex: 1, min-width: 0
 └ .editor-view-body     NEW: flex row (what .editor-view is today)
     ├ .editor-tree-column + .editor-tree-resize-handle   (when expanded)
     └ .editor-main-column                                (mode toolbar + body, minus the tab row)
```

`.editor-tab-strip-row` (the full-width border carrier) is folded into `.editor-tab-bar`. The tree column loses 28px of height; the tree has its own scrolling, so nothing else changes.

The composition is local to the editor. **`PaneTabStrip` and `DocTabStrip` are not changed**: no `leading` slot is added to the shared component, which agent and terminal strips also use. If Hangar or Media ever need a sidebar toggle, promote the pattern then (§7).

### 3.2 The bar is always shown

The bar renders whether or not any document is open, so the toggle works in an empty Editor. With zero tabs the strip still shows its `+` (new scratch buffer), as `SPEC_DOCUMENT_TABS_2026_10_02.md` §6.1 already intends ("the strip always shown"). The Markdown mode toolbar stays conditional on `tabs > 0 && isMarkdown`.

### 3.3 The toggle

- One icon, `fa-folder-tree`, for both states. State is shown by styling and `aria-pressed`, not by swapping glyphs: pressed (tree open) gets the active background used by the strip's active tab; unpressed is the plain ghost button.
- `title`: "Hide file tree" / "Show file tree". `aria-label="File tree"`, `aria-pressed={treeExpanded}`.
- Not rendered when `model.connection()` is non-empty (problem 4). The bar then starts with the tabs, as the strip does today.
- Click calls the existing `toggleTreeExpanded()` unchanged: same signal, same `editor:tree_expanded` meta write, same default (`true`), same collapsed start for scratch, Markdown and `collapse_tree`.
- Inherits the editor's zoom from `.editor-view` (`style={{ zoom }}`) like the strip does. It must **not** set its own zoom (the same double-zoom trap documented in `editor-tab-strip.tsx`).

### 3.4 The pane header

- `viewIcon` returns the plain identity icon, a string (`file-lines`, as the manifest says), so the header shows the pane's identity and nothing clickable. `headerIcon` in `editor.tsx` goes with it.
- **Delete** the clickable-header-icon path, since the editor was its only consumer: `viewIconControlElem` in `blockframe.tsx`, `.block-frame-view-icon-control` and its inset in `mixins.scss`, and the `IconButtonDecl` arm of `PaneTabInstance.headerIcon` if no other view sets it (verify with a grep in the PR). Leaving an unused contract invites the next view to repeat this.
- The empty-state hint changes from "Open the file tree (chevron) to browse files." to "Use the file tree button in the tab bar to browse files."

## 4. Edge cases

| Case | Behaviour |
|---|---|
| Editor on a remote host | No toggle, no tree (unchanged); the bar starts with the tabs |
| Tree open, window narrow | The toggle is fixed; the strip shrinks and scrolls as today (`min-width: 0`) |
| Tree collapsed at open (scratch, Markdown, `collapse_tree`) | Toggle shown, unpressed |
| Dirty tab, save-as input, preview tab | Untouched: they are inside `EditorTabStrip` |
| Dragging a document tab | Untouched; the toggle is outside the strip's drop targets |
| Pane in a stack, switch pane tabs | Header no longer changes with the active pane tab (problem 2) |
| Floating / tear-off editor | Same component; same behaviour |

## 5. Changes by file

| File | Change |
|---|---|
| `view/editor/editor-view.tsx` | New `.editor-tab-bar` (toggle + strip, always rendered); wrap tree + main column in `.editor-view-body`; update the file-header comment and the empty-state hint |
| `view/editor/editor-tree-toggle.tsx` | New: the toggle button |
| `view/editor/editor-view.scss` | `.editor-view` becomes a column; add `.editor-tab-bar` and `.editor-view-body`; fold `.editor-tab-strip-row` into the bar; toggle styles |
| `view/editor/editor-model.ts` | `viewIcon` returns the identity string; drop the `IconButtonDecl` memo; no change to tree state or `toggleTreeExpanded()` |
| `view/editor/editor.tsx` | Drop `headerIcon` |
| `block/blockframe.tsx`, `mixins.scss` | Remove `viewIconControlElem` and `.block-frame-view-icon-control` (§3.4) |
| `view/editor/*.md`/specs | Mark the "Header toggle" section of `SPEC_EDITOR_FILE_TREE_2026-05-26.md` as superseded, pointing here |

No backend, RPC, meta key or persistence change. Existing panes keep their `editor:tree_expanded` and `editor:tree_width`.

## 6. Tests

- Toggle renders in the tab bar, before the strip, **with zero tabs**.
- Click flips `treeExpandedAtom` and persists `editor:tree_expanded`; `aria-pressed` follows it.
- Not rendered when `connection()` is set; the bar still renders.
- The toggle's DOM position is independent of tree state (it is a sibling before the strip, not inside the tree column).
- `viewIcon()` is a string; `editor.test.tsx` no longer expects an icon button; the header renders no `.block-frame-view-icon-control` for the editor (update `PaneHeaderTabStrip.test.tsx` / `PaneTabStrip.test.tsx` cases that exercise the control).
- Existing `editor-doc-tabs.test.ts` (collapsed tree on open) and tree/resize tests pass unchanged.
- A live check (the repo's `task dev` flow): open the Editor with and without files, toggle repeatedly without the pointer moving, drag the tree resize handle, stack an Editor with an Agent and switch between them watching the header, open on a remote host, zoom in and out.

## 7. Open questions

1. **Shortcut.** None exists today (the editor has no tree-toggle key). `Ctrl+B` is the VS Code convention, but CodeMirror and Markdown editing may want it. Recommendation: not in this change; decide separately.
2. **Promote to a shared `leading` slot?** Only if a second pane needs a sidebar toggle in its document bar. Recommendation: no, until one does.
3. **Button size.** 28×28 matches the strip height. If the strip is later made denser (`SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md`), the toggle follows the strip's height token, not a literal.

## 8. Rollout

One PR, frontend only, with a changeset. Risk is layout, not data: the highest-risk piece is the `.editor-view` column restructure (height/overflow of the body row with the tree resize handle), which is why §6 asks for a live check, not unit tests alone.
