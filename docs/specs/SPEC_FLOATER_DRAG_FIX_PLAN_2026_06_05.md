# Fix Plan — Floater Drag Remaining Issues

**Date:** 2026-06-05
**Branch:** agenty/floater-drag-native-loop
**Parent retro:** `docs/retro/retro-floater-drag-state-bugs-2026-06-05.md`

Covers the four remaining items from the deep audit not addressed by the previous commit.

---

## FIX-1 (P1) — `resolve_window_at_cursor` exclude_label race

**File:** `agentmux-cef/src/commands/window/motion.rs`

**Problem:**
```rust
let exclude_hwnd: Option<isize> = if exclude_label.is_empty() {
    None
} else {
    hwnds_by_label.get(exclude_label).copied()  // ← None if not in cache yet
};
```
`window_hwnds` is populated synchronously inside `CreateFloatingWindowTask::execute` (before the CEF browser is created). There is a brief window where the floater's HWND exists in the OS but its label is not yet in `window_hwnds`. If `resolve_window_at_cursor` fires during this window (e.g. a hover emit from a concurrent drag), `exclude_hwnd` is `None` — the floater is NOT excluded, appears topmost at the cursor, and gets returned as the target. `tryRedockAtCursor` receives the floater as its own redock target and the RPC fails.

**Fix:**
When `exclude_label` is non-empty but `hwnds_by_label` has no entry, fall back to `resolve_window_hwnd(state, exclude_label)` — the same label-aware HWND lookup used by `start_window_drag`. This handles the cache-miss window.

```rust
let exclude_hwnd: Option<isize> = if exclude_label.is_empty() {
    None
} else {
    hwnds_by_label
        .get(exclude_label)
        .copied()
        .or_else(|| {
            // Cache miss — floater HWND registered in OS but not yet
            // inserted into window_hwnds (brief window after HWND creation
            // but before label→HWND map update). Use label-aware HWND
            // lookup as fallback so the floater is always excluded.
            let h = unsafe { resolve_window_hwnd(state, exclude_label) };
            if h.is_null() { None } else { Some(h as isize) }
        })
};
```

---

## FIX-2 (P0) — Backend layout writes outside saga boundary

**File:** `agentmux-srv/src/server/service.rs`

**Problem:**
After the `RedockFloatingPane` saga completes (block moved in SQLite), `queue_target_layout_insert` and `queue_source_layout_delete` run with warn-only error handling. If either fails, the code still builds the full `updates` payload and broadcasts it. The broadcast includes the source `Tab` with `blockids=[]` — which triggers the floater's auto-close watcher — but the target `LayoutState` has no `InsertNode` action enqueued, so the block appears nowhere. The user sees the block disappear.

**Fix:**
Treat both layout writes as required preconditions for broadcasting. If either fails, return an error **before** building `updates` and **before** broadcasting. The block is already moved in SQLite/memory (saga completed), so the state is dirty but invisible — the user sees no change and can retry. This is strictly better than the current "block vanishes" outcome.

```rust
// Queue target layout insert — required before broadcast.
if let Err(e) = queue_target_layout_insert(store, &target_tab_id, &block_id) {
    tracing::error!(...);
    return WebReturnType::error(format!("layout insert failed: {e}"));
}
// Queue source layout delete — required before broadcast.
if let Err(e) = queue_source_layout_delete(store, &source_tab_id, &block_id) {
    tracing::error!(...);
    return WebReturnType::error(format!("layout delete failed: {e}"));
}
// Both layout writes succeeded → safe to build updates and broadcast.
```

Note: leaves the SQLite block ownership dirty on error. Full compensation (reverse MoveBlock) is tracked as gap F1.A in sagas/mod.rs — out of scope for this fix.

---

## FIX-3 (P0) — I5: `AgentMuxFloatingPane` window class without `dir_hash`

**Files:** `agentmux-cef/src/floating_pane.rs`, `agentmux-cef/src/commands/window/lifecycle.rs`

**Problem:**
`CLASS_NAME = "AgentMuxFloatingPane"` (line 481, `floating_pane.rs`) is a global fixed string. Two parallel AgentMux instances both call `RegisterClassExW` with the same name. The second call silently succeeds (Windows returns the existing atom) but the class's `wndproc`/`hInstance` point to the first process. Floater windows in the second instance get the wrong WndProc dispatched (`floating_pane_wndproc` is correct only within the owning process). Also, `find_main_window` in `lifecycle.rs` skips windows whose class matches `FLOATING_PANE_CLASS_NAME` — with a shared global name it may skip the alien instance's floaters, leading to wrong main-window resolution.

**Fix:**
Read `AGENTMUX_IPC_HASH` env var (set by the launcher to `hash(data_dir, version)`) at startup and suffix the class name. Fall back to the bare name in dev builds where the launcher may not have set it.

```rust
// floating_pane.rs
fn floater_class_name() -> String {
    match std::env::var("AGENTMUX_IPC_HASH") {
        Ok(h) => format!("AgentMuxFloatingPane-{}", h),
        Err(_) => "AgentMuxFloatingPane".to_string(),
    }
}
```

`FLOATING_PANE_CLASS_NAME` constant in `lifecycle.rs` becomes a function `floater_class_name()` (lazy-static or called at use site) so `find_main_window`'s filter always matches the runtime-suffixed name.

---

## FIX-4 (P1) — macOS/Linux: `window_drag_ended` carries cursor position for redock

**Files:** `agentmux-cef/src/ui_tasks.rs`, `frontend/app/workspace/floating-pane-workspace.tsx`

**Problem:**
`StartWindowDragTask::execute` emits `window_drag_ended { moved: false }`. The renderer's `window_drag_ended` handler resets `dragging = false` (safety net) but since `moved=false`, `onMouseUp` — even if it fires — can't pass `hasMoved` gate. Redock on macOS/Linux is still non-functional after the drag completes.

If `BeginWindowDrag` delivers a DOM `mouseup` to the renderer (F3 hope), then `onMouseUp` fires, `hasMoved` is set by the prior `onMouseMove` calls, and `tryRedockAtCursor(e.screenX, e.screenY)` runs correctly. **No host change needed for that path.**

If `BeginWindowDrag` does NOT deliver a DOM `mouseup` (F3 failure), we need host-side cursor coords. After `f(raw_ptr)` returns (the OS drag is complete), the cursor is at the release position. We can get this via a CEF API or a platform `get_cursor_point` IPC.

**Fix (two-pronged):**

**(a) Emit `moved: true` when BeginWindowDrag fired** (not `false`) — the drag ran, so `hasMoved` should be considered true for the renderer's `onMouseMove`-driven path.

**(b) Belt-and-suspenders in renderer:** If `window_drag_ended { moved: true }` arrives AND `dragging` is still `true` (meaning `onMouseUp` never fired — BeginWindowDrag absorbed mouseup), call `tryRedockAtCursor` using a `get_cursor_point` IPC to obtain the current cursor position.

```ts
// window_drag_ended handler — expanded
stopEndedListener = safeListenEvent<{ label: string; moved: boolean }>(
    "window_drag_ended",
    async (ev) => {
        if (!ev.label || ev.label !== label) return;
        const wasDragging = dragging;
        dragging = false;
        // If onMouseUp already fired (Win32 path), dragging was false → skip.
        // If dragging is still true here (BeginWindowDrag absorbed mouseup),
        // we need to fire tryRedockAtCursor ourselves.
        if (ev.moved && wasDragging && hasMoved) {
            try {
                const pt = await invokeCommand<{ x: number; y: number }>("get_cursor_point", {});
                invokeCommand("clear_floating_redock_hover", {}).catch(() => {});
                void tryRedockAtCursor(pt.x / (isWindows() ? (window.devicePixelRatio || 1) : 1),
                                       pt.y / (isWindows() ? (window.devicePixelRatio || 1) : 1));
            } catch { /* host unavailable */ }
        }
    },
);
```

**(c) Host change:** In `StartWindowDragTask::execute`, change `moved: false` to `moved: true` after confirming `begin_window_drag` ran.

**Scope note:** This fix makes redock work on macOS/Linux if the `get_cursor_point` IPC exists (verify against `motion.rs` or `commands`). If the IPC doesn't exist, macOS/Linux redock remains F3-pending but dragging state is still correctly reset.

---

## Implementation order

| # | Fix | Risk | Time |
|---|-----|------|------|
| FIX-1 | exclude_label fallback | Low — additive | 15 min |
| FIX-2 | layout write atomicity | Low — error-path only | 20 min |
| FIX-3 | window class dir_hash | Medium — class name change affects 2 files | 30 min |
| FIX-4 | macOS/Linux cursor coords | Medium — new IPC interaction | 30 min |
