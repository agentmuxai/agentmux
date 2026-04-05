# Window Close Process Cleanup Spec

**Date:** 2026-04-04
**Status:** Ready to implement
**Priority:** P0 — orphaned processes accumulate and leak system resources

---

## Problem

Closing a single pane correctly kills its shell process. But closing an entire window (or tab) leaves all shell processes running as orphans. The processes remain visible in Task Manager until manually killed or the backend exits.

**Repro:**
1. Open AgentMux, open 2-3 terminal panes
2. Open a second window, open 2-3 terminal panes there
3. Close the second window
4. Check Task Manager — the shell processes (pwsh, bash, cmd) from the second window are still running

---

## Root Cause

Two functions delete database objects without stopping the shell controllers:

### 1. `close_window()` — `agentmux-srv/src/backend/wcore/window.rs:134`

```rust
pub fn close_window(store: &WaveStore, window_id: &str) -> Result<(), StoreError> {
    let mut client = get_client(store)?;
    client.windowids.retain(|id| id != window_id);
    store.update(&mut client)?;
    store.delete::<Window>(window_id)?;  // deletes Window, nothing else
    Ok(())
}
```

Deletes the Window object but does NOT cascade to workspace → tabs → blocks → controllers.

### 2. `delete_tab_inner()` — `agentmux-srv/src/backend/wcore/tab.rs:72`

```rust
pub(super) fn delete_tab_inner(store: &WaveStore, tab_id: &str) -> Result<(), StoreError> {
    if let Ok(tab) = store.must_get::<Tab>(tab_id) {
        if !tab.layoutstate.is_empty() {
            let _ = store.delete::<LayoutState>(&tab.layoutstate);
        }
        for block_id in &tab.blockids {
            let _ = store.delete::<Block>(block_id);  // deletes Block from DB only
        }
    }
    let _ = store.delete::<Tab>(tab_id);
    Ok(())
}
```

Deletes Block objects from the database but does NOT call `blockcontroller::delete_controller()` to kill the shell process.

### The working path (for reference)

`DeleteBlock` RPC handler in `service.rs:102-123` does it correctly:

```rust
blockcontroller::delete_controller(&block_id);  // kill process FIRST
wcore::delete_block(store, &tab_id, &block_id); // then delete from DB
```

---

## Data Model

```
Client
  └─ windowids: Vec<String>
       └─ Window
            ��─ workspaceid → Workspace
                               ├─ tabids: Vec<String>
                               └─ pinnedtabids: Vec<String>
                                    └─ Tab
                                         └─ blockids: Vec<String>
                                              └─ Block (has controller with shell process)
```

---

## Fix

### 1. `delete_tab_inner()` — Kill controllers before DB cleanup

**File:** `agentmux-srv/src/backend/wcore/tab.rs`

Add `blockcontroller::delete_controller()` call for each block before deleting from DB:

```rust
pub(super) fn delete_tab_inner(store: &WaveStore, tab_id: &str) -> Result<(), StoreError> {
    if let Ok(tab) = store.must_get::<Tab>(tab_id) {
        // Kill shell processes FIRST
        for block_id in &tab.blockids {
            crate::backend::blockcontroller::delete_controller(block_id);
        }
        // Then delete from database
        if !tab.layoutstate.is_empty() {
            let _ = store.delete::<LayoutState>(&tab.layoutstate);
        }
        for block_id in &tab.blockids {
            let _ = store.delete::<Block>(block_id);
        }
    }
    let _ = store.delete::<Tab>(tab_id);
    Ok(())
}
```

### 2. `close_window()` — Cascade through workspace → tabs → blocks

**File:** `agentmux-srv/src/backend/wcore/window.rs`

Before deleting the window, walk the full hierarchy:

```rust
pub fn close_window(store: &WaveStore, window_id: &str) -> Result<(), StoreError> {
    // Cascade: window → workspace → tabs → blocks (with controller cleanup)
    if let Ok(window) = store.must_get::<Window>(window_id) {
        if let Ok(workspace) = store.must_get::<Workspace>(&window.workspaceid) {
            let all_tab_ids: Vec<String> = workspace.tabids.iter()
                .chain(workspace.pinnedtabids.iter())
                .cloned()
                .collect();
            for tab_id in &all_tab_ids {
                let _ = delete_tab_inner(store, tab_id);
            }
            let _ = store.delete::<Workspace>(&window.workspaceid);
        }
    }

    let mut client = get_client(store)?;
    client.windowids.retain(|id| id != window_id);
    store.update(&mut client)?;
    store.delete::<Window>(window_id)?;
    Ok(())
}
```

### 3. CEF `on_before_close` — Notify backend when window closes

**File:** `agentmux-cef/src/client.rs` — `LifeSpanHandler::on_before_close()`

Currently this only unregisters the browser from the multi-window map. It should also tell the backend to clean up:

```rust
fn on_before_close(&self, browser: Option<&mut Browser>) {
    // ... existing browser unregister logic ...

    // Notify backend to clean up this window's resources
    if let Some(window_id) = get_window_id_for_browser(browser) {
        // Fire-and-forget RPC to backend
        let _ = rpc_call("window", "CloseWindow", json!({ "windowid": window_id }));
    }
}
```

This ensures the backend cleans up even if the frontend didn't send a close RPC before the window was destroyed (e.g., Alt+F4, OS kill, crash).

---

## Edge Cases

1. **Last window close = app exit:** When the last window closes, `on_before_close` calls `CefQuitMessageLoop()`. The backend sidecar is in a Job Object with `KILL_ON_JOB_CLOSE`, so it dies with the CEF process. Shell processes die because their parent (backend) dies. This path is fine — no orphans possible.

2. **Secondary window close:** This is the broken path. The backend stays alive (attached to the primary window), so shell processes orphan.

3. **Tab close (middle tab):** Same bug as window close — `delete_tab_inner` doesn't kill controllers.

4. **`delete_controller()` is idempotent:** If the controller was already stopped (e.g., shell exited on its own), `delete_controller` is a no-op. Safe to call unconditionally.

5. **Windows PTY cleanup:** On Windows, `ShellController::stop()` closes the PTY input channel which delivers EOF. The shell process should exit. If it doesn't (hung process), there's no `SIGKILL` equivalent — may need `TerminateProcess` as a fallback.

---

## Files to Change

| File | Change |
|---|---|
| `agentmux-srv/src/backend/wcore/tab.rs` | Add `delete_controller()` calls in `delete_tab_inner()` |
| `agentmux-srv/src/backend/wcore/window.rs` | Cascade cleanup through workspace → tabs in `close_window()` |
| `agentmux-cef/src/client.rs` | Fire CloseWindow RPC in `on_before_close()` (safety net) |

---

## Verification

1. Open two windows with 2+ terminal panes each
2. Close the second window
3. `tasklist | grep -E "pwsh|bash|cmd"` — no orphaned shells from the closed window
4. Close a tab in the first window — same check
5. Close the last window — app exits cleanly, all processes gone
