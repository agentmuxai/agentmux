# DnD Bug Fix Analysis — PR #83

**Date:** 2026-03-09
**Branch:** agenta/drag-drop-files

---

## Bug 1: Files/folders copied to wrong directory (or not at all)

### Root cause

Two issues combined:

**1a. Directories not supported**
`copy_file_to_dir` called `source.is_file()` and returned an error for directories.
When a user drags a folder, the Tauri `File` object's `.path` points to a directory — the copy silently failed.

**1b. OSC 7 path normalization vs. Windows paths**
`termosc.ts` normalizes CWD from OSC 7 sequences to forward-slash form (e.g. `C:/Users/foo`).
Rust's `std::path::Path::new("C:/Users/foo")` works on Windows, but `std::path::Path::exists()` on Windows requires canonical backslash separators in some edge cases depending on the Rust version and Windows API path.
Normalizing to `MAIN_SEPARATOR` (backslash on Windows) before Path construction is the safe approach.

### Fix (`src-tauri/src/commands/file_ops.rs`)

- Added `copy_recursive(src, dst)` helper that handles both files and directories.
  - For files: `std::fs::copy`
  - For directories: `create_dir_all` + recursive walk via `read_dir`
- Removed the `source.is_file()` guard — any existing path (file or dir) is now accepted.
- Added CWD normalization: `target_dir.replace('/', std::path::MAIN_SEPARATOR_STR)` before constructing the `Path`.

---

## Bug 2: DragOverlay stuck in "Copy..." after dragging away without dropping

### Root cause

The original `onDragLeave` handler used a **bounds-check** to distinguish "left the element" from "moved to a child element":

```typescript
const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
if (x <= rect.left || x >= rect.right || y <= rect.top || y >= rect.bottom) {
    setIsDragOver(false);
}
```

This fails in two cases:

1. **Child element nesting:** `dragLeave` fires when the pointer moves from the parent to any child element (e.g. the xterm canvas). The coordinates at that moment are _inside_ the parent rect, so the condition is false and state stays true. On the next `dragEnter` of the child, `isDragOver` gets set true again — but if the drag exits through a child element, the final `dragLeave` on the parent may not fire with out-of-bounds coords.

2. **Drag cancelled outside the window:** When the user drags the file out of the Tauri window and releases (or presses Escape), no `drop` event fires on the React element. On Windows, `dragLeave` sometimes fires with `clientX/Y = 0`, which may or may not be inside the rect depending on the element's position on screen.

### Fix (`frontend/app/hook/useFileDrop.ts`)

Replaced bounds-check with a **reference counter**:

- `dragCounter` ref is incremented on every `dragEnter` (for files only) and decremented on every `dragLeave`.
- `isDragOver` is only cleared when counter reaches 0 — meaning the drag has truly left the outermost drop zone.
- `onDrop` resets the counter to 0 unconditionally.
- A `useEffect` adds `dragend` and `drop` listeners on `document` as a safety net — these fire when the OS drag ends outside the window, ensuring the counter and state are always reset even if React events are missed.

---

## Files changed

| File | Change |
|------|--------|
| `src-tauri/src/commands/file_ops.rs` | Add `copy_recursive`, support dirs, normalize CWD separator |
| `frontend/app/hook/useFileDrop.ts` | Replace bounds-check with counter + document `dragend` safety net |
