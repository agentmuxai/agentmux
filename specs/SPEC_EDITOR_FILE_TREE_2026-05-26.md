# Spec: Editor Pane — File Tree Explorer + Extensions

**Branch:** `agenty/editor-file-tree-spec`
**Status:** Draft — design
**Date:** 2026-05-26
**Author:** AgentY

---

## TL;DR

The Editor pane is **CodeMirror 6 + an empty-state path input** today. Users have to know (or paste) an absolute path to open anything. This spec adds:

1. A **collapsible file-tree explorer** down the pane's left side, rooted at `$HOME`
2. A **header toggle** (`📁` chevron) that expands/collapses the tree; default state **expanded**
3. **Click-to-load** — clicking any file replaces the editor's current document
4. A **research-backed proposal** for extension support — strong recommendation to ride CodeMirror 6's native extension model (curated, settings-toggleable) rather than reinvent VS Code's extension-host architecture

Spec covers UX, backend RPCs (one new + one helper), frontend component breakdown, and a phased rollout. Phase 1 is the tree. Extensions live behind their own follow-up.

---

## Current state

The editor pane (`frontend/app/view/editor/`) is ~380 lines across four files:

| File | Role |
|------|------|
| `editor.tsx` | Barrel — wires `viewComponent` onto the model |
| `editor-model.ts` | `EditorViewModel` — file path / content / language / dirty / readOnly signals, `openFile()`, `saveFile()`, `onContentChange()` |
| `editor-view.tsx` | `EditorViewComponent` — CodeMirror lifecycle + empty-state path input |
| `editor-view.scss` | Styling |

Active features:
- CodeMirror 6 with lazy-loaded language packs (7 languages: TS/JS, Python, Rust, HTML, CSS, JSON, Markdown)
- `oneDark` theme (fixed), `lineWrapping`, `basicSetup`, search
- Ctrl+S → save, dirty tracking (`* ` suffix in pane title)
- Read-only support (set by backend response)
- File path persisted in block meta — restored on pane reopen

Backend RPCs already in place:
- `ReadEditorFileCommand({ path }) → { content, read_only }`
- `WriteEditorFileCommand({ path, content }) → void`

What's missing:
- No file discovery surface (no tree, no recent files, no breadcrumb)
- Empty state is a literal path input — bad for anyone who doesn't already know the path
- No browse from one open file to a sibling

---

## Goals

1. **Discovery** — the operator can open the editor with no path and immediately see their files
2. **Familiar** — file tree matches VS Code / GitHub web conventions (chevron expand/collapse, indent, type icons, single-click loads)
3. **Reversible** — header toggle hides the tree to give CodeMirror the full pane width; preference persists per-pane in block meta
4. **No regressions** — the existing path-input + Ctrl+S + dirty state continue to work; clicking a file in the tree is *additive*, not a replacement

## Non-goals (this spec)

- Multi-root workspaces (single root = `$HOME`)
- Rename / delete / new-file / cut / copy / paste / drag-drop in the tree
- File watching (live updates when files appear/disappear outside the app)
- Symbol/outline navigation within the file
- Git decorations (modified, staged, conflicted)
- Multi-cursor / multi-tab document model (one file open per pane stays the model)
- VS Code-extension compatibility (see § Extensions for why)

Each of these is a sensible future-PR; calling them out so this spec stays focused.

---

## UX design

### Layout

```
┌─ Editor — main.tsx * ─────────────────────── [📁 chevron] [other header items] ─┐
├──────────────────┬────────────────────────────────────────────────────────────┤
│ ▼ asaf            │  import { ... } from "...";                              │
│   ▶ Desktop       │                                                          │
│   ▼ src           │  export function foo() {                                 │
│     ▼ components  │    return bar;                                           │
│     ▶ utils       │  }                                                       │
│     ● main.tsx    │                                                          │
│   ▶ tests         │                                                          │
│   ▶ node_modules  │                                                          │
│   ▶ .git          │                                                          │
└──────────────────┴────────────────────────────────────────────────────────────┘
```

Tree column on the left (default ~240 px wide, resizable via a vertical drag handle); CodeMirror fills the right. The currently-open file gets a filled-dot indicator (`●` instead of the file icon) and the row is highlighted.

When the user clicks the header chevron, the tree column slides closed and the chevron rotates to indicate "expand to show tree":

```
┌─ Editor — main.tsx * ────────────────────── [▶ chevron] [other header items] ─┐
├──────────────────────────────────────────────────────────────────────────────┤
│  import { ... } from "...";                                                  │
│  ...                                                                         │
└──────────────────────────────────────────────────────────────────────────────┘
```

### Header toggle

Placed at the **left** end of the pane header (per the existing `viewText` slot), before any other items. Icon: `folder-tree` (FontAwesome). State:

- **Expanded** (default): icon shows `chevron-left` next to the folder; click collapses
- **Collapsed**: icon shows `folder-tree` only; click expands

Persisted to block meta as `editor:tree_expanded` (boolean). Default `true`. The setting is *per pane* — two editor panes in the same window can have independent tree states.

### Tree behavior

| Interaction | Behavior |
|---|---|
| Click chevron next to a folder row | Expand if collapsed, collapse if expanded (lazy-load on first expand) |
| Click a folder row (not the chevron) | Same as clicking the chevron — toggle |
| Click a file row | Calls `model.openFile(absolutePath)` — replaces the editor's current document |
| Double-click a file row | Same as single-click (left for future "open in new pane" gesture) |
| Hover a row | Background tint; reveals chevron rotation |
| Right-click a row | (Out of scope this PR; future context menu) |
| Arrow keys (when tree focused) | Up/Down navigates rows; Right expands / focuses first child; Left collapses / jumps to parent |
| Enter | Same as single-click on the focused row |
| Drag the column divider | Resize the tree column. Width persists per pane as `editor:tree_width` (default 240) |

The active file's row stays selected (highlighted background + filled-dot icon) across scrolls; if it's off-screen, opening a sibling auto-scrolls it into view.

### File / folder visuals

- **Folders** — `chevron-right` (collapsed) or `chevron-down` (expanded) + small folder icon
- **Files** — language-typed icon mapped by extension (e.g. `file-code` for `.ts`, `file-image` for `.png`, plain `file` fallback). Reuses the `defwidget` icon vocabulary so the visual is consistent with the widget bar
- **Hidden files** (`.git`, `.DS_Store`, dotfiles, plus `node_modules`) — **hidden by default**. Togglable via the toolbar (§Toolbar below)
- **Symlinks** — **followed** (VS Code semantics). Symlinked rows get a small `↗` overlay on the icon so they're identifiable
- **Indentation** — 16 px per level, with a faint vertical guideline at each level (subtle, matches VS Code)

### Toolbar

A row of **small square icon buttons** at the **top** of the tree column (more discoverable than bottom, matches VS Code's view-toolbar placement). For v1, three buttons:

| Button | Type | Default | Tooltip | Behavior |
|---|---|---|---|---|
| `eye` / `eye-slash` | toggle | off (hidden) | "Show hidden files" / "Hide hidden files" | Flips visibility of dotfiles + standard noise (`node_modules`, `.git`, `.DS_Store`, `Thumbs.db`). State per-pane in block meta as `editor:show_hidden`. |
| `square-minus` | action | — | "Collapse all folders" | One-shot — collapses every expanded folder back to the root. |
| `arrows-rotate` | action | — | "Refresh tree" | Re-fetches the currently expanded folder paths; keeps expansion state. |

Buttons are 24×24 px, transparent background, icon-only. Hover gives a subtle background tint. Active toggles (when "on") get a filled background to signal state.

#### Tooltips — **0 ms show delay**

Use the existing **`data-tip` CSS pattern** that the status bar already uses (`frontend/app/statusbar/StatusBar.scss`). It's a pure-CSS `:hover::after` reveal with `content: attr(data-tip)` — **no JS, no delay, no animation**. Mouse enters the button → tooltip is visible the same frame.

The toolbar's SCSS adds a scoped `[data-tip]:hover::after` block (don't reach for the existing status-bar one — keep scope tight to the editor). Markup is plain:

```tsx
<button class="editor-tree-toolbar-btn" data-tip="Show hidden files" aria-label="Show hidden files" onClick={...}>
    <i class="fa fa-eye" />
</button>
```

Do **not** add a native `title` attribute alongside `data-tip` — would create a double tooltip. `aria-label` covers screen-reader a11y.

Future toolbar buttons (Phase 2):
- `magnifying-glass` — filter input (highlight + filter modes, matching VS Code)
- `house` — jump tree root back to `$HOME` (relevant once multi-root or directory-browse beyond home arrives)

### Empty-state pane vs tree

When the pane has no file open (`filePathAtom() === ""`), CodeMirror is hidden and the tree fills the pane width (tree column expands to 100% if there's room). The path-input empty-state moves to a smaller affordance at the **bottom** of the tree:

```
┌─ Editor ─ [📁 chevron] ──────────────────────┐
│ 👁  ⊟  🔄                                    │ ← toolbar (top)
├─────────────────────────────────────────────│
│ ▼ asaf                                       │
│   ▼ src                                      │
│     main.tsx                                 │
│     util.ts                                  │
│   ...                                        │
├─────────────────────────────────────────────│
│ Open by path: [/path/to/file.ts__________] │
└──────────────────────────────────────────────┘
```

So path-input is preserved as an escape hatch (useful when an LLM hands you an absolute path), but the tree is the primary surface.

---

## Backend

One new RPC + one helper. Both live in `agentmux-srv/src/server/` (probably alongside the existing `ReadEditorFileCommand` handler).

### `ListEditorDirectoryCommand`

```
Request:  { path: string }
Response: {
    path: string,            // canonical absolute path of the listed dir
    entries: DirEntry[],     // sorted: folders first then files, each alphabetical
}

DirEntry: {
    name: string,            // basename only
    is_dir: boolean,
    size?: number,           // bytes (files only; omitted for dirs)
    mtime?: number,          // unix ms (omitted on platforms without it)
}
```

Behavior:
- Path is normalized and canonicalized server-side (resolves symlinks, `..`)
- Errors map to existing editor error semantics (`EACCES` → "permission denied", `ENOENT` → "not found", etc.)
- Sort: folders first (alphabetical, case-insensitive), then files (alphabetical, case-insensitive). Dotfiles included in the response — frontend hides them per the visibility toggle
- Pagination is **not** in v1 — the spec assumes any single directory fits in a single response. A `node_modules/` with 5k entries would still render. If this becomes a problem, we add a `limit` + cursor in a follow-up
- The endpoint is auth-gated under the same auth middleware as the rest of `/agentmux/*`

### `GetEditorHomeCommand` (helper)

```
Request:  {}
Response: { home: string }   // OS home dir, e.g. "C:\Users\asaf"
```

Resolved server-side via Rust `dirs::home_dir()` (already a dependency). Lets the frontend ask once at mount and avoid hard-coding `process.env.HOME` (which doesn't behave consistently across CEF, Tauri, etc.).

---

## Frontend

### Files added

| File | Role |
|------|------|
| `frontend/app/view/editor/file-tree.tsx` | `FileTree` component — recursive tree rendering, expand/collapse state, click handlers |
| `frontend/app/view/editor/file-tree-model.ts` | Tree state — Map of `path → { expanded, loading, entries }`; lazy-load on expand |
| `frontend/app/view/editor/file-tree-types.ts` | `DirEntry`, tree node types |

### Files modified

| File | Change |
|------|--------|
| `editor-view.tsx` | Split layout into `tree-column` + `cm-column`; mount `FileTree`; wire click → `model.openFile()` |
| `editor-view.scss` | Tree column styles, divider, hover, active row, indent guides |
| `editor-model.ts` | Adds `treeExpandedAtom` + `setTreeExpanded()`; persists via `SetMetaCommand` (key `editor:tree_expanded`). Header `viewText` exposes the chevron toggle item |
| `frontend/app/store/rpc-api.ts` | Add `ListEditorDirectoryCommand`, `GetEditorHomeCommand` typed wrappers |

### State machine for tree nodes

Each folder node has three states: `unloaded`, `loading`, `loaded`. Click expand → if `unloaded`, transition `unloaded → loading → loaded` (RPC call); if already `loaded`, just toggle visibility. State is held in the tree model, not in the DOM — collapsing a folder keeps its loaded children in memory so re-expand is instant.

```
                 click expand
unloaded ──────────────────────► loading
                                    │
                                    │ ListEditorDirectoryCommand
                                    ▼
                                  loaded ◄──┐
                                    │       │ click expand
                                    │       │ click collapse
                                    └───────┘
```

Errors transition `loading → error`; subsequent click attempts retry.

### Performance

- Single directory render is O(n) DOM nodes. For sane HOMEs that's fine. If a user navigates into a 10k-entry directory we'll see jank — the spec calls this a known limitation, with virtualization (à la `@tanstack/solid-virtual`) listed as future work
- No tree-wide watching in v1. Tree is a snapshot per expand; refresh requires re-collapsing + re-expanding (or a small "refresh" affordance, TBD)

---

## Extensions support

The user's prompt asks: *"we may also want to support code extensions."* Real recommendation here, based on the research:

### Tier A — CodeMirror 6 extension shipping (RECOMMENDED for v1)

CodeMirror 6's design is a tree of composable extensions imported from npm. We already use this model (`basicSetup`, `oneDark`, `search`, language packs are extensions). **Adding more is just adding imports**; no plugin runtime needed.

Examples of high-value CM6 extensions worth bundling:
- `@codemirror/lint` — diagnostic underlines (paired with a backend linter or `tsc --noEmit`)
- `@codemirror/autocomplete` — context-aware completion (works without a language server for many cases)
- `@codemirror/commands` — full default keymap (history, indent, etc.)
- `@uiw/codemirror-themes-all` — large theme catalog
- `vim` / `emacs` mode (`@replit/codemirror-vim`, `@replit/codemirror-emacs`)

**Settings hook:** `editor:extensions` in `settings.json` — a string array of enabled extension keys. Default set on first launch; user toggles per their preference. Adding a new extension is a one-PR change (add the import + key + settings entry).

This gets us "VS Code parity for what users actually use day-to-day" without standing up a plugin system.

### Tier B — User-supplied extensions / plugin API (future)

This is the "VS Code-style extensibility" path: let third-party authors ship plugins that AgentMux loads at runtime. **Recommendation: defer indefinitely until there's clear user demand.** Why:

- CodeMirror itself has no plugin-host architecture — every "plugin" is a tree-shaken ES module. Loading user code at runtime requires either dynamic `import()` from a trusted local source or a sandbox we'd have to build
- VS Code's extension-host model is enormous (process isolation, contribution points, activation events, manifest schema, marketplace, signing). Replicating it is a multi-quarter project
- Most CodeMirror functionality the user cares about (themes, languages, vim mode, linting) already exists as an npm-able extension we can ship in Tier A

If demand emerges, the migration is: switch from a hardcoded extension array to one assembled from a manifest read at boot, drop user packages into `~/.agentmux/channels/<channel>/editor-extensions/`, and load them via dynamic `import()`. That's a follow-up spec, not this one.

### Tier C — Language Server Protocol (LSP) integration (future, big win)

LSP gives us real diagnostics, completion, hover docs, go-to-definition — what users actually expect from an IDE — by talking to off-the-shelf language servers (rust-analyzer, tsserver, pyright, etc.). CodeMirror has a `codemirror-languageserver` community extension that bridges LSP to CM6 facets.

**Recommendation: separate spec.** LSP brings process-management complexity (one server per workspace, per language) that's worth its own design discussion. The file-tree spec doesn't depend on it.

---

## Implementation phases

### Phase 1 — Tree shell + RPC + click-to-load (the spec's primary scope)

- Backend: `ListEditorDirectoryCommand` + `GetEditorHomeCommand`
- Frontend: `file-tree.tsx` + `file-tree-model.ts`; mount in `editor-view.tsx`; header chevron toggle; per-pane meta persistence
- File click → `model.openFile()`; current file highlighted; basic visuals
- Acceptance: open editor pane, see HOME tree, click a `.ts` file, syntax-highlighted content loads

### Phase 2 — Polish

- Hidden-file toggle (eye icon)
- Resizable column divider
- Keyboard navigation (arrows, Enter)
- Refresh affordance per folder
- Active-file auto-scroll-into-view

### Phase 3 — Extensions (Tier A from §Extensions)

- Bundle a curated extension set: lint, autocomplete, commands, theme catalog
- `editor:extensions` settings array + UI toggles (in the Settings widget that opens settings.json, or a small inline picker)
- Document the supported extension keys on docs.agentmux.ai

### Phase 4 — Stretch / open-ended

- Multi-root workspaces (`editor:workspace_roots` array in settings)
- File watching for live tree updates (mDNS-style local FS notify; respect debouncing)
- LSP integration (Tier C)
- Symbol outline (`@codemirror/lang-*` parse trees already give us most of what we'd need)

Each phase is independently shippable — Phase 1 is the only one in this spec's primary scope.

---

## Open questions

1. **What's a "file"?** — `dirs::home_dir()` may be a network share; listing a slow share blocks the RPC. Should we add a timeout + "Listing taking a while…" indicator? Skip for v1; revisit if it hurts.
2. **Theme** — `oneDark` is hardcoded. Should the tree pick up the same theme tokens (background, accent) automatically? Recommend yes; pull the theme's `--editor-bg`, `--accent` etc. from CodeMirror CSS variables and reuse them.

**Decided** (locked into the spec body):

- ✅ **Tree column persistence: per-pane.** Stored in block meta as `editor:tree_expanded` + `editor:tree_width`. Two editor panes in the same window can have independent tree states (e.g. one full-width for diffs, one tree-open for browsing).
- ✅ **Hidden files: hide by default.** Dotfiles + `node_modules` + standard noise (`.DS_Store`, `Thumbs.db`) hidden on first launch. Toolbar `eye` toggle flips per-pane (`editor:show_hidden` in block meta).
- ✅ **Symlinks: follow.** Matches VS Code semantics. Symlinked rows get a small `↗` overlay so they're identifiable.
- ✅ **Tree refresh: explicit only.** Toolbar `arrows-rotate` button refreshes the currently expanded folders. No background watcher in v1.

---

## Acceptance criteria

### Phase 1

- [ ] Editor pane opens with a tree column on the left, rooted at `$HOME`
- [ ] Tree's expand/collapse chevrons work; clicking a folder toggles visibility
- [ ] Clicking a file calls `model.openFile()` and the editor loads the content with syntax highlighting
- [ ] The active file's row is visually marked (highlight + filled-dot icon)
- [ ] Header chevron toggles tree visibility; preference persists per-pane across reload
- [ ] Toolbar with 3 buttons (`eye` / `square-minus` / `arrows-rotate`); tooltips appear instantly on hover (no delay) via the `data-tip` CSS pattern
- [ ] Hidden files (dotfiles, `node_modules`, `.DS_Store`, `Thumbs.db`) hidden by default; `eye` toggle flips per-pane and persists
- [ ] Collapse-all action returns the tree to root-only state in one click
- [ ] Refresh action re-fetches expanded folders without losing expansion state
- [ ] Symlinks are followed; symlinked rows carry a `↗` overlay
- [ ] No regression on the existing path-input flow or Ctrl+S save
- [ ] Backend `ListEditorDirectoryCommand` returns folders-first, alphabetical, with `is_dir` / `size` / `mtime`
- [ ] Backend `GetEditorHomeCommand` returns the correct OS home dir on Windows / macOS / Linux

### Phase 2 (nice-to-have, not gating)

- [ ] Hidden files toggle works
- [ ] Resizable divider persists width
- [ ] Arrow-key navigation across tree rows

### Phase 3 (extensions)

- [ ] `editor:extensions` setting controls which CM6 extensions activate
- [ ] At least one new extension shipped (autocomplete or lint)
- [ ] Documented on docs.agentmux.ai

---

## References

- [VS Code UX guidelines — Tree Views](https://code.visualstudio.com/api/ux-guidelines/overview)
- [VS Code Tree View API](https://code.visualstudio.com/api/extension-guides/tree-view)
- [Tree View UX patterns](https://uxpatterns.dev/patterns/data-display/tree-view)
- [Interaction Design for Trees — Hagan Rivers](https://medium.com/@hagan.rivers/interaction-design-for-trees-5e915b408ed2)
- [CodeMirror 6 reference manual](https://codemirror.net/docs/ref/)
- [CodeMirror 6 — list of core extensions](https://codemirror.net/docs/extensions/)
- [Plugin vs Extension — CodeMirror discuss](https://discuss.codemirror.net/t/plugin-vs-extension/5203)
- Existing code: `frontend/app/view/editor/{editor.tsx,editor-model.ts,editor-view.tsx}`
- Backend RPC pattern: `RpcApi.ReadEditorFileCommand` (`frontend/app/store/rpc-api.ts` + the handler in `agentmux-srv/src/server/`)
