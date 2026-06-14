# SPEC: File Tree Right-Click Context Menu

Status: Draft
Date: 2026-06-14
Depends on: `SPEC_BROWSER_AND_EDITOR_PANES_2026_04_16.md`
Related: `SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md`

---

## Problem

The editor pane's file tree (`frontend/app/view/editor/file-tree.tsx`) supports
single-click (preview open) and double-click (pin tab) — nothing else. Users have
no way to create, rename, delete, or copy file paths from inside the tree. Every
such action requires switching to a terminal. This is a significant friction point
that makes the editor pane feel incomplete as a file-management surface.

---

## Goals

1. Right-click on a **file** → context menu with file-level actions.
2. Right-click on a **folder** → context menu with folder-level actions.
3. Right-click on **empty space** in the tree → context menu for tree-level actions.
4. All destructive actions require confirmation.
5. All file-mutation actions go through backend RPCs (no shell commands from frontend).
6. Backend validates all paths against allowed roots (same policy as `readeditorfile` /
   `writeeditorfile`).

---

## Non-Goals

- Drag-and-drop move/copy (separate feature).
- Clipboard integration for cut/copy/paste of files (separate feature).
- Multi-select context menus (separate feature; single-node only for now).
- Symlink creation or management.

---

## Menu Definitions

### File node context menu

| Action | Shortcut | Notes |
|--------|----------|-------|
| **Open** | — | Pinned tab (same as double-click) |
| **Open to the Side** | — | Split-right of current pane |
| **Open in New Tab** | — | New app tab |
| *(separator)* | | |
| **Copy Path** | — | Absolute path to clipboard |
| **Copy Relative Path** | — | Relative to workspace root |
| *(separator)* | | |
| **Rename…** | F2 | Inline rename (see §Rename flow) |
| **Delete** | Del | Confirm dialog before RPC |
| *(separator)* | | |
| **Reveal in Explorer** | — | `open_in_shell` RPC (platform-aware) |

### Folder node context menu

| Action | Shortcut | Notes |
|--------|----------|-------|
| **New File…** | — | Inline name entry in tree (see §New File flow) |
| **New Folder…** | — | Inline name entry in tree |
| *(separator)* | | |
| **Rename…** | F2 | Inline rename |
| **Delete** | Del | Recursive confirm dialog |
| *(separator)* | | |
| **Open in Terminal** | — | `agent.open` or `term` pane with `cwd` set to this folder |
| **Reveal in Explorer** | — | Platform shell open |
| *(separator)* | | |
| **Collapse Folder** | — | Collapses this node and all descendants |

### Empty-space context menu

| Action | Notes |
|--------|-------|
| **New File…** | Creates in workspace root |
| **New Folder…** | Creates in workspace root |
| *(separator)* | |
| **Refresh** | Re-runs `listeditordir` from root |
| **Change Workspace Root…** | Inline path-entry in tree toolbar (already exists as a gap — covered here) |

---

## Flows

### Rename flow

1. User picks "Rename…" or presses F2 on a selected tree node.
2. The node label is replaced with an `<input>` pre-filled with the current name
   (not the full path — name only).
3. Enter confirms; Esc cancels.
4. On confirm: `renameeditorfile { old_path, new_name }` RPC.
   Backend constructs `new_path = parent(old_path) / new_name`, validates, renames.
5. If the renamed file was open in a tab: tab's `filePath` and `displayName` are
   updated via a new `FileRenamed` tab command in the editor-pane-state-store.
6. Tree node updates in place (no full re-scan needed — update node label locally,
   re-sort siblings if needed).

### New File / New Folder flow

1. User picks "New File…" from folder or empty-space menu.
2. A placeholder node appears inline in the tree (inside the target folder,
   or at root level) with an `<input>` for the filename, auto-focused.
3. Enter confirms; Esc cancels and removes the placeholder.
4. On confirm:
   - **New File:** `createeditorfile { parent_path, name }` RPC. Backend writes an
     empty file. Frontend opens it in a preview tab.
   - **New Folder:** `createeditordir { parent_path, name }` RPC. Backend creates the
     directory. Tree node is added collapsed.
5. Collision: if a file/folder with that name already exists, the input border turns
   red with an inline error label; the placeholder stays focused for re-entry.

### Delete flow

1. User picks "Delete" or presses Del.
2. A confirmation dialog appears:
   - File: "Delete `filename`? This cannot be undone."
   - Folder: "Delete `foldername` and all its contents? This cannot be undone."
3. Confirm: `deleteeditorfile { path, recursive: boolean }` RPC.
4. If the deleted file/folder had open tabs: those tabs are closed with a
   `dirty: false` override (we don't prompt to save a file that no longer exists).
5. Tree node is removed immediately on RPC success.

### Open in Terminal flow

1. Backend: `pane.open { view: "term", cwd: folder_path }` via the existing App API.
2. The terminal opens in a split-right position by default.

---

## New RPC Commands

All go in `agentmux-srv/src/server/websocket.rs` with types in `rpc_types.rs`.

### `renameeditorfile`

```
Request:  { cmd: "renameeditorfile", data: { old_path: string, new_name: string } }
Response: { new_path: string }
Errors:   PATH_DENIED, DESTINATION_EXISTS, IO_ERROR
```

### `createeditorfile`

```
Request:  { cmd: "createeditorfile", data: { parent_path: string, name: string } }
Response: { file_path: string }
Errors:   PATH_DENIED, DESTINATION_EXISTS, IO_ERROR
```

### `createeditordir`

```
Request:  { cmd: "createeditordir", data: { parent_path: string, name: string } }
Response: { dir_path: string }
Errors:   PATH_DENIED, DESTINATION_EXISTS, IO_ERROR
```

### `deleteeditorfile`

```
Request:  { cmd: "deleteeditorfile", data: { path: string, recursive: boolean } }
Response: {}
Errors:   PATH_DENIED, IO_ERROR
```

`recursive: true` is required for non-empty directories; the backend enforces this
(a non-empty dir with `recursive: false` returns `IO_ERROR: "directory not empty"`).

### `openinshell` (reveal in Explorer/Finder)

```
Request:  { cmd: "openinshell", data: { path: string } }
Response: {}
```

Platform behavior:
- **Windows:** `explorer.exe /select,<path>` (selects the file in Explorer)
- **macOS:** `open -R <path>` (reveals in Finder)
- **Linux:** `xdg-open <parent_dir>` (opens parent directory)

This RPC should be path-validated (allowed roots) to prevent exposing arbitrary
filesystem paths, even though it's read-only.

---

## Frontend: `<ContextMenu>` Component

No SolidJS context menu primitive exists in the codebase today. A minimal one is
needed.

**Component signature:**

```typescript
// frontend/app/components/context-menu.tsx
interface ContextMenuItem {
  type: "action" | "separator";
  label?: string;
  shortcut?: string;
  disabled?: boolean;
  danger?: boolean;  // red label for destructive actions
  onSelect?: () => void;
}

function ContextMenu(props: {
  items: ContextMenuItem[];
  x: number;
  y: number;
  onClose: () => void;
}): JSX.Element
```

**Behavior:**
- Renders as a `position: fixed` overlay at `(x, y)`, clamped to viewport bounds.
- Click outside or Esc → `onClose()`.
- Keyboard: Arrow Up/Down to navigate, Enter to select, Esc to close.
- Single-level only (no submenus for Phase 1).
- Styled consistent with the existing tab context menu (`tab-context-menu-cleanup.md`).

**Integration point in `file-tree.tsx`:**

```typescript
// Add onContextMenu handler to each TreeNode and the tree root:
const [ctxMenu, setCtxMenu] = createSignal<{ x: number; y: number; items: ContextMenuItem[] } | null>(null);

// On right-click:
function handleContextMenu(e: MouseEvent, node: NodeData | null) {
  e.preventDefault();
  setCtxMenu({ x: e.clientX, y: e.clientY, items: buildMenuItems(node) });
}
```

---

## Path Safety

All new backend RPCs follow the same validation as the existing `readeditorfile` and
`writeeditorfile` handlers:

1. Resolve the path to absolute.
2. Check it is under `home_dir()` (current rule). Phase 2: check against user-configured
   allowed roots once that system exists.
3. Reject any path containing `..` components that escape the allowed root.
4. Return `PATH_DENIED` on violation — never a partial success.

---

## Tab Synchronization

When file-system mutations happen, open editor tabs must stay consistent:

| RPC | Tab effect |
|-----|-----------|
| `renameeditorfile` | Tabs with matching `filePath` get `FileRenamed { new_path }` command |
| `deleteeditorfile` | Tabs with matching `filePath` or a matching path prefix (folder delete) get `TabForceClose` |
| `createeditorfile` | No effect on existing tabs; new file opens in a new preview tab |
| `createeditordir` | No effect on existing tabs |

The `FileRenamed` and `TabForceClose` commands need to be added to
`EditorPaneCommand` in `editor-pane-state-store.ts`.

---

## Implementation Phases

### Phase 1 — Read-only actions (1–2 days)

- `<ContextMenu>` component
- Right-click on file: Open, Open to the Side, Copy Path, Copy Relative Path, Reveal in Explorer
- Right-click on folder: Reveal in Explorer, Collapse Folder
- Right-click on empty space: Refresh
- `openinshell` RPC

No mutations, no new backend file-ops. Unlocks the most common user actions immediately.

### Phase 2 — Rename + New File/Folder (2–3 days)

- `renameeditorfile`, `createeditorfile`, `createeditordir` RPCs
- Inline rename flow (F2 / "Rename…")
- Inline new-file/folder placeholder in tree
- `FileRenamed` tab command + synchronization

### Phase 3 — Delete + Open in Terminal (1–2 days)

- `deleteeditorfile` RPC
- Delete confirmation dialog component (reusable)
- `TabForceClose` tab command + synchronization
- "Open in Terminal" action (via `pane.open term`)

### Phase 4 — Keyboard shortcuts + multi-select (future)

- F2 for rename, Del for delete, without context menu
- Multi-file select + batch operations

---

## Open Questions

- **Undo for delete?** Recommendation: no undo in Phase 3 (confirmation dialog is
  the safety net). Trash/recycle-bin integration is a separate, platform-specific
  feature.
- **Rename collision behavior:** Should renaming to an existing name offer to
  overwrite or always reject? Recommendation: always reject with an inline error;
  overwrite is an explicit copy+delete, not a rename.
- **"Open in Terminal" split position:** Right of current pane (default) or user-
  configurable? Recommendation: right of current pane for Phase 3; make configurable
  later via settings.
