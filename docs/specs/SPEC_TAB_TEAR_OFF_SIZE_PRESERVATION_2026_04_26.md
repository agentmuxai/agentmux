# Tab Tear-Off — Chrome-Faithful Window-Move Architecture

**Date:** 2026-04-26 (rewritten to drop the canvas-ghost approach)
**Status:** Spec only (no implementation yet)
**Trigger:** User feedback —
  > *"chrome tabs don't ghost, you just drag it out, and the
  > entire non-faded window drags around"*

  > *"can we replicate that exactly?"*
**Scope:** Cross-window tab tear-off using Win32 `SC_MOVE` modal
           loop, the same mechanism Chrome / Edge / Arc use.
**Owner:** TBD
**Supersedes:** the canvas-ghost approach in this file's first
           draft (commit history available via git).

---

## 1. The behaviour we are replicating

Chrome's tab tear-off, end-to-end:

1. User mousedowns on a tab and starts dragging.
2. While the cursor stays inside the tab strip → tabs reorder.
3. The moment the cursor passes a vertical threshold (typically
   the bottom edge of the strip), the tab is **torn off**:
   - The tab vanishes from the source window's strip instantly.
   - A real, full-fidelity OS window materialises at the cursor
     position, already containing the tab's pane content.
   - That new window enters Win32's built-in modal window-move
     loop, so it follows the cursor at full opacity, no fade,
     no ghost.
4. While the user moves, Chrome watches the cursor for any other
   Chrome window's tab strip underneath.
5. On mouseup:
   - Cursor over another window's strip → the dragged window's
     tab is **merged** into that window (dragged window is
     destroyed; tab inserted at the cursor's X position in the
     destination strip).
   - Cursor anywhere else → the dragged window simply stays where
     dropped. No-op finalize.
6. If the user releases back over the source strip without ever
   leaving it → the tear-off is undone (or never started — see
   §4.1 threshold).

There is no HTML5 drag, no `setDragImage`, no transparent
overlay, no canvas. It is a sequence of (a) "spawn a window with
this tab's content" and (b) "have Windows move that window for me
until mouseup."

## 2. Goals

G1. The torn-off tab arrives in its new window at exactly the
    same width / height it had in the source — to within 1 device
    pixel. (Width preservation; see §5.)
G2. The torn-off window appears at the cursor with no first-paint
    flash. The user perceives "I picked up the window" instantly.
G3. Cross-window merge: dropping on another AgentMux window's tab
    strip moves the tab into that window's strip at the visually-
    indicated insertion point.
G4. Cancel-back-to-source: starting a tear-off and then dropping
    on the source window's tab strip restores the tab to its
    original position; the spawned window vanishes.
G5. The implementation must work on Win32 first; macOS and Linux
    follow the same architecture using each platform's native
    move-window API (see §7).

## 3. Non-goals

NG1. Cross-process drag (dragging a tab into Chrome, Slack, etc.)
     — out of scope; the OS doesn't surface a clean way to do
     this for non-OLE participants and Chrome itself doesn't.
NG2. Animated re-flow of the source strip when the tab leaves.
     The remaining tabs simply re-layout without animation; users
     are looking at the cursor, not the strip they just left.
NG3. Restoring tear-off on Linux/X11 if compositor supports
     differ wildly. We accept "best effort" on non-Win32 in the
     first cut.

## 4. Architecture (Win32)

### 4.1 Tear threshold

The frontend's existing tab DnD (`tabbar-dnd.ts`) tracks an
in-bar drag using pragmatic-dnd's `monitorForElements`. We extend
that monitor with a tear-threshold check:

```
onDrag({ location }) {
  const r = tabBarScrollRef?.getBoundingClientRect();
  const y = location.current.input.clientY;
  const TEAR_PAST = 24; // px past the bottom edge of the strip
  if (r && y > r.bottom + TEAR_PAST) {
    requestTearOff(draggedTabId);
  } else {
    setInsertionPoint(computeInsertionPoint(location.current.input.clientX));
  }
}
```

`requestTearOff` is a one-shot — once fired, in-bar reorder logic
shuts off for the rest of this drag, and the host takes over.

### 4.2 The tear-off handshake

`requestTearOff(tabId)` invokes a single host command:

```
api.tearOffTab({
    sourceWindowId,
    tabId,
    workspaceId,
    cursorX, cursorY,           // screen coords from getApi().getCursorPoint()
    snapshot: TabSnapshot,      // §5
})
```

Host (Rust, `agentmux-cef`) handles it as follows:

1. **Cancel the in-progress HTML5 drag** in the source webview.
   The host sends a `__tab_drag_canceled` message to the source
   renderer; renderer dispatches `dragend` programmatically and
   pragmatic-dnd's monitor sees the cancellation.
2. **Allocate a destination window.** Two options:
   - *Cold path:* `WindowApi::new_window_with_tab(workspace,
     tab_id, snapshot)` — creates window, points it at the tab.
     ~150-300ms first paint.
   - *Warm path:* the host keeps a single hidden, fully-painted
     "scratch" window in a pool. On `tearOffTab`, the scratch
     window is shown at `(cursorX, cursorY) - tabClickOffset`,
     sized to the snapshot, and points its renderer at the
     transferred tab. The pool re-spawns a replacement
     immediately after. <16ms first paint.
3. **Move the tab data** from source to destination workspace.
   Reuse `WorkspaceService.MoveTabToWorkspace(tabId, srcWsId,
   destWsId)` — already exists for cross-window drops.
4. **Hand off cursor capture.** This is the timing-critical bit:
   - Source window: `ReleaseCapture()` (drops OLE drag capture).
   - Destination window: `SetForegroundWindow(destHwnd)`,
     `SetCapture(destHwnd)`.
   - Destination window: `PostMessage(destHwnd, WM_SYSCOMMAND,
     SC_MOVE | HTCAPTION, MAKELPARAM(cursorX, cursorY))`.
5. **Windows takes over.** From this point until mouseup,
   Windows runs its own modal `GetMessage`-based move loop. No
   AgentMux frame is processed; cursor follows the window
   one-to-one.

The handshake (steps 1-4) must complete in a single host-side
call before returning, so the renderer's drag-cancel and the
host's `SC_MOVE` happen back-to-back without a paint frame in
between. Empirically Chrome does this in ~5-8 ms. Our budget
should be ≤16 ms (one 60 Hz frame) to avoid a visible glitch.

### 4.3 Tracking the cursor during the move-loop

Because the move-loop is modal, AgentMux's normal renderer
message handlers don't run. To detect "is the cursor over another
AgentMux window's tab strip?", we use a **`WH_MOUSE_LL`
low-level mouse hook**, installed by the host before the
`SC_MOVE` and uninstalled on mouseup.

The hook handler (runs on a background thread):

```
on every WM_MOUSEMOVE:
    let hwnd = WindowFromPoint(cursor)
    let agentmux_window = lookup_agentmux_window(hwnd)
    if agentmux_window != current_target {
        notify_destination_renderer(agentmux_window, cursorX, cursorY)
        current_target = agentmux_window
    }

on WM_LBUTTONUP:
    finalize_tear_off(cursor, current_target)
    uninstall_hook()
```

`notify_destination_renderer` posts an IPC event that the
candidate destination window's tab strip receives — it can then
draw the same insertion-point indicator as a normal in-bar
hover, so the user gets visual feedback during the move.

### 4.4 Drop finalisation

On mouseup, `finalize_tear_off(cursor, target)`:

- **target is another AgentMux window:**
  1. Compute insertion index in target's strip from `cursor.x`.
  2. Move the tab from the dragged window's workspace into
     target's workspace at that index (reuse existing
     `MoveTabToWorkspace`).
  3. Destroy the dragged window (it's now empty).
  4. Show the target window (was likely already visible).
- **target is the source window's strip** (cancel-back path):
  1. Move the tab back to source's workspace at its original
     index (the host kept the original index in the tear-off
     state).
  2. Destroy the dragged window.
- **target is none / empty desktop:**
  1. The dragged window stays where the user released. No-op.
  2. The tab (now the only tab in the dragged window) is the
     window's content.

In all cases, the drag ends and the move-loop hook is removed.

### 4.5 Pre-warmed window pool

The "warm path" in §4.2 keeps the tear-off feeling instant. A
single hidden, fully-painted blank `agentmux-cef` window lives in
the background. On tear-off:

1. Scratch window is shown, resized, repositioned, and re-pointed
   at the tab content via the existing layout/blockcontroller
   plumbing. The renderer is already up; it only needs to paint
   one tab's content, which is bounded.
2. The host immediately spawns a replacement scratch in the
   background (subject to a "max 1 in-flight respawn" rule to
   avoid runaway window creation).

The scratch window costs RAM (~50-80 MB for an empty CEF
renderer). Acceptable for the UX win on a desktop app.

If pool unavailable (race, replacement still spawning, etc.) →
fall back to the cold path. User sees ~200ms first-paint flash;
not great but not broken.

## 5. Width preservation (unchanged from prior draft)

The width snapshot mechanism survives this rewrite — it's
orthogonal to whether we ghost or move-window.

### 5.1 Capture (source side, on tear-off)

```
type TabSnapshot = {
    cssPxWidth: number;     // getBoundingClientRect().width
    cssPxHeight: number;
    devicePixelRatio: number;
    zoomFactor: number;
    color: string | null;
    name: string;
};
```

Captured at tear-threshold-crossing in the source renderer,
travels in the `tearOffTab` payload.

### 5.2 Apply (destination side)

The destination renderer receives the snapshot, writes
`tab:torn-off-width` and `tab:torn-off-at` to the tab's meta,
and applies `style={{ width: \`${snapshot.cssPxWidth}px\` }}` on
the tab DOM element until either:

- 30 seconds elapse, OR
- the user renames / re-drags the tab.

After release, the tab returns to normal `width: auto`.

CSS pixels are DPR-invariant, so cross-monitor tear-offs preserve
the source's *shape* but scale to the destination's chrome.

## 6. Implementation phases

### Phase 1 — Tear-threshold detection (½ day, frontend only)

- Extend `tabbar-dnd.ts` / `tabbar.tsx` `monitorForElements` with
  the `TEAR_PAST = 24` check.
- Stub `requestTearOff(tabId)` that just `console.log`s for now.
- Verify in dev: dragging a tab past the strip's bottom edge
  fires the stub exactly once per drag.

### Phase 2 — Host tear-off command (cold path) (1.5 days, Rust)

- New IPC: `tear_off_tab(payload) -> Result<()>`.
- Implement steps 1-4 of the handshake (§4.2) using cold-path
  window allocation.
- No mouse hook yet — call `SC_MOVE` and let the user mouseup;
  on mouseup the dragged window stays where it is. (i.e. drop-
  on-empty-desktop case only.)
- Verify: drag a tab past threshold → new window spawns at
  cursor → drag around → release → window stays. No merge yet.

### Phase 3 — Width snapshot (½ day, frontend + RPC)

- Extend the tear-off payload with `TabSnapshot`.
- Apply the width on destination via meta keys + inline style.
- Verify: torn-off tab in the new window matches source width.

### Phase 4 — Mouse hook + merge detection (1 day, Rust)

- Install `WH_MOUSE_LL` for the duration of the move-loop.
- Track candidate destination via `WindowFromPoint` + AgentMux
  window registry.
- Push hover events to candidate's renderer for insertion-point
  preview.
- On mouseup, decide: merge (target exists) or no-op (no target).

### Phase 5 — Cancel-back-to-source + finalise (½ day)

- Add the source-window cancel path (§4.4).
- Edge cases: source window already closed, target window
  destroyed mid-drag, ESC pressed during move-loop (cancel and
  restore).

### Phase 6 — Pre-warmed window pool (1 day)

- Implement scratch-window factory + auto-respawn.
- Wire warm-path into `tear_off_tab`.
- Validate: tear-off first-paint flash drops from ~200ms to
  <16ms.

### Phase 7 — Polish + cross-platform stubs (½–1 day)

- macOS: `[NSWindow performWindowDragWithEvent:]` instead of
  SC_MOVE; `CGEventTap` instead of `WH_MOUSE_LL`.
- Linux/X11: `_NET_WM_MOVERESIZE` (compositor-dependent quality).
- Linux/Wayland: `xdg_toplevel.move()` (limited to logical
  pointer position).

## 7. Cross-platform notes

| Capability             | Win32                      | macOS                                          | Linux/X11                  | Linux/Wayland               |
|------------------------|----------------------------|------------------------------------------------|----------------------------|------------------------------|
| Initiate window-move   | `WM_SYSCOMMAND/SC_MOVE`    | `[NSWindow performWindowDragWithEvent:]`       | `_NET_WM_MOVERESIZE`       | `xdg_toplevel::move`        |
| Global cursor tracking | `WH_MOUSE_LL`              | `CGEventTap` (needs Accessibility permission)   | `XQueryPointer` polling    | none — Wayland forbids it   |
| Window-from-point      | `WindowFromPoint`          | `[NSWindow windowNumberAtPoint:belowWindowWithWindowNumber:]` | `XQueryTree` walk          | none reliable               |
| Pre-warm windows       | trivial (`CreateWindowEx`) | trivial (`NSWindow`)                            | trivial (X11)              | trivial (xdg_toplevel)       |

Wayland is the worst case: no global cursor tracking is
permitted, so we lose the merge-detection feature on Wayland.
Acceptable: torn-off tab simply becomes a standalone window;
user can drag again to merge if desired.

## 8. Edge cases

E1. **Drag started on tab, never crosses threshold.** Just an
    in-bar reorder; nothing new happens.
E2. **User holds Esc during move-loop.** Windows cancels the
    move. Hook sees no `WM_LBUTTONUP`; we time out after 5s of
    no movement and treat as no-op.
E3. **Source window closed mid-drag.** State stored host-side
    survives renderer death; cancel-back path becomes "no-op,
    leave the dragged window standalone."
E4. **Two tear-offs in quick succession (impossible per UI but
    defensive).** Pool serializes.
E5. **DPI change during move (cursor crosses monitors with
    different scale).** Windows handles re-scaling
    automatically; renderer's `--zoomfactor` is per-window so
    the dragged window keeps its source's scale until
    mouseup, then the destination workspace's scale applies.
E6. **Tab is the only tab in source workspace.** Tearing it off
    would leave source empty. Two policy choices:
    - *Permit:* source becomes empty, user can close it.
    - *Forbid:* don't allow tear-off; treat as a no-op past
      threshold. Chrome chooses *permit*. We follow Chrome.

## 9. Validation checklist

Per phase. Phase 1 first; downstream phases each add their own
rows.

**Phase 1**
- [ ] Drag a tab within the strip — no tear, normal reorder.
- [ ] Drag past the strip's bottom edge — `requestTearOff` fires
      exactly once.
- [ ] Resume drag back into the strip after the threshold —
      `requestTearOff` doesn't fire again.

**Phase 2 (cold path)**
- [ ] Tear past threshold → new window spawns at cursor with the
      tab's content visible (after first-paint flash).
- [ ] Move the new window with the cursor (Windows SC_MOVE).
- [ ] Release → window stays. No merge attempts.

**Phase 3 (width snapshot)**
- [ ] Source tab "Hello" at width 142px → torn-off tab at 142px
      ± 1 device pixel in destination.
- [ ] Cross-monitor: width preserved in CSS pixels; physical
      size differs (correct).
- [ ] After 30s, dropped tab relaxes to auto-width.

**Phase 4 (merge detection)**
- [ ] Drag torn tab over another AgentMux window's strip —
      insertion indicator appears in destination.
- [ ] Mouseup over destination strip → tab merges at indicated
      index; dragged window destroyed.

**Phase 5 (cancel-back)**
- [ ] Drag past threshold, drag back over source strip,
      mouseup → tab returns to source at original index;
      spawned window destroyed.
- [ ] ESC during move-loop → no-op restoration.

**Phase 6 (warm path)**
- [ ] Frame-time of tear-off (mousedown-on-tab + threshold-cross
      → first paint of new window) ≤ 16ms.

## 10. Sources

- Existing in-window DnD: `frontend/app/tab/tabbar-dnd.ts`,
  `frontend/app/tab/tabbar.tsx`,
  `frontend/app/tab/droppable-tab.tsx`
- Existing cross-window drag (will be partially replaced):
  `frontend/app/drag/CrossWindowDragMonitor.win32.tsx`
- Window/tab data plumbing:
  `agentmux-srv/src/backend/wcore/tab.rs`,
  `agentmux-srv/src/backend/wcore/window.rs`,
  `frontend/app/store/services.ts` (`MoveTabToWorkspace`)
- Chrome source — TabDragController: where Chrome actually
  implements this. Useful for understanding cancel/merge
  semantics:
  https://source.chromium.org/chromium/chromium/src/+/main:chrome/browser/ui/views/tabs/tab_drag_controller.cc
- Win32 SC_MOVE pattern (canonical reference, MSDN /
  StackOverflow folklore):
  https://learn.microsoft.com/en-us/windows/win32/menurc/wm-syscommand
- `WH_MOUSE_LL`:
  https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelmouseproc
- `WindowFromPoint`:
  https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-windowfrompoint
- macOS `performWindowDragWithEvent:`:
  https://developer.apple.com/documentation/appkit/nswindow/1419032-performwindowdragwithevent
- Wayland xdg_toplevel.move limitation context:
  https://wayland.app/protocols/xdg-shell#xdg_toplevel:request:move
- Prior retro on auto-width sub-pixel jitter (motivation for the
  width-snapshot mechanism):
  `docs/retros/RETRO_SUBPIXEL_RENDERING_RESEARCH_2026_04_26.md`
