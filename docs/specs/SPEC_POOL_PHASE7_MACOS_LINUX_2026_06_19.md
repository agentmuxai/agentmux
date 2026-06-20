# Phase 7 — Pre-warmed Window Pool for macOS and Linux

**Date:** 2026-06-19  
**Status:** SHIPPED — merged as #1595 on 2026-06-20  
**Author:** agent investigation + consolidation  
**Supersedes / consolidates:**
- `docs/specs/SPEC_TEAR_OFF_POOL_PATH_2026_05_06.md` — pool routing for CrossWindowDragMonitor (pane branch now done via #1182/linux PR; tab branch pool-attempts already in `tear-off-pool-helper.ts`)
- `docs/specs/linux-pool-startup-fill-2026-05-08.md` — UNBLOCKED (both blockers resolved; see §2)

---

## 1. Problem

Tab tear-off on macOS and Linux always takes the cold path:

```
[pool] pool exhausted on tear-off — frontend will cold-path
[create-window] task entered UI thread        ← 0ms
browser_view_create returned                  ← +80ms
Window registered                             ← +150ms
[initTauriNewWindow] tear-off                 ← +2.4s (frontend cold-load)
DragOverlay listening on the new window       ← user-visible "done", +3.7s
```

On Windows the same operation is ~150ms (pool promote + frontend paint) because a pre-warmed pool of 2 CEF windows is maintained at startup, and tear-off promotes one rather than spawning from scratch.

The pool machinery (spawn, register, promote, refill) is fully implemented for Windows. The only missing pieces for macOS/Linux are:

1. **`init_pool()` returns immediately** on non-Windows (line: `window_pool.rs`, `#[cfg(not(target_os = "windows"))] { return; }`). Pool windows are never seeded.
2. **`promote_pool_window()` returns `None`** on non-Windows (the stub at `window_pool.rs:913-927`). Even if pool windows were spawned, they could never be consumed.

---

## 2. Prior Blockers — Now Resolved

`linux-pool-startup-fill-2026-05-08.md` identified two blockers (status 2026-05-10):

| Blocker | Original status | Current status |
|---|---|---|
| `promote_pool_window` Windows-only stub | BLOCKS | **This spec implements it** |
| Wayland `-32000,-32000` hack makes pool windows appear on-screen | BLOCKS | **Resolved** — agentmux forces CEF Ozone-X11 on Linux (`--use-gl=desktop`); Wayland sessions get XWayland and inherit X11 virtual-screen semantics. `-32000,-32000` is invisible there. Native Wayland tear-off is a separate deferred effort; not in scope here. |

---

## 3. Current State of Related Work

| Feature | Status | Notes |
|---|---|---|
| Pool spawn (`spawn_pool_window`) | ✅ Cross-platform | No platform gates inside the function; uses `post_create_window` which is cross-platform |
| Pool register (`register_pool_window`) | ✅ Cross-platform for non-HWND parts; Win32 taskbar-hide is `#[cfg(windows)]` skipped on others | Fine — taskbar hiding not needed on macOS/Linux |
| Pool init on `on_after_created` for "main" | ✅ Call site exists | `client/mod.rs:664` calls `init_pool(state)` — just needs early-return removed |
| Pool promote (Windows) | ✅ Full Win32 path | HWND cache + `SetWindowPos` + taskbar show + cascade hook + refill |
| Pool promote (non-Windows) | ❌ Stub returns `None` | **This spec implements it** |
| Pool window offscreen hiding (macOS) | ✅ CEF Views accepts any coordinates | `(-32000,-32000)` in DIP works on macOS Cocoa/CEF Views — window off all monitors, invisible |
| Pool window offscreen hiding (Linux/X11) | ✅ X11 virtual screen accepts any coords | Already documented as working |
| Pool window offscreen hiding (Linux/Wayland native) | ⚠️ Not applicable | App runs X11/XWayland on Linux; deferred with native Wayland support |
| Tab tear-off frontend pool attempt | ✅ All platforms | `tear-off-pool-helper.ts::openTearOffWindow` tries `tearOffPoolPromote` first, falls back to cold path |
| Pane tear-off (macOS) | ✅ Phase A done — PR #1182 | Routes to `open_floating_pane_window`, gets `?floatingPaneId=` URL, renders chromeless |
| Pane tear-off (Linux) | ✅ Phase A done | Same as macOS — `CrossWindowDragMonitor.linux.tsx` routes pane → `open_floating_pane_window` |
| Owned-window lifecycle, header drag, redock (macOS/Linux) | ⏳ Phase B | Tracked in existing cross-platform floating pane specs; out of scope here |
| `set_window_position` / `get_window_position` (macOS/Linux) | ✅ Implemented | `ui_tasks.rs` has `SetWindowPositionTask` / `GetWindowPositionTask` using CEF Views `set_bounds()` / `bounds()` — cross-platform |

---

## 4. Architecture

### How the pool works (Windows reference)

```
Startup
  └─ on_after_created("main") → init_pool() → spawn_pool_window()
       └─ post_create_window(label="window-pool-{uuid}", x=-32000, y=-32000, url=?pool=1)
            └─ on_after_created("window-pool-*") → register_pool_window()
                 └─ frontend loads, installs pool:promote listener, sends "pool_window_ready" IPC
                      └─ mark_pool_window_renderer_ready() → move unpromoted→queue
                           └─ if below target → spawn_pool_window() [refill chain]

Tear-off (tab)
  frontend: openTearOffWindow() → api.tearOffPoolPromote(wsId, x, y, w, h)
  host: promote_pool_window() → pop queue → validate HWND → SetWindowPos() → emit pool:promote → spawn_pool_window()
  frontend: awaitPoolPromote() receives pool:promote → attach workspace → paint (near-instant)
```

### What changes for macOS/Linux (Phase 7)

```
Startup (same IPC chain; only init_pool() gate removed)
  └─ on_after_created("main") → init_pool() [no longer early-returns] → spawn_pool_window()
       └─ post_create_window(x=-32000, y=-32000)  ← same, already cross-platform
            └─ register_pool_window() [Win32 parts skip; non-Windows path continues]
                 └─ mark_pool_window_renderer_ready() → queue insertion + refill chain
                      [same as Windows from here]

Tear-off (tab) — macOS/Linux
  frontend: openTearOffWindow() → api.tearOffPoolPromote(wsId, x, y, w, h)
  host: promote_pool_window() [non-Windows impl]:
    1. Reducer pop (cross-platform: HostCommand::PopAndPromoteFrontPoolWindow)
    2. Validate browser in state.browsers (no HWND; CEF presence is the liveness check)
    3. Post PromotePoolWindowTask to CEF UI thread
         └─ get_window_on_ui(state, label) → window.set_bounds(x, y, w, h)
         └─ emit_event_to_window(state, label, "pool:promote", workspace_id)
    4. spawn_pool_window() [refill]
    5. Return Some(label)
  frontend: awaitPoolPromote() receives pool:promote → attach workspace → paint
```

### No changes needed to

- `tear-off-pool-helper.ts` — already tries pool on all platforms  
- `CrossWindowDragMonitor.darwin.tsx` / `.linux.tsx` — tab branch already calls `openTearOffWindow` → pool-aware; pane branch already routes to `open_floating_pane_window`  
- Frontend pool-promote handshake (`pool:promote` listener, `awaitPoolPromote`) — platform-agnostic  
- Pool URL construction (`?pool=1` flag) — already cross-platform in `spawn_pool_window`

---

## 5. Implementation

### 5.1 `agentmux-cef/src/commands/window_pool.rs` — `init_pool()`

Remove the `#[cfg(not(target_os = "windows"))]` early-return block. The function body is already cross-platform (`spawn_pool_window` has no Win32 deps). After removal, `init_pool()` becomes:

```rust
pub fn init_pool(state: &Arc<AppState>) {
    let current = state.pool_queue_size();
    if current >= POOL_TARGET_SIZE {
        return;
    }
    spawn_pool_window(state);
    // Refill recursion: each spawned window calls mark_pool_window_renderer_ready
    // on first-paint, which calls spawn_pool_window again until target is reached.
}
```

The `#[cfg(target_os = "windows")]` inner block that held the identical logic becomes the whole function body (without the cfg guard).

### 5.2 `agentmux-cef/src/commands/window_pool.rs` — `promote_pool_window()` (non-Windows)

Replace the non-Windows stub (lines 913-927) with a real implementation:

```rust
#[cfg(not(target_os = "windows"))]
pub fn promote_pool_window(
    state: &Arc<AppState>,
    workspace_id: &str,
    screen_x: i32,
    screen_y: i32,
    width: Option<i32>,
    height: Option<i32>,
    tab_anchor_x: Option<i32>,
    tab_anchor_y: Option<i32>,
) -> Option<String> {
    // Pop atomically from pool queue via reducer. Returns None if empty
    // (caller falls back to cold path).
    let dispatch = state.host_dispatch(
        crate::reducer::HostCommand::PopAndPromoteFrontPoolWindow,
    );
    let label = dispatch.promoted_pool_label?;

    tracing::info!(
        target: "dnd:tearoff:pool",
        label = %label,
        workspace_id = %workspace_id,
        screen_x,
        screen_y,
        "[pool] promoting pool window (non-Windows)"
    );

    // Validate browser is still alive in state. Unlike Windows, we don't
    // need HWND lookup — CEF Views presence in state.browsers is the
    // liveness indicator. If the browser is gone, run orphan cleanup.
    if state.get_browser(&label).is_none() {
        tracing::warn!(
            target: "dnd:tearoff:pool",
            label = %label,
            "[pool] promoted browser not found in state — running orphan cleanup"
        );
        cleanup_failed_promote_orphan(state, &label);
        return None;
    }

    // Compute window position. If the frontend provided a tab anchor
    // (outer top-left so cursor lands on the dragged tab's visual position),
    // use it verbatim. Otherwise default to (screen_x, screen_y).
    let x = tab_anchor_x.unwrap_or(screen_x);
    let y = tab_anchor_y.unwrap_or(screen_y);
    let w = width.unwrap_or(crate::commands::window_pool::POOL_WIDTH);
    let h = height.unwrap_or(crate::commands::window_pool::POOL_HEIGHT);

    // Reposition and show the pool window on the CEF UI thread.
    // CEF Views set_bounds() accepts DIP coordinates — frontend-supplied
    // x/y/width/height are already in CSS/DIP pixels, no scaling needed
    // (unlike Windows which works in physical pixels and needs DPI conversion).
    crate::ui_tasks::post_promote_pool_window(
        state,
        &label,
        workspace_id,
        x,
        y,
        w,
        h,
    );

    // Refill the pool asynchronously.
    spawn_pool_window(state);

    Some(label)
}
```

### 5.3 `agentmux-cef/src/ui_tasks.rs` — `post_promote_pool_window()` + `PromotePoolWindowTask`

Add a new task modeled after `SetWindowPositionTask`. This runs on the CEF UI thread so it can safely call `window.set_bounds()` and `emit_event_to_window()`.

```rust
// ── Pool window promote (macOS / Linux) ───────────────────────────────────
//
// Moves the pre-warmed pool window from its offscreen holding position to
// the tear-off destination and emits pool:promote so the renderer attaches
// the new workspace. Used on non-Windows platforms where Win32 SetWindowPos
// is unavailable; CEF Views Window::set_bounds() is the cross-platform
// equivalent and runs correctly on the UI thread on all platforms.
//
// Windows uses its own promote path (promote_pool_window cfg(windows)) with
// HWND caching + SetWindowPos + taskbar show + floater cascade hook. This
// task is intentionally non-Windows only — keep them in sync if the logic
// diverges.
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct PromotePoolWindowTask {
        state: Arc<AppState>,
        label: String,
        workspace_id: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }

    impl Task {
        fn execute(&self) {
            let Some(window) = get_window_on_ui(&self.state, &self.label) else {
                tracing::warn!(
                    target: "dnd:tearoff:pool",
                    label = %self.label,
                    "[pool:promote] window not found on UI thread — pool window may have closed"
                );
                return;
            };

            // Move from offscreen to tear-off destination.
            // CEF Views Window::set_bounds() is in DIP coordinates on all
            // platforms. macOS Cocoa and X11 both support off-screen → on-screen
            // repositioning without an intermediate show step.
            window.set_bounds(Some(&cef::Rect {
                x: self.x,
                y: self.y,
                width: self.width,
                height: self.height,
            }));

            tracing::info!(
                target: "dnd:tearoff:pool",
                label = %self.label,
                x = self.x,
                y = self.y,
                width = self.width,
                height = self.height,
                "[pool:promote] window repositioned via set_bounds"
            );

            // Signal the renderer to attach the workspace. The frontend's
            // awaitPoolPromote() listener was installed at pool-spawn time
            // (mark_pool_window_renderer_ready gates on this); it receives
            // the event and calls initHostNewWindow with the workspace ID.
            crate::events::emit_event_to_window(
                &self.state,
                &self.label,
                "pool:promote",
                Some(serde_json::json!({ "workspaceId": self.workspace_id })),
            );

            tracing::info!(
                target: "dnd:tearoff:pool",
                label = %self.label,
                workspace_id = %self.workspace_id,
                "[pool:promote] pool:promote event emitted — renderer will attach workspace"
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn post_promote_pool_window(
    state: &Arc<AppState>,
    label: &str,
    workspace_id: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let mut task = PromotePoolWindowTask::new(
        state.clone(),
        label.to_string(),
        workspace_id.to_string(),
        x,
        y,
        width,
        height,
    );
    post_task(ThreadId::UI, Some(&mut task));
}
```

---

## 6. Platform Notes

### macOS

- CEF Views `Window::set_bounds()` in DIP: works on macOS — this is the same API used by `SetWindowPositionTask` which is already tested on macOS.
- Pool window offscreen hiding: `(-32000, -32000)` in DIP works. Cocoa accepts any `NSRect` origin including large negatives; the window is off all monitors and invisible.
- No taskbar/Dock hiding needed: macOS Dock shows apps, not windows. Pool windows are just another window belonging to the AgentMux process; they don't create a separate Dock entry.
- No cascade hook on promote: floater-follows-parent lifecycle is Phase B (NSPanel `addChildWindow:ordered:`); not in scope here.
- DPI: CEF Views coordinates are DIP; no cross-monitor DPI conversion needed (Retina 2x is handled internally by Cocoa/CEF).

### Linux (X11 / XWayland)

- CEF Views `Window::set_bounds()` in DIP: works on X11 — same as macOS.
- Pool window offscreen hiding: `(-32000, -32000)` in DIP works on X11 (large negative coords are in the X11 virtual screen but off all configured monitor regions).
- XWayland: app runs X11 backend; Wayland sessions get XWayland and inherit X11 semantics. Off-screen positioning works as on native X11.
- No taskbar hiding needed for Phase A. Phase B adds `_NET_WM_STATE_SKIP_TASKBAR` for floater windows.
- Native Wayland backend: deferred. When/if the app gains a native Wayland backend, pool window hiding will need `xdg_toplevel.set_minimized()` or similar instead of off-screen positioning.

---

## 7. What This Does NOT Do (Deferred)

| Feature | Why deferred |
|---|---|
| Owned-window lifecycle (NSPanel `addChildWindow`, GTK `transient-for`) | Phase B; tracked in floating-pane cross-platform spec |
| JS-driven header drag on macOS/Linux floaters | Phase B; needs `get/set_window_position`, `get_cursor_point` (partially done for macOS) |
| Redock (drop floater onto parent to merge) | Phase B/C; needs `resolve_window_at_cursor` on macOS/Linux |
| Native Wayland pool hiding | Blocked on CEF Ozone-Wayland support; deferred |
| Cross-monitor DPI handoff (macOS Retina, Linux mixed-DPI) | CEF handles DIP internally; explicit DPI math is a Phase B follow-up |
| Pool-size configuration | Fixed at `POOL_TARGET_SIZE = 2`; configurable pool is a separate feature request |

---

## 8. Files Changed

| File | Change |
|---|---|
| `agentmux-cef/src/commands/window_pool.rs` | Remove `init_pool()` non-Windows early-return; replace non-Windows `promote_pool_window()` stub with real implementation |
| `agentmux-cef/src/ui_tasks.rs` | Add `PromotePoolWindowTask` + `post_promote_pool_window()` (non-Windows) |

No frontend changes required. No new IPC commands. No changes to the launcher or srv.

---

## 9. Latency Improvement (Expected)

| Platform | Before | After |
|---|---|---|
| macOS | ~200-400ms cold path (CEF render process spawn) | ~150ms (frontend paint; CEF already pre-warmed) |
| Linux | ~200-400ms cold path | ~150ms (frontend paint; CEF already pre-warmed) |
| Windows | ~150ms (unchanged; pool already working) | unchanged |

First tear-off after a cold launch goes from 3-4 seconds to ~150ms on macOS/Linux.

---

## 10. Test Plan

- [ ] macOS: fresh launch, wait ~2s for first-paint. `muxlog host '[pool]'` should show `[pool] spawning pool window` within ~1s of `on_load_end`.
- [ ] macOS: `muxlog host` should show `BrowserRegistered … is_pool: true` for 2 pool windows.
- [ ] macOS: tear off a tab. Expect `[pool:promote]` in log, NOT `pool exhausted on tear-off`. New window appears near-instantly.
- [ ] macOS: tear off a pane. Chromeless floating window appears (unchanged — pane path doesn't use pool, routes via `open_floating_pane_window`).
- [ ] macOS: tear off twice in quick succession. Second tear-off may cold-path (pool refilling) — verify it doesn't crash.
- [ ] Linux: same smoke steps as macOS above.
- [ ] Windows: no regression — both `init_pool` and `promote_pool_window` cfg(windows) paths are unchanged.
- [ ] Quit during pool warmup: verify pool windows drain cleanly (`[wrr] quit_state=Draining` in log; no hang).

---

## 11. References

- `agentmux-cef/src/commands/window_pool.rs` — pool implementation (spawn, register, promote, refill)
- `agentmux-cef/src/ui_tasks.rs` — CEF UI-thread task wrappers; `SetWindowPositionTask` is the reference pattern
- `agentmux-cef/src/client/mod.rs:664` — `init_pool()` call site (on_after_created "main")
- `frontend/app/drag/tear-off-pool-helper.ts` — `openTearOffWindow` with pool-first / cold-path fallback
- `docs/specs/SPEC_TEAR_OFF_POOL_PATH_2026_05_06.md` — original pool routing spec (pane branch now done; tab branch works once host promote is implemented)
- `docs/specs/linux-pool-startup-fill-2026-05-08.md` — startup fill analysis; both blockers now resolved
- `docs/specs/SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md` — macOS pane tear-off (Phase A done; Phase B deferred)
- `docs/specs/SPEC_LINUX_FLOATING_PANE_TEAROFF_2026_05_30.md` — Linux pane tear-off (Phase A done)
