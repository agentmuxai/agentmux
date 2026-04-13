# Process State Tracker — Spec

## Goal

A live, always-accurate in-process state model that knows:
- Every open **window** (CEF label, backend window ID)
- Every open **tab** per window (tab ID, workspace ID)
- Every open **pane** per tab (block ID, view type)
- Every **shell process** per pane (OS PID, PTY status)

This state is used for:
1. Correct cleanup on window/tab/pane close
2. Debugging orphaned processes
3. Future: process health monitoring, status indicators

---

## Data Model

```
AppState
└── process_tracker: ProcessTracker
    └── windows: HashMap<String, WindowState>   // key = CEF label
        ├── label: String                        // "main", "window-{uuid}"
        ├── backend_window_id: String            // backend WaveWindow OID
        └── tabs: HashMap<String, TabState>      // key = tab ID
            ├── tab_id: String
            ├── workspace_id: String
            └── panes: HashMap<String, PaneState> // key = block ID
                ├── block_id: String
                ├── view_type: String             // "term", "agent", etc.
                └── pid: Option<u32>              // OS PID, None if not a shell pane
```

---

## API

```rust
pub struct ProcessTracker {
    pub windows: Mutex<HashMap<String, WindowState>>,
}

impl ProcessTracker {
    /// Called when a CEF window finishes loading (on_load_end) OR when
    /// register_backend_window IPC arrives.
    pub fn register_window(&self, label: &str, backend_window_id: &str);

    /// Called in on_before_close. Returns backend_window_id for cleanup.
    pub fn unregister_window(&self, label: &str) -> Option<String>;

    /// Called when a tab is created/loaded. Replaces window_id_map.
    pub fn register_tab(&self, window_label: &str, tab_id: &str, workspace_id: &str);

    /// Called when a tab closes (CloseTab IPC) or window closes (bulk remove).
    pub fn unregister_tab(&self, window_label: &str, tab_id: &str);

    /// Called when a pane is opened and its shell PID is known.
    pub fn register_pane(&self, window_label: &str, tab_id: &str,
                          block_id: &str, view_type: &str, pid: Option<u32>);

    /// Called when a pane closes.
    pub fn unregister_pane(&self, window_label: &str, tab_id: &str, block_id: &str);

    /// Dump full state as JSON for diagnostics.
    pub fn dump(&self) -> serde_json::Value;

    /// Get all PIDs for a window (for kill-on-close).
    pub fn pids_for_window(&self, label: &str) -> Vec<u32>;

    /// Get all PIDs for a tab (for kill-on-tab-close).
    pub fn pids_for_tab(&self, label: &str, tab_id: &str) -> Vec<u32>;
}
```

---

## State Lifecycle

### Window

| Event | Action |
|-------|--------|
| `open_new_window` in `commands/window.rs` | `register_window(label, "")` — placeholder, no backend ID yet |
| Frontend calls `register_backend_window` IPC | `register_window(label, window_id)` — fills in backend ID |
| `on_before_close` fires | `unregister_window(label)` → returns backend_window_id |

### Tab

| Event | Action |
|-------|--------|
| Frontend sends `register_tab` IPC (new) | `register_tab(label, tab_id, workspace_id)` |
| Frontend sends `close_tab` IPC | `unregister_tab(label, tab_id)` |
| Window closes | all tabs removed via `unregister_window` |

### Pane / Shell

| Event | Action |
|-------|--------|
| Backend spawns shell via `ControllerResync` | sidecar reports PID via new `register_pane_pid` IPC |
| Pane closes (delete block) | `unregister_pane` |
| Tab closes | all panes in tab removed |

---

## New IPC Commands (CEF host)

| Command | Args | Description |
|---------|------|-------------|
| `register_tab` | `{label, tab_id, workspace_id}` | Called from frontend after tab is active |
| `unregister_tab` | `{label, tab_id}` | Called when tab closes |
| `register_pane_pid` | `{label, tab_id, block_id, pid}` | Called when shell PID is known |
| `get_process_state` | `{}` | Returns full `ProcessTracker.dump()` |

---

## Cleanup Strategy

### On Window Close (`on_before_close`)

```rust
let state = tracker.unregister_window(label);
if browser_list.is_empty() {
    quit_message_loop(); // Job Object kills sidecar → kills all shells
} else {
    // Call backend CloseWindow — kills only this window's shells
    backend_close_window(&web_endpoint, &auth_key, &state.backend_window_id);
}
```

### On Tab Close (`close_tab` IPC)

```rust
tracker.unregister_tab(label, tab_id);
// Backend already handles shell kill via CloseTab
```

### On Pane Close (already working via backend)

No CEF-side action needed — backend's `delete_tab_inner` handles it.

---

## Diagnostics

### Debug dump IPC

`get_process_state` returns:

```json
{
  "windows": {
    "main": {
      "backend_window_id": "abc-123",
      "tabs": {
        "tab-xyz": {
          "workspace_id": "ws-456",
          "panes": {
            "block-789": { "view_type": "term", "pid": 12345 }
          }
        }
      }
    },
    "window-abc": {
      "backend_window_id": "def-456",
      "tabs": {}
    }
  }
}
```

### Log file

All state changes logged to `%TEMP%\agentmux-state-debug.txt` (same pattern as current `agentmux-close-debug.txt`).

---

## Implementation Plan

1. **`state.rs`**: Add `process_tracker: ProcessTracker` alongside existing fields.
   - Replace `window_id_map` (it becomes part of `WindowState`).
   - `ProcessTracker` lives in `agentmux-cef/src/process_tracker.rs`.

2. **`client.rs`**: Update `on_after_created`, `on_before_close` to use `ProcessTracker`.

3. **`commands/window.rs`**: Update `register_backend_window`, `close_tab`, `create_tab` to use `ProcessTracker`.

4. **`ipc.rs`**: Add routing for new IPC commands.

5. **Frontend (`wave.ts`, `cef-api.ts`, `custom.d.ts`)**: Add `registerTab`, `unregisterTab` calls at tab lifecycle points.

6. **Sidecar bridge (future)**: Emit `register_pane_pid` from backend when shell spawns.

---

## Scope of Current Bug Fix

The immediate bug (`window_id_map` key mismatch) is fixed by the `pending_window_labels` queue in v0.33.47. The `ProcessTracker` is the longer-term replacement that makes the whole system observable and correct by construction — no more label mismatches because the tracker is the single source of truth.
