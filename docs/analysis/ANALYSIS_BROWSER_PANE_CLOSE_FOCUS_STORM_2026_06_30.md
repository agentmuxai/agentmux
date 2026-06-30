# Analysis: Browser pane close → stale WndProc subclass → infinite focus storm

**Date:** 2026-06-30
**Against:** local-main-b28b7a @ v0.49.8
**Log:** `~/.agentmux/channels/local-main-b28b7a-50e8c174/versions/0.49.8/logs/agentmux-host-v0.49.8.log.2026-06-30`

**Symptom (reported):** After opening a browser pane and then closing it, switching to an external browser and returning to the AgentMux window causes typing to stop everywhere. Recovery requires opening a new AgentMux window, selecting it, typing in it, then returning to the working window.

---

## 0. Executive summary

Two bugs combine to produce an infinite focus storm once any browser pane has been opened and closed in the session:

| # | Defect | Symptom | Layer |
|---|--------|---------|-------|
| **A** | **Stale WndProc subclass on closed pane HWNDs** — when a pane closes, `on_before_close_browser_pane` removes its entry from `BROWSER_PANE_HWND_CONTEXT` and `BROWSER_PANE_WNDPROCS` for the outer HWND only. Child HWNDs (`Chrome_RenderWidgetHostHWND`, `Chrome_WidgetWin_1`) subclassed during `on_load_end_browser_pane` are never unsubclassed. Their focus-redirect WndProc survives the HWND value being recycled by Windows, or the HWND remaining live during deferred CEF teardown. | Redirect hook fires on what should be the main render widget | Host — `browser_pane/hwnd.rs`, `browser_pane/callbacks.rs` |
| **B** | **`find_main_render_widget` filter blind to unregistered panes** — `MainFocusReclaimTask` builds `pane_outer_hwnds` from `state.list_browsers()` filtered to `"browser-pane-*"` labels. After `BrowserUnregistered`, no label matches, so `pane_outer_hwnds` is empty. The ancestor-chain filter in `find_main_render_widget` sees no panes to exclude, and returns the first `Chrome_RenderWidgetHostHWND` found — which may be a still-live child of the closed pane. | `panes_excluded=0` → wrong HWND targeted | Host — `ui_tasks.rs` |

---

## 1. Observed timeline (from host log, 2026-06-30)

```
01:50:10  Startup. Browser pane at outer HWND 0x40926 created, subclassed.
          LAST_FOCUSED_BY_ROOT[0x10914] not yet set.

01:50:23  MainFocusReclaimTask: target=0x10916 (correct main render widget)
          panes_excluded=0 ← already 0 at startup; at this point 0x40926 is the
          pane outer HWND but the filter checks host.window_handle() which may
          return the outer HWND (not tracked via the render-widget enumeration).

17:50:37  Two browser panes opened:
          - browser-pane-d290b541 → https://web.whatsapp.com/
          - browser-pane-986761d2 → https://agentmux.ai/

17:50:53  Chrome_RenderWidgetHostHWND 0x2d709a0 subclassed
          (child of one of the above panes, registered via on_load_end_browser_pane)

17:53:33  BrowserUnregistered label=browser-pane-d290b541-...-1
17:54:26  BrowserUnregistered label=browser-pane-986761d2-...-2
          Both panes removed from state.browsers.
          HWND 0x2d709a0 subclass NOT removed — wndproc_hook still installed.

~17:54 – 22:22  (gap) HWND 0x2d709a0 remains live with redirect WndProc installed.

22:22:49  User returns to AgentMux from external browser.
          WM_ACTIVATE on 0x10914 → focus-restore → SetFocus(LAST_FOCUSED_BY_ROOT)
          → storm begins.
```

---

## 2. Storm mechanics

Every storm iteration follows this exact pattern (confirmed in log at 22:22:49 onwards):

```
[ipc] main_window_focus window_label=main
  ↓
MainFocusReclaimTask::execute (ui_tasks.rs:2080)
  host.set_focus(1) on label=main         ← Chromium: main browser focused
  [pane-wndproc] WM_KILLFOCUS hwnd=0x2d709a0   ← pane render widget loses Chromium focus
  find_main_render_widget(top, panes=[])
    → EnumChildWindows scans Views subtree
    → finds 0x2d709a0 (Chrome_RenderWidgetHostHWND, panes=[] so no filter applies)
    → returns Some(0x2d709a0)
  record_intentional_focus(0x2d709a0)
    → LAST_FOCUSED_BY_ROOT[0x10914] = 0x2d709a0   ← cements wrong target
  Win32 SetFocus(0x2d709a0)
    → wndproc_hook WM_SETFOCUS fires
    → ALLOW_BROWSER_PANE_FOCUS_ONCE=false → redirect
    → SetFocus(GetAncestor(0x2d709a0, GA_ROOT)) = SetFocus(0x10914)
    → Chromium on_got_focus → frontend emits main_window_focus IPC
  defocus_all (no live panes → no-op)
  ↑ loop
```

Log evidence:
```
[ipc] main_window_focus window_label=main
[main-focus-reclaim] host.set_focus(1) on label=main
[pane-wndproc] WM_KILLFOCUS hwnd=0x2d709a0
[focus-track] LAST_FOCUSED_BY_ROOT[root=0x10914] <= child=0x2d709a0
[main-focus-reclaim] Win32 SetFocus target=0x2d709a0 render_found=true panes_excluded=0
```
This repeats continuously from 22:22:49 to end of log (22:35:36+, log still running).

---

## 3. Defect A — stale WndProc on closed pane child HWNDs

### What closes cleanly
`on_before_close_browser_pane` (`browser_pane/callbacks.rs:122`) calls:
- `state.browser_panes.drain_closed_label(state, label)` — reducer cleanup
- `remove_contexts_for_block(block_id)` (`hwnd.rs:49`) — removes from `BROWSER_PANE_HWND_CONTEXT`

`remove_contexts_for_block` operates on `BROWSER_PANE_HWND_CONTEXT` (keyed by outer pane HWND), not `BROWSER_PANE_WNDPROCS`.

### What is NOT cleaned up
`BROWSER_PANE_WNDPROCS` (`hwnd.rs:24`) maps every subclassed HWND → original WndProc. It is written during:
- `install_browser_pane_focus_redirect` at `on_after_created_browser_pane` (outer HWND)
- `on_load_end_browser_pane` → `install_browser_pane_focus_redirect` again (picks up any new `Chrome_RenderWidgetHostHWND` children)
- `enum_children` inside `install_browser_pane_focus_redirect` (all child HWNDs at install time)

None of these are reversed on close. The `SetWindowLongPtrW` that installed the hook is never undone with a matching `SetWindowLongPtrW(hwnd, GWLP_WNDPROC, original)`.

### Why the HWND lives past `BrowserUnregistered`
CEF browser teardown is asynchronous. `BrowserUnregistered` fires when the browser object is removed from `state.browsers`, but the underlying HWND tree (owned by the CEF browser process) can remain live until the renderer process exits. For a pre-warmed pool browser the HWND may survive for the full session.

Additionally, Windows recycles HWND values. If the old HWND IS destroyed, a new `Chrome_RenderWidgetHostHWND` for any browser (including the main window's renderer) may receive the same value `0x2d709a0` from the allocator. Because `BROWSER_PANE_WNDPROCS` keyed on `usize` (the raw HWND value), it would register the new HWND as still-subclassed when `already_hooked` is checked — and `wndproc_hook` would fire on the new HWND's messages.

### Fix (Defect A)
In `on_before_close_browser_pane`, or in `destroy_hwnd` in `browser_panes.rs`, restore the original WndProc for every HWND in the closing block before the HWND tree is destroyed:

```rust
// Proposed addition to on_before_close_browser_pane (Windows-only):
#[cfg(target_os = "windows")]
{
    if let Some(rest) = label.strip_prefix("browser-pane-") {
        if let Some(dash) = rest.rfind('-') {
            let block_id = &rest[..dash];
            crate::browser_pane::hwnd::uninstall_focus_redirect_for_block(block_id);
        }
    }
}
```

New function `uninstall_focus_redirect_for_block`:
```rust
pub fn uninstall_focus_redirect_for_block(block_id: &str) {
    // Find outer HWNDs for this block, then walk BROWSER_PANE_WNDPROCS
    // restoring every original WndProc and removing the entry.
    // Need to cross-reference BROWSER_PANE_HWND_CONTEXT (block_id → outer HWND)
    // with BROWSER_PANE_WNDPROCS (outer HWND + children → original proc).
    //
    // Simplest approach: collect outer HWND from BROWSER_PANE_HWND_CONTEXT,
    // then enumerate its children to find all subclassed descendants.
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowLongPtrW, EnumChildWindows, GWLP_WNDPROC,
    };
    let outer: Option<usize> = BROWSER_PANE_HWND_CONTEXT.lock().ok().and_then(|m| {
        m.iter()
            .find(|(_, ctx)| ctx.block_id == block_id)
            .map(|(&hwnd, _)| hwnd)
    });
    let Some(outer) = outer else { return };
    let hwnds_to_restore: Vec<(usize, isize)> = {
        let mut v = vec![];
        if let Ok(map) = BROWSER_PANE_WNDPROCS.lock() {
            // outer itself
            if let Some(&orig) = map.get(&outer) { v.push((outer, orig)); }
            // children: enumerate and check map
            // (EnumChildWindows would need unsafe; collect keys from map that
            // are descendants — simpler: restore all entries whose HWND is a
            // descendant of outer)
        }
        v
    };
    unsafe {
        for (hwnd, orig) in hwnds_to_restore {
            let proc_fn: unsafe extern "system" fn(*mut std::ffi::c_void, u32, usize, isize) -> isize =
                std::mem::transmute(orig);
            SetWindowLongPtrW(hwnd as _, GWLP_WNDPROC, orig);
        }
    }
    // Clean up maps
    if let Ok(mut map) = BROWSER_PANE_WNDPROCS.lock() {
        map.retain(|hwnd, _| /* not in outer's subtree */ true); // fill in real check
    }
    // BROWSER_PANE_HWND_CONTEXT is already cleaned by remove_contexts_for_block
}
```

Also clear `LAST_FOCUSED_BY_ROOT` for any root whose recorded child was part of the closing block — call `forget_focus_for_child` for each subclassed HWND being uninstalled. This prevents the `WM_ACTIVATE` restore path from trying to focus a stale HWND.

---

## 4. Defect B — `find_main_render_widget` filter blind to unregistered panes

### The filter
`MainFocusReclaimTask::execute` (`ui_tasks.rs:2133`):
```rust
let pane_outer_hwnds: Vec<*mut std::ffi::c_void> = self
    .state
    .list_browsers()
    .into_iter()
    .filter(|(k, _)| k.starts_with("browser-pane-"))
    .filter_map(|(_, mut b)| {
        b.host().and_then(|h| { … Some(wh.0 as *mut _) })
    })
    .collect();
```

After `BrowserUnregistered`, no `"browser-pane-*"` labels remain → `pane_outer_hwnds = []` → `panes_excluded=0` in log.

`find_main_render_widget` with an empty exclusion list returns the first `Chrome_RenderWidgetHostHWND` found under the Views top HWND — which may be the stale pane render widget.

### Fix (Defect B)
Two complementary approaches:

**B1 (defence-in-depth):** Also source pane HWND candidates from `BROWSER_PANE_HWND_CONTEXT`, which still has entries for panes that are live at the HWND level even if unregistered from `state.browsers`. Any HWND found in `BROWSER_PANE_HWND_CONTEXT` that is a parent of a `Chrome_RenderWidgetHostHWND` should be treated as a pane outer HWND.

**B2 (primary fix = Defect A):** If Defect A's fix properly unsubclasses HWNDs on close, `find_main_render_widget` will return the correct HWND regardless — the stale redirect hook will not fire even if the wrong HWND is focused momentarily.

---

## 5. Why the new-window workaround sometimes helps

Opening a new AgentMux window and typing in it causes:
1. `main_window_focus` IPC for the NEW window's label
2. `MainFocusReclaimTask` for the new window — no pane HWNDs in that window → `find_main_render_widget` correctly finds the new window's main render widget
3. `record_intentional_focus(correct_hwnd)` for the new window's root → `LAST_FOCUSED_BY_ROOT[new_root] = correct`

When you return to the main window, `WM_ACTIVATE` fires again on `0x10914`. `LAST_FOCUSED_BY_ROOT[0x10914]` still points at `0x2d709a0` so the storm fires again — but `defocus_all` running in each storm iteration eventually causes the redirect to stop firing (Chromium's internal focus state settles), OR a user click lands on a non-subclassed HWND, seating focus correctly and breaking the cycle.

The recovery is fragile: a second switch to an external browser and back will restart the storm.

---

## 6. Fix order

1. **(Highest impact) Defect A:** unsubclass all child HWNDs on pane close + call `forget_focus_for_child` for each. ~40–60 LOC in `hwnd.rs` + `callbacks.rs`.
2. **(Defence-in-depth) Defect B:** augment `pane_outer_hwnds` with entries from `BROWSER_PANE_HWND_CONTEXT`. ~10 LOC in `ui_tasks.rs`.

---

## 7. References

**Code**
- `agentmux-cef/src/browser_pane/hwnd.rs` — `BROWSER_PANE_WNDPROCS`, `install_browser_pane_focus_redirect`, `remove_contexts_for_block`, `forget_focus_for_child`
- `agentmux-cef/src/browser_pane/callbacks.rs` — `on_before_close_browser_pane`, `on_load_end_browser_pane`
- `agentmux-cef/src/ui_tasks.rs:2080` — `MainFocusReclaimTask::execute`
- `agentmux-cef/src/ui_tasks.rs:2179` — `find_main_render_widget`

**Related analysis**
- `docs/analysis/ANALYSIS_BROWSER_PANE_REDOCK_BLACK_TYPING_LOCK_2026_06_15.md` — same Defect A class (focus orphaning), different trigger (redock vs. close)
- `docs/analysis/ANALYSIS_BROWSER_PANE_REDOCK_LOAD_RACE_2026_05_29.md`

**Issues**
- #768 — Phantom browser pane lifecycle divergence (lifecycle events still missing)
- #1190 — Browser pane: native CEF child windows; keystrokes bypass host WebView
