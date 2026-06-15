# Analysis: Browser-pane redock → black page, no URL, global typing lock

**Date:** 2026-06-15
**Against:** main @ `91bc8bf2`
**Symptom (reported):** Redocking a browser pane (drag a floating/torn-off browser pane back into a docked position) yields a **black page with no URL**, and **typing is locked everywhere** — every pane (agent, terminal, even the address bar) stops accepting keystrokes. The only recovery is to **open another window and come back**, after which typing works again everywhere.

---

## 0. Executive summary

There are **two independent defects** that fire together on redock, plus a **reducer/lifecycle divergence** that makes both more likely. They are separable and can be fixed independently.

| # | Defect | Symptom it causes | Layer |
|---|--------|-------------------|-------|
| **A** | **Keyboard-focus orphaning on pane HWND destroy** — `destroy_hwnd()` hides + destroys the pane's native HWND but never hands keyboard focus back to the surviving window's render widget. | **Global typing lock** until a focus-reclaim fires. | Host (CEF / Win32) |
| **B** | **Redock re-create load race + slot re-init** — the redocked pane's frontend slot re-initializes to `url:""` and the host `create` can be deferred (pane still `Closing`), so no nav-state returns. | **Black page, empty URL.** | Host + frontend |
| **C** | **Reducer divergence on move** — `MoveBlock` doesn't touch the layout tree, doesn't clear focused-node, and doesn't sync the slice-#9 browser slot; three sources of truth drift on a path that bypasses the block-removal saga. | Makes A and B reachable; stale focus node; phantom slots. | Host reducer + frontend store |

**Why "open another window and come back" fixes typing:** that interaction sends the `main_window_focus` IPC, which posts `MainFocusReclaimTask` — the *only* code path that explicitly `SetFocus`es the main render widget and `defocus_all()`s the panes. It is the manual workaround for Defect A. (`agentmux-cef/src/ipc.rs:520`, `ui_tasks.rs:1193`.)

This matches the still-open triage on **issue #768** (host/frontend lifecycle divergence → black pane that drops typed URLs) and the architecture note in **issue #1190** (browser panes are native CEF child windows; keystrokes go to the CEF HWND, not the host WebView).

---

## 1. Defect A — focus orphaning on pane HWND destroy (the global typing lock)

### What the code does
On close/redock the floater's pane HWND is torn down by `destroy_hwnd`:

`agentmux-cef/src/browser_panes.rs:92`
```rust
fn destroy_hwnd(&self, hwnd: usize) {
    #[cfg(target_os = "windows")]
    unsafe {
        let h = hwnd as *mut std::ffi::c_void;
        let parent = GetParent(h);          // captured before destroy
        ShowWindow(h, SW_HIDE);             // stop DWM compositing the dead surface
        DestroyWindow(h);
        if !parent.is_null() {
            InvalidateRect(parent, null, 1); // repaint the hole
            UpdateWindow(parent);
        }
    }
    // ❌ No SetFocus / focus handoff to the parent or main render widget.
}
```

It hides, destroys, and repaints — but **never re-assigns keyboard focus**. If the pane HWND (or a descendant) held Win32 keyboard focus at destroy time, focus is now orphaned on a dead HWND. Windows has nowhere valid to route keystrokes, so **all** typing dies — not just the browser pane, because focus is a single per-thread/queue concept, not per-pane.

### Why a browser pane usually *does* hold focus
Browser panes are native CEF child windows (issue #1190). The pane subclass actively keeps focus on the pane and tracks it:
- `agentmux-cef/src/browser_pane/hwnd.rs:299` — `WM_SETFOCUS` handling; intentional focus is recorded via `record_intentional_focus`.
- `agentmux-cef/src/browser_pane/hwnd.rs:94` — `LAST_FOCUSED_BY_ROOT: HashMap<root_hwnd, child_hwnd>` remembers the last focused pane per top-level window.

So at the moment of a redock drag-drop, the floater's pane is typically the focused HWND. Destroying it without a handoff is exactly the orphaning case.

### Why the new-window workaround restores it
The reclaim path exists but is only triggered by `main_window_focus`:

`agentmux-cef/src/ipc.rs:520`
```
"main_window_focus" => { … post MainFocusReclaimTask for `window_label` … }
```
`agentmux-cef/src/ui_tasks.rs:1209` (`MainFocusReclaimTask::execute`)
```rust
host.set_focus(1);                              // Chromium: main Browser has focus
// Win32: find main render widget (skipping pane HWNDs), SetFocus(target)
SetFocus(target);
record_intentional_focus(target);
self.state.browser_panes.defocus_all(&self.state);  // clear Chromium focus on panes
```
This is the **only** place that re-seats focus on the main render widget and defocuses panes. The frontend sends `main_window_focus` when the user clicks a main-DOM input / on window activation — i.e. "open another window and come back." Nothing fires it automatically on pane destroy, so the lock persists until the user does that dance.

### Fix (Defect A)
On the pane-destroy path, **hand focus back deterministically**. Two options:
1. **Minimal:** in `destroy_hwnd`, after `DestroyWindow`, if the destroyed HWND (or its descendant) was the focused window for `parent`'s root, `SetFocus` the main render widget of that root and `record_intentional_focus(target)` — mirroring what `MainFocusReclaimTask` does for the Win32 half.
2. **Robust (preferred):** have the close/redock saga post `MainFocusReclaimTask` for the surviving window once the pane is gone, so both the Chromium (`host.set_focus`/`defocus_all`) and Win32 halves run. Reuses tested code; closes the gap for both explicit-close and drain-close paths.

Also clear the dead entry: on destroy, remove `LAST_FOCUSED_BY_ROOT[root]` if it points at the dying child, so the `WM_ACTIVATE` restore hook (`client/wndproc.rs:170`) doesn't have to rely on its `IsWindow` guard.

---

## 2. Defect B — black page / empty URL on redock

### The redock create flow
A redock = backend saga moves the block to the target tab; the floater auto-closes; the target window re-renders the block and calls `browser_pane_create`. Relevant pieces:

- Frontend re-create + slot init: `frontend/app/store/browser-pane-state-store.ts` — `registerPane()` seeds `initialState()` with **`url: ""`**.
- Frontend create: `frontend/app/view/browser/browser-view.tsx:191` (`createPane`) — on success sets `paneCreated(true)` + `model.onLoad()`, **with no retry on the deferred case**.
- Host create gate: `agentmux-cef/src/browser_panes.rs:242` — if the floater's old pane is still `Closing`, create returns `RegisterResult::Closing` and **stashes** the create; the IPC still resolves `Ok(())`, so the frontend believes it succeeded.
- Deferred replay: `browser_panes.rs:454` (`replay_pending_create`) re-issues the create on close-completion (`CompleteBrowserPaneClose` / `DrainBrowserPaneByLabel`).

### Why it goes black
If the create is deferred while the floater pane drains:
1. Frontend marks the pane created and stops driving navigation.
2. The slot's `url` is `""` (fresh `initialState()`), so the address bar is empty.
3. The replayed create brings up a fresh HWND at `about:blank`; no `Navigate` is re-dispatched against the new browser, and no nav-state event returns to populate the URL.

Net: a black `about:blank` pane with an empty address bar that silently drops what you type — exactly issue #768's "orphan / black pane drops typed URLs."

### What's already mitigated
- **#1168** deterministic re-create-after-close + **#1166** deterministic redock-onto-main reduced the *intermittent load failure*. The **stash/replay** in `browser_panes.rs` is the in-code remedy.
- **#1133** evicts stale HWNDs from `window_hwnds` on close (`client/mod.rs:778`).
- **#1156** native-pane reflow re-samples + pushes bounds during layout change (`browser-view.tsx:159`), preventing a stale/zero-bounds invisible pane.
- Analysis doc already on disk: `docs/analysis/ANALYSIS_BROWSER_PANE_REDOCK_LOAD_RACE_2026_05_29.md`.

### Remaining gap (Defect B)
The **slot re-init to `url:""` + lack of re-navigation** survives the mitigations. On a deferred/replayed create the new browser is never told to navigate to the pane's intended URL, and the slot has no URL to show. Fix:
- Carry the **intended URL** through the redock (it's already in the stashed `PendingBrowserPaneCreate`), and on replay **navigate the fresh browser to it** (not `about:blank`), then emit a nav-state so the slot's `url` repopulates.
- Frontend: don't latch `paneCreated(true)` until a nav-state/`BrowserRegistered` confirms a live browser; treat the deferred case as not-yet-created so `syncPosition`/navigation re-drive once it lands. (Implements the **`browser-pane-unregistered` / re-handshake** events issue #768 still calls for.)

---

## 3. Defect C — reducer/lifecycle divergence on move (the enabler)

### Move doesn't touch layout or focus
`agentmux-srv/src/reducer/block.rs:77` (`handle_move_block`) only mutates tab membership + `block.tab_id` and emits `BlockMoved`. It does **not**:
- mutate the layout tree (frontend persists that locally — `sagas/redock_floating_pane.rs` "saga does not touch LayoutState"), or
- clear/repoint `focused_node_id`. Compare `layout.rs::handle_layout_delete_node` (which *does* validate + clear stale focus) — there is **no equivalent on move**. So after redock, `focused_node_id` can point at a node id from the pre-move tree.

`frontend/layout/lib/layoutFocus.ts:15` (`validateFocusedNode`) clears a focused node that's gone from the tree, but it runs on the frontend's local tree recompute — timing-dependent relative to the move persist.

### Three sources of truth drift
Per issue #768 there are three lifecycle authorities:
1. Frontend slice-#9 slot store — `frontend/app/store/browser-pane-state-store.ts`
2. Host `BrowserPaneManager` registry (real CEF state)
3. Host reducer state (`BrowserRegistered`/`BrowserUnregistered`)

A redock that **bypasses the block-removal saga** doesn't dispose the slice-#9 slot, so the old slot lingers (`closed` never flips) while the host tears the HWND down — the divergence #768 describes. `BrowserViewModel.closed` reads `bpSnapshot(blockId)?.closed ?? true` (`browser-model.ts:165`), which can sit in a not-closed-yet state.

### Node remount vs. block keep-alive
`frontend/layout/lib/TileLayout.win32.tsx:324` keys the tile `<Key>` by **`node.id`**, but `block.tsx` caches the ViewModel by **`blockId`** (`getBlockComponentModel`). On redock the LayoutNode id changes → the `DisplayNode` DOM remounts while the `BrowserViewModel` survives. If disposal/recreate timing doesn't align with the DOM unmount, IPC can fire into a half-alive pane — another route into Defects A/B.

### Fix (Defect C)
- Add a **focus-validation arm to `handle_move_block`** (mirror `handle_layout_delete_node`) so `focused_node_id` is repointed/cleared on move.
- Route redock through (or emit) a **`browser-pane-unregistered`** lifecycle event so the slice-#9 slot is disposed deterministically when the host HWND goes away — the missing half of issue #768.
- Consider keying the tile by a stable id (blockId) or otherwise guaranteeing dispose-before-recreate ordering on move.

---

## 4. Recommended fix order

1. **Defect A (highest impact, smallest change):** post `MainFocusReclaimTask` (or inline the Win32 focus handoff) on the pane-destroy/redock path, and clear `LAST_FOCUSED_BY_ROOT` for the dying child. → kills the global typing lock outright. ~30–50 LOC, host-only.
2. **Defect B:** on deferred/replayed create, navigate the fresh browser to the stashed URL and gate `paneCreated` on a real nav-state; repopulate the slot URL. → fixes black page / empty URL.
3. **Defect C:** focus-validation on `MoveBlock` + the `browser-pane-unregistered` handshake from #768. → removes the divergence that makes A/B reachable and prevents stale-focus regressions.

---

## 5. Verification plan

- **Repro harness:** float a browser pane on a loaded URL, redock it onto the main window; assert (a) the pane shows the URL and renders (not black), (b) typing works immediately in the redocked pane *and* in a sibling agent/terminal pane without opening a new window.
- **Focus (A):** add a host log on the destroy path showing the focused HWND before destroy and the focus target after handoff; confirm focus lands on the main render widget (reuse the `[main-focus-reclaim]` log format).
- **Load (B):** confirm `replay_pending_create` issues a `Navigate` to the stashed URL and a nav-state event returns; assert the slot `url` is non-empty post-redock.
- **Divergence (C):** assert the slice-#9 slot for the old pane flips `closed=true` on redock (no lingering slot), and `focused_node_id` resolves in the new tree.
- Cross-platform: the focus orphaning is Win32-specific; verify macOS/Linux native-pane paths separately (`browser_pane/` per-OS).

---

## 6. References

**Issues / discussions**
- #768 — Phantom browser pane (orphan + tearoff): host/frontend lifecycle divergence. *(Triage: idempotent dispose shipped; the two host→frontend lifecycle events + recovery overlay still missing.)*
- #1190 — Browser pane: native CEF child windows; keystrokes bypass the host WebView (input-first Phase 0.3, discussion #1161).
- #864 — persist layout-tree events through `apply_event_to_wstore` (layout events still wcore-direct; bridge inconsistency).
- Discussion #707 — reducer-stack architecture tracking thread.

**Host (CEF) code**
- `agentmux-cef/src/browser_panes.rs:92` — `destroy_hwnd` (no focus handoff — Defect A).
- `agentmux-cef/src/browser_panes.rs:242` / `:454` — `RegisterResult::Closing` gate + `replay_pending_create` (Defect B).
- `agentmux-cef/src/browser_pane/hwnd.rs:94` / `:299` — `LAST_FOCUSED_BY_ROOT`, `WM_SETFOCUS` subclass.
- `agentmux-cef/src/client/wndproc.rs:170` — `WM_ACTIVATE` focus-restore (stale-child `IsWindow` guard).
- `agentmux-cef/src/ipc.rs:520` + `ui_tasks.rs:1209` — `main_window_focus` → `MainFocusReclaimTask` (the manual recovery).
- `agentmux-cef/src/client/mod.rs:778` — stale-HWND eviction (#1133).
- `agentmux-cef/src/commands/window/lifecycle.rs:212` — `resolve_window_hwnd` 3-tier lookup.

**Frontend / reducer code**
- `frontend/app/view/browser/browser-view.tsx:100/136/159/191` — paneRect, syncPosition, reflow, createPane.
- `frontend/app/store/browser-pane-state-store.ts` — slice #9 slot store (`initialState().url = ""`).
- `frontend/app/view/browser/browser-model.ts:165/546` — `closed` snapshot + `dispose()`.
- `agentmux-srv/src/reducer/block.rs:77` — `handle_move_block` (no layout/focus mutation).
- `agentmux-srv/src/reducer/layout.rs` — `handle_set_focused_node`, `handle_layout_delete_node` (has focus validation move lacks).
- `agentmux-srv/src/sagas/redock_floating_pane.rs` — redock saga (`MoveBlock`, no LayoutState touch).
- `frontend/layout/lib/TileLayout.win32.tsx:324` — tile keyed by `node.id`; `block.tsx` caches VM by `blockId`.
- `frontend/layout/lib/layoutFocus.ts:15` — `validateFocusedNode`.

**Prior analysis / specs**
- `docs/analysis/ANALYSIS_BROWSER_PANE_REDOCK_LOAD_RACE_2026_05_29.md`
- `docs/analysis/ANALYSIS_PANE_OPEN_CLOSE_ANIMATION_2026_05_29.md`
- `docs/specs/SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md`
- `docs/specs/SPEC_PHANTOM_BROWSER_PANE_RECOVERY.md` (branch `agenta/spec-phantom-browser-pane-recovery`)
- `docs/specs/MASTER_REDUCER_STACK_STATUS_2026-05-05.md` §8.15

**Relevant merged PRs:** #1112 (MVP redock), #1156 (pane reflow), #1166 (deterministic redock-onto-main), #1168 (deterministic re-create-after-close), #1133 (stale-HWND eviction), #1249 (redock dwell/velocity gate).
