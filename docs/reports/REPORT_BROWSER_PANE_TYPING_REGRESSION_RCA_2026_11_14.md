# RCA: a click in a browser pane's page did not select the pane, and typing went nowhere (Windows, 0.60.26)

**Date:** 2026-11-14. Code read at `origin/main`, logs from a task dev build on Windows 11, and from the operator's own clicks in it.
**Task:** #47567. **Author:** Camper@narko.
**Status:** draft. The cause of the first section is shown by the code and the host log. Section 3 is a reading of one logged exception, not a proof, and the fix's own live check settles it.
**Readers:** AgentY, the operator, whoever reviews the overlay fix.
**Publishing note:** ships in the pull request that fixes the overlay path. It cites no private-repo material.

The operator's words (2026-11-13, relayed by AgentY): "cannot type into browser panes at all", "clicking on the browser doesnt select it. but clicking on the pane header does", and, to Camper: "typing still not working in the browser. we had this a long long time ago, i thought it was fixed for good". And 2026-11-14: "why was the browser pane type changed? whats the rca for the broken browser pane?"

## 1. What happened, in one paragraph

On 2026-11-12 browser panes on Windows changed kind by default: from child windows (the page is a native window inside the pane's own native wrapper) to CEF Views overlays (the page is drawn by the app window's own compositor, as on macOS and Linux). The reason was resize: a child-window pane is composed by the OS a frame or more apart from the pane border, and an overlay is not. The overlay kind had one gap that the child-window kind had closed a month earlier (2026-10-15): when the user clicks in a page, nothing tells the window's input owner that the user chose that pane. So the pane is not selected, the keyboard stays with the app window, and the keys go to the app and never to the page. Clicking the pane's header works, because the header is app content and says "select this pane" itself.

## 2. What a native click on a pane page does, in each pane kind

The input owner (`crates/cef/src/input_owner/`) is the one place that decides which of the window's surfaces has the keyboard: the app, or one pane. It acts on events: a pane was selected, a pointer went down in a pane, a surface gained or lost focus. A click in a pane's page must become one of them, or the keyboard does not move.

**Child-window pane (before 0.60.26, and again by default after #7235).**
- The page's native window (`Chrome_RenderWidgetHostHWND`) sits inside a wrapper window that AgentMux owns. A click goes to the page's window, which AgentMux has subclassed (`browser_pane/hwnd.rs`).
- The subclass sees the button-down message first, and reports a pointer-down in this pane to the input owner (`views::pane_pointer_down`, run at once on the UI thread). The owner gives the pane the Win32 and CEF focus before the click lands, so the next key goes to the page.
- Chromium creates a new page window on every navigation, so the subclass is installed again at the end of every page load (`on_load_end_browser_pane`). That is the part of the earlier fix that was hard to get right, and it is why this bug was "fixed for good" once.
- Seen in this task's dev log with child-window panes: a click gives `pane-pointer-down(pane:...)`, `emit-clicked`, and focus on the pane within a few milliseconds.

**Overlay pane (the default from 0.60.26, still available with `AGENTMUX_PANE_BACKEND=views`).**
- The pane is a Views browser view added to the app window. It has no wrapper window of AgentMux's and no subclass. `on_load_end_browser_pane` skips the re-subclassing for overlay panes on purpose (`!browser_is_views_pane`).
- The page is still a native `Chrome_RenderWidgetHostHWND` child of the app window's own window, so a click goes to that page window and the renderer. No AgentMux code sees the button-down message.
- The only path from a click in the page to the input owner is CEF's `on_set_focus` with the source set to the system (`pane_chosen`, `client/handlers.rs`). That fires only if Chromium asks to give the page focus as a result of the click.
- Seen in the dev log with overlay panes (the operator's clicks, 2026-11-14, 02:19 to 02:52 UTC): the input-watch line reads `click in win=main went to <hwnd> class=Chrome_RenderWidgetHostHWND; views focus: app=true pane:...=false`. No `on_set_focus` with a system source follows, no `select(pane)` follows, the pane view has no focus, and the app keeps it. The keys go to the app.

So the keys never "went to the wrong place" by a bug in routing. The input owner was never told the user had picked the page, and its answer to "who has the keyboard" stayed the app.

## 3. Why it is intermittent

In the same log one click (02:52:46) left `pane=true`, so the click selected the pane and the typing worked. The reading that fits what is logged: a click gives the pane focus when Chromium itself takes focus for the page window as part of handling it, which happens for a page window that has just been created (the first click after a navigation or a restore), and not for a page window that already exists but is not the focus window. The first is the case that the earlier tests, which opened a pane and clicked once, saw work. The second is every later click in a page the user is reading.

This is a reading of one logged exception, not a proof. The fix does not depend on it: it reports the click itself, whatever Chromium does next (section 7).

## 4. Why the earlier fix for child windows did not carry over

- It was a subclass of the page's window (#4976 and the input-watch in #4982). It lives in `browser_pane/hwnd.rs`, runs only for child-window panes, and is re-installed after each load. An overlay pane has no wrapper and no subclass, so nothing of it applies, and nothing fails: no error, no log line, no test, since the code simply never runs.
- When the overlay kind was added (#6371, opt-in), the click path for it was `on_set_focus`. That is how Linux gets a click, and #7024 (2026-11-11) turned it on for Windows overlay panes on the reading that Windows would behave as Linux does. On Linux a pane's page is part of the Views focus system, so a click moves Views focus and CEF reports it. On Windows the page is a native child window, and a click on it does not go through the Views focus system the same way. That difference is what the reading rested on and nobody had checked it with a pointer.
- The platform's own side effect was invisible: a native click on a native child window is not a message the app window's procedure gets.

## 5. Why it shipped

- #7024 said so in its own words: "Not checked: the click, which needs a real click in a pane's page; the code path is the one Linux already uses." It was a reading of the code, listed as unchecked, merged as opt-in, where an unchecked item costs nothing because nobody has the switch on.
- #7069 (2026-11-12) flipped the default. Its body lists what was not checked on Windows: "they need a real pointer and keyboard, and are the operator's part", with resize, hover and clicks in and around a menu, and "typing with a menu open". It did not list a plain click and a plain key in a page. The operator's own try was of the resize look ("I did try the browser lock and approved it. it worked well"), and the PR said so. The check that would have found this (click in a page, type) was neither on the list nor done.
- What was checked before the flip did not touch the click path: unit tests of the setting and the input owner, a look at the host's reported pane kind, and a picture of two panes inside their borders. A synthetic mouse event sent over the debug port goes to the page's renderer directly and not through the native window message that a real pointer produces, so a check made that way cannot show a missing report of a native click either.
- The input-watch warning that should have shown it was noise: it read every key typed in an overlay pane as "went to the app" and logged that the owner believed someone else had the keyboard. That is wrong for an overlay pane (all panes share the app window's one handle), so the line had been firing on correct runs, and a line that fires on correct runs is not read. #7201 (2026-11-13) fixed that and added a line that says which view has the focus when a click lands. That line is what made this repro readable from the log.
- The release (0.60.26) carried it to the operator before anyone had typed in a page with the default.

## 6. What would have caught it

1. **A rule that a default flip waits for its listed real-input checks.** #7069 said what was unchecked and merged. The flip should not merge, or should merge behind a switch left off, until a person has clicked in a page and typed on the build with the new default. That one check is cheaper than the release was.
2. **A test with native input.** A test that sends a button-down message to the page's own native window (not a debug-port event, not a posted key) and asks the input owner who has the keyboard. It fails on the overlay code at `origin/main`, because the owner is never told. The overlay fix (#47567) carries its decision as unit tests with a fake window system (`input_owner/native_click_tests.rs`: which pane a button-down names, and that the owner then gives that pane the keyboard), and was checked live: a left button-down posted to an overlay pane's `Chrome_RenderWidgetHostHWND` in a dev build with `AGENTMUX_PANE_BACKEND=views` logs `[input] native click in pane:...`, the owner goes from `want=app have=app` to `have=pane:...`, and the window selects the pane. The same post to the app's page selects nothing. The unit tests cannot place a click on the real window; only that live check does.
3. **A line in the PR template for a change of default:** which of mouse, touch, keyboard-only, a hand-off pane and a second window were done with real input.
4. **Keep log lines honest.** A warning that fires on correct runs hides the one that matters; #7201 removed one such line.

## 7. What is done and what is not

- Done: #7235 makes child-window panes the default on Windows again, and `AGENTMUX_PANE_BACKEND=views` opts into overlays. With it a click selects the pane (seen in the dev log) and the operator typed in a page in that build.
- Done (#47567, the PR that carries this report): the overlay fix. `input_owner/native_click.rs` reports a button going down (left, right, middle, and touch or pen as `WM_POINTERDOWN`) in a shown overlay's page, outside any menu cut-out, to the input owner from the UI thread's message hook (`watch.rs`), as the child-window subclass does for its kind; `views::pane_clicked` selects the pane, gives it the keyboard and tells the window (`browser-pane-clicked`). Every way in: mouse and touch by the hook; a second window by its own window handle (`label_for_hwnd` of the click's root window) and its own panes; a floating window's pane is a child-window pane by default, which already had its subclass; a pane handed to the person is an ordinary pane; tab-in has no click and still arrives by CEF's `on_set_focus`. Overlays stay off by default until the operator has tried a build with it; the PR does not change the default.
- Not covered, on purpose: a click that lands in the cut-out of a pane under a menu belongs to the app's page and is left to it; a keyboard-only entry has no click to hook.
- The one to three second freeze around a click and a navigation that was open here is not this path: it is #47585, a process started on the UI thread by every load in a dev build (`is_dev_self`), so installed builds do not have it. Its own PRs: the fix (Camper) and the permanent `[pane-open]` timing (#7257).
