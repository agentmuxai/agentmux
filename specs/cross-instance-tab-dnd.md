# Universal Drag-and-Drop: Tabs and Panes Across All Boundaries

> **Status:** SPEC
> **Date:** 2026-03-07
> **Author:** Agent1
> **Priority:** HIGH
> **Target:** 0.34.x series
> **Supersedes:** (initial cross-instance-tab-dnd draft)

---

## Executive Summary

AgentMux should allow **any content** — a pane (block) or a tab — to be moved **anywhere**: within the same tab, between tabs in the same window, between different windows, or out into a brand-new window. Dragging should feel as natural as Chrome tab tear-off or VS Code editor group splitting. This spec defines the complete drag-and-drop system holistically, covering every boundary crossing and the architectural changes needed to support them.

**Design Principle:** There is one unified drag system. Whether you're moving a pane three inches to the right or across two monitors into a new window, the same state machine governs the operation. The only thing that changes is the *scope of the commit*.

---

## 1. Conceptual Model

### 1.1 Entity Hierarchy

```
Instance (Window)
  Workspace (1:1 with window, currently)
    Tab  (appears in tab bar, has a layout)
      LayoutTree (binary tree of splits)
        LayoutNode (leaf = pane, internal = split)
          Block (terminal, editor, agent, webview, etc.)
```

### 1.2 What Can Be Dragged

| Draggable | What It Is | What Moves With It |
|-----------|------------|-------------------|
| **Pane** | A leaf LayoutNode | The block it contains (terminal session, editor state, etc.) |
| **Tab** | A Tab object | Its entire LayoutTree and all blocks within it |

### 1.3 Where Things Can Be Dropped

| Drop Target | Result |
|-------------|--------|
| **Same tab's layout** | Rearrange panes (existing behavior) |
| **Different tab's layout** (same window) | Move pane from tab A to tab B |
| **Tab bar** (same window) | Promote pane to its own new tab |
| **Different window's layout** | Move pane into that window's active tab |
| **Different window's tab bar** | Move tab between windows, or promote pane to tab in other window |
| **Outside all windows** | Create new window with the dragged content |

### 1.4 The Two Drag Scopes

**Scope 1 — Intra-Window (react-dnd)**
Drag stays within a single webview. Handled entirely by the existing react-dnd system in the frontend. No backend coordination needed beyond persisting the final state.

**Scope 2 — Cross-Window (Rust-coordinated)**
Drag crosses webview boundaries. Since browser DnD doesn't work across Tauri webviews, the Rust backend coordinates via its event system. The frontend transitions from Scope 1 to Scope 2 when the cursor leaves the window bounds.

---

## 2. Drag State Machine

Every drag operation follows the same state machine regardless of scope:

```
                     mousedown + 5px movement
                            |
                            v
     +------ IDLE -----> DRAGGING (intra-window) ------+
     |                      |                           |
     |         cursor leaves window bounds              |
     |                      |                           |
     |                      v                           |
     |              DRAGGING (cross-window)             |
     |                      |                           |
     |         cursor re-enters same window             |
     |                      |                           |
     |                      v                           |
     |              DRAGGING (intra-window)             |
     |                      |                           |
     |        mouseup       |       Escape              |
     |          |           |         |                  |
     |          v           |         v                  |
     |        DROP          |      CANCEL ---------------+
     |          |           |         |
     |          v           |         v
     +<--- COMMITTED       +<---- REVERTED
```

### 2.1 State Definitions

| State | Owner | Description |
|-------|-------|-------------|
| `IDLE` | Frontend | No drag in progress |
| `DRAGGING_LOCAL` | Frontend (react-dnd) | Drag within current window. Existing TileLayout DnD. |
| `DRAGGING_CROSS` | Rust backend | Cursor left window. Backend holds session, broadcasts events. |
| `DROP_LOCAL` | Frontend | Commit layout change locally, persist to backend |
| `DROP_CROSS` | Rust backend | Commit cross-window transfer atomically |
| `DROP_EXTERNAL` | Rust backend | Create new window, transfer content |
| `CANCEL` | Either | Revert all pending state |

### 2.2 Scope Transition: Local to Cross-Window

The critical moment is when the cursor leaves the source window during a drag. Detection:

```typescript
// Frontend: during an active react-dnd drag
document.addEventListener("mouseleave", (e) => {
    if (!activeDrag) return;

    // Cursor has left the webview — escalate to cross-window mode
    escalateToCrossWindow(activeDrag, e.screenX, e.screenY);
});
```

On escalation:
1. Frontend calls Tauri command `start_cross_drag` with drag metadata
2. react-dnd drag is synthetically ended (no commit)
3. Backend takes over cursor tracking via OS-level APIs
4. All windows receive `cross-drag-active` event and show drop zones

When cursor re-enters *any* window:
1. Backend detects via window geometry hit-test
2. Emits `cross-drag-enter` to that window
3. That window's frontend takes over with local react-dnd preview
4. If cursor leaves again, back to cross-window mode

---

## 3. Architecture

### 3.1 Current Architecture (Reference)

```
Window (Tauri webview)
  React app
    Workspace atom → Tab atom → LayoutModel → TileLayout (react-dnd)
      LayoutNode tree → Block components

agentmuxsrv-rs (Rust sidecar)
  SQLite: WaveObj store (blocks, tabs, workspaces, layouts)
  WebSocket: push updates to all connected frontends
```

**Key constraint:** Each window is a separate webview with its own JS runtime. No shared memory, no shared DOM. All cross-window state goes through the Rust backend.

### 3.2 New Components

```
src-tauri/src/
  commands/
    drag.rs              # NEW: Cross-window drag coordination
  state.rs               # MODIFIED: Add DragSession to AppState

frontend/
  layout/lib/
    layoutDrag.ts         # NEW: Unified drag controller (extracted from LayoutModel)
  app/tab/
    tabbar.tsx            # MODIFIED: Drop target for tabs + panes
    tab.tsx               # MODIFIED: Draggable tab handles
  app/drag/
    DragOverlay.tsx       # NEW: Cross-window drop zone overlay
    DragGhost.tsx         # NEW: Floating preview during cross-window drag

agentmuxsrv-rs/src/
  rpc/
    tab_transfer.rs       # NEW: MoveBlock, MoveTab RPCs
```

### 3.3 Backend Drag Session

```rust
// src-tauri/src/state.rs

#[derive(Debug, Clone, Serialize)]
pub struct DragSession {
    pub drag_id: String,
    pub drag_type: DragType,          // Pane or Tab
    pub source_window: String,        // Window label
    pub source_workspace_id: String,
    pub source_tab_id: String,
    pub payload: DragPayload,
    pub started_at: u64,              // Unix ms
}

#[derive(Debug, Clone, Serialize)]
pub enum DragType { Pane, Tab }

#[derive(Debug, Clone, Serialize)]
pub enum DragPayload {
    Pane {
        block_id: String,
        node_id: String,              // LayoutNode ID in source tree
    },
    Tab {
        tab_id: String,
    },
}

pub struct AppState {
    // ... existing fields ...
    pub active_drag: Mutex<Option<DragSession>>,
}
```

### 3.4 Tauri Commands (Cross-Window Drag)

```rust
// src-tauri/src/commands/drag.rs

/// Escalate a local drag to cross-window mode.
/// Called when cursor leaves the source window during an active drag.
#[tauri::command]
pub async fn start_cross_drag(
    app: AppHandle,
    state: State<'_, AppState>,
    drag_type: String,          // "pane" | "tab"
    source_tab_id: String,
    source_workspace_id: String,
    payload: serde_json::Value, // DragPayload
) -> Result<String, String>;   // Returns drag_id

/// Called periodically (throttled) with screen cursor position.
/// Backend resolves target window and emits events.
#[tauri::command]
pub async fn update_cross_drag(
    app: AppHandle,
    state: State<'_, AppState>,
    drag_id: String,
    screen_x: f64,
    screen_y: f64,
) -> Result<CrossDragUpdate, String>;

/// Complete the cross-window drag at current cursor position.
#[tauri::command]
pub async fn complete_cross_drag(
    app: AppHandle,
    state: State<'_, AppState>,
    drag_id: String,
    screen_x: f64,
    screen_y: f64,
    target_info: Option<DropTargetInfo>, // From target window, if known
) -> Result<DragResult, String>;

/// Cancel an active cross-window drag.
#[tauri::command]
pub async fn cancel_cross_drag(
    app: AppHandle,
    state: State<'_, AppState>,
    drag_id: String,
) -> Result<(), String>;

#[derive(Serialize)]
pub struct CrossDragUpdate {
    pub target_window: Option<String>, // Window label cursor is over
}

#[derive(Serialize)]
pub struct DropTargetInfo {
    pub window_label: String,
    pub target_type: String,        // "layout" | "tabbar" | "tabbar-gap"
    pub target_tab_id: Option<String>,
    pub insert_index: Option<i32>,  // For tab bar drops
    pub drop_direction: Option<String>, // For layout drops
    pub target_node_id: Option<String>, // For layout drops
}
```

---

## 4. Operations Matrix

### 4.1 Pane (Block) Drag Operations

| # | From | To | Scope | Mechanism | Backend Mutation |
|---|------|----|-------|-----------|------------------|
| P1 | Tab A layout | Tab A layout (same tab) | Local | react-dnd `Move`/`Swap` | Update layout tree (existing) |
| P2 | Tab A layout | Tab B layout (same window) | Local | react-dnd + tab-aware drop | `MoveBlockToTab(blockId, srcTabId, destTabId, insertDirection, targetNodeId)` |
| P3 | Tab A layout | Tab bar (same window) | Local | Drop on tab bar | `PromoteBlockToTab(blockId, srcTabId, workspaceId, insertIndex)` |
| P4 | Window 1 layout | Window 2 layout | Cross | Rust-coordinated | `MoveBlockToTab(blockId, srcTabId, destTabId, insertDirection, targetNodeId)` |
| P5 | Window 1 layout | Window 2 tab bar | Cross | Rust-coordinated | `PromoteBlockToTab(blockId, srcTabId, destWorkspaceId, insertIndex)` |
| P6 | Any layout | Outside all windows | Cross | Rust-coordinated | `TearOffBlock(blockId, srcTabId, screenX, screenY)` |

### 4.2 Tab Drag Operations

| # | From | To | Scope | Mechanism | Backend Mutation |
|---|------|----|-------|-----------|------------------|
| T1 | Tab bar | Tab bar (same window, reorder) | Local | react-dnd sortable | `ReorderTab(workspaceId, tabId, newIndex)` |
| T2 | Tab bar (Window 1) | Tab bar (Window 2) | Cross | Rust-coordinated | `MoveTabToWorkspace(tabId, srcWorkspaceId, destWorkspaceId, insertIndex)` |
| T3 | Tab bar | Outside all windows | Cross | Rust-coordinated | `TearOffTab(tabId, srcWorkspaceId, screenX, screenY)` |

### 4.3 Keyboard Modifier: Copy

Holding **Ctrl** (Windows/Linux) or **Option** (macOS) during any drag operation converts it from **move** to **copy**:
- **Copy pane**: Duplicates the block (new terminal at same CWD, editor with same file, etc.)
- **Copy tab**: Duplicates tab + all its blocks

This is a stretch goal — not required for initial implementation.

---

## 5. Backend RPC Mutations

All cross-boundary operations are atomic RPCs to `agentmuxsrv-rs`. Each one transacts against SQLite and emits `waveobj:update` events for all affected objects.

### 5.1 MoveBlockToTab

Move a single block (pane) from one tab to another. Tabs may be in different workspaces/windows.

```
MoveBlockToTab {
    block_id: String,
    source_tab_id: String,
    dest_tab_id: String,
    drop_direction: DropDirection,  // Where to insert in dest layout
    target_node_id: Option<String>, // Adjacent node in dest layout (if any)
}
```

**Steps:**
1. Remove `block_id` from `source_tab.blockids`
2. Remove corresponding leaf node from source tab's layout tree
3. If source tab's layout is now empty and it's not the last tab, delete the tab
4. Add `block_id` to `dest_tab.blockids`
5. Insert new leaf node into dest tab's layout tree at specified position
6. Persist both tabs' layout states + block arrays
7. Emit updates for both tabs

### 5.2 PromoteBlockToTab

Extract a block from its current tab and create a new tab containing just that block.

```
PromoteBlockToTab {
    block_id: String,
    source_tab_id: String,
    dest_workspace_id: String,
    insert_index: i32,        // Position in workspace tabids, -1 = append
}
```

**Steps:**
1. Remove `block_id` from source tab (same as MoveBlockToTab step 1-3)
2. Create new Tab object with `blockids: [block_id]`
3. Create new LayoutState with single-node tree
4. Insert new tab into `dest_workspace.tabids` at `insert_index`
5. Set new tab as `activetabid`
6. Persist and emit

### 5.3 MoveTabToWorkspace

Move an entire tab from one workspace to another.

```
MoveTabToWorkspace {
    tab_id: String,
    source_workspace_id: String,
    dest_workspace_id: String,
    insert_index: i32,
}
```

**Steps:**
1. Validate: source workspace has >1 tab (cannot remove last tab)
2. Remove `tab_id` from `source_workspace.tabids` and `pinnedtabids`
3. If `source_workspace.activetabid == tab_id`, set to adjacent tab
4. Insert `tab_id` into `dest_workspace.tabids` at `insert_index`
5. Set as `dest_workspace.activetabid`
6. Persist both workspaces, emit updates

### 5.4 ReorderTab

Reorder a tab within its workspace.

```
ReorderTab {
    workspace_id: String,
    tab_id: String,
    new_index: i32,
}
```

### 5.5 TearOffBlock

Remove a block from its tab and create a new window containing it.

```
TearOffBlock {
    block_id: String,
    source_tab_id: String,
    screen_x: f64,
    screen_y: f64,
}
```

**Steps:**
1. Remove block from source tab (MoveBlockToTab steps 1-3)
2. Rust: call `open_new_window()` at `(screen_x, screen_y)`
3. New window initializes with new workspace + new tab
4. Move block into the new tab
5. New window becomes visible with single-pane content

### 5.6 TearOffTab

Remove a tab from its workspace and create a new window containing it.

```
TearOffTab {
    tab_id: String,
    source_workspace_id: String,
    screen_x: f64,
    screen_y: f64,
}
```

**Steps:**
1. Validate: source workspace has >1 tab
2. Create new window at cursor position
3. Move tab to new window's workspace via `MoveTabToWorkspace`

---

## 6. Frontend Implementation

### 6.1 Intra-Window: Extending react-dnd

The existing react-dnd setup in `TileLayout` handles pane-to-pane within a single tab (operation P1). We extend it for P2 and P3.

#### Multi-Tab Drop Targets

Currently, only the active tab's `OverlayNode` components are drop targets. To support dropping a pane into a different tab (P2), we need the tab bar itself to be a drop target:

```typescript
// tabbar.tsx — each tab in the bar accepts pane drops
const [{ isOver }, dropRef] = useDrop({
    accept: "TILE_ITEM",
    hover: (item: LayoutNode) => {
        // Highlight this tab as a valid drop target
        setTabDropHighlight(tabId);
    },
    drop: (item: LayoutNode) => {
        // Pane dropped onto a tab — move block to that tab
        RpcApi.MoveBlockToTab({
            block_id: item.data.blockId,
            source_tab_id: activeTabId,
            dest_tab_id: tabId,
            drop_direction: "Center",  // Append to that tab's layout
        });
    },
});
```

#### Tab Bar as "New Tab" Drop Zone

An empty area at the end of the tab bar accepts pane drops to promote them to new tabs (P3):

```typescript
// tabbar.tsx — empty zone at end of tab bar
const [{ isOver }, newTabDropRef] = useDrop({
    accept: "TILE_ITEM",
    drop: (item: LayoutNode) => {
        RpcApi.PromoteBlockToTab({
            block_id: item.data.blockId,
            source_tab_id: activeTabId,
            dest_workspace_id: workspaceId,
            insert_index: -1,
        });
    },
});
```

#### Tab Dragging (T1)

Tab reordering within the tab bar uses react-dnd with a new item type:

```typescript
const TAB_ITEM = "TAB_ITEM";

// tab.tsx — make tabs draggable
const [{ isDragging }, dragRef] = useDrag({
    type: TAB_ITEM,
    item: { tabId, workspaceId },
    canDrag: () => tabCount > 1, // Cannot drag last tab
});
```

### 6.2 Cross-Window: Escalation Protocol

When a local drag's cursor leaves the window, we escalate to cross-window mode.

#### Detecting Window Exit

```typescript
// DragEscalationMonitor — wraps the active drag
useEffect(() => {
    if (!isDragging) return;

    const onMouseLeave = async (e: MouseEvent) => {
        // Cursor left the webview — escalate
        const dragId = await invoke<string>("start_cross_drag", {
            dragType: currentDragType,     // "pane" or "tab"
            sourceTabId: activeTabId,
            sourceWorkspaceId: workspaceId,
            payload: currentDragPayload,
        });

        // Cancel the local react-dnd drag
        // (react-dnd has no clean cancel API — we simulate mouseup)
        cancelLocalDrag();

        // Start tracking cursor at document level
        startCrossWindowTracking(dragId);
    };

    document.addEventListener("mouseleave", onMouseLeave);
    return () => document.removeEventListener("mouseleave", onMouseLeave);
}, [isDragging]);
```

#### Cross-Window Cursor Tracking

Once in cross-window mode, the source window tracks the mouse via the document (mouse events still fire even when cursor is outside, as long as a button is held from a mousedown that originated in the window):

```typescript
function startCrossWindowTracking(dragId: string) {
    const onMouseMove = throttle(16, async (e: MouseEvent) => {
        const update = await invoke<CrossDragUpdate>("update_cross_drag", {
            dragId,
            screenX: e.screenX,
            screenY: e.screenY,
        });
        // Backend broadcasts target info to all windows
    });

    const onMouseUp = async (e: MouseEvent) => {
        await invoke("complete_cross_drag", {
            dragId,
            screenX: e.screenX,
            screenY: e.screenY,
        });
        cleanup();
    };

    const onKeyDown = async (e: KeyboardEvent) => {
        if (e.key === "Escape") {
            await invoke("cancel_cross_drag", { dragId });
            cleanup();
        }
    };

    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
    document.addEventListener("keydown", onKeyDown);
}
```

#### Target Window Drop Zones

Every window listens for cross-drag events and shows drop zone overlays:

```typescript
// DragOverlay.tsx — rendered in every window
useEffect(() => {
    const unlisten = listen<CrossDragEvent>("cross-drag-update", (event) => {
        const { targetWindow, dragType, screenX, screenY } = event.payload;

        if (targetWindow === myWindowLabel) {
            // This window is the target — show drop zones
            const localPoint = screenToLocal(screenX, screenY);
            setDropZones(computeDropZones(localPoint, dragType));
        } else {
            setDropZones(null);
        }
    });

    return () => unlisten.then(f => f());
}, []);
```

Drop zones render as translucent overlays:
- **Layout zones**: Same 9-zone system as existing TileLayout (Top/Bottom/Left/Right/Center + Outer variants)
- **Tab bar zone**: Insertion line between tabs
- **New tab zone**: Highlighted empty area at end of tab bar

### 6.3 Visual Feedback

#### Ghost Preview

During cross-window drag, a small floating preview follows the cursor. Two approaches:

**Option A (Recommended): CSS cursor + overlay indicators**
- Change cursor to `grabbing` during drag
- Target window shows highlighted drop zone with preview of where content will land
- No floating window needed
- Simpler, works on all platforms

**Option B (Future polish): Tauri overlay window**
- Create a tiny borderless transparent window showing a miniature of the dragged content
- Follows cursor position via `update_cross_drag`
- More visually impressive but more complex

#### Drop Zone Indicators

```
Tab bar:  [ Tab1 ] |<insertion line>| [ Tab2 ] [ Tab3 ] [ + drop here ]

Layout:   +--------+--------+
          |  zone  |  zone  |
          | (left) | (right)|
          +--------+--------+
          |    zone (bottom) |
          +------------------+
```

Colors:
- Valid drop zone on hover: `var(--accent-color)` with 20% opacity fill
- Insertion line: 2px solid `var(--accent-color)`
- Invalid zone: no highlight

---

## 7. Edge Cases

### 7.1 Last Pane / Last Tab Guards

| Scenario | Behavior |
|----------|----------|
| Drag last pane out of a tab | Block if tab would be empty AND it's the last tab in workspace. Allow if workspace has other tabs (empty tab auto-closes). |
| Drag last tab out of a window | Block. Cannot have an empty window. Show "bounce-back" animation. |
| Drag last pane out of last tab | Block entirely. At least one pane must exist. |

### 7.2 Process Continuity

| Content Type | On Move | Behavior |
|--------------|---------|----------|
| Terminal (PTY) | Pane or tab move | PTY session continues. Managed by `agentmuxsrv-rs`, not the webview. Terminal reconnects in new location. |
| Code editor | Move | File handle and buffer preserved. Monaco instance unmounts/remounts but unsaved state lives in block metadata. |
| AI agent | Move | Agent session continues. WebSocket connection to backend is window-independent. |
| Webview | Move | URL and navigation state preserved in block metadata. Webview re-renders at new location. |
| Sysinfo | Move | Stateless — re-renders with current data. |

### 7.3 Timing and Concurrency

| Scenario | Resolution |
|----------|------------|
| Two drags at same time (different users/inputs) | Backend rejects second `start_cross_drag` while one is active. Return error. |
| Window closes during cross-drag | If source window: cancel drag. If target window: re-evaluate target on next cursor update. |
| Tab deleted during cross-drag of its pane | Cancel drag. Source no longer valid. |
| Very fast drag (skip cross-window, land on external) | `complete_cross_drag` handles directly — no need for intermediate `update` calls. |
| Drag into minimized/hidden window | Cannot target minimized windows. Backend skips them in hit-test. |

### 7.4 Multi-Monitor

The backend's window hit-test uses `outer_position()` and `outer_size()` which return screen coordinates. This works across monitors regardless of DPI or arrangement. No special handling needed.

---

## 8. Data Flow Examples

### 8.1 Move Pane from Tab A to Tab B (Same Window) — P2

```
User drags pane from Tab A's layout, drops on Tab B in tab bar

1. react-dnd detects drop on Tab B's drop target
2. Frontend calls: RpcApi.MoveBlockToTab({
       block_id: "blk-123",
       source_tab_id: "tab-A",
       dest_tab_id: "tab-B",
       drop_direction: "Center",
   })
3. Backend:
   a. Remove "blk-123" from tab-A.blockids
   b. Remove leaf node from tab-A's layout tree
   c. Add "blk-123" to tab-B.blockids
   d. Add leaf node to tab-B's layout tree
   e. Persist both layout states
   f. Emit waveobj:update for tab-A, tab-B, layout-A, layout-B
4. Frontend:
   a. Tab A's LayoutModel re-renders (pane gone)
   b. Tab B's LayoutModel re-renders (pane appears)
   c. If tab A is now empty and not last tab, auto-close it
```

### 8.2 Tear Off Pane to New Window — P6

```
User drags pane, cursor leaves all windows, releases mouse

1. Local react-dnd drag starts in source window
2. Cursor leaves window → escalate to cross-window mode
3. Frontend calls: invoke("start_cross_drag", { ... })
4. Cursor is outside all windows, user releases mouse
5. Frontend calls: invoke("complete_cross_drag", {
       dragId: "drag-456",
       screenX: 1500.0,
       screenY: 800.0,
   })
6. Backend (complete_cross_drag):
   a. No target window found → tear-off mode
   b. Call open_new_window() → new window at (1500, 800)
   c. New window initializes: creates workspace + tab
   d. Call MoveBlockToTab(block_id, src_tab, new_tab, Center)
   e. Block appears in new window
7. Source window: pane disappears (via waveobj:update subscription)
8. New window: pane appears (rendered on init)
```

### 8.3 Move Tab Between Windows — T2

```
User drags Tab 3 from Window A's tab bar, drops on Window B's tab bar

1. react-dnd drag starts on tab (type: TAB_ITEM)
2. Cursor leaves Window A → escalate
3. Backend broadcasts cross-drag events
4. Window B shows tab bar insertion indicator
5. User releases over Window B's tab bar (between Tab 1 and Tab 2)
6. Window B emits DropTargetInfo: { target_type: "tabbar", insert_index: 1 }
7. Backend calls: MoveTabToWorkspace(tab3, wsA, wsB, 1)
8. Window A: tab disappears, activates adjacent tab
9. Window B: tab appears at index 1, becomes active
```

---

## 9. Implementation Plan

### Phase 1: Intra-Window Pane DnD Enhancements
**Scope:** P1 (existing), P2 (new), P3 (new)

- [ ] Add `TILE_ITEM` drop handling to tab bar items (P2)
- [ ] Add "new tab" drop zone at end of tab bar (P3)
- [ ] Implement `MoveBlockToTab` RPC in agentmuxsrv-rs
- [ ] Implement `PromoteBlockToTab` RPC in agentmuxsrv-rs
- [ ] Auto-close empty tabs (when not last)
- [ ] Tests: move pane between tabs, promote pane to tab

### Phase 2: Tab Bar DnD (Intra-Window)
**Scope:** T1
**Depends on:** workspace-tab-modernization spec (tab bar UI)

- [ ] Add `TAB_ITEM` drag type to `tab.tsx`
- [ ] Add sortable drop zones in tab bar
- [ ] Implement `ReorderTab` RPC
- [ ] Tests: reorder tabs within tab bar

### Phase 3: Cross-Window Drag Infrastructure
**Scope:** Backend coordination layer

- [ ] Add `DragSession` to `AppState`
- [ ] Implement `start_cross_drag`, `update_cross_drag`, `complete_cross_drag`, `cancel_cross_drag` commands
- [ ] Window hit-test using `outer_position()` + `outer_size()`
- [ ] Tauri event broadcasting: `cross-drag-update`, `cross-drag-end`
- [ ] Register all commands in `src-tauri/src/lib.rs`
- [ ] Tests: unit tests for hit-testing, session management

### Phase 4: Cross-Window Pane DnD
**Scope:** P4, P5, P6

- [ ] `DragOverlay.tsx` — drop zone overlay component for target windows
- [ ] `DragEscalationMonitor` — detect cursor leaving window during react-dnd drag
- [ ] Cross-window cursor tracking (source window `mousemove` → Tauri command)
- [ ] Target window drop zone computation from screen coordinates
- [ ] Tear-off: create new window at cursor position (P6)
- [ ] Tests: cross-window pane transfer, tear-off

### Phase 5: Cross-Window Tab DnD
**Scope:** T2, T3

- [ ] Tab drag escalation (same mechanism as pane, different payload)
- [ ] Tab bar drop zones in target windows
- [ ] Implement `TearOffTab` flow
- [ ] Last-tab guards (prevent removing last tab from window)
- [ ] Tests: move tab between windows, tab tear-off

### Phase 6: Polish
- [ ] Drop zone animations (fade in/out, insertion line pulse)
- [ ] Bounce-back animation on blocked drops
- [ ] Ctrl/Option+drag to copy instead of move
- [ ] Keyboard-only tab/pane move via context menu
- [ ] Accessibility: screen reader announcements for drag operations

---

## 10. Testing Strategy

### Unit Tests (vitest)
- `MoveBlockToTab`: pane moves, source/dest layout trees update correctly
- `PromoteBlockToTab`: new tab created with block, source updated
- `MoveTabToWorkspace`: tab transfers, active tab selection updates
- `ReorderTab`: tab order changes correctly
- `TearOffBlock` / `TearOffTab`: new window + workspace + tab created
- Last-pane/last-tab guard enforcement
- Window hit-test geometry (multi-monitor scenarios)

### Integration Tests
- Open window → drag pane to tab bar → verify new tab created
- Open two tabs → drag pane from tab 1 to tab 2 → verify transfer
- Terminal pane: verify PTY survives move (output continues)
- Editor pane: verify unsaved content survives move

### E2E Tests (Playwright via `e2e` CLI)
- Multi-window: open 2 windows, drag tab between them
- Tear-off: drag pane outside window, verify new window appears
- Cancel: start drag, press Escape, verify no state change
- Last-tab guard: attempt to drag out last tab, verify blocked

---

## 11. Alternatives Considered

### A. Use a docking library (FlexLayout, Dockview, GoldenLayout)

**Rejected.** These libraries manage their own layout trees and state, conflicting with AgentMux's existing `LayoutModel` + `WaveObj` persistence system. Integrating would require replacing the entire layout layer. The benefit (cross-window DnD) doesn't justify the cost (rewrite + dependency).

### B. HTML5 dataTransfer cross-window

**Rejected.** Doesn't work across Tauri webviews (isolated browsing contexts). Even in Electron, this approach is fragile and browser-dependent.

### C. SharedWorker / BroadcastChannel

**Rejected.** Tauri webviews don't guarantee shared origin. And we already have a better coordination channel: the Rust backend with its event system.

### D. OS-native drag-and-drop (OLE on Windows, XDnD on Linux)

**Deferred.** Would provide the most native feel but requires platform-specific Rust code for each OS. The Tauri-event approach covers 95% of the UX. Can be explored later if the custom approach feels sluggish.

### E. Always tear-off, then merge (Chrome model pure)

**Partially adopted.** The Chrome model (drag past edge = immediate tear-off, then merge by proximity) is elegant for tabs. But for panes, users expect to drag directly into another window's layout without creating an intermediate window. Our hybrid approach uses Chrome-style tear-off as the *external drop* fallback, while also supporting direct cross-window placement.

---

## 12. Open Questions

1. **Should empty tabs auto-close?** When the last pane is dragged out of a tab (but the workspace has other tabs), should the empty tab close automatically or remain as an empty workspace the user can populate?
   - **Recommendation:** Auto-close. Empty tabs serve no purpose.

2. **Should pinned tabs be draggable cross-window?** They can be dragged but should they lose pin status?
   - **Recommendation:** Yes, preserve pin status on transfer. It's one extra field.

3. **Should we support dragging multiple tabs at once?** (Shift+click to multi-select, then drag all)
   - **Recommendation:** Defer. Single-tab drag covers the common case. Multi-select adds significant complexity.

4. **Should the tab bar show tabs from inactive windows as merge targets?** (Like VS Code's "Move to Window" dropdown)
   - **Recommendation:** Add a context menu item "Move to Window X" as a non-drag alternative. Lower implementation cost, covers keyboard-only users.

---

## Appendix A: File Inventory

| File | Status | Role |
|------|--------|------|
| `src-tauri/src/commands/drag.rs` | NEW | Cross-window drag Tauri commands |
| `src-tauri/src/state.rs` | MODIFY | Add `DragSession` |
| `src-tauri/src/lib.rs` | MODIFY | Register new commands |
| `agentmuxsrv-rs/src/rpc/tab_transfer.rs` | NEW | MoveBlockToTab, PromoteBlockToTab, MoveTabToWorkspace, TearOff RPCs |
| `frontend/app/drag/DragOverlay.tsx` | NEW | Cross-window drop zone overlay |
| `frontend/app/drag/DragEscalationMonitor.tsx` | NEW | Detects cursor leaving window during drag |
| `frontend/app/tab/tab.tsx` | MODIFY | Add TAB_ITEM drag source |
| `frontend/app/tab/tabbar.tsx` | MODIFY | Add TILE_ITEM + TAB_ITEM drop targets |
| `frontend/app/tab/tabcontent.tsx` | MODIFY | Wire up DragEscalationMonitor |
| `frontend/layout/lib/types.ts` | MODIFY | Export TILE_ITEM constant |
| `frontend/layout/lib/TileLayout.tsx` | MODIFY | Expose drag state for escalation |
| `frontend/types/gotypes.d.ts` | MODIFY | Add RPC types |

## Appendix B: Tauri Events

| Event Name | Direction | Payload | When |
|------------|-----------|---------|------|
| `cross-drag-update` | Backend → All windows | `{ dragId, dragType, payload, targetWindow, screenX, screenY }` | Every throttled cursor update during cross-drag |
| `cross-drag-end` | Backend → All windows | `{ dragId, result }` | Drag completed or cancelled |
| `window-instances-changed` | Backend → All windows | `count: usize` | Window opened or closed (existing) |
