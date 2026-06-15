# SPEC: Editor Widget Default UX — Scratch File + Collapsed Tree

Status: Draft
Date: 2026-06-14
Depends on: `SPEC_BROWSER_AND_EDITOR_PANES_2026_04_16.md`, `app-api-pane-open.md`
Related: `SPEC_FILE_TREE_CONTEXT_MENU_2026_06_14.md`

---

## Problem

Clicking the **Editor** widget in the dock today calls `createBlock` with
`blockdef.meta = { view: "editor", file: "" }`. The result is an editor pane with
no file open and an expanded (but empty) file tree — a blank-stare UX. Users have
to manually navigate the tree, find a file, and click it before they can do anything.

Two specific gaps:

1. **No scratch/untitled buffer.** There is no way to open the editor and start
   typing without first picking a file. `EditorTab` requires a `filePath`; the model
   has no concept of an in-memory or deferred-path buffer.

2. **File tree is open by default.** For users who just want to take a quick note or
   paste something, the tree is noise. It should start collapsed and expand on demand.

---

## Goals

1. Clicking the Editor widget opens the pane with the file tree **collapsed**.
2. A new **scratch buffer** (Untitled-1, Untitled-2, …) is automatically open and
   focused — the user can start typing immediately.
3. The scratch file is backed by a cache file on disk so content survives crashes
   and app restarts.
4. Saving promotes the scratch file to a real path (Save As flow).
5. Multiple scratch buffers are supported (one per click, or via New File action).
6. Old unsaved scratch files are auto-recovered on next open; old *saved or
   discarded* scratch files are cleaned up.

---

## Non-Goals

- A full "workspace" model (no multi-root, no `.agentmux-workspace` file).
- System file-picker dialog (we use an inline rename-style path entry instead).
- Scratch files shared between panes or across windows.

---

## Design

### 1. Widget default behavior change

`agentmux-srv/src/config/widgets.json` — update the editor widget blockdef:

```json
{
  "defwidget@editor": {
    "blockdef": {
      "meta": {
        "view": "editor",
        "editor:scratch": true,
        "editor:tree_expanded": false
      }
    }
  }
}
```

- `editor:scratch: true` — signals the editor view to create/reopen a scratch buffer
  instead of showing an empty state.
- `editor:tree_expanded: false` — tree starts collapsed. User can expand via the
  tree-toggle button (already exists in the toolbar). The expanded/collapsed state
  is then persisted per-pane in block meta.

### 2. Scratch file infrastructure (`ScratchFileService`)

New backend service in `agentmux-srv/src/editor/scratch.rs`:

```
~/.agentmux/cache/scratch/
    <uuid>.md          ← active scratch file, auto-created
    <uuid>.md.meta     ← JSON sidecar: { displayName, createdAt, lastModifiedAt, savedTo }
```

**Directory:** `{data_dir}/cache/scratch/` — inside the per-instance data dir so
multiple AgentMux instances each have isolated scratch files. On first write, the dir
is created if absent.

**Lifecycle:**

| Event | Action |
|-------|--------|
| User opens editor widget (`editor:scratch: true`) | Service checks for an existing active scratch file (no `savedTo`, modified < 30 days). Reuses the most recent one, or creates a new UUID file. |
| User types | `writeeditorfile` saves to the scratch path on every Ctrl+S (same as regular files). Auto-save every 30 s of idle if dirty (Phase 2). |
| User hits Ctrl+Shift+S / "Save As" | Inline path-entry field appears in the tab header. User types a destination path. Backend copies scratch → destination, sets `savedTo` in the `.meta` sidecar, reopens the tab with the real path. Scratch file is then eligible for cleanup. |
| Pane closed (buffer still unsaved) | File stays in `cache/scratch/`. On next open, it is recovered (shown with ↻ recovery indicator in the tab name). |
| App restarted, scratch file exists | Editor widget auto-reopens the unsaved scratch file (recovery). |
| Scratch file is older than 30 days and has never been saved | Cleanup pass on startup deletes it. |

### 3. `EditorTab` model changes

`frontend/app/store/editor-pane-state-store.ts` — extend `EditorTab`:

```typescript
interface EditorTab {
  // ... existing fields ...
  filePath: string | null;  // null = scratch/untitled (was always string)
  scratchId?: string;       // UUID of the backing scratch file; set when filePath is null
  displayName: string;      // "Untitled-1", "report.md", etc. — used in tab label
  isScratch: boolean;       // true when backed by a scratch cache file
}
```

**Key invariant:** `filePath === null` iff `isScratch === true && scratchId !== null`.
All read/write RPCs route to the scratch path when `filePath` is null.

### 4. New RPC: `createscratchfile`

`agentmux-srv/src/server/websocket.rs` — new command:

```
Request:  { cmd: "createscratchfile", data: { display_name?: string } }
Response: { scratch_id: string, file_path: string, display_name: string }
```

The frontend calls this once when `editor:scratch: true` is set and no active
scratch tab exists yet. The response path is used as the backing `filePath` internally
(not shown in the tab label — `displayName` is shown instead).

### 5. `pane.open` with `is_new: true`

Per the updated `app-api-pane-open.md`, agents can also trigger scratch buffer creation:

```json
{ "view": "editor", "is_new": true, "language": "markdown" }
```

The handler calls `createscratchfile` internally, then opens the tab pointing to
the returned scratch path with `isScratch: true`.

### 6. Save As flow (inline path entry)

When the user saves an untitled/scratch tab (Ctrl+S first time, or Ctrl+Shift+S any time):

1. The tab header's filename area switches to an editable `<input>` field,
   pre-filled with the suggested path (e.g. `~/notes/untitled.md`).
2. User edits the path and presses Enter (or Esc to cancel).
3. On Enter: backend runs `movescratchfile { scratch_id, destination_path }`.
   - Validates destination path is within allowed roots.
   - Creates parent dirs if needed.
   - Moves (not copies) the scratch file to the destination.
   - Updates `.meta` sidecar with `savedTo`.
4. Tab `filePath` is updated to the real path, `isScratch` → false, `scratchId` cleared.
5. Tab `dirty` is cleared.

No system file-picker dialog is used — the inline approach is consistent with the
tab-rename interaction already used for tab titles.

### 7. New RPC: `movescratchfile`

```
Request:  { cmd: "movescratchfile", data: { scratch_id: string, destination_path: string } }
Response: { file_path: string } | { error: string }
```

Errors: `PATH_DENIED`, `PARENT_DIR_CREATE_FAILED`, `DESTINATION_EXISTS` (offer
overwrite prompt on frontend).

### 8. File tree collapsed state

`editor:tree_expanded` is a block meta key (persisted in sidecar per pane):

- Default from widget blockdef: `false` (collapsed).
- Toggling the tree-toggle button in the editor toolbar sets
  `editor:tree_expanded: true/false` in the pane's block meta.
- Survives tab close/reopen and pane resize.

No new infrastructure needed — block meta persistence already exists.

---

## Infrastructure Summary

| Component | Change | New? |
|-----------|--------|------|
| `widgets.json` | Add `editor:scratch: true`, `editor:tree_expanded: false` | No (edit) |
| `EditorTab` interface | `filePath: string \| null`, add `scratchId`, `displayName`, `isScratch` | No (edit) |
| `editor-pane-state-store.ts` | Handle `filePath: null` in all read/write paths | No (edit) |
| `agentmux-srv/src/editor/scratch.rs` | `ScratchFileService` — create, recover, clean up | **Yes (new)** |
| `websocket.rs` | `createscratchfile` command | **Yes (new)** |
| `websocket.rs` | `movescratchfile` command | **Yes (new)** |
| `rpc_types.rs` | `CommandCreateScratchFile`, `CommandMoveScratchFile` structs | **Yes (new)** |
| `frontend/app/view/editor/editor-tab-header.tsx` | Save As inline input field | No (edit) |
| `frontend/app/view/editor/editor-view.tsx` | Handle `editor:scratch` meta on mount | No (edit) |
| `app-api-pane-open.md` | `is_new: true` variant | Updated |

---

## Implementation Phases

### Phase 1 — Scratch buffer (3–4 days)

- `ScratchFileService` with create + recover + cleanup
- `createscratchfile` RPC
- `EditorTab.filePath: string | null` + `isScratch` flag
- Widget blockdef: `editor:scratch: true`
- Tab label shows "Untitled-1" (no Save As yet — Ctrl+S saves to scratch path, title
  stays "Untitled-1 ●")

### Phase 2 — Save As flow (1–2 days)

- `movescratchfile` RPC
- Inline path-entry field in tab header on Ctrl+Shift+S
- Ctrl+S on a scratch file triggers Save As the first time

### Phase 3 — Auto-save + recovery UI (1 day)

- 30-second idle auto-save to scratch path
- Recovery indicator (↻) in tab name when reopening an unsaved crash survivor
- Startup recovery scan and popup: "Restore 2 unsaved files?"

### Phase 4 — `pane.open is_new` + agent access (1 day)

- Wire `is_new: true` in `app-api-pane-open.md` handler to `ScratchFileService`
- Update agent system prompt docs

---

## Open Questions

- **Multiple untitled buffers:** Should a second click on the editor widget create a
  second scratch file (Untitled-2), or focus the existing untitled pane?
  Recommendation: create a new one; the user clicked the widget intentionally.
- **Scratch file directory location:** `{data_dir}/cache/scratch/` vs a fixed
  `~/.agentmux/cache/scratch/` shared across instances?
  Recommendation: per-instance data dir to match the isolation invariants (I6).
- **Auto-save default on/off:** Recommendation: off in Phase 1 (explicit Ctrl+S only),
  on in Phase 3 after recovery UI exists so users can verify it's working.
