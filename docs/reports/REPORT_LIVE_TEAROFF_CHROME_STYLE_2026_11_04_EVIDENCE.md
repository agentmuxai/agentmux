# #22174 tear-off runs on Area54 (Posa), dev instance built from origin/main 27505b54c

**Status:** analysis. The raw runs behind `REPORT_LIVE_TEAROFF_CHROME_STYLE_2026_11_04.md`.

Host: Windows 10 19045, ONE 900x1600 monitor at 100% DPI, taskbar 60 px (work area 900x1540), host at 80-100% CPU from other agents' builds the whole time.
Dev window 630x1077 at (135,231). Own `task dev` instance, isolated home, never the operator's.
Driving: PowerShell + mouse_event/keybd_event (real OS input), screenshots, ffmpeg gdigrab. All raw results: `tearoff/results-*.jsonl`, screenshots `tearoff/shots/`.

## READ THIS FIRST: my first campaign did not raise hook mouse-move events
Runs v1 moved the cursor with SetCursorPos. That raises NO WM_MOUSEMOVE for the host's WH_MOUSE_LL hook (button events and keys do arrive). I found this when the follow spike did nothing: the hook never saw a move. v2 uses injected absolute mouse_event moves, which the hook sees. v2 numbers (5 reps per kind) are the faithful ones; v1 (10 reps per kind) is still useful for the drop/landing/cancel behaviour but its reveal and foreground timings differ a lot from v2. Cancel runs (C1/C2) were only done in v1.

## Tear-off results

| kind | run | torn off | foreground at +400 ms | foreground at +3 s | reveal ms (median / min-max) | landing |
|---|---|---|---|---|---|---|
| window tab, default content | v1 10 | 10/10 | 10/10 | 10/10 | 1140 / 629-1500 | (270,462) every time |
| window tab, terminal printing every 50 ms | v1 10 | 10/10 | 0/10 | 10/10 | 1277 / 922-2042 | (270,462) every time |
| window tab, terminal loop | v2 5 | 4/5 (+1 miss in my detection, log shows complete) | 4/4 | 4/4 | 577 / 319-629 | (0,462) |
| pane (header bar), terminal loop | v1 10 | 10/10 | 2/10 | 10/10 | 1428 / 657-3694 | top-left = drop point |
| pane (header bar), terminal loop | v2 5 | 4/5 | 4/4 | 4/4 | 493 / 414-666 | top-left = drop point |
| pane tab, terminal loop | v1 10 | 9/10 (rep 10 = my setup failed, no terminal tab) | 6/9 | 9/9 | 1091 / 540-1607 | top-left = drop point |
| pane tab, terminal loop | v2 5 | 4/5 | 4/4 | 4/4 | 520 / 425-536 | top-left = drop point |

Notes:
- Landing, window tab: the new window is the source's size (630x1077) and lands CLAMPED to the work area, not at the drop point: drops at x 20-100 / y 600-900 gave (270,462) in v1 and (0,462) in v2 (bottom clamp 1540-1077=463). On a 900x1600 screen the window cannot fit anywhere else, so I could not measure drop-point accuracy for window tabs. Unclamped landing not tested.
- Landing, pane / pane tab (floater, 312x497 = the pane's size): the floater's top-left is exactly the release point (drop (60,580) -> window (60,580)), not offset by where the pane was grabbed, and NOT clamped: a probe drop at x=850 gave a floater at (850,620) 312 wide on a 900 px screen, i.e. mostly off-screen.
- v2 failures: pane rep 5 and pane-tab rep 5 (drop (100,900)): log has only "dragend with no drop effect", no tear-off. A leftover floater from earlier reps was under (100,900) (the screenshot v2-pane-5.png shows several), so I think the drop landed on another AgentMux window; my close step (WM_CLOSE, 500 ms) did not always remove floaters. Treat as my harness, not proven product failure. Tab rep 4 is a detection miss (the promoted pooled window already existed as an HWND in my "before" list).
- Content intact: every torn-off terminal kept printing (screenshots v1 and v2). Not tested: a browser pane and a real agent mid-stream (I used the terminal loop as the recipe allows; the seeded fake agent only replies when asked).
- Odd, once: in v1 pane rep 10 the grabbed pane was a different one (tab strip crowded to 19 tabs, different layout): the torn window came out 207x996 and the SOURCE window shrank from 630 to 423 wide (a 207 px "mother resize" equal to the torn pane's width). Not reproduced on purpose.
- Odd: Alt+F4 does NOT close a torn-off floater (floating-pool-...); WM_CLOSE does. Alt+F4 closes torn-off windows.
- Odd: in v1, a window tab torn off while a terminal streamed was not foreground 400 ms after release (0/10) but was by 3 s; with real hook moves (v2) it was.

## Cancel runs (v1 only, 5 per cell; terminal content)

| kind | C1: Esc while outside, then release | C2: back over the source, then release |
|---|---|---|
| window tab | 5/5 back in place, no new window ("cross-window drag aborted via Escape") | 5/5 no new window, tab still in the strip |
| pane | 4/5. Rep 5: Esc NOT honoured, the pane was torn off anyway: log "dragend ... buttonStillDown:false, escapeSeen:null", the tear-off snapshot took 1792 ms (host at 90-100% CPU) so I think Esc arrived while it was still being captured | 5/5 no new window, BUT the pane was docked into the layout at the drop spot (Agent/CPU+terminal/Swarm became Agent/terminal/Supervisor columns). Dropping back on a tile moves the pane; it is not a cancel |
| pane tab | 5/5 back in its stack, still streaming | 5/5 no new window, tab still in its stack |
No orphan windows or workspaces except the one C1 pane rep 5 tear-off (which I closed).

## Spike 1: follow from the hook thread (spike-follow-22174.md sections 1-3)
Patch applied on a scratch branch (never pushed), one file, plus: label/mode/env read from files and the toggle keys (the first attempts did nothing only because of the missing hook moves above; env and file reads work in the host: `env_flag:true, file:Ok("async")` in the log). Target = a second visible window; auto-picked.
- Async (SWP_ASYNCWINDOWPOS): the other window followed the cursor exactly. Sampled every ~30 ms, window position = cursor - (200,20) on every sample (240 samples over 4 s, 40 steps per leg). `[spike] follow` lines: last_us 12-15, worst_us 4046 (one cold first call).
- Sync: same positions on every sample; last_us 420-470 per move (about 0.45 ms each), worst 4046 (that first-call figure is shared across the modes).
- The drag still ended normally in both: tab torn off on release, hooks uninstalled ("hooks uninstalled, thread exiting"), no stuck cursor seen in the log. (Both runs dragged within the source window's bounds, so the tear-off happened at release as usual.)
- Smoothness: I have the positions above, and a video, but I did not review frames for visual tearing: `video-spike-async.mp4`.

## Spike 2: Esc, then our own move loop (section 4)
First attempt did nothing because my path stayed inside the 630x1077 source window (the trigger is "cursor outside the source rect"), second attempt with a path out through the left edge worked:
- At cursor (127,462) (leaving the window): "[spike] outside the source window: Esc, then manual move loop", then "[dnd:tabdrag] ESC pressed - session retired", hooks uninstalled, and "[start_window_drag] manual move loop begin ... anchor=(127,462) origin=(500,950)".
- The drag ended (frontend: "tab drag aborted via Escape", dragend cancelled:true while buttonStillDown:true). No tear-off ran, the tab stayed in the source strip (source title "Tab 2" = the tab still there, no new window).
- The second window then followed the cursor until I released, relative to where it was (no jump): e.g. cursor (60,510) -> window (432,998) = origin + (cursor - anchor). Positions on the samples match that formula. Ended at (673,1938) when the cursor ended at (300,1450): it is not clamped and went below the screen.
- Mouse-up: no `[dnd:tearoff]` finalize line (hook already gone); nothing else ran. Not verified: the cursor shape (I could only read the log), nor the visual smoothness: `video-spike-escmove.mp4`.

## Files
- `tearoff/REPORT_22174.md` (this), `results-*.jsonl`, `shots/`, `video-tab-tearoff.mp4` (v1 tab tear-off, default content), `video-spike-async.mp4`, `video-spike-escmove.mp4`. Harness: `lib.ps1`, `run-kind.ps1`, `spike-run.ps1` (kept on the test host).
- The spike patch stays only uncommitted on branch posa/spike-22174-scratch in my clone; I am reverting it now.
