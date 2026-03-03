# Spec: Window Instance Indicator

**Status:** Draft
**Version:** 0.31.11
**Scope:** Frontend (window-controls.tsx) + Rust (window.rs, lib.rs)

---

## Overview

When multiple AgentMux windows are open, each window should display a sequential instance number next to the version string in the title bar. When only one window is open, no number is shown.

```
Single window:     agentmux v0.31.11
Two windows:       agentmux v0.31.11 (1)    agentmux v0.31.11 (2)
Three windows:     agentmux v0.31.11 (1)    agentmux v0.31.11 (2)    agentmux v0.31.11 (3)
Back to one:       agentmux v0.31.11
```

---

## Current State

**Version display** is in `frontend/app/window/window-controls.tsx` (line 37):
```tsx
<span>agentmux v{getApi().getAboutModalDetails()?.version ?? "?"}</span>
```

**Existing APIs available:**
- `getApi().listWindows()` → `Promise<string[]>` — returns all open Tauri window labels
- `getApi().getWindowLabel()` → `Promise<string>` — returns this window's Tauri label
- `getApi().isMainWindow()` → `Promise<boolean>` — true if label is `"main"`

**Window labels:** Main window is labeled `"main"`. Additional windows are `"window-{uuid}"`.

**Close tracking:** `src-tauri/src/lib.rs` lines 201–242 already counts remaining windows to decide when to kill the backend sidecar.

---

## Behaviour

### Instance Numbers

- The **main** window is always instance **1**.
- Each new window opened via `openNewWindow()` receives the next available integer in creation order.
- Numbers are **stable for the lifetime of a window** — they do not renumber when another window closes.
  - Example: Open 3 windows (1, 2, 3). Close window 2. Windows 1 and 3 keep their numbers.
- When **all extra windows are closed** and only window 1 remains, the indicator disappears entirely.

### Display Rule

| Open windows | Display |
|---|---|
| 1 | `agentmux v0.31.11` |
| 2+ | `agentmux v0.31.11 (N)` |

The `(N)` is shown inline, visually lightweight — same font, slightly dimmer than the version string.

---

## Architecture

### Rust Side — `src-tauri/src/commands/window.rs` + `lib.rs`

#### Global State

Add a thread-safe instance registry to the Tauri app state:

```rust
// src-tauri/src/state.rs (or inline in lib.rs)
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

pub struct WindowInstanceRegistry {
    // label → instance number (1-based)
    instances: HashMap<String, u32>,
    // monotonically increasing counter (never reused)
    next_num: u32,
}

impl WindowInstanceRegistry {
    pub fn new() -> Self {
        let mut reg = Self { instances: HashMap::new(), next_num: 1 };
        // Main window is always instance 1
        reg.instances.insert("main".to_string(), 1);
        reg.next_num = 2;
        reg
    }

    pub fn register(&mut self, label: &str) -> u32 {
        let num = self.next_num;
        self.instances.insert(label.to_string(), num);
        self.next_num += 1;
        num
    }

    pub fn unregister(&mut self, label: &str) {
        self.instances.remove(label);
    }

    pub fn get(&self, label: &str) -> Option<u32> {
        self.instances.get(label).copied()
    }

    pub fn count(&self) -> usize {
        self.instances.len()
    }
}

pub type SharedWindowRegistry = Arc<Mutex<WindowInstanceRegistry>>;
```

Register as Tauri app state in `lib.rs`:

```rust
let registry: SharedWindowRegistry = Arc::new(Mutex::new(WindowInstanceRegistry::new()));
app.manage(registry.clone());
```

#### New Window Command (window.rs)

When `open_new_window` is called, assign the next instance number and emit a `window-instances-changed` event to **all** windows:

```rust
#[tauri::command]
pub async fn open_new_window<R: Runtime>(
    app: tauri::AppHandle<R>,
    registry: tauri::State<'_, SharedWindowRegistry>,
) -> Result<String, String> {
    let label = format!("window-{}", uuid::Uuid::new_v4().simple());

    // Build and show window (existing code) ...

    // Register instance number
    let instance_num = {
        let mut reg = registry.lock().unwrap();
        reg.register(&label)
    };

    tracing::info!("New window {} assigned instance #{}", label, instance_num);

    // Notify all windows that instance count changed
    emit_instances_changed(&app, &registry);

    Ok(label)
}
```

#### Close Handler (lib.rs)

In the existing `on-window-event` / close handler, unregister the window and emit:

```rust
WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed => {
    let label = window.label().to_string();
    {
        let mut reg = state.window_registry.lock().unwrap();
        reg.unregister(&label);
    }
    emit_instances_changed(&app_handle, &state.window_registry);
    // ... existing backend shutdown logic ...
}
```

#### New Tauri Commands

```rust
// Returns the instance number for the calling window (0 if not found)
#[tauri::command]
pub fn get_instance_number(
    window: tauri::Window,
    registry: tauri::State<'_, SharedWindowRegistry>,
) -> u32 {
    let reg = registry.lock().unwrap();
    reg.get(window.label()).unwrap_or(0)
}

// Returns total number of open windows
#[tauri::command]
pub fn get_window_count(
    registry: tauri::State<'_, SharedWindowRegistry>,
) -> usize {
    let reg = registry.lock().unwrap();
    reg.count()
}
```

#### Event: `window-instances-changed`

Emitted to **all** windows whenever any window opens or closes:

```rust
fn emit_instances_changed<R: Runtime>(
    app: &tauri::AppHandle<R>,
    registry: &SharedWindowRegistry,
) {
    let reg = registry.lock().unwrap();
    let count = reg.count();
    // Emit to all windows so each can re-query their instance number
    let _ = app.emit("window-instances-changed", serde_json::json!({ "count": count }));
}
```

---

### Frontend Side

#### AppApi additions (`frontend/types/custom.d.ts`)

```ts
getInstanceNumber: () => Promise<number>;   // get-instance-number
getWindowCount: () => Promise<number>;      // get-window-count
```

#### Tauri API bridge (`frontend/util/tauri-api.ts`)

```ts
getInstanceNumber: () => invoke<number>("get_instance_number"),
getWindowCount: () => invoke<number>("get_window_count"),
```

#### Jotai Atoms (`frontend/app/store/global.ts`)

```ts
// Instance number for this window (stable, set once at init)
export const windowInstanceNumAtom = atom<number>(0);

// Total open window count (reactive, updates via Tauri event)
export const windowCountAtom = atom<number>(1);
```

Initialize at startup in `wave.ts` (after Tauri init completes):

```ts
const instanceNum = await getApi().getInstanceNumber();
globalStore.set(windowInstanceNumAtom, instanceNum);

const count = await getApi().getWindowCount();
globalStore.set(windowCountAtom, count);

// Subscribe to changes
listen("window-instances-changed", (event: { payload: { count: number } }) => {
    globalStore.set(windowCountAtom, event.payload.count);
});
```

#### window-controls.tsx

```tsx
import { useAtomValue } from "jotai";
import { windowInstanceNumAtom, windowCountAtom } from "../store/global";

export function WindowControls() {
    const instanceNum = useAtomValue(windowInstanceNumAtom);
    const windowCount = useAtomValue(windowCountAtom);
    const version = getApi().getAboutModalDetails()?.version ?? "?";

    const instanceLabel = windowCount > 1 ? ` (${instanceNum})` : "";

    return (
        // ...existing JSX...
        <span className="version-label">
            agentmux v{version}
            {windowCount > 1 && (
                <span className="instance-num"> ({instanceNum})</span>
            )}
        </span>
        // ...
    );
}
```

#### window-controls.scss

```scss
.version-label {
    // existing styles...

    .instance-num {
        opacity: 0.6;          // visually secondary to the version
        font-size: 0.9em;
        letter-spacing: 0;
    }
}
```

---

## Files to Modify

| File | Change |
|------|--------|
| `src-tauri/src/commands/window.rs` | Add `get_instance_number`, `get_window_count`; update `open_new_window` to register + emit |
| `src-tauri/src/lib.rs` | Initialize registry in app state; unregister + emit on window close |
| `src-tauri/src/state.rs` | Add `WindowInstanceRegistry` struct (or inline in lib.rs) |
| `frontend/types/custom.d.ts` | Add `getInstanceNumber`, `getWindowCount` to `AppApi` |
| `frontend/util/tauri-api.ts` | Wire up `invoke` calls for both new commands |
| `frontend/app/store/global.ts` | Add `windowInstanceNumAtom`, `windowCountAtom` |
| `frontend/wave.ts` | Init atoms + subscribe to `window-instances-changed` event |
| `frontend/app/window/window-controls.tsx` | Consume atoms, conditionally render `(N)` |
| `frontend/app/window/window-controls.scss` | Style `.instance-num` |

---

## Edge Cases

| Scenario | Behaviour |
|---|---|
| Single window open | No `(N)` shown at all |
| Window 2 closes, window 3 remains | Window 1 shows `(1)`, window 3 shows `(3)` — numbers don't renumber |
| Window 1 (main) closes while others open | Remaining windows keep their numbers; backend stays alive |
| New window opened rapidly (race) | Registry mutex serializes; each gets a unique number |
| Window crashes / force-killed | Tauri `Destroyed` event fires; registry unregisters; `window-instances-changed` emitted |
| Dev mode with hot reload | Frontend reinitializes; atoms re-fetched from Tauri commands on mount |

---

## Out of Scope

- Clicking the instance indicator to switch to a different window (separate feature)
- Persisting instance numbers across app restarts
- Showing instance numbers in the OS taskbar / window title (OS title remains unchanged)
- Any reordering or renumbering of existing windows

---

## Implementation Notes

- `windowInstanceNumAtom` is **set once** at init and never changes for the lifetime of a window. Only `windowCountAtom` is reactive.
- The `window-instances-changed` event carries only the total count, not the full map — this keeps the payload small. Each window already knows its own instance number from init time.
- The Tauri event listener in `wave.ts` must be set up **after** the window is fully initialized (after `initTauriWave` / `initTauriNewWindow` completes) to avoid missing the initial state.
- The registry must be initialized **before** any window events fire — do it in `Builder::setup()` before `run()`.
