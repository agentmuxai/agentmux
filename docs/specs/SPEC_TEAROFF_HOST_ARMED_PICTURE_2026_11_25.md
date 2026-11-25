# Spec: tear-off, the host arms the pane's picture at the press (#48707)

**Status:** Draft, for the platform lead's decision on the spike (section 4). Not committed; it ships with the first PR that implements it.

## 1. What is wanted

A drag of a pane starts drawing at once, a plain click on a pane header costs nothing the person can feel, and the long frames at the start of a drag go. Pass numbers (from the brief): the drag's drawing is held 0 ms in every drag; no long frame over 50 ms that comes from the picture or the ghost, in drags or clicks; the window is shown no later than 314 to 406 ms after the drag; checked with the same script on a Windows test host (6 drags started 90 ms after the press, 3 ordinary, 10 plain clicks, cold start and warm).

## 2. What the measurements settled

| Picture taken | Plain click | Drag started 90 ms after the press | Ordinary drag |
| --- | --- | --- | --- |
| at the press (CDP `Page.captureScreenshot`, now) | pays 150 to 400 ms of host time and holds the page's frames while it runs | held 0 ms | held 0 ms |
| at the first 3 px of movement (tried, reverted) | free | held 112 to 142 ms | held 0 ms |

Both rows pay the same price when the picture is needed: the screenshot takes the host 50 to 130 ms and the page's frames stand still meanwhile. Moving it earlier or later only moves who waits. A host that merely arms and takes the same screenshot at the first movement is the second row again. So the screenshot itself has to get cheap, or it has to leave the page's frames alone.

## 3. Design

**Take the picture natively in the host, at the press, off the page.** The host already owns the window's handle and the thread that follows the pointer (`commands/live_follow.rs`, `tear_off_hook`), and `crates/mcp` already captures windows with `PrintWindow` for agents. A native copy of the window's pixels (`PrintWindow` with the full-content flag, or the window's DWM surface) does not ask the page to render, does not change the viewport (nothing flickers) and is expected to take a few ms to 20 ms for a window this size, against 50 to 130 ms plus held frames for the CDP screenshot.

- **Arm.** At the press on a pane's header the page tells the host once (a call it does not wait for: block id, the pane's rect and the viewport width, whether it is a tab's drag). The host takes the native picture at once and keeps it (one per window, replaced by the next press, dropped after a short time).
- **A click.** It pays the native copy, on a host thread. Pass condition: a group of 10 plain clicks shows no long frame that comes from it. If the copy is too slow to be free, the arm falls back to the movement or hold trigger of the reverted version, with the native copy doing the work (the 90 ms case then waits a few ms, not 112).
- **Cut.** The host cuts the pane out of the kept picture when a drag begins or a tear-off needs it (`pane_picture::cut_pane`, and `live_follow`'s `stage`: the cut and the landing picture are already host code). The page decodes and draws nothing, and nothing goes to the page and back.
- **The drag's drawing gate** (`pane-capture-gate.ts`) stays until the numbers show it never holds; then it goes.
- **Browser panes and a tab dragged alone** keep their own paths (the page's screenshot of a native browser pane; the pill drawn from the window's picture) until the native copy is shown to carry them: the spike checks whether a native copy of the window includes the separate browser surface.

**The drag ghost.** The in-window ghost is the native drag image, which the page must set synchronously at `dragstart`, and it is a restyled clone, not a screenshot of the pane. A host picture cannot be read synchronously by the page, and sending it to the page at the press would put a decode on the page's thread at every press, which is the cost being removed. So the ghost does not use the host picture; the page keeps drawing it, and the filtered clone (only what the ghost shows, #48705) is its fix. Using the host picture for the ghost would also change what the person sees during the drag, which goes to the platform lead first with a screenshot.

**macOS and Linux.** No picture is taken at the press there today and none will be: the host's arm is Windows only, the page does not ask elsewhere, and nothing visible changes, so no look check. The in-window ghost is the shared part (#48705).

## 4. The spike (first, small, measured)

One PR: a native capture function in the host (Windows only) that is used by nothing yet except a debug command and a log line (`[tearoff-perf] native window picture: N ms, WxH, nonblank`), plus the page's one call at the press that only logs. It answers three questions with numbers on the test host: how long the native copy takes at 890 x 900 and larger windows; whether its pixels are right (the pane's content, not black, including with a browser pane); whether a click group shows any long frame from it. Decision after it: go on with arm-at-the-press as designed, or fall back as written above.

## 5. Order

1. The spike (section 4), measured; the numbers decide.
2. The arm and the host-side cut replace the CDP screenshot for a plain pane at the press; the page's capture code for that path goes.
3. Per-drag log lines: press to first drawn frame, press to window shown, and whether the click path took a picture (the gate-hold line stays).
4. Browser panes and tabs on the native copy, if the spike shows it carries them.

Each is one PR with before and after numbers from the same script.
