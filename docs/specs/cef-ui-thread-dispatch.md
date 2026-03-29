# Spec: CEF UI Thread Dispatch for IPC Handlers

**Date:** 2026-03-29
**Status:** Ready to implement
**Priority:** Critical — devtools, zoom, and transparency all blocked

---

## Problem

CEF browser host methods (`host.show_dev_tools()`, `host.set_zoom_level()`, etc.)
must be called on the CEF UI thread. The AgentMux IPC server runs on tokio/axum
threads. Calling host methods directly from IPC handlers deadlocks the CEF message
loop, which:

- Freezes all JS execution (timers, promises, microtasks)
- Freezes all subsequent IPC requests (sendLog, etc.)
- Eventually crashes the process (devtools toggle killed the app)

### Affected Commands

| IPC Command | Rust Function | CEF API Called | Status |
|-------------|---------------|----------------|--------|
| `toggle_devtools` | `window::toggle_devtools` | `host.show_dev_tools()` | **Crashes app** |
| `set_zoom_factor` | `window::set_zoom_factor` | `host.set_zoom_level()` | **Deadlocks** (skipped) |
| `set_window_transparency` | stubbed | N/A (needs impl) | Stubbed |
| `close_window` | `window::close_window` | `host.try_close_browser()` | **Potential deadlock** |
| `minimize_window` | `window::minimize_window` | Window API | **Potential deadlock** |
| `maximize_window` | `window::maximize_window` | Window API | **Potential deadlock** |

---

## Solution: `wrap_task!` + `post_task(ThreadId::UI)`

The CEF Rust bindings provide:

```rust
use cef::{wrap_task, post_task, ThreadId};

wrap_task! {
    pub struct MyTask {
        // captured state
        browser: Browser,
    }

    impl Task {
        fn execute(&self) {
            // This runs on the CEF UI thread — safe to call host methods
            if let Some(host) = self.browser.host() {
                host.show_dev_tools(...);
            }
        }
    }
}

// From any thread:
let mut task = MyTask::new(browser.clone());
post_task(ThreadId::UI, Some(&mut task));
```

This marshals the work to the CEF UI thread's message loop, avoiding deadlocks.

---

## Implementation Plan

### Step 1: Create `ui_tasks.rs` module

New file: `agentmux-cef/src/ui_tasks.rs`

Define task structs for each host operation:

```rust
use cef::*;
use std::sync::Arc;
use crate::state::AppState;

// ── DevTools ──────────────────────────────────────────────────

wrap_task! {
    pub struct ShowDevToolsTask {
        state: Arc<AppState>,
    }

    impl Task {
        fn execute(&self) {
            let browser = self.state.browser.lock().unwrap();
            if let Some(ref browser) = *browser {
                if let Some(host) = browser.host() {
                    let window_info = WindowInfo {
                        runtime_style: RuntimeStyle::ALLOY,
                        ..Default::default()
                    };
                    host.show_dev_tools(Some(&window_info), None, None, None);
                }
            }
        }
    }
}

// ── Zoom ──────────────────────────────────────────────────────

wrap_task! {
    pub struct SetZoomLevelTask {
        state: Arc<AppState>,
        zoom_level: f64,
    }

    impl Task {
        fn execute(&self) {
            let browser = self.state.browser.lock().unwrap();
            if let Some(ref browser) = *browser {
                if let Some(host) = browser.host() {
                    host.set_zoom_level(self.zoom_level);
                }
            }
        }
    }
}
```

### Step 2: Update IPC command handlers

**`commands/window.rs`:**

```rust
use crate::ui_tasks::*;

pub fn toggle_devtools(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let mut task = ShowDevToolsTask::new(state.clone());
    cef::post_task(cef::ThreadId::UI, Some(&mut task));
    Ok(serde_json::Value::Null)
}

pub fn set_zoom_factor(state: &Arc<AppState>, args: &Value) -> Result<Value, String> {
    let factor = args.get("factor").and_then(|v| v.as_f64())
        .ok_or_else(|| "Missing factor".to_string())?;
    let factor = factor.clamp(0.5, 3.0);
    *state.zoom_factor.lock().unwrap() = factor;

    let zoom_level = factor.ln() / 1.2_f64.ln();
    let mut task = SetZoomLevelTask::new(state.clone(), zoom_level);
    cef::post_task(cef::ThreadId::UI, Some(&mut task));

    // Emit zoom-factor-change event (this is safe from any thread)
    crate::events::emit_event_from_state(state, "zoom-factor-change", &json!(factor));
    Ok(Value::Null)
}
```

### Step 3: Remove frontend workarounds

**`frontend/wave.ts`:** Remove the `isTauriHost()` guard on `setZoomFactor`:
```typescript
// BEFORE (workaround)
if (isTauriHost() && api && typeof api.setZoomFactor === "function") {
    api.setZoomFactor(1.0);
}

// AFTER (fixed)
if (api && typeof api.setZoomFactor === "function") {
    api.setZoomFactor(1.0);
}
```

**`frontend/util/cef-api.ts`:** Restore normal `toggleDevtools`:
```typescript
// BEFORE (workaround)
toggleDevtools: () => {
    window.open("http://localhost:9222", "_blank");
},

// AFTER (fixed)
toggleDevtools: () => {
    invokeCommand("toggle_devtools").catch(console.error);
},
```

### Step 4: Apply same pattern to all window commands

All commands that touch `state.browser` should use `post_task`:
- `close_window` → `CloseBrowserTask`
- `minimize_window` → direct Win32 API (no CEF host needed, safe)
- `maximize_window` → direct Win32 API (safe)

---

## Testing

1. Click devtools widget → DevTools opens in a new CEF window
2. Ctrl+/- zoom → terminal font changes (zoom works without deadlock)
3. Transparency setting → block bg becomes transparent
4. Close window → app shuts down cleanly
5. All IPC commands continue to work after any of the above

---

## Notes

- `post_task` is async (fire-and-forget) — the IPC handler returns immediately
  while the task executes later on the UI thread
- `emit_event_from_state` (event emission via JS injection) appears safe from
  any thread — it calls `frame.execute_javascript()` which CEF documents as
  thread-safe
- The `wrap_task!` macro requires the captured fields to be `Send` — `Arc<AppState>`
  is `Send`, so this works
- Keep `remote_debugging_port: 9222` as a fallback for when DevTools needs to be
  accessed from an external browser
