# Spec: Drag & Drop Files Into Panes

**Status:** Ready for implementation
**Date:** 2026-03-08

## Problem

Users cannot drag files from their OS file manager into AgentMux panes. The only pane that supports file drops today is the Wave AI chat panel (`aipanel/`), which accepts images and documents for AI analysis. Terminal panes and agent panes have no drop handling at all.

Drag & drop is a fundamental desktop interaction. Users expect to drag a file onto a terminal and have it land there.

## Goal

Support file drag & drop across all pane types with pane-appropriate behavior:

| Pane Type | Drop Behavior |
|-----------|---------------|
| **Terminal** | Copy file to the shell's current working directory |
| **Agent** | Attach file to the agent conversation (existing AI panel behavior) |

Visual feedback (overlay, highlight) indicates the drop target and action before the user releases.

## Design

### 1. Terminal Pane — Copy to CWD

**User story:** Drag `report.pdf` from Explorer/Finder onto a terminal pane. The file is copied to the terminal's current working directory.

#### How it works

1. User drags file(s) over a terminal pane
2. Drop overlay appears: `"Copy {n} file(s) to {cwd}"`
3. User drops
4. For each file:
   - Read the file path from the `DataTransfer` (Tauri exposes native file paths via `dataTransfer.files`)
   - Invoke a new Tauri command `copy_file_to_dir` with `{ sourcePath, targetDir }`
   - The Rust command copies the file using `std::fs::copy`
5. After copy, inject a terminal notification line: `# Copied report.pdf to /home/user/project/`

#### CWD resolution

The terminal's working directory is already tracked via OSC 7 sequences and stored in block metadata at `cmd:cwd`. The drop handler reads it from the block's meta, same as `FilePathLinkProvider` does today:

```typescript
const cwd = blockData?.meta?.["cmd:cwd"];
```

If CWD is unknown (no OSC 7 received yet), fall back to the user's home directory.

#### Multiple files

Multiple files are copied sequentially. The overlay shows the count: `"Copy 3 files to ~/project/"`. Each file gets its own copy command. If a file with the same name already exists, the copy is skipped and a warning is shown.

#### Conflict handling

- **Same name exists:** Skip and show warning in terminal: `# Skipped report.pdf (already exists)`
- **Permission error:** Show error in terminal: `# Failed to copy report.pdf: permission denied`
- **No CWD:** Copy to `$HOME` and warn: `# No working directory detected, copied to ~/`

### 2. Agent Pane — Attach to Conversation

**User story:** Drag `screenshot.png` onto an agent pane. The file is attached to the current conversation, same as using the existing AI panel's drop zone.

This already works in `aipanel/aipanel.tsx`. The agent pane (`view/agent/`) needs to:

1. Add the same `onDragOver`/`onDragEnter`/`onDragLeave`/`onDrop` handlers
2. Reuse the existing `isAcceptableFile()` / `validateFileSize()` / `model.addFile()` logic from aipanel
3. Show the same drag overlay with `"Drop files for analysis"`

#### Implementation

Extract the drag & drop logic from `aipanel/aipanel.tsx` into a shared hook:

```typescript
// frontend/app/hook/useFileDrop.ts
export function useFileDrop(onFilesDropped: (files: File[]) => Promise<void>) {
    const [isDragOver, setIsDragOver] = useState(false);
    // ... dragOver, dragEnter, dragLeave, drop handlers
    return { isDragOver, handlers: { onDragOver, onDragEnter, onDragLeave, onDrop } };
}
```

Both `aipanel/` and `view/agent/` consume this hook.


## Tauri Backend

### New command: `copy_file_to_dir`

```rust
// src-tauri/src/commands/file_ops.rs

#[tauri::command]
pub async fn copy_file_to_dir(
    source_path: String,
    target_dir: String,
) -> Result<String, String> {
    let source = std::path::Path::new(&source_path);
    let target_dir = std::path::Path::new(&target_dir);

    let file_name = source.file_name()
        .ok_or("Invalid source path")?;
    let target = target_dir.join(file_name);

    if target.exists() {
        return Err(format!("File already exists: {}", target.display()));
    }

    std::fs::copy(source, &target)
        .map_err(|e| format!("Copy failed: {}", e))?;

    Ok(target.display().to_string())
}
```

### Capability update

The current `fs:allow-write-file` is scoped to `$APPDATA/**` and `$APPCONFIG/**`. The copy command uses `std::fs::copy` directly (not Tauri's fs plugin), so no capability change is needed — it runs in the Rust backend with full filesystem access.

### File path from DataTransfer

In Tauri v2, dragging files from the OS file manager into the webview provides native file paths in the `DataTransfer`. The `file.path` property (Webkit/Blink extension) or Tauri's `onDragDropEvent` can be used:

```typescript
// Option A: DataTransfer (works in Tauri webview)
const filePath = (file as any).path; // Webkit provides native path

// Option B: Tauri drag-drop event listener
import { listen } from "@tauri-apps/api/event";
listen<{ paths: string[] }>("tauri://drag-drop", (event) => {
    // event.payload.paths contains native file paths
});
```

**Recommended:** Use Tauri's `tauri://drag-drop` event for reliable cross-platform native paths, combined with frontend drag state tracking to know which pane is the target.

## Frontend Architecture

### Drop target detection

Each pane type registers its own drop handlers. The pane wrapper (layout system) does NOT handle drops globally — each pane opts in to its own behavior.

```
Terminal pane:  onDrop -> copy_file_to_dir(path, cwd)
Agent pane:     onDrop -> model.addFile(file)
Other panes:    no drop handler (default browser behavior / ignore)
```

### Drag overlay component

Shared `DragOverlay` component used by all pane types:

```typescript
// frontend/app/element/dragoverlay.tsx
interface DragOverlayProps {
    message: string;  // "Copy 2 files to ~/project/" or "Drop files for analysis"
    visible: boolean;
}
```

Styled consistently with the existing AI panel's `AIDragOverlay`.

### Terminal drop handler

Added to `frontend/app/view/term/term.tsx`:

```typescript
const handleDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    const cwd = blockData?.meta?.["cmd:cwd"] ?? homedir;

    // Use Tauri drag-drop event for native paths
    // OR extract from DataTransfer
    for (const file of files) {
        try {
            await invoke("copy_file_to_dir", {
                sourcePath: file.path,
                targetDir: cwd,
            });
            // Inject feedback into terminal
            writeToTerminal(`\r\n# Copied ${file.name} to ${cwd}/\r\n`);
        } catch (err) {
            writeToTerminal(`\r\n# ${err}\r\n`);
        }
    }
};
```

## Implementation Plan

### Phase 1: Terminal drop (core feature)

1. Add `copy_file_to_dir` Tauri command in `src-tauri/src/commands/`
2. Register command in `src-tauri/src/lib.rs`
3. Add drop handlers to terminal pane (`frontend/app/view/term/term.tsx`)
4. Add `DragOverlay` shared component
5. Wire up Tauri `tauri://drag-drop` event for native file paths

### Phase 2: Agent pane drop

1. Extract `useFileDrop` hook from `aipanel/aipanel.tsx`
2. Add drop handlers to agent pane (`frontend/app/view/agent/agent-view.tsx`)
3. Refactor aipanel to use the shared hook

### Phase 3: Polish

1. Handle edge cases (no CWD, permission errors, name conflicts)
2. Animate the drag overlay (fade in/out)
3. Show file count and destination in overlay text

## Out of Scope

- **Drag OUT of panes** (e.g., drag a file path from terminal to Explorer) — separate feature
- **Drag between panes** (e.g., drag file from one terminal to another) — separate feature
- **Directory drops** — copy directories recursively (add later if needed)
- **Remote SSH terminals** — file copy would need to go through the SSH connection; not covered here

## Testing

- Drag single file from Explorer/Finder onto terminal -> file appears in CWD
- Drag multiple files -> all copied, overlay shows count
- Drag onto terminal with unknown CWD -> copies to home dir with warning
- Drag file that already exists -> skip with warning
- Drag image onto agent pane -> attached to conversation
- Drag unsupported file onto agent pane -> rejection error shown
- Cross-platform: Windows (Explorer), macOS (Finder), Linux (Nautilus/Dolphin)
