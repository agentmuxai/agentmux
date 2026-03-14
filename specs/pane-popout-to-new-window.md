# Spec: Pane Pop-Out to New Window

Replace the magnify button with a pop-out button that moves a pane (and its live content) to a new AgentMux window.

## Architecture Context

AgentMux runs a single backend (`agentmuxsrv-rs`) per app version. Multiple frontend windows (Tauri WebView instances) connect to this shared backend. Each window has its own tabs and panes, but all panes across all windows share the same backend process, database, and block controllers.

When a pane is "moved" to a new window, the block controller (PTY, agent session, etc.) keeps running in the backend — only the frontend reference changes. The new window's frontend reconnects to the existing controller via `controllerresync`.

## Current State

### Magnify button
- **Icon:** `frontend/app/asset/magnify.svg` — rendered via `frontend/app/element/magnify.tsx`
- **Placement:** `BlockFrame_Header` in `frontend/app/block/blockframe.tsx` (~line 120)
- **Behavior:** Toggles in-place magnification (pane fills tab, siblings fade)
- **Disabled when:** Only one pane in the tab (`numLeafs <= 1`)

### Existing block transfer
The backend supports reparenting blocks between tabs:

| RPC | What it does |
|-----|-------------|
| `TearOffBlock(blockId, sourceTabId, sourceWsId, autoClose?)` | Removes block from source tab, creates new tab in a new container, reparents block. |
| `MoveBlockToTab(wsId, blockId, srcTabId, destTabId)` | Moves block between tabs. |

### Existing cross-window drag-and-drop (PR #81)
`CrossWindowDragMonitor.tsx` handles drag-out-of-window:
1. Detects `didDrop === false` (drag ended with no valid drop target)
2. Gets cursor position via Tauri `get_cursor_point`
3. Calls `TearOffBlock` to reparent the block
4. Calls `open_window_at_position(screenX, screenY)` to create a new Tauri window
5. New window connects to the same backend and renders the reparented block

### Window creation
- `open_new_window()` in `src-tauri/src/commands/window.rs` — creates a Tauri window, assigns instance number
- `open_window_at_position(x, y)` — same but positioned at cursor
- New window loads `index.html`, connects to the shared backend, gets its tab/block state

---

## Design

### Part 1: Pop-out button (replaces magnify)

**Icon:** New SVG — arrow pointing top-right (↗). Replace `magnify.svg` with `popout.svg`. Same dimensions and stroke style.

**Placement:** Same position in `BlockFrame_Header` where magnify lives.

**Click handler:**
```
popOutPane(blockId, tabId):
  1. Call TearOffBlock(blockId, tabId, autoClose=true)
     → backend removes block from source tab, reparents to new tab
  2. Call open_new_window() via Tauri
     → new window opens, connects to shared backend
     → frontend renders the reparented block
  3. Source window receives block-removal event via EventBus
     → layout tree removes the node
```

**Disabled when:** Only one pane in the tab (`numLeafs <= 1`). Popping out the last pane leaves an empty tab — do nothing.

**Remove magnify entirely.** Not used, adds clutter. Can return as a keyboard shortcut later.

### Part 2: Drag-out-of-window creates new window with pane

This already works via `CrossWindowDragMonitor.tsx`. Refactor to share the same code path as the button.

**Extract shared utility:**

```typescript
// frontend/app/util/popout.ts

async function popOutBlock(
  blockId: string,
  sourceTabId: string,
  position?: { x: number; y: number }  // screen coords for drag-out, undefined for button click
): Promise<void> {
  // 1. Tear off block in backend (reparents block to new tab)
  await ObjectService.TearOffBlock(blockId, sourceTabId, true /* autoClose */);

  // 2. Open new window
  if (position) {
    await getApi().openWindowAtPosition(position.x, position.y);
  } else {
    await getApi().openNewWindow();
  }

  // 3. Source layout cleanup happens via backend event
}
```

### What transfers with the block

The block record in the backend contains all persistent state. Block controllers run keyed by `blockId` and are not tied to any window.

| Content type | How it transfers |
|-------------|-----------------|
| **Terminal (PTY)** | Controller keeps running. New frontend calls `controllerresync` for scrollback + reconnects input. |
| **Agent session** | Same as terminal — agent pane wraps a PTY. Session continues. |
| **Code editor** | File path + unsaved content stored in block meta. New frontend opens same state. |
| **Sysinfo** | Stateless — new frontend subscribes to same reactive endpoint. |
| **Webview** | URL stored in block meta. Page reloads (no session transfer). |
| **Forge** | State in block meta. Reconnects to Forge data source. |

---

## Implementation Plan

### Step 1: Icon swap
- Create `frontend/app/asset/popout.svg` — top-right arrow (↗), same stroke weight as magnify
- Create `frontend/app/element/popout.tsx` — same pattern as `magnify.tsx`
- Delete `frontend/app/asset/magnify.svg` and `frontend/app/element/magnify.tsx`

### Step 2: Shared pop-out utility
- Create `frontend/app/util/popout.ts` with `popOutBlock()`

### Step 3: Button handler
- In `blockframe.tsx`, replace magnify `IconButtonDecl` with pop-out
- Click → `popOutBlock(blockId, tabId)`
- Disabled when `numLeafs <= 1`

### Step 4: Unify drag-out path
- Refactor `CrossWindowDragMonitor.tsx` `performTearOff()` to call `popOutBlock(blockId, tabId, { x, y })`
- Remove duplicated tear-off logic

### Step 5: Remove magnify
- Delete `layoutMagnify.ts` magnify functions (`magnifyNodeToggle`, etc.)
- Remove magnify from context menu in `pane-actions.ts`
- Remove magnify CSS variables (`--magnified-block-opacity`, `--magnified-block-blur`)
- Clean up `NodeModel` magnify atoms

### Step 6: Verify content transfer
- Terminal pop-out: scrollback preserved, input continues, no reconnect flash
- Agent pop-out: session continues, no re-auth
- Editor pop-out: unsaved changes preserved
- Last pane: button disabled, does nothing
- Drag-out: new window appears at cursor with pane content
