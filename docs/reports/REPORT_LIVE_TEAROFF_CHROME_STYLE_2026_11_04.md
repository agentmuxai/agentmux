# Report: Chrome-style live tear-off: the torn-off window follows the cursor during the drag

**Status:** analysis. Step 1 of the build (window tab, Windows) is in #6457; the rest is designed here and not built.
**Date:** 2026-11-04
**Author:** Agent2@narko (runs on Area54 by Posa@Area54)
**Task:** #22174 (research). The build is #22175 and waits on the operator's approval.
**Asked by:** the operator, 2026-11-04: *"how is chrome able to tear off with the entire tab showing in the drag? i believe we looked at this before, can we investigate if we can make that possible for agentmux?"* and, on scope: *"lets figure out how reliable the first 3 categories are"* (window tabs, pane bars, pane tabs; document tabs are out).
**Code read:** origin/main `27505b54c` (the tear-off foreground fix #6395 and Redock to origin #6397 are in). Runs: Posa's own `task dev` instance of the same commit, one 900x1600 monitor at 100%, real OS mouse input (SendInput-style moves), host at 80-100% CPU from other builds the whole time. Raw results, screenshots and harness: Posa's `REPORT_22174.md` (copied next to this report as `REPORT_LIVE_TEAROFF_CHROME_STYLE_2026_11_04_EVIDENCE.md`) and three videos.

## 0. The answer in five lines

1. **Yes, on Windows it can be done, and the smallest route is smaller than anyone assumed.** A window can be moved by the host's mouse-hook thread *while the HTML5/OLE drag is still running*: Posa's spike followed the cursor exactly (position = cursor minus offset on all 240 samples), 12-15 µs per move when asynchronous. Nothing in the drag has to be rewritten.
2. **What Chrome does that we don't:** Chrome runs its own drag loop (SetCapture + SetWindowPos per move); we use the browser's drag session (OLE) and create the new window only *after* the mouse is released. The window therefore cannot follow. The May SC_MOVE attempt failed because OLE owns the mouse capture; a plain `SetWindowPos` from another thread does not need capture.
3. **Recommended design ("preview-follow, adopt at release"):** when the drag passes the tear-off threshold, show a warm pool window carrying the picture of the tab or pane that is already taken at pointer-down, and move it with the cursor from the hook thread. On release the existing commit-on-release tear-off runs unchanged and *adopts* that window (it is already in place, painted, and its live content arrives in about 0.3-0.7 s). On cancel the window goes back to the pool and nothing in the data ever moved.
4. **Honest limit:** during the drag the window shows a still picture of the content (as of pointer-down), not live-updating content. Chrome shows the live page. A second route that gives live content during the drag (end the OLE drag, then our own move loop) also works mechanically (spike 2) but rewires how the drag ends and costs about twice as much; section 6.
5. **Today's reliability is good for the tear-off itself** (every kind tore off in all clean runs; Esc cancels back 14 of 15) with four real defects found on the way (section 1.3). Windows only: macOS needs its own spike, Linux works on X11 only, and native Wayland cannot position windows (keep the current fallback there).

## 1. How reliable is today's tear-off (the three kinds)

Posa ran each kind on the agent-style content she could drive (a terminal printing a line every 50 ms stands in for an agent mid-stream; **a browser pane and a real streaming agent were not tested**). Two campaigns: v1 (10 per kind) moved the cursor with `SetCursorPos`, which raises no `WM_MOUSEMOVE` for the host's mouse hook, so v1 timings differ from real use; v2 (5 per kind) used injected mouse moves and is the faithful one for timing and foreground. Cancel runs are v1 only.

### 1.1 Does it tear off every time?

| kind | v1 | v2 | notes |
|---|---|---|---|
| window tab | 20/20 | 4/5 | the v2 miss is the harness (log shows the tear-off completed) |
| pane (header bar) | 10/10 | 4/5 | v2 miss: a leftover floater was under the drop point, so the drop landed on another AgentMux window (the harness did not always close earlier floaters) |
| pane tab | 9/10 | 4/5 | v1 miss: setup failed (no terminal tab); v2 miss as for the pane |

No product failure was proven. Content stayed intact in every torn-off terminal.

### 1.2 Where it lands, whether it is in front, how fast the content shows

| kind | lands | in front at +400 ms (v1 / v2) | reveal after release (v2 median, min-max) |
|---|---|---|---|
| window tab | source-sized (630x1077), **clamped to the work area**; on this screen it cannot sit anywhere else, so drop-point accuracy was not measurable | **0/10** with a streaming terminal, 10/10 with default content / 4/4 | 577 ms (319-629); v1 1140-1277 ms |
| pane (header bar) | floater top-left **exactly at the release point**, not offset by where the pane was grabbed, **not clamped** (a drop at x=850 put a 312-wide floater mostly off a 900 px screen) | 2/10 / 4/4 | 493 ms (414-666); v1 1428 ms (657-3694) |
| pane tab | same as pane | 6/9 / 4/4 | 520 ms (425-536); v1 1091 ms |

Everything was in front within 3 s. The v1 numbers carry the busy host and the missing hook moves; the v2 numbers are the better guide to real use.

### 1.3 What goes wrong (four findings)

1. **A window-tab tear-off is not reliably brought to the front.** `promote_pool_window` (`crates/cef/src/commands/window_pool.rs`, around line 1818) shows the window with `ShowWindow(SW_SHOW)` and never calls `SetForegroundWindow`. #6395 fixed the same gap for floaters only (`pane_pool.rs`). v1 saw 0/10 in front at 400 ms for a window tab with a streaming terminal; v2 saw 4/4, so the failure depends on timing. It is a one-line follow-up using the same helper. (Not part of this research; I will file it as a fix unless told otherwise.)
2. **A floater can land off-screen.** The floater is placed at the release point with no clamp to the monitor work area; the window-tab path is clamped. The follower design below removes the difference because the landing is the follower's last position.
3. **Esc can be ignored right after the drop.** One pane run (1 of 15 cancel runs) tore off despite Esc: the tear-off picture took 1792 ms to capture on the busy host and Esc arrived while it was still being captured (`dragend ... escapeSeen:null`). The picture is taken at pointer-down and the release waits up to 60 ms for it (`tearoff-snapshot.ts`), so this is the host being starved, but it shows the cancel decision is made late.
4. **Pane "cancel" by dropping back is a dock, not a cancel.** Releasing a dragged pane over a tile of its own window docks it there (the layout was rearranged in 5 of 5 runs). That is the intended tile drop, but it means "back over the source" cannot be the cancel gesture for panes; Esc is.

Also seen once: a crowded layout gave a 207x996 floater and shrank the source window by the floater's width (the mother-window resize), not reproduced on purpose. Alt+F4 does not close a torn-off floater while WM_CLOSE does (a floater is an owned popup). No orphan windows or workspaces were left behind by the cancel runs, apart from the one Esc miss.

### 1.4 Cancel

| kind | Esc while outside, then release | back over the source, then release |
|---|---|---|
| window tab | 5/5 back in place | 5/5 no new window |
| pane | 4/5 (the miss above) | 5/5 no new window, but the pane docked (finding 4) |
| pane tab | 5/5 back in its stack, still streaming | 5/5 tab still in its stack |

## 2. What happens today, step by step

**Common to all three:** the drag is an HTML5 drag that Chromium turns into an OLE `DoDragDrop` session, so the browser owns the mouse for the whole gesture. Nothing is moved or created until release (the tab is "committed on release", #2175; the live variant was dropped because SC_MOVE never engaged under OLE and because users did not want the tear before they let go).

**Window tab.** `droppable-tab.tsx` starts a pragmatic-dnd drag and installs the host's `WH_MOUSE_LL` hook in `TabDrag` mode (`start_tab_drag_tracking`, `tear_off_hook/windows.rs`); the hook already sees every mouse move during the drag and emits `tearoff:hover-changed` to a window under the cursor (cross-window merge) but moves nothing. A picture of the window is taken at pointer-down. When the tab is released below the strip, `requestTearOff(skipScMove = true)` (`tab-tearoff-rpc.ts`) calls `WorkspaceService.TearOffTab` (new workspace), then `openTearOffWindow` (a warm window-pool window, positioned from the grab offset, shown with the picture) and ends. The old mid-drag SC_MOVE step is dead code.

**Pane header and pane tab.** `CrossWindowDragMonitor.win32.tsx` listens for the drag; at `dragend` it reads the cursor, asks the host whether another AgentMux window is under it (`startCrossDrag`/`updateCrossDrag`, a `GetWindowRect` test), and either drops onto that window or calls `performTearOff` (pane header: `TearOffBlock` for the pane's members; pane tab: one member through `pane-tab-tearoff.ts`). The saga (`service/tear_off.rs`) now also records the Redock origin. `openFloatingPane` promotes a warm pane-pool window (`promote_pane_pool_window`, `pane_pool.rs`), places it with its top-left at the release point, shows the picture, then reveals the live content. There is **no hook session during a pane drag**; only tab drags have one.

**Where the time goes after release:** window shown at +30-40 ms, brain splash or picture until the reveal gate (v2: 320-670 ms), then the view's own data (`SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md`, phase 1 shipped, phases 2-3 not started).

**The host's manual move loop** (`ui_tasks/drag.rs`: `Win32BeginMoveTask`, `MacWindowDragTask`; `commands/window/motion.rs::start_window_drag`) exists but only moves frameless windows and floaters by their own header; tear-off never calls it.

**History worth knowing** (so nobody repeats it): the May SC_MOVE plan failed under OLE; the July native pointer-capture rewrite (`SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28.md`) worked mechanically but was shelved because replacing the HTML5 drag cost drop-zone precision and ghost fidelity (`docs/retro/retro-native-pointer-drag-tearoff-shelved-2026-07-29.md`). The design below deliberately keeps the HTML5 drag.

## 3. What blocks live follow, per platform

| platform | blocker | state |
|---|---|---|
| Windows | OLE holds the mouse capture, so SC_MOVE and any capture-based move loop cannot start; the window does not exist before release | **solved by moving the window from the hook thread** (spike 1) or by ending the OLE drag first (spike 2) |
| macOS | AppKit windows may only be moved on the main thread; the drag runs in the dragging-session run-loop mode, so a task posted from the event-tap thread (`post_set_window_rect`) may not run until the drag ends. The event tap itself already exists for tab drags (`tear_off_hook/macos.rs`). Chrome uses `performWindowDragWithEvent:` here | **unknown; needs a spike on starpower** |
| Linux X11 / XWayland | no hook exists (`start_tab_drag_tracking` is a no-op); needs a pointer poll (XQueryPointer) and `post_set_window_rect` on the UI thread | untested; plausible |
| Linux native Wayland (GNOME, preferred since CEF 148) | a client cannot position its windows or read the global pointer; Chromium itself falls back to a bitmap drag image there | **cannot be done; keep today's behaviour** (the existing `wayland-drag-image.ts` picture) |

## 4. Spikes (Posa, Windows, throwaway patch to `tear_off_hook/windows.rs`, never merged)

**Spike 1, move another window from the hook thread during the tab drag.** Async (`SWP_ASYNCWINDOWPOS`): the window followed exactly, position = cursor - (200, 20) on every one of 240 samples over 4 s, 12-15 µs per call, one cold first call of 4 ms. Synchronous: same positions, about 0.45 ms per call. The drag still ended normally in both (tab torn off on release, hook uninstalled, no stuck cursor in the log). Frame-by-frame smoothness was not reviewed (video: `video-spike-async.mp4`). Two facts for the build: the hook only sees moves from real mouse input (a harness that uses `SetCursorPos` shows nothing), and a `WH_MOUSE_LL` callback must stay fast (Windows silently removes a hook that answers late), so the follower must use the asynchronous flag.

**Spike 2, end the OLE drag, then our own move loop.** When the cursor left the source window the hook sent a synthetic Esc; the drag aborted (frontend: `dragend cancelled:true` with the button still down, the tab stayed in its strip, no tear-off), and `Win32BeginMoveTask` then engaged on the other window with the button still down and moved it relative to where it started (no jump, not clamped to the screen) until release; mouse-up did nothing more. Cursor shape and visual smoothness were not checked (video: `video-spike-escmove.mp4`).

## 5. The design: one live-follow mechanism, three entry points

### 5.1 The shared part ("preview-follow, adopt at release")

Host (new module next to `tear_off_hook`, Windows first):

1. **Begin** (`live_follow_begin`): called when a drag may tear off; carries the kind, the grab offset, the size to give the window and the picture (already held by `tearoff-snapshot.ts`, at most 1 MB). It takes a warm pool window (window pool or pane pool, by kind) and keeps it hidden.
2. **Show/hide by zone.** Each kind has a tear zone: the follower is shown (no activation, `HWND_TOP`) only while the cursor is in the zone and hidden otherwise, so it never covers a drop target.
3. **Move.** In the hook's `WM_MOUSEMOVE` the follower is moved with `SWP_ASYNCWINDOWPOS` to cursor minus grab offset, clamped to the monitor work area (which also fixes finding 2). The follower is excluded from the hook's candidate test (`dragged_label`) and from the host's window hit-tests.
4. **Preview content.** The pool window shows the picture through the existing `showTearOffSnapshot` path (the reload cover already uses it without a workspace) via a small `pool:preview` event; it needs no workspace until release.
5. **Adopt at release.** The existing commit-on-release code runs unchanged except that `openTearOffWindow` / `promote_pane_pool_window` take the follower's window instead of popping a new one, keep its position (the landing is exactly where the user saw it) and call the foreground helper (finding 1). The reveal starts at the moment of adoption, and for a typical 0.5-1.5 s drag past the threshold the content is nearly ready when the user lets go.
6. **Cancel and merge.** Esc, a drop on another window (merge or redock), or a drop on the source (tab strip, tile) hides the follower and returns it to the pool. No data moved, so there is nothing to restore; the cancel results in section 1.4 stay as they are.
7. **Fallbacks.** No warm pool window, no picture within a budget, or any host error: no follower, today's behaviour. The feature degrades to what ships now.

Everything about the drag itself (reorder, tile drop zones, cross-window hover, Esc handling by the drag session) is untouched, which is why the July rewrite's regressions do not apply.

**Built differently (step 1, window tab).** The follower is not a pool window. A pool window shown early touches the pool queue, CEF Views visibility, activation and the OS drop target, and a cancelled drag would have to give it back. The build uses a click-through, window-sized picture window of its own (`commands/live_follow.rs`) with its own message loop; the hook moves it with `SWP_ASYNCWINDOWPOS` while the cursor is below the strip and not over another AgentMux window, and on release the existing commit-on-release tear-off promotes a pool window at the picture's last position (`landing`) and the picture is dropped. What the user sees is the same. The two parts of 5.1 that stay for later are the warm start (the pool window's renderer starting at the threshold, not at release) and, for panes, the new hook session.

### 5.2 What each entry point needs beyond the shared part

| | window tab | pane header bar | pane tab |
|---|---|---|---|
| pool | window pool | pane pool (floaters; owned popup, taskbar-hidden) | pane pool |
| hook session | exists (`TabDrag`, started at drag start) | **new**: a `PaneDrag` session started at the pane's drag start, same thread/hook | new, same as the pane |
| tear zone | below the strip by the existing `TEAR_PAST_PX`, signalled once by the frontend, and outside all windows | **outside every AgentMux window** (inside, the same drag means a tile drop) | outside every window (inside the window the tab goes to a pane strip or a tile) |
| size / offset | source window size; `tab-grab-offset` | the pane's rect; the pane's `grabOffsetSnapshot` (today ignored for landing) | the tab's pane size (`paneSize`) |
| merge / redock targets | another window's tab strip: the hook already emits `tabdrag:merge-direct`; follower hides while a candidate is hovered | another window's tiles: `CrossWindowDropOverlay` / redock; follower hides over any window | cancels today (no cross-window pane-tab drop); follower hides over a window |
| origin record | none (tab restore uses its own index) | recorded at `TearOffBlock` as now (#6397) | recorded as now, with the stack mate |
| extras | last-tab and pinned-tab rules unchanged | `mother resize` still after release | the source pane keeps its other tabs; nothing changes until release |

The three need no separate mechanisms, only three small adapters (zone, size/offset, which pool and which adopt call). Pane drags are the only ones that need a *new* hook session; the others reuse an existing one.

### 5.3 The alternative: end the OLE drag, then our own move loop (spike 2)

This is what Chrome effectively does: at the tear threshold the drag ends, the real tear-off runs immediately (the pool window is promoted with the real workspace), and `Win32BeginMoveTask` follows the cursor until release; the content is live during the drag. It costs more because the drag session is gone: the frontend sees an abort (`dragend cancelled:true`) which today means "user pressed Esc", so Esc, merge, cancel-back and standalone would all need the (currently dead) `HookMode::TearOff` finalisation revived and re-tested, the data really moves at the threshold so cancel means `RestoreTornOffTab` (tabs) or Redock to origin (panes, now possible thanks to #6397), and the in-window drop zones stop working once the drag is aborted. It is the right second step only if the operator wants the content to *update* while dragging.

## 6. Answers to the five questions

1. **What happens today:** section 2.
2. **What blocks live follow:** Windows: OLE capture (no longer blocking, section 4); macOS: unknown, needs a spike; Linux: Wayland impossible, X11 plausible; section 3.
3. **Smallest change on Windows first:** the preview-follow of section 5.1. About 600-900 lines of Rust and TypeScript with tests: the follower module and its begin/hide/adopt calls in the host; the `PaneDrag` hook session; the `pool:preview` event; the three adapters; the adopt change in the two promote functions (plus the foreground call for the window pool). Roughly 4-6 working days including a harness-driven verification like Posa's (a build on Area54 and a recorded run per kind). macOS and Linux X11 follow as separate, smaller tasks after a spike each; Wayland keeps the current fallback.
4. **How fast the content shows:** the window appears at the threshold with the picture, so the user sees the tab's content immediately (the picture exists from pointer-down). After release the adopted window reveals live content in about 0.3-0.7 s (v2 medians 493-577 ms) and usually earlier, because the pool window's renderer starts at the threshold instead of at release. A pre-bootstrapped pool (phase 2 of the paint-latency spec) would cut it further and is independent of this work.
5. **Interactions:** Redock to origin is untouched by the recommended design because the data tear-off, and so the origin record, still happens at release. With the alternative design, the record is made at the threshold and cancel-back becomes a Redock to origin. The follower must be excluded from every hit test (the hook candidate test, `hit_test_windows`, the floater redock lookup) and hidden over any window that can take the drop so the drop overlays stay visible. The floater's landing position becomes the follower's last position, replacing the unclamped top-left-at-release rule.

## 7. Recommendation, effort, risks

**Recommend building preview-follow for Windows (task #22175), window tab and pane header and pane tab together on the shared mechanism, tab first.** It reuses the drag that works, adds no data risk (nothing moves until release, as today), degrades silently to today's behaviour, and removes two of today's defects (landing/clamp and the late foreground) as a side effect. Proposed order inside the build: (1) shared follower + window tab, verified by a harness run; (2) pane header; (3) pane tab; (4) the macOS spike (Masty/Clare/Maop on starpower) and, if it passes, the macOS port; (5) Linux X11 only if wanted.

| risk | mitigation |
|---|---|
| the hook callback must not block (Windows drops slow hooks) | async move only; no locks held across `SetWindowPos`; counters for slow calls |
| no warm pool window at the threshold | no follower; today's behaviour; log it |
| the picture is late or stale (it took 1.8 s on a starved host) | show the follower only when the picture is ready; otherwise none |
| the follower stealing foreground or covering a drop overlay | `SWP_NOACTIVATE`, zone rules of 5.2, hidden over any window that can take the drop |
| adopting the follower races a second tear-off | one follower per drag; adopt by label, drop it on any error |
| per-DPI and multi-monitor placement | use the existing physical-pixel host path (`get_monitor_work_area_physical`) for clamping; **multi-monitor and unequal DPI were not tested** (the test host has one 100% monitor) |

**What this research did not prove:** the browser pane and a real streaming agent as the torn-off content; window-tab landing without the clamp; multi-monitor; the visual smoothness of the follower frame by frame (positions were exact; the videos exist but were not reviewed frame by frame); macOS and Linux behaviour at all.

## 8. Later items (out of scope, per the operator)

- Document tabs: no Editor-to-Editor live drag (they move only between Editors, #6356).
- A document tab that creates a new pane.

## 9. Files

- This report, and the evidence copy `REPORT_LIVE_TEAROFF_CHROME_STYLE_2026_11_04_EVIDENCE.md` (Posa's full tables) in the same folder.
- Videos from the Windows test host: a window-tab tear-off and the two spikes (not kept in the repo).
- The spikes were a throwaway patch to `tear_off_hook/windows.rs` (not merged).
- Read: `RESEARCH_TAB_TEAROFF_CROSS_PLATFORM_2026-05-07.md`, `SPEC_TAB_TEAROFF_NATIVE_DRAG_LOOP_2026-05-07.md`, `SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28.md` and its retro, `SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md`, `SPEC_FLOATING_PANE_REDOCK_TO_ORIGIN_2026_11_03.md`.
