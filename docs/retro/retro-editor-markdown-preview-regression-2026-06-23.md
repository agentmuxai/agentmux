# Retro: Editor markdown preview regressions — split layout, missing toggle, tree state

**Date:** 2026-06-23  
**Discovered by:** Manual test — agents opening `.md` files via `pane.open` App API  
**PRs involved:** #1522 (styled markdown preview + toggle), #1655 (collapsible split panel)  
**Status:** Fixed in PR #1743

---

## What happened

PR #1522 (`feat(editor): styled markdown preview with rendered/source toggle`) shipped a
full-overlay markdown preview: opening a `.md` file showed rendered markdown occupying the
full pane, with a "Source" / "Preview" toggle button to switch modes. CodeMirror was hidden
when in preview mode.

PR #1655 (`feat(editor): collapsible live markdown preview panel`) replaced this with a
persistent top/bottom split panel — CodeMirror always on top, rendered markdown in a
collapsible bottom pane that can be dragged to resize or dismissed with Mod+Shift+V.

The replacement introduced three regressions:

---

## Regression 1 — Split-screen layout is unexpected

**Old behavior:** Opening a `.md` file showed a full-screen rendered preview. CodeMirror
was completely hidden. The pane felt like a reading surface.

**New behavior:** Opening a `.md` file shows a top/bottom split — CodeMirror editor on top,
rendered markdown below. Both panels are always visible. There is no "reading mode."

**Impact:** The UX intent changed from "view this document" to "edit this document with a
live preview." Agents that open `.md` files to surface documentation or a plan now expose the
raw source to the user, degrading signal-to-noise.

**Root cause:** PR #1655 was designed as an editing aid (live preview while writing), not as
a viewer mode replacement. The original viewer-first contract — `.md` opens in rendered mode
by default — was not preserved.

---

## Regression 2 — Preview/code toggle button removed

**Old behavior:** A toolbar button labeled "Source" / "Preview" toggled between full-overlay
rendered markdown and full CodeMirror source view. The `mdSourceTabs` signal tracked
per-tab mode. `showRendered()` = `!mdSourceTabs().has(currentTabId)` drove what was visible.

**New behavior:** The toggle button no longer exists. The chevron button in the preview panel
header collapses the bottom pane (height → 28px header-only), but CodeMirror on top is always
visible. There is no path to a code-only or preview-only full-pane view.

**Impact:** Users who want to read a markdown file without the editor gutter/syntax
highlighting have no option. Users who want to edit without the preview panel taking vertical
space must drag the handle all the way up (awkward) or use Mod+Shift+V to collapse the
bottom panel — but then the code pane still occupies only the top half of the frame until
the split is manually dragged back.

**Root cause:** The `mdSourceTabs` signal and `toggleMdMode()` function were deleted as part
of the PR #1655 rewrite. The new architecture has no equivalent of "hide one side entirely."

---

## Regression 3 — File tree opens expanded when it should start collapsed

**Old behavior:** Agents could open an editor with `tree_expanded: false` and the tree
would start collapsed, giving the markdown content full horizontal width.

**Current state:** The `build_pane_meta()` handler in `app_api.rs` (line 1308) only writes
`editor:tree_expanded` when the caller explicitly passes `tree_expanded`. The MCP tool
`OpenEditor` passes `tree_expanded: None` by default (line 3233), so the key is absent from
meta and the frontend defaults to `true` (expanded).

**Impact:** When agents open `.md` files, the tree occupies the left column by default,
consuming ~240px of horizontal space. Combined with the split-screen layout, the user sees
three regions (tree, code editor, preview) instead of the intended "preview-first" single
column.

**Root cause:** The App API is correctly wired — the opt-in mechanism works — but nothing
sets the default to collapsed when the opened file is markdown and no explicit tree state
was requested. Previously this wasn't visible because the old full-overlay preview rendered
over the tree.

---

## Why the old behavior worked

In the pre-#1655 world:
- Opening a `.md` file defaulted to preview mode (full-overlay rendered markdown).
- The full-overlay covered the tree panel visually — whether the tree was expanded or not was irrelevant.
- Users never needed a toggle because the opening mode was the right default.

PR #1655 broke the implicit contract: it assumed both panels would always be visible, which
exposed the tree-state and source-visibility problems that the overlay had previously masked.

---

## What needs to be fixed

### Fix A — Restore a full-preview / full-source toggle

Add back a mode signal per tab: `"preview-only" | "source-only" | "split"`.

- Default for `.md` files: `"preview-only"` (rendered markdown fills the pane, CodeMirror hidden).
- Default for all other files: `"source-only"` (current behavior, no bottom panel).
- Toggle button in the header switches between modes; keyboard shortcut Mod+Shift+V retained.
- The split panel is the `"split"` mode and remains available.

Alternatively: the split panel can stay as the architecture, but `.md` files should default
to a very tall preview height (e.g. 80% of pane) with the CodeMirror strip collapsed to a
minimal strip (or fully hidden) — approximating the old full-screen feel.

### Fix B — Default tree to collapsed for markdown files opened via App API

In `build_pane_meta()` (`app_api.rs`), detect `.md` extension and default
`editor:tree_expanded` to `false` when not explicitly set:

```rust
// In build_pane_meta, editor branch:
if let Some(expanded) = cmd.tree_expanded {
    meta.insert("editor:tree_expanded".to_string(), json!(expanded));
} else if file.ends_with(".md") {
    // Markdown files are opened for reading; start with tree collapsed.
    meta.insert("editor:tree_expanded".to_string(), json!(false));
}
```

Or push this decision into the MCP `OpenEditor` tool so agents calling it for markdown
automatically get a collapsed tree without having to know the convention.

### Fix C — Preserve App API field for initial preview mode

Add `preview_open: Option<bool>` to `CommandPaneOpenData`. Map to `editor:preview_open` in
`build_pane_meta`. Agents that want preview-only mode can pass `preview_open: true` and
(once Fix A lands) `source_hidden: true` or similar.

---

## Action items

| # | What | Owner |
|---|------|-------|
| 1 | Restore full-preview / full-source modes alongside split — default `.md` to preview-only | — |
| 2 | Default `editor:tree_expanded = false` for `.md` files in `build_pane_meta` | — |
| 3 | Add preview-mode field to `CommandPaneOpenData` so agents can set it via App API | — |
| 4 | Write regression test: open `.md` via App API, assert tree collapsed + source hidden | — |
