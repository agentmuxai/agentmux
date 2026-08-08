# Floating pane tear-off (subordinate window, owned by mother instance)

**Status:** Proposed (implemented — see note below)
**Owner:** AgentA
**Date:** 2026-05-11

> **2026-08-07 audit note:** Implemented, superseded by later tearoff specs
> building on it (`CrossWindowDragMonitor.*`, `DragOverlay.tsx`,
> `tear-off-pool-helper.ts`; e.g. `SPEC_PANE_DRAG_TO_TAB_2026_07_10.md`).
> Status field was never updated. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.
**Driving observation:** *"When tearing off panes, we don't create a new instance (no new taskbar icon). Instead it's a floating window owned by the mother instance. Panes currently tear off and create a new instance; instead they would tear to subpanels."*

Today, tearing a tab off creates a new full AgentMux instance — own backend sidecar, own data dir, own taskbar entry. That's the right model for tabs (workspace-level isolation). For **panes** within a tab, the new ask is different: tear into a **floating subordinate window** owned by the source instance, like Photoshop's palettes or VSCode's "Move Editor into New Window".

This spec covers panes specifically. Tab tear-off behavior is unchanged.

---

## 1. Target behavior

When the user drags a pane out of its tile layout and drops it outside the source window:

- A floating window opens at the cursor.
- The floating window has **no taskbar entry**, **no Alt-Tab entry**.
- The floating window **minimizes/restores with the source window** automatically.
- Closing the source window **destroys all its floating panes**.
- The floating pane shares **the source instance's backend sidecar, data dir, reducer state**. There is no new sidecar process, no new data dir, no new browser-pane HWND pool.
- The pane can be dragged from its titlebar to any position on any monitor (not clipped to the source window's bounds).
- A "dock back" affordance: drag the floater into the source window's layout, or use a button on its title bar.

The user gets a free-floating pane that's a full peer of the docked panes (same state, same broadcasts, same drag-drop into other tiles).

---

## 2. Why owned, not standalone

The current "new instance" path is overkill for panes:

| Cost | New full instance (today) | Owned floating (proposed) |
|---|---|---|
| Backend sidecar | Spawns a new one (~150-300 ms cold start) | Reuses the parent's |
| Data directory | New `~/.agentmux/versions/<v>/` dir | Shares parent's |
| Taskbar entry | One per torn-off pane | None |
| Cross-pane state | Requires inter-instance IPC | Shared in-process |
| Persistence | Per-instance layout file | Single layout file with `isFloating` flag |
| Resource overhead | ×N sidecars, ×N data dirs, ×N CEF process pools | One of each |
| Crash blast radius | Per-instance isolation | Floaters die with parent (acceptable for palettes) |

Photoshop, Figma, After Effects, VSCode "Move Editor", Chrome PiP — every modern UI that supports detached panels uses the owned-floating pattern. The full-instance model is reserved for genuinely independent workspaces.

---

## 3. Architecture

### 3.1 Win32 window styles

The floating window's HWND is created with:

```c++
CreateWindowEx(
    WS_EX_TOOLWINDOW,                       // no taskbar, no Alt-Tab
    "AgentMuxFloatingPane",
    "AgentMux — <pane title>",
    WS_OVERLAPPEDWINDOW | WS_POPUP,         // resizable, frameless-or-not, free-positioned
    x, y, w, h,
    sourceMainWindowHwnd,                   // OWNER (not parent — Win32 names it confusingly)
    NULL, hInstance, NULL
);
```

Critical bits:

| Flag / arg | Effect |
|---|---|
| `WS_EX_TOOLWINDOW` | Removes from taskbar AND Alt-Tab |
| `WS_POPUP` (not `WS_CHILD`) | Free positioning, not clipped to source window bounds |
| Owner HWND in arg 8 | OS auto-handles minimize/restore cascade and destroy cascade |

`WS_CHILD` is **wrong** — children are clipped to parent's client area. Owned `WS_POPUP` is the right pattern.

### 3.2 CEF browser hosting

The floating window's outer HWND is created by us; the Chromium browser inside is a `WS_CHILD` of that outer HWND, via standard CEF embed:

```c++
CefWindowInfo info;
info.SetAsChild(outerHwnd, CefRect{0, 0, w, h});
CefBrowserHost::CreateBrowser(info, clientHandler, frontendUrl, browserSettings, ...);
```

**Don't** use `SetAsPopup` — that's for `window.open()`-style CEF-managed popups and bypasses our owner relationship.

Known CEF + owner-window pitfalls (from existing literature):

- Focus chain: CEF needs explicit `OnFocus`/`SetFocus` plumbing. An owned window doesn't auto-forward focus to its embedded browser. We already have this plumbing for the main window's webview (`agentmux-cef/src/browser_pane/hwnd.rs::install_focus_redirect_wndproc`); the floating-pane HWND needs the same treatment.
- Drag-from-titlebar with a frameless window: implement `WM_NCHITTEST` returning `HTCAPTION` over the draggable region. We already do this for the main window; mirror the pattern.
- DPI: handle `WM_DPICHANGED` on the floating HWND separately from the parent. CEF won't auto-rescale; we already handle this for the main window.
- `GWLP_HWNDPARENT` mutation must happen before `ShowWindow` for clean taskbar behavior. Set the owner at create time, not later.

### 3.3 Reducer & state

The floating pane is a **view of the same workspace** the parent window owns:

- The pane's block stays in the same workspace's `blockids` (no `TearOffBlock` mutation that moves the block to a new workspace).
- A new layout field `floating: { paneId → { monitor, x, y, w, h, isFloating: true } }` records float state.
- The layout reducer reads `isFloating` to decide whether to render the pane in the tile layout (omits it) or in a separate floating overlay (renders it).
- Re-dock: drag the floater into the tile layout, or click a "dock" button. Reducer command `RedockFloatingPane(paneId, dropTargetNodeId)`. Layout returns to standard tile flow.

### 3.4 The frontend Pane component is unchanged

The pane's component (`<Block>` in `frontend/app/block/block.tsx`) is identical whether it renders inside a `<TileLayout>` or inside a floating window. Same RPC, same atoms, same focus handling. The DIFFERENCE is who owns the surrounding HWND:

- Docked: source window's main HWND, positioned via TileLayout's flex math.
- Floating: a separate HWND owned by the source window, positioned via window manager.

This means the existing browser-pane / agent-pane / terminal renderers Just Work in floating mode — no per-pane work.

### 3.5 What changes in the tear-off pipeline

Today (cf. `frontend/app/drag/CrossWindowDragMonitor.win32.tsx::performTearOff` + `agentmux-cef/src/commands/drag.rs::open_window_at_position`):

1. User drops a pane outside the window
2. Frontend calls `WorkspaceService.TearOffBlock` → moves block to new workspace
3. Frontend calls `openTearOffWindow` → host creates a new top-level AgentMux instance pointing at the new workspace
4. SC_MOVE handshake transfers cursor capture

For panes after this spec ships, branch on `dragType === "pane"`:

1. User drops pane outside the window
2. Frontend calls **`openFloatingPaneWindow(paneId, x, y, w, h)`** — new host command
3. Host creates an OWNED HWND with `WS_EX_TOOLWINDOW` (per §3.1)
4. CEF browser embedded into the owned HWND, navigated to the frontend's same URL with `?floatingPaneId=<id>&windowLabel=floating-<n>`
5. Frontend in the floating window:
   - Detects the `floatingPaneId` query param
   - Boots into a minimal `<FloatingPaneShell>` that renders only the one Block
   - Connects to the parent instance's backend via the shared sidecar IPC
6. Reducer dispatch: `MarkPaneFloating(paneId, {monitor, x, y, w, h})`. The tile layout drops the pane from its leaf order; floating overlay picks it up.
7. SC_MOVE handshake still applies (same cursor-grab anchor math), so the floating window opens with its title bar under the cursor.

Tab tear-off path remains unchanged.

---

## 4. Floating-pane shell

A new frontend entry point: `frontend/app/floating-pane/`:

- `floating-pane-shell.tsx` — minimal layout: a small title bar with pane title + dock button + close button, then the `<Block>` filling the rest.
- The title bar uses `-webkit-app-region: drag` plus our `WM_NCHITTEST` shim (§3.2) so dragging anywhere in the title bar moves the HWND.
- The shell shares the global Solid store with the main instance via... wait, no — these are separate CEF browser instances in DIFFERENT processes. They share the BACKEND (one sidecar), not the frontend store.
- So the shell connects to the sidecar via the same RPC client (`TabRpcClient`) — same authKey, same IPC port. The reducer state in the floating window's frontend syncs to the same backend objects.
- A WebSocket subscription on the floating window listens for `block:update` events for its pane and re-renders.

This is the same pattern modal-v2 / DevTools secondary window already uses for cross-window state sync.

---

## 5. Re-docking

User can:

1. **Drag the floater's title bar back into the source window's current tile layout.** Drop on any leaf or split target. Layout reducer dispatches `RedockFloatingPane(paneId, dropTargetTabId, dropTargetNodeId)`. The floating HWND is destroyed; the pane re-renders inside the target tab's TileLayout.
2. **Drag the floater's title bar into a different tab on the source window's tab bar.** Same instance, different tab. Reducer command moves the block from its current `tabId` to the dropped-on `tabId`, updates the target tab's `blockids` array, and destroys the floating HWND. Free given the same plumbing as #1 — the drop target just resolves to the tab's root layout node rather than a leaf.
3. **Drag the floater's title bar into another AgentMux instance.** Cross-process — uses the existing cross-window-drag IPC (`startCrossDrag`/`updateCrossDrag`/`completeCrossDrag`) that today supports tab cross-drop. Source instance serializes the block's persistent state (meta, view-specific data); destination instance deserializes into its sidecar, inserts into the target tab/workspace, and the source floating HWND is destroyed. Deferred to Phase 7 — the same-instance flows (1, 2) cover most cases without cross-process serialization.
4. **Click the "dock" button** on the floater's title bar. Pane re-docks into the source window at its last docked position (saved on tear-off).
5. **Close the floater.** Same as closing a docked pane — destroys the block.

---

## 6. Persistence

The source window's layout file gains a `floating` map:

```ts
{
    layout: { rootNode, leaforder, focusednodeid },
    floating: {
        [paneId: string]: {
            monitor: number;      // monitor index for restore
            x: number; y: number; // screen coords
            w: number; h: number;
            isFloating: true;
        };
    };
}
```

On instance restart:
- Layout reducer reads `floating` map.
- For each entry, host creates an owned floating window at the saved geometry.
- If the saved monitor is no longer present (laptop unplugged from dock), clamp to primary monitor's working area.

---

## 7. Multi-monitor & DPI

- Floating HWND can sit on any monitor — Win32 owned `WS_POPUP` supports this natively.
- DPI: each floating window receives its own `WM_DPICHANGED` when moved across DPI boundaries. We forward this to the CEF browser (existing `WM_DPICHANGED` handler in `agentmux-cef/src/client/wndproc.rs`) and re-render with the new device pixel ratio.
- When parent moves across monitors, owned floaters stay on their CURRENT monitor (they don't follow). This is the OS-standard behavior; documented for the user.

---

## 8. Edge cases

- **Source window closes while floaters are open:** OS cascade destroys all owned windows. Reducer command `WindowClosed` clears the `floating` map. No orphan windows.
- **Source window crashes:** WER kills the process; OS cascade still cleans up owned windows. Acceptable failure mode.
- **User drags floater onto another AgentMux instance's tile layout:** out of scope. Re-docking only into the source window. To move a pane between instances, the user must close the floater and re-open in the target instance (or drag the underlying block via a future cross-instance handle).
- **User puts floater on a virtual desktop different from source:** owned windows follow the owner across virtual desktops (Win+Ctrl+arrow). User can't "split" the owner and a floater onto different virtual desktops. This is OS behavior; document it.
- **Source window minimized:** floaters auto-hide. On restore, they reappear at their previous positions.
- **Source window full-screen:** floater still shows ABOVE its owner (owned-window behavior). If user wants floaters hidden during full-screen, that's a future preference.
- **Floater positioned mostly off-screen at restart:** clamp to the working area of the closest visible monitor.

---

## 9. Implementation phases

| Phase | Scope | LOC est. | Risk | Ship as |
|---|---|---|---|---|
| **1 — Host API + owned HWND** | `openFloatingPaneWindow(paneId, x, y, w, h)` CEF command. Creates owned `WS_POPUP \| WS_EX_TOOLWINDOW` HWND. CEF browser embedded. Loads `?floatingPaneId=…`. Minimal shell renders "hello". | ~400 | Medium (new Win32 path) | Standalone PR for the windowing primitive. |
| **2 — Floating-pane shell** | New `frontend/app/floating-pane/` shell renders `<Block>` for the requested paneId. Sidecar IPC via existing TabRpcClient. Title bar + dock + close buttons. WM_NCHITTEST drag region. | ~300 | Low | Bundle with Phase 1 if small enough; otherwise separate. |
| **3 — Tear-off routing** | `CrossWindowDragMonitor.win32.tsx::performTearOff` branches on `dragType`. `pane` → call `openFloatingPaneWindow`. `tab` → unchanged. Add `MarkPaneFloating` reducer command. | ~200 | Medium | Separate PR. |
| **4 — Re-dock (same-window, same-tab + cross-tab)** | Drag floater onto tile layout in current tab → re-dock as normal pane. Drag onto another tab in the same source window's tab bar → move block to that tab and destroy floater. Drop targets accept floating-pane payloads. `RedockFloatingPane(paneId, tabId, nodeId?)` reducer command. | ~300 | Medium | Separate PR. |
| **5 — Persistence** | `floating` map in layout file. Restore floaters on startup. Monitor-clamp logic for missing monitors. | ~150 | Low | Bundle with Phase 4. |
| **6 — Polish** | Per-pane title in floater title bar, last-docked-position memory, escape-key behavior, keyboard shortcuts. | ~150 | Low | Standalone polish PR. |
| **7 — Cross-instance floater drop (optional)** | Drag floater into a different AgentMux instance's tab bar or tile layout. Uses existing `startCrossDrag`/`updateCrossDrag`/`completeCrossDrag` IPC. Source serializes the block's state (meta, view-specific persistence); destination deserializes into its sidecar. | ~400 | High (cross-process serialization correctness) | Follow-up; not required for MVP. |

Phases 1-3 give a working tear-off-to-float MVP. Phases 4-5 round out the same-instance experience (drag floater between tabs of the source window included). Phase 6 polishes. Phase 7 extends to cross-instance, which mirrors what tab tear-off already does.

---

## 10. Out of scope

- **Tab tear-off behavior.** Tabs continue to spawn new full instances. Only panes change.
- **Floater-to-floater drag.** Can't drag a pane from one floating window into another. Only floater ↔ source-window-tile-layout-or-tab.
- **Floater on macOS / Linux.** This spec is Windows-only initially. macOS has its own owned-window model (`NSWindow.addChildWindow`); Linux compositors vary. Cross-platform is a follow-up.
- **Pinning a floater always-on-top globally.** `WS_EX_TOPMOST` is intentionally NOT used — global topmost windows are user-irritating. A "pin" toggle that adds `WS_EX_TOPMOST` could be a future preference but defaults off.

(Note: floater → another tab in the source window, and floater → another AgentMux instance, are both in scope — Phases 4 and 7 respectively.)

---

## 11. Test plan

- [ ] Tear a pane off → floating window appears at cursor, no taskbar entry, no Alt-Tab entry
- [ ] Minimize source window → all floaters disappear
- [ ] Restore source window → all floaters reappear at their previous positions
- [ ] Close source window → all floaters destroyed (no orphans in Task Manager)
- [ ] Move floater to second monitor → DPI scales correctly
- [ ] Drag floater's title bar into source window's tile layout (current tab) → re-docks as normal pane
- [ ] Drag floater's title bar into a different tab on the source window's tab bar → block moves to that tab, floater destroyed
- [ ] (Phase 7) Drag floater into another AgentMux instance → block transfers cross-process, floater destroyed
- [ ] Click floater's dock button → re-docks at last docked position
- [ ] Restart AgentMux → floating panes restore at their saved positions
- [ ] Floater can render every pane type: browser, agent, terminal, sysinfo, swarm, etc. (no per-type work needed — same `<Block>` renderer)
- [ ] Existing tab tear-off behavior unchanged: spawns new full instance with its own taskbar entry

---

## 12. Cross-references

- Current tear-off: `frontend/app/drag/CrossWindowDragMonitor.win32.tsx`, `frontend/app/tab/tabbar.tsx::requestTearOff`
- Host create-window: `agentmux-cef/src/commands/window.rs::open_window_at_position`, `agentmux-cef/src/commands/window_pool.rs::promote_pool_window`
- Focus chain plumbing: `agentmux-cef/src/browser_pane/hwnd.rs`
- Block renderer: `frontend/app/block/block.tsx`
- Existing similar pattern (cross-window state sync): modal-v2, DevTools secondary window
- Win32 owned-window docs: [MSDN — Window Features](https://learn.microsoft.com/windows/win32/winmsg/window-features)
- VSCode auxiliary windows precedent: [microsoft/vscode#10121](https://github.com/Microsoft/vscode/issues/10121)
- CEF embed pattern: `CefWindowInfo::SetAsChild` (NOT `SetAsPopup`)
- Tab tear-off behavior (unchanged): `docs/specs/SPEC_TAB_TEAROFF_POSITION_AND_PAINT_2026-05-07.md`

---

## 13. Driving observation (verbatim)

> "lets also smoke test tab tear … when tearing off panes, we don't create a new instance (no new taskbar icon) instead its a floating window owned by the mother instance. panes currently tear off and create a new instance, instead they would tear to subpanels. lets do best practice research, write spec to file"
