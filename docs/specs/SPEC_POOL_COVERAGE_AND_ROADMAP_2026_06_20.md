# Pre-warmed Window Pool — Coverage Map and Implementation Roadmap

**Date:** 2026-06-20  
**Status:** Living document — update as phases ship  
**Author:** agent investigation + consolidation  

---

## 1. Current Coverage (as of 2026-06-20)

| Flow | Windows | macOS | Linux | Notes |
|------|---------|-------|-------|-------|
| **Tab tear-off** | ✅ Phase 6 | ✅ Phase 7 (#1595) | ✅ Phase 7 (#1595) | Full pool on all platforms |
| **Pane (floating) tear-off** | ❌ cold | ❌ cold | ❌ cold | Separate spec §3 below |
| **New window** | ❌ cold | ❌ cold | ❌ cold | Separate spec §4 below |

**Cold path latency:** ~150–300 ms CEF spawn + ~2–4 s frontend cold-load  
**Warm path latency:** ~5–30 ms (set_bounds/SetWindowPos + renderer already painting)

Pool constants (window_pool.rs):
```
POOL_TARGET_SIZE = 2
POOL_OFFSCREEN_X/Y = (-32000, -32000)
POOL_WIDTH  = 1200
POOL_HEIGHT = 800
```

---

## 2. Architecture Primer

### Tab tear-off pool (reference implementation)

```
Startup
  on_after_created("main")
    → init_pool()
      → spawn_pool_window()   [label="window-pool-{uuid}", url=?pool=1, x=-32000, y=-32000]
        → on_after_created("window-pool-*")
          → register_pool_window()         [Win32: cache HWND, hide from taskbar]
            → frontend: installs pool:promote listener, sends "pool_window_ready" IPC
              → mark_pool_window_renderer_ready()  [unpromoted → queue]
                → if pool_size < target → spawn_pool_window() [refill chain]

Tear-off (tab)
  frontend: openTearOffWindow()
    → api.tearOffPoolPromote(wsId, x, y, w, h)
      → host: promote_pool_window()
          Windows: HWND cache → SetWindowPos → ShowWindow(SW_SHOW)
          macOS/Linux: reducer pop → PromotePoolWindowTask → window.set_bounds() → window.show()
        → emit pool:promote { workspaceId }
          → spawn_pool_window() [refill]
  frontend: awaitPoolPromote() receives pool:promote → attach workspace → paint
```

### Why pane and new-window need separate treatment

**Pane windows** are created with `frameless=true` (CEF Views `is_frameless`). This is set at window creation time and cannot be changed afterward. A tab-pool window (frameless=false, has title bar) cannot be repurposed as a pane window without a visible flash of chrome. Therefore pane pool requires a **dedicated pool of frameless windows**.

**New-window** opens top-level chrome windows identical to tab tear-off targets. The existing tab pool CAN be shared, because the difference between "tab tear-off" and "open new window" is only in the workspace attachment step (pool:promote vs pool:new-window event).

---

## 3. Pane (Floating Pane) Tear-off Pool

### 3.1 Problem

`open_floating_pane_window` always takes the cold path on all platforms:

```
user drags pane out
  → frontend: CrossWindowDragMonitor calls open_floating_pane_window()
    → host: post_create_window(..., frameless=true, url=?floatingPaneId=X&workspaceId=Y)
      CEF spawn: ~80ms
      Window registered: ~150ms
      Frontend cold-load: +2–4s
      FloatingPaneWorkspace attaches: user sees pane — total ~2.5–4s
```

### 3.2 Design

Introduce a second pool keyed by label prefix `floating-pool-{uuid}`. Pool windows are created frameless (`?pane-pool=1` URL flag) and held offscreen at `(-32000, -32000)`. At promote time, inject `floatingPaneId` and `workspaceId` via a `pool:pane-promote` event and reposition to the drop target.

```
New label namespaces
  window-pool-{uuid}   ← existing tab pool (frameless=false, 1200×800)
  floating-pool-{uuid} ← new pane pool (frameless=true, PANE_POOL_WIDTH×PANE_POOL_HEIGHT)
```

Pool target: `PANE_POOL_TARGET_SIZE = 1` (panes are less frequently torn off than tabs; 1 is sufficient to cover the common burst).

### 3.3 Backend changes

#### 3.3.1 `window_pool.rs` — new constants and spawn function

```rust
pub const PANE_POOL_TARGET_SIZE: usize = 1;
const PANE_POOL_WIDTH: i32  = 900;
const PANE_POOL_HEIGHT: i32 = 600;

pub fn spawn_pane_pool_window(state: &Arc<AppState>) {
    // Same pattern as spawn_pool_window but:
    //   label prefix = "floating-pool-"
    //   url = ?pane-pool=1
    //   frameless = true
    //   size = PANE_POOL_WIDTH × PANE_POOL_HEIGHT
    // Use a separate semaphore (pane_pool_spawn_in_flight) to avoid
    // interfering with the tab pool's in-flight guard.
}
```

#### 3.3.2 State / reducer

Add a `pane_pool_queue: VecDeque<String>` and `pane_pool_unpromoted: HashSet<String>` to `HostState` (mirrors the existing `pool_queue` / `pool_unpromoted`).

New `HostCommand` variants:
- `PanePoolWindowSpawnStart { label }`
- `PanePoolWindowReady { label }`
- `PanePoolWindowDestroyedBeforePromote { label }`
- `PopAndPromoteFrontPanePoolWindow`

#### 3.3.3 `floating_pane.rs` — promote_pane_pool_window()

```rust
pub fn promote_pane_pool_window(
    state: &Arc<AppState>,
    pane_id: &str,
    workspace_id: &str,
    screen_x: i32,
    screen_y: i32,
    width: i32,
    height: i32,
) -> Option<String> {
    // 1. Reducer pop (PopAndPromoteFrontPanePoolWindow)
    // 2. Validate browser still in state
    // 3. Windows: SetWindowPos + ShowWindow
    //    macOS/Linux: PromotePanePoolWindowTask (set_bounds + show)
    // 4. emit pool:pane-promote { paneId, workspaceId } to the pool window
    // 5. spawn_pane_pool_window() [refill]
    // 6. Return Some(label)
}
```

#### 3.3.4 `ui_tasks.rs` — PromotePanePoolWindowTask (macOS/Linux)

Identical structure to `PromotePoolWindowTask` (Phase 7). Emits `pool:pane-promote` instead of `pool:promote`.

#### 3.3.5 `client/mod.rs` — init_pane_pool()

Call `spawn_pane_pool_window()` once after main window's `on_load_end` fires, guarded by `label == "main"`:

```rust
// After existing init_pool() call:
crate::commands::window_pool::spawn_pane_pool_window(state);
```

#### 3.3.6 `commands/floating_pane.rs` — open_floating_pane_window() — pool-first path

```rust
pub fn open_floating_pane_window(state, args) -> Result<...> {
    // ... existing validation ...

    // Try pane pool first
    if let Some(label) = promote_pane_pool_window(state, &pane_id, &workspace_id, x, y, w, h) {
        return Ok(json!({ "windowLabel": label }));
    }

    // Cold fallback (existing code)
    // ...
}
```

### 3.4 Frontend changes

#### 3.4.1 `app-init.ts` — pane-pool init path

Handle `?pane-pool=1` URL flag (analogous to `?pool=1`):
- Skip normal workspace init
- Install `pool:pane-promote` event listener
- Send `pane_pool_window_ready` IPC command when renderer is ready

#### 3.4.2 `pool:pane-promote` handler

When `pool:pane-promote { paneId, workspaceId }` fires:
- Attach the floating pane workspace (same as cold-path init, but without a round-trip window creation)
- Render `<FloatingPaneWorkspace paneId={paneId} workspaceId={workspaceId} />`

#### 3.4.3 New IPC command: `pane_pool_window_ready`

Analogous to `pool_window_ready`. Routes to `mark_pane_pool_window_renderer_ready()` in host.

### 3.5 Windows-specific notes

Windows uses Win32 for pane windows (`WS_POPUP + WS_EX_TOOLWINDOW`). The pane pool on Windows needs:
- HWND cache for pane pool windows (analogous to `init_pool_window_hwnd`)
- `WS_EX_TOOLWINDOW` style applied at creation (so they don't appear in Win+Tab)
- `ShowWindow(SW_SHOWNA)` at creation (show off-screen without activating)
- `SetWindowPos` + `ShowWindow(SW_SHOW)` at promote

This can be a follow-on PR; the macOS/Linux CEF Views path is lower-risk.

### 3.6 Files changed

| File | Change |
|------|--------|
| `agentmux-cef/src/commands/window_pool.rs` | Add pane pool constants, `spawn_pane_pool_window`, `promote_pane_pool_window`, `mark_pane_pool_window_renderer_ready`, `cleanup_failed_pane_promote_orphan` |
| `agentmux-cef/src/reducer.rs` | Add pane pool state fields + 4 new HostCommand variants |
| `agentmux-cef/src/commands/floating_pane.rs` | Add pool-first path in `open_floating_pane_window` |
| `agentmux-cef/src/ui_tasks.rs` | Add `PromotePanePoolWindowTask` + `post_promote_pane_pool_window` (non-Windows) |
| `agentmux-cef/src/client/mod.rs` | Add `init_pane_pool()` call after main window `on_load_end` |
| `agentmux-cef/src/ipc.rs` | Route `pane_pool_window_ready` command |
| `frontend/app-init.ts` | Handle `?pane-pool=1` flag + `pool:pane-promote` event |
| `frontend/util/cef-api.ts` | Add `paneTearOffPoolReady()` API call |

### 3.7 Expected latency improvement

| Platform | Before | After |
|----------|--------|-------|
| All | ~2.5–4s cold | ~30ms warm (renderer already painting) |

---

## 4. New Window Pool

### 4.1 Problem

`open_new_window` (menu item, keyboard shortcut, second-instance forward, Dock reopen) always spawns cold:

```
user: Cmd+N
  → IPC: open_new_window
    → open_window_with_kind(FullInstance, None)
      → get_offset_position() → 30px right+down from current window
      → post CreateWindowTask
        CEF spawn: ~150ms
        frontend cold-load: ~2–3s
        window interactive: ~2.5–3.5s total
```

### 4.2 Design

Reuse the **existing tab pool** (`window-pool-{uuid}` windows). The tab pool and the new-window pool are structurally identical — both are full-chrome top-level windows. The only difference is the workspace attachment step:

- Tab tear-off: `pool:promote { workspaceId }` (existing workspace moved in)
- New window: `pool:new-window {}` (fresh workspace created)

No new pool type needed. The tab pool target size (`POOL_TARGET_SIZE = 2`) can be raised to 3 if pops from both paths cause contention, but start at 2 and observe.

### 4.3 Backend changes

#### 4.3.1 `commands/window/creation.rs` — open_new_window() pool-first path

```rust
pub fn open_new_window(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    // Existing guards (pane-close H.7 invariant, quit-state check)
    if state.any_browser_pane_closing() { ... }

    // Try pool first
    let (pos_x, pos_y) = get_offset_position();
    let (win_w, win_h) = get_secondary_window_size(pos_x, pos_y);

    if let Some(label) = crate::commands::window_pool::promote_pool_window_for_new_window(
        state, pos_x, pos_y, Some(win_w), Some(win_h),
    ) {
        return Ok(json!(label));
    }

    // Cold fallback (existing code path)
    open_window_with_kind(state, WindowKind::FullInstance, None)
}
```

#### 4.3.2 `window_pool.rs` — promote_pool_window_for_new_window()

```rust
pub fn promote_pool_window_for_new_window(
    state: &Arc<AppState>,
    pos_x: i32,
    pos_y: i32,
    width: Option<i32>,
    height: Option<i32>,
) -> Option<String> {
    let dispatch = state.host_dispatch(HostCommand::PopAndPromoteFrontPoolWindow);
    let label = dispatch.promoted_pool_label?;

    // Validate liveness
    if state.get_browser(&label).is_none() {
        cleanup_failed_promote_orphan_cross_platform(state, &label);
        return None;
    }

    tracing::info!(target: "pool:new-window", label = %label, "[pool] promoting pool window for new-window");

    // Windows: SetWindowPos + ShowWindow(SW_SHOW)
    // macOS/Linux: PromoteNewWindowPoolWindowTask (set_bounds + show — same as tab promote task)
    #[cfg(target_os = "windows")]
    promote_pool_window_win32(state, &label, pos_x, pos_y, width, height);
    #[cfg(not(target_os = "windows"))]
    crate::ui_tasks::post_promote_pool_window_for_new_window(state, &label, pos_x, pos_y,
        width.unwrap_or(POOL_WIDTH), height.unwrap_or(POOL_HEIGHT));

    spawn_pool_window(state); // refill
    Some(label)
}
```

#### 4.3.3 `ui_tasks.rs` — PromoteNewWindowPoolWindowTask (macOS/Linux)

Same as `PromotePoolWindowTask` but emits `pool:new-window` instead of `pool:promote`:

```rust
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct PromoteNewWindowPoolWindowTask { ... }
    impl Task {
        fn execute(&self) {
            // set_bounds + show (same as PromotePoolWindowTask)
            window.set_bounds(Some(&cef::Rect { x, y, width, height }));
            window.show();
            // No workspaceId — frontend creates a fresh workspace
            emit_event_to_window(&self.state, &self.label, "pool:new-window", &json!({}));
        }
    }
}
```

#### 4.3.4 `commands/drag.rs` — update stale comments

Replace the pre-Phase 7 "pool not implemented" comment and `"pool_not_implemented"` error string on the `else` branch with accurate post-Phase 7 copy:

```rust
// On Windows: pool should always have slots — WARN on unexpected exhaustion.
// On macOS/Linux: pool is implemented (Phase 7); same WARN applies.
tracing::warn!(
    target: "dnd:tearoff:pool",
    workspace_id = %workspace_id,
    "[pool] pool exhausted on tear-off — frontend will cold-path"
);
Err("pool_exhausted".to_string())
```

### 4.4 Frontend changes

#### 4.4.1 `app-init.ts` — handle pool:new-window event

Pool windows already listen for `pool:promote`. Add a parallel listener for `pool:new-window`:

```typescript
// In the ?pool=1 init branch (alongside pool:promote listener):
cefApi.on("pool:new-window", async () => {
    // Create a fresh workspace (same as Cmd+N cold path)
    const wsId = await WorkspaceService.CreateWorkspace();
    await initHostNewWindow(wsId);
});
```

#### 4.4.2 `cef-api.ts` — openNewWindowPoolPromote()

Add an explicit API wrapper for completeness (mirrors `tearOffPoolPromote`). Not strictly required if the IPC route returns a label that the frontend can use, but explicit is better.

### 4.5 Files changed

| File | Change |
|------|--------|
| `agentmux-cef/src/commands/window/creation.rs` | Pool-first path in `open_new_window()` |
| `agentmux-cef/src/commands/window_pool.rs` | Add `promote_pool_window_for_new_window()` |
| `agentmux-cef/src/ui_tasks.rs` | Add `PromoteNewWindowPoolWindowTask` + `post_promote_pool_window_for_new_window` (non-Windows) |
| `agentmux-cef/src/commands/drag.rs` | Fix stale "pool not implemented" comment + error string |
| `frontend/app-init.ts` | Handle `pool:new-window` event |

### 4.6 Expected latency improvement

| Platform | Before | After |
|----------|--------|-------|
| All | ~2.5–3.5s cold | ~30ms warm |

---

## 5. Implementation Order

| Priority | Work | Effort | Impact |
|----------|------|--------|--------|
| P0 | Fix stale drag.rs comments (pool_not_implemented → pool_exhausted) | 5 min | Correctness / log hygiene |
| P1 | New window pool (§4) | ~1 day | High — Cmd+N is frequent |
| P2 | Pane tear-off pool (§3) | ~2–3 days | Medium — pane tear-off less common than tab; requires new pool type |

---

## 6. Shared Pool vs. Separate Pools

The pane pool MUST be separate because `frameless` is a creation-time CEF flag — a tab-pool window (with title bar) cannot become a pane window at promote time without a visible flash. Every other attribute (URL pattern, offscreen position, reducer queue shape) is the same.

The new-window pool CAN share the tab pool because new windows and tear-off targets are both full-chrome top-level windows — structurally identical, different only in which workspace gets attached.

---

## 7. Memory Cost

| Pool | Target size | Approx RSS per window | Total |
|------|-----------|-----------------------|-------|
| Tab pool (existing) | 2 | ~75 MB | ~150 MB |
| New window (shared with tab pool) | 2 (no change) | — | no change |
| Pane pool (new) | 1 | ~75 MB | ~75 MB |

New-window pooling costs nothing (shares slots). Pane pooling adds ~75 MB RSS. Both pools can be made configurable (`POOL_TARGET_SIZE`, `PANE_POOL_TARGET_SIZE`) if memory pressure is a concern.

---

## 8. References

- `agentmux-cef/src/commands/window_pool.rs` — pool implementation (tab)
- `agentmux-cef/src/commands/floating_pane.rs` — pane creation (cold path reference)
- `agentmux-cef/src/commands/window/creation.rs` — new-window creation (cold path reference)
- `agentmux-cef/src/ui_tasks.rs` — `PromotePoolWindowTask` (Phase 7, CEF Views pattern)
- `agentmux-cef/src/commands/drag.rs` — `tear_off_pool_promote` (tab pool promote entry point)
- `frontend/app/drag/tear-off-pool-helper.ts` — frontend pool-first / cold fallback pattern
- `frontend/app-init.ts` — `?pool=1` init branch (pattern for `?pane-pool=1`)
- `docs/specs/SPEC_POOL_PHASE7_MACOS_LINUX_2026_06_19.md` — Phase 7 (tab pool macOS/Linux) — SHIPPED #1595
- `docs/specs/linux-pool-startup-fill-2026-05-08.md` — startup fill analysis — RESOLVED by Phase 7
