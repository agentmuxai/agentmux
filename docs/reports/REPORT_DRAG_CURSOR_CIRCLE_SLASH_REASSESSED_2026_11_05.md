# Report: the circle-slash cursor during pane and tab drags, reassessed

**Status:** option D implemented in #6498 (on by default, `AGENTMUX_DND_CURSOR=0` turns it off). Options E to H and macOS/Linux are analysis only.

**Date:** 2026-11-05 (sources read 2026-11-04). **Scope:** Windows first, then macOS and Linux. **Method:** the earlier retros, specs and host code were read, then the Chromium, CEF and Win32 sources were re-read online. Nothing was built or run.

## 0. Answer in short

We are not stuck. The earlier research was right that Chromium's drag source owns the cursor. It was wrong that nothing can change it during an OLE drag.

Chromium's `GiveFeedback` returns `DRAGDROP_S_USEDEFAULTCURSORS`. So the cursor is whatever OLE derives from the drop effect that the window under the pointer returns. Outside our window nothing of ours answers, so the effect is NONE and OLE shows its slash. Two small Windows-only changes can fix that without leaving OLE:

- **D, wrap the drop source.** Patch the `DoDragDrop` import so `GiveFeedback` sets an arrow and returns `S_OK`. Microsoft documents `S_OK` as "the source set the cursor".
- **E, make the picture window a drop target.** The click-through picture that follows the cursor becomes an OLE drop target that answers move or copy.

Both keep the HTML5 drag. So none of the regressions that shelved the native-pointer rewrite apply. D is recommended, with E as the fallback.

## 1. Why the earlier research said "stuck"

Sources: `docs/retro/retro-pane-tearoff-cursor-never-fixed-2026-07-27.md` (with its 07-29 update) and `docs/retro/retro-native-pointer-drag-tearoff-shelved-2026-07-29.md`. The other documents that were checked do not bear on the cursor:

- `docs/analysis/ANALYSIS_CURSOR_POINTER_AUDIT_2026_06_20.md` is about the link hand, not the drag cursor.
- `docs/analysis/cross-window-drag-wiring.md` only records the old `set_drag_cursor` wiring. That code is dead now.
- `docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md` lists `set_drag_cursor` and `restore_drag_cursor` for deletion.
- `docs/specs/SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24.md` does not touch the cursor.

What the retros found:

- The tab fix (#2175) sets `dropEffect="copy"` on `dragover` outside the strip. Its pane port was "insufficient in live testing".
- The cause they gave was that Chromium calls `SetCursor` continuously, so a 2 ms polling `SetCursor` thread loses the race.
- Their way out was to remove OLE (Pointer Events plus `setPointerCapture`). That removed the slash but regressed drop-zone precision, ghost fidelity and DPI. It was shelved.

Assumptions that were never tested:

- **A.** The only levers considered were `dropEffect`, a competing `SetCursor` or `SetSystemCursor`, or removing OLE. Nobody asked who answers `DragOver` outside our window, or whether the drop source can be wrapped.
- **B.** `dropEffect` was judged on in-window behaviour only. Outside the window Chromium's drop target is never consulted.
- **C.** "Continuous SetCursor" was read as Chromium setting a cursor. Chromium sets none. OLE does, because Chromium asks it to.
- **D.** The 07-29 suggestion to call `SetCursor` from the `WH_MOUSE_LL` hook was never tried. It would lose for the same reason the polling thread did.

## 2. What the sources say now

### 2.1 Chromium and OLE on Windows

- `ui/base/dragdrop/drag_source_win.cc` (chromium main, read 2026-11-04, no commit pinned): `DragSourceWin::GiveFeedback(DWORD effect) { return DRAGDROP_S_USEDEFAULTCURSORS; }`. `QueryContinueDrag` cancels on Escape or `cancel_drag_`, and drops when both buttons are up. https://raw.githubusercontent.com/chromium/chromium/main/ui/base/dragdrop/drag_source_win.cc
- `ui/views/widget/desktop_aura/desktop_drag_drop_client_win.cc` (same date):
  - `StartDragAndDrop` calls `::DoDragDrop(IDataObject, drag_source_, DragOperationToDropEffect(allowed_ops), &effect)`.
  - `drag_source_` is `ui::DragSourceWin::Create()`.
  - `effect` is forced to NONE unless the result is `DRAGDROP_S_DROP`.
  - The file has no cursor code of its own. https://raw.githubusercontent.com/chromium/chromium/main/ui/views/widget/desktop_aura/desktop_drag_drop_client_win.cc
- The Win32 contract (MS Learn, updated 2024-02-22):
  - `DoDragDrop` calls `GiveFeedback` with the effect the drop target returned.
  - After `DragLeave` it calls `GiveFeedback` with `DROPEFFECT_NONE`.
  - `GiveFeedback` "is responsible for changing the cursor shape".
  - `DRAGDROP_S_USEDEFAULTCURSORS` makes OLE pick its defaults. https://learn.microsoft.com/en-us/windows/win32/api/oleidl/nf-oleidl-idropsource-givefeedback
  - A source that wants its own cursor calls `SetCursor` and returns `S_OK`. A VSTGUI developer reports exactly this fix: https://forums.steinberg.net/t/setting-the-cursor-shape-during-a-dragging-session/729857
- Outside our window there is no Chromium drop target. The target is Explorer, another app, or nothing, so the effect is NONE and OLE shows its default "no" cursor. The page cannot influence it. This also explains why `dropEffect` tuning only helped inside the window.

### 2.2 Setting the cursor from outside the drag

- **`SetCursor`** (MS Learn, 2025-07-01): "A window should set the cursor shape only when the cursor is in its client area or when the window is capturing mouse input." The hook thread is neither, and OLE's next `GiveFeedback` overwrites it. That matches the retro's live result. https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setcursor
- **`SetSystemCursor`** (MS Learn, 2024-02-22) replaces a system cursor and destroys the handle. https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setsystemcursor
  - It is machine-wide, survives a crash, and our `restore_drag_cursor` (`SPI_SETCURSORS`) reloads every cursor.
  - OLE's slash probably comes from its own cursor set, not `OCR_NO`. This was not verified, and the 07-29 retro saw no effect. Not recommended.
- **`ClipCursor`** confines the pointer and does not change its shape. Not applicable.
- **A layered window under the cursor** does not help by itself. During `DoDragDrop` the cursor is set by OLE, not by `WM_SETCURSOR` to the window below. It matters only as a drop target (option E).
- **`IDropSourceNotify`** only reports target changes and cannot set the cursor. It is useful for tracing. This is from API knowledge, with no source fetched.

### 2.3 CEF

- `CefDragHandler` has only `OnDragEnter` (return true to cancel) and `OnDraggableRegionsChanged`. We already use both, in `crates/cef/src/client/handlers.rs`. There is no cursor hook. https://raw.githubusercontent.com/chromiumembedded/cef/master/include/cef_drag_handler.h
- `CefRenderHandler::StartDragging` and `UpdateDragCursor` exist only for off-screen rendering (CEF forum and issue tracker threads, dates not retrieved, background only). AgentMux is windowed, and off-screen rendering is not a realistic switch for one cursor.
- A patch to `GiveFeedback` in the CEF fork would work. D does the same from the host without a Chromium patch.

### 2.4 How Chrome avoids it

- Chrome's tab tear-off is not an OLE drag.
  - `TabDragController` detaches the tab into a new window and runs a move loop on it.
  - On Windows that is `HWNDMessageHandler::RunMoveLoop`, which calls `ReleaseCapture()` and sends `WM_SYSCOMMAND` with `SC_MOVE` (chromium main, read 2026-11-04). https://raw.githubusercontent.com/chromium/chromium/main/ui/views/win/hwnd_message_handler.cc
  - The window under the pointer is the dragged window itself, so the cursor is an ordinary arrow.
  - The raw `tab_drag_controller.cc` fetch returned 404. The `RunMoveLoop`/`EndMoveLoop` chain comes from search snippets and https://issues.chromium.org/issues/40598679.
- Wayland confirms the pattern. Chrome uses `xdg-toplevel-drag-v1`, a staging protocol where the window follows the cursor during a real drag. Since Chromium 133 it falls back to a plain drag with a snapshot icon. https://wayland.app/protocols/xdg-toplevel-drag-v1
- `docs/reports/REPORT_LIVE_TEAROFF_CHROME_STYLE_2026_11_04.md` (§5.2, §5.3) reached the same picture:
  - Spike 1 moves a window from the hook thread during OLE, and it works.
  - Spike 2 aborts OLE with a synthetic Escape and then runs our own move loop. It works mechanically.

### 2.5 Electron, VS Code, others

- VS Code has the same slash and has not fixed it.
  - Issue 198128 (Windows 11 and macOS) was closed "not planned" on 2023-12-01. https://github.com/microsoft/vscode/issues/198128
  - The maintainer said the feedback comes from the target app and that changing it could cause side effects there.
  - Issue 245145 reports the forbidden cursor on Linux (date not retrieved). https://github.com/microsoft/vscode/issues/245145
- That maintainer argument shows the only safe options. Either we become the target under the pointer (E), or we override only the "nothing accepts" case (D). Neither tells a third-party app it may accept the drop.
- No Electron or CEF app that fixes this was found. Electron's source for a drag-source hook could not be read, so "nobody does it" is likely but unproven.

### 2.6 macOS and Linux

- **macOS.** AppKit picks the cursor from the destination's operation. The source gets `draggingSession:endedAt:operation:` only after the end, so `NSCursor` calls cannot change it mid-drag. https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/DragandDrop/Concepts/dragsource.html
  - We already disable the slide-back app-wide (`crates/cef/src/macos_compat.rs`, `disable_macos_drag_slideback`).
  - No cursor complaint is recorded for macOS.
  - The equivalent of E would be an `NSDraggingDestination` on the picture window.
- **Linux X11.** Chromium's `XDragDropClient` picks the cursor from the target's `XdndStatus`. `DRAG_NONE` gives the no-drop cursor, and the cursor is swapped by re-grabbing the pointer. The equivalent of E is an `XdndAware` window under the pointer that accepts.
- **Linux Wayland.** The compositor draws the cursor from `wl_data_source.action`. There are no global coordinates and no host-side picture window. Only `xdg-toplevel-drag` helps, and CEF does not expose it. We already hide the drag image there (`frontend/app/drag/wayland-drag-image.ts`). Leave it as is.

## 3. Options

Today's state:

- The `WH_MOUSE_LL` hook is in `crates/cef/src/commands/tear_off_hook/windows.rs`.
- `crates/cef/src/commands/live_follow.rs` shows a picture window (`WS_EX_LAYERED | WS_EX_TRANSPARENT`, `WM_NCHITTEST` returns `HTTRANSPARENT`) on its own thread, moved with `SWP_ASYNCWINDOWPOS`.
- The picture shows below the tear line for tabs, and outside every AgentMux window for panes.

| # | Option | Removes slash on Windows? | What breaks | Effort | Risk |
|---|---|---|---|---|---|
| A | `dropEffect` on `dragover` (today, `frontend/app/tab/tab-reorder.ts`) | Inside our window only | It can overwrite a drop target's own effect (the 07-29 Codex P2) | done | none |
| B | `SetCursor` from the hook or a poller | No: OLE resets the cursor on every `DragOver`/`GiveFeedback` | Nothing | 1 day | wasted effort, already failed live |
| C | `SetSystemCursor(OCR_NO)` | Unlikely, unverified | System-wide, survives a crash, resets the user's cursors | 1 day | high, do not ship |
| D | **Wrap `IDropSource`**: patch the `DoDragDrop` import so `GiveFeedback` returns `S_OK` after `SetCursor(arrow)` when the effect is NONE during an AgentMux tear-off drag | Yes, in every zone | Nothing in the drag: same HTML5 drag, hook, Escape and cross-window drops. Over a foreign app that rejects the drop we show an arrow and a release tears off, as today | 2 to 4 days | Medium: in-process import patch of `libcef.dll` (check it is not delay-loaded), a small COM object that must forward `QueryContinueDrag` |
| E | **Drop target on the picture window**: remove `WS_EX_TRANSPARENT`, answer `HTCLIENT`, `OleInitialize` on the picture thread, `RegisterDragDrop` returning `DROPEFFECT_MOVE` or `COPY` | Yes, wherever the picture is under the pointer. Not where it is hidden (above the strip, or before the tear line) | `dragend` now reports `move`/`copy` instead of `none`, so `frontend/app/drag/native-cancel.ts` (`endedByEscape`) and `maybeDroppedInAnotherWindow` need rechecking. The picture blocks hit-tests under it, which is fine since it is hidden over AgentMux windows. Cross-thread COM calls | 3 to 5 days | Medium: unverified that OLE's window lookup finds a per-pixel-alpha layered window |
| F | Cancel the HTML5 drag when the pointer leaves, then continue with our own session (spike 2) | Yes, no OLE | Out-and-back cannot resume. Pane drop zones in other windows need hook hit-testing instead of DOM targets. `dragend cancelled:true` must be told apart from a real Escape | 1 to 2 weeks | High |
| G | Full native pointer drag (the shelved July branch) | Yes | See §4 | 3+ weeks | High |
| H | Patch `GiveFeedback` in the CEF fork | Yes | Fork maintenance at every Chromium bump | 1 week+ | Medium, and D does the same without it |

## 4. Why the native pointer drag was shelved, and what changed

Reasons, from the 07-29 retro:

1. Drop-zone precision: the nested browser hit-test became a nearest-center guess.
2. Ghost fidelity and DPI: a DOM ghost replaced the OS drag image and rendered larger on HiDPI.
3. `pointermove` volume needed rAF coalescing.
4. A Solid disposal bug after tearing off the dragged node. The cause of the "blank window" was never found.
5. Redock latency (pre-existing per-move IPC) became reachable.
6. The cursor-freeze fix (`SetCursor` under capture) was never shown live.
7. Escape, cancel-back, pin/close and multi-monitor were never covered.

What today's infrastructure changes:

- **Items 5 and 7 (redock, Escape), plus cross-window merge.** The hook tracks the cursor, emits hover and merge events without per-move IPC, and sees Escape (`tabdrag:escape-pressed`). The picture follows at about 12 to 15 microseconds per move in the spike.
- **Items 1, 2, 3.** A hybrid (D or E) keeps the HTML5 drag, so these never arise. Only a full replacement (G) has them.
- **Items 4 and 6.** Independent of this decision.

So the shelving reasons mostly disappear if we do not replace the drag. They remain for F and G.

## 5. Recommendation and plan

Do D first, keep E as the fallback, and do not do F or G just for the cursor.

- **Why D over E:** it works in every zone (the picture is hidden in some), it does not change what the page sees at `dragend`, and its state lives in one small module.
- **Why E is only the fallback:** it needs OLE's window lookup to accept a per-pixel-alpha layered window. MS documents `WindowFromPoint` only as skipping hidden or disabled windows, so this is unconfirmed. https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-windowfrompoint (2025-07-01)

### 5.1 Quick experiment (debug build, dev instance; steps only, not run)

Goal: prove that wrapping the drop source removes the slash, with minimal code behind an env var.

1. Run `dumpbin /imports` on the CEF build's `libcef.dll` and confirm `DoDragDrop` is in the normal `ole32.dll` import table, not delay-loaded. If it is delay-loaded, the patch target is the delay-import table. Record the finding.
2. Add a Windows-only module `crates/cef/src/commands/drop_source_wrap.rs`: a COM object implementing `IDropSource`.
   - `QueryContinueDrag` forwards unchanged.
   - `GiveFeedback(effect)` forwards to the original. If `effect == DROPEFFECT_NONE` and a static `WRAP_ACTIVE` flag is set, it calls `SetCursor(LoadCursorW(null, IDC_ARROW))` and returns `S_OK`. Otherwise it returns the original's result.
   - `windows_sys` already has the `Win32_System_Ole` feature (`crates/cef/Cargo.toml`). The vtable is written by hand.
3. Add a replacement `DoDragDrop` that wraps `source` and calls the real one. Patch `libcef.dll`'s import entry once at startup (`VirtualProtect`, write the pointer, restore). Gate it on a debug build and on `AGENTMUX_DND_CURSOR_EXPERIMENT=1`.
4. Set `WRAP_ACTIVE` where hook sessions start and clear it where they end (`start_tab_drag_tracking`, `start_pane_drag_tracking` and their stops in `tear_off_hook/windows.rs`).
5. Log each distinct `effect` passed to `GiveFeedback`, and each override, at debug level. This shows the real sequence of COPY/MOVE inside and NONE outside.
6. Launch the isolated dev instance as in the repo's `run` skill (own data home, free-port check), with the window not maximised. Then:
   - a. Drag a window tab past the strip and out onto the desktop. Expect an arrow and no slash.
   - b. Drag a pane header out onto the desktop, then onto Explorer. Expect an arrow, and a release tears off.
   - c. Drag out and back in. Reorder still works.
   - d. Drag a tab onto another AgentMux window's strip. It merges as before.
   - e. Press Escape outside. The drag cancels, the picture disappears, nothing tears off.
   - f. After release, confirm the system cursors are untouched. We never call `SetSystemCursor`.
   - g. Repeat with the env var unset as a control. The slash is back.
7. **Pass criterion:** a to c show no slash, and d to f match the control. If the slash survives while the override is logged as applied, something sets the cursor after `GiveFeedback`, and we move to E.

### 5.2 Incremental plan

As shipped in #6498: PR 1 and PR 2 were merged into one change. The wrapper is installed at startup by default, active only while a tear-off hook session runs, and `AGENTMUX_DND_CURSOR=0` is the opt-out (the control run in §5.1 step 7g). The `AGENTMUX_DND_CURSOR_EXPERIMENT=1` gate and the off-by-default setting below were not used. The `tab-reorder.ts` `dragover` hack is untouched and can be removed once the cursor is confirmed live.

1. **PR 1:** the wrapper behind a setting, off by default. Include a unit test of the pure effect-to-cursor decision and the `dumpbin` finding. No frontend change.
2. **PR 2:** turn it on for tab and pane drags. Remove the Windows `dragover` hack in `tab-reorder.ts` if it proves redundant. That also fixes the Codex P2 about overwriting a strip drop effect.
3. **PR 3 (optional):** use the wrapper's `QueryContinueDrag` for Escape and drop the synthetic-Escape path. Log `IDropSourceNotify` for diagnostics only.
4. Delete the dead `set_drag_cursor` / `restore_drag_cursor` (already planned in the consolidation spec).
5. macOS and Linux X11: only if the operator asks. Each needs a destination window and its own spike. Wayland: no change.

### 5.3 Risks to state before building

- A CEF bump may change `libcef.dll`'s imports. The patch must fail soft: log it and leave the slash.
- Antivirus may flag import patching. Ship it behind the setting first.
- If drag data includes `text/plain` or a URL, a terminal or editor under the pointer could accept the drop. D overrides only NONE, so it is unaffected, but check what the pane and tab drags put in `dataTransfer`.
- A plain arrow is a choice. A distinct "opens a new window" cursor is possible later, since any `HCURSOR` will do.

## 6. Sources and dates

- Chromium `drag_source_win.cc`, `desktop_drag_drop_client_win.cc` and `hwnd_message_handler.cc` (main, read 2026-11-04, no commit pinned).
- MS Learn: `IDropSource::GiveFeedback` (2024-02-22), `SetSystemCursor` (2024-02-22), `SetCursor` (2025-07-01), `WindowFromPoint` (2025-07-01), `RegisterDragDrop` (2025-07-01; needs `OleInitialize` and a thread that pumps messages).
- CEF `cef_drag_handler.h` (master, 2026-11-04).
- VS Code issues 198128 (closed 2023-12-01) and 245145. Wayland `xdg-toplevel-drag-v1`. Chromium issue 40598679. Chromium `x11_drag_drop_client.h`.
- Not retrieved or not verified: Electron's drag source; Chromium `tab_drag_controller.cc` (404); the cursor bitmaps OLE uses per effect; `WindowFromPoint` behaviour for per-pixel-alpha layered windows.
