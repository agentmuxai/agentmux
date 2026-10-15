# Report: Browser pane input focus, the state of the code

**Date:** 2026-10-15
**Author:** AgentX@narko
**Status:** analysis (no code changed; line numbers are against main `18d94fa55`)
**Platform:** Windows first; macOS and Linux differences in §2.4 and §2.5
**Trigger:** Operator report, 2026-10-15, Windows, build 0.59.26 (`b7d2552b5`). A browser pane stopped taking the mouse wheel until the operator switched window tabs and came back. Sometimes keystrokes don't reach a browser pane, or go somewhere else, until something resets it. The operator asked for the state of this code and for "a really rigorous state machine for the browser's input focus".
**Related:** `SPEC_BROWSER_PANE_FOCUS_LOCK.md`, `SPEC_WINDOW_REACTIVATE_FOCUS_RESTORE_2026_05_23.md`, `SPEC_FOCUS_FOLLOWS_SELECTION_2026_10_08.md`, `SPEC_MACOS_BROWSER_PANE_KEYBOARD_FOCUS_2026_09_24.md`, `SPEC_BROWSER_PANE_Z_ORDER_2026_04_21.md`, `INCIDENT_2026_10_07_MULTI_WINDOW_FOCUS_STORM.md`

## Summary

- **No one owns input for a browser pane.** On Windows, about a dozen places in host Rust, frontend TypeScript and Chromium decide where keys go. Nothing records which surface should have the keyboard, and nothing checks that Win32 agrees. Wheel and click routing is a separate mechanism (window z-order, rect and clip region) with no owner at all.
- **In this incident the wheel failure was a hit-testing problem, not a keyboard-focus problem.** From 12:50 to 13:04:30 UTC no wheel message and no click reached any window of the stuck pane (`a2958da2`). Two clicks on it reached the app UI's placeholder under the page instead. Keyboard focus moved to and from the pane normally the whole time. The pane took the wheel again 0.5 s after the tab switch re-sent its rect, which also raises its window to the top.
- **Likely mechanism, medium-low confidence:** something in the main window's own HWND tree sat above the pane's window. The page stayed visible but no longer received pointer input. The pane's z-order is set only as a side effect of creating or moving it. The frontend never re-sends an unchanged rect, so nothing re-raised it until the tab switch. The window was hidden and shown at 13:02:12, which may be the trigger. This is unverified: the log has no z-order data. §9 lists the one diagnostic that would settle it.
- **For keyboard routing, the code has at least five ways to leave keys in a pane the user can't see, or nowhere** (§3.2). The two most direct:
  - Switching window tabs collapses a focused pane to 0×0 without moving focus.
  - The `WM_ACTIVATE` restore picks a child window without checking the frontend's selection, and Chromium undoes it within 0.2 ms; the log shows this at 13:02:11.
  - The rest come from the single global one-shot "allow focus" flag and an off-thread `SetFocus`.
- **Nothing merged after 0.59.26 changes Windows input routing** (§4).
- **Recommendation:** one host-side input-owner state machine per window, owning both the keyboard owner and the pointer-surface invariants:
  - a pure transition table, unit-tested;
  - effects run on the UI thread;
  - `GetFocus()` and `WindowFromPoint()` checked after every transition, with recovery on a mismatch;
  - one log line per transition.
  - Ship the diagnostics and five small fixes first (§8).

## 1. What the operator saw, and what the log shows

Source: the operator's host log for 0.59.26 (2026-10-15, timestamps UTC). The `[pane-wndproc]` lines are logged for every wheel and key message that reaches a subclassed pane window (`hwnd.rs:422-433`), so their absence is meaningful.

Pane windows, matched from creation and load lines:

| block | window tab | Chrome widget HWND | page HWND (wheel target) |
|---|---|---|---|
| `a2958da2` (the stuck pane) | `e46ccbd0` | `0x5e1e3e` | `0xa31e4e` |
| `c6888002` | `e46ccbd0` | `0x14310e8` | `0x7af0dca` |
| `7b792679` (scrolled during recovery) | `99324eb0` | `0x621444` | `0x471d88` |

Main window: top-level `0x54a0582`, main render widget `0x9a20a32`.

Timeline:

| time (UTC) | line (trimmed) | meaning |
|---|---|---|
| 12:49:42 | `tab-reveal whole-tab target=e46ccbd0` | the pane's tab becomes active |
| 12:50:10–12:50:12 | `DragStarted` / `DragEnded outcome=Cancelled`, then `moveNode` | a pane drag, then a layout move in that tab |
| 12:50:23 | `[drag-snapshot] size 717x1755 != pane 0x0; dropped` | the page's real size matched the new layout; the placeholder was 0×0 for a moment |
| 12:51:02 | `WM_KILLFOCUS hwnd=0x14310e8` | the window loses activation (operator away for 11 min) |
| 13:02:11.3758 | `WM_SETFOCUS allowed (intentional)` on `0x14310e8` | the activation restore gives `c6888002` the keys... |
| 13:02:11.3766 | `WM_KILLFOCUS hwnd=0x14310e8` | ...and Chromium's own activation handling takes them back 0.8 ms later |
| 13:02:11.3798 | `[ipc] browser_pane_focus block_id=c6888002` | the frontend's window-focus net puts them back |
| 13:02:12.36 → .73 | `[wave-turn] ... visibility: visible→hidden`, then back | the main window was hidden and shown |
| 13:03:23.109–.224 | 4× `browser_pane_focus block_id=a2958da2`, 3× `giveFocus` | the operator selects the stuck pane; only one `WM_SETFOCUS` follows |
| 13:03:27 | `main-focus-reclaim ... target=0x9a20a32` | a click elsewhere takes the keys back |
| 13:03:31 | the same pattern again (4 IPCs, 3 `giveFocus`) | second attempt on the stuck pane |
| 13:04:27.39 | `browser_pane_focus block_id=d1f989aa`, `tab-reveal ... 99324eb0` | switch to the other tab |
| 13:04:28.62 | `mouse-wheel hwnd=0x471d88` (22 messages) | wheel works in the other tab's pane |
| 13:04:30.93 | `tab-reveal whole-tab target=e46ccbd0` | back to the first tab |
| 13:04:31.37 | `mouse-wheel hwnd=0xa31e4e` (10, then 78 at 13:07) | the stuck pane takes the wheel again |

What this establishes:

- **The wheel never reached the stuck pane.** Its page window got no `WM_MOUSEWHEEL` between 10:43 and 13:04:31, and the post-recovery wheel went to the same HWND (`0xa31e4e`), so the window wasn't replaced in between.
- **Clicks didn't reach it either.** There is no `emit-clicked` for any pane after 12:38. A native click on a pane always logs one (`hwnd.rs:444-459`). Each selection at 13:03:23 and 13:03:31 produced four `browser_pane_focus` calls but only three `giveFocus` lines. The fourth fits the placeholder's DOM `onMouseDown` (`browser-view.tsx:392-408`), which fires only when the click lands on the app UI under the page, not on the page's window. (Inference: the log doesn't say which call came from where.)
- **Keyboard focus did move as asked.** Each request produced `WM_SETFOCUS allowed` on the pane, and each click elsewhere produced `WM_KILLFOCUS` plus a main reclaim. No key messages were logged after 12:47, so this incident says nothing about typing.
- **No overlay-clip call happened between 13:03:38 and the recovery.** A stale `SetWindowRgn` hole is not the cause: the tab switch doesn't touch the clip region, yet it fixed the pane.
- **The recovery coincides with the tab switch re-sending the pane's rect.** The rect goes to 0×0 on the hidden tab (`use-pane-rect-sync.ts:157`) and back on the visible one. `apply_windows` moves the wrapper with `DeferWindowPos` and a null insert-after, which raises it to the top of its siblings (`bounds.rs:426-437`).

## 2. How input reaches a browser pane today

### 2.1 Windows: the window tree

Each pane is a windowed CEF browser inside an app-owned wrapper HWND, which is a `WS_CHILD` of the main top-level window (`browser_pane/wrapper.rs:186-236`). Main's own web content is drawn by the top-level and its `Chrome_RenderWidgetHostHWND` (found by `find_main_render_widget`, `ui_tasks/window.rs:2087`). Pane wrappers and main's child windows are siblings. A tab switch or an inactive pane tab doesn't hide the pane: the frontend sends a 0×0 rect (`use-pane-rect-sync.ts:157`) and the host moves the wrapper (`bounds.rs:402-455`).

### 2.2 Windows: who changes or assumes keyboard focus

| # | Where | What it does | Thread / owner |
|---|---|---|---|
| K1 | `browser_pane/hwnd.rs:326-590` | Subclasses the pane HWND and every descendant. Called at create (`callbacks.rs:294`) and after every load end (`callbacks.rs:503`) | host, UI |
| K2 | `hwnd.rs:494-523` | `WM_SETFOCUS` on a pane: if the global flag is set, let it through and record it; otherwise `SetFocus(GA_ROOT)` and swallow the message | host, UI |
| K3 | `hwnd.rs:291-309` | Redirect refused if another top-level is foreground or within 100 ms per root. The message is still swallowed | host, UI |
| K4 | `hwnd.rs:190`, armed at `:445` (`WM_LBUTTONDOWN`), `clip.rs:431`, `wndproc.rs:253` | `ALLOW_BROWSER_PANE_FOCUS_ONCE`: one process-global `AtomicBool`, consumed by the next `WM_SETFOCUS` on any pane | host, any thread |
| K5 | `hwnd.rs:213-258` | `LAST_FOCUSED_BY_ROOT`: the last "intentional" focus child per top-level | host |
| K6 | `browser_panes/clip.rs:414-442` via `ipc.rs:713-717` | `browser_pane_focus`: `host.set_focus(1)`, arm K4, Win32 `SetFocus(pane)`. **Runs inline on an axum/tokio worker**, not the UI thread | frontend → host, tokio |
| K7 | `ui_tasks/window.rs:1893-2043` via `ipc.rs:1054-1109` | `main_window_focus`: posted to the UI thread. Foreground-only (`:2050`). `set_focus(1)` on main, Win32 `SetFocus(main render widget)`, record K5, `defocus_all` | frontend → host, UI |
| K8 | `clip.rs:23-35` | `defocus_all`: `set_focus(0)` on every live pane (Chromium level only) | host, UI |
| K9 | `client/wndproc.rs:202-271` | Top-level `WM_ACTIVATE`: `SetFocus(K5 child)` **before** passing the message to CEF's own window procedure | host, UI |
| K10 | `wndproc.rs:139-161`, `:213-241` | Hands activation back from an invisible pool window | host, UI |
| K11 | `browser_panes/close.rs:166-169` | After a pane HWND is destroyed: K7 for the foreground window | host, UI |
| K12 | `hwnd.rs:57-145` | Restores window procedures and clears K5 on close | host, UI |
| K13 | `client/handlers.rs:148-174` | Pane `FocusHandler::on_set_focus`: logs, never cancels. No `on_got_focus` / `on_take_focus` anywhere | Chromium → host |
| K14 | `client/navigation.rs:146,518` | `set_focus(1)` on a window's browser when it is first shown | host, UI |
| F1 | `browser-model.ts:787-801` | `giveFocus()` → `browser_pane_focus` | frontend |
| F2 | `browser-view.tsx:392-408` | Placeholder `onMouseDown` → `browser_pane_focus` (only fires when the click misses the native page) | frontend |
| F3 | `browser-model.ts:47-73`, `:562-580` | `browser-pane-clicked` (from K2's `WM_LBUTTONDOWN`) → blur DOM input → `refocusNode` → F1 | host → frontend |
| F4 | `focusManager.ts:85-98` | `ensureSelectionFocused` (tab activated, reveal, orphan, window focus). **Bails when `!document.hasFocus()`** (`:89`) | frontend |
| F5 | `focusManager.ts:312-326` | Window `focus` event → F4 after 100 ms if the caret is on `<body>` | frontend |
| F6 | `block.tsx:266-289` | `focusin` in any block → `reclaimWindowFocus` → K7 | frontend |
| F7 | `browser-nav-bar.tsx:254,626,647` | Address bar mousedown/focus → K7 | frontend |
| F8 | `keymodel-dispatch.ts:231-248` | A host-forwarded app shortcut from a pane → K7, then run the command | host → frontend |
| F9 | `use-pane-rect-sync.ts:246` | Pane created while selected → `claimFocusOnMount` → F1 | frontend |

### 2.3 Windows: who decides where the wheel and clicks go

Windows 10 and later deliver the wheel to the window under the cursor by default ("scroll inactive windows"). Chromium also reroutes a wheel message to the window under the cursor if it belongs to the same process. Either way, the pane gets the wheel only if its window wins hit-testing at the cursor. That depends on:

| # | Where | What sets it |
|---|---|---|
| P1 | `callbacks.rs:275-284` | Raise to `HWND_TOP` at create. Comment: "so mouse-wheel events reach the pane renderer rather than main's" |
| P2 | `bounds.rs:426-437` | Every rect apply raises the wrapper (null insert-after, no `SWP_NOZORDER`) |
| P3 | `use-pane-rect-sync.ts:158-166` | The frontend sends a rect only when it changed. An unchanged pane is never moved, so never re-raised |
| P4 | `clip.rs:58-350` | `SetWindowRgn` holes for DOM overlays. Applied inline on whichever tokio worker takes the request. The frontend fires it without awaiting (`pane-overlay.ts:204`) |
| P5 | `hwnd.rs:402-420` | Ctrl+wheel is consumed for per-pane zoom |

Nothing re-asserts P1/P2 after a window is hidden, shown or activated. Nothing checks which window is under a pane's rect.

### 2.4 macOS

Each pane is an overlay `NSWindow`. Keys reach a page only while its overlay is the key window, so focus there means key status:

- `make_pane_overlay_key`, with a request parked for up to 1.5 s if the overlay isn't shown yet (`ui_tasks/platform_macos.rs:338-440`). `focus()` posts it (`clip.rs:419-420`).
- `reclaim_key_for_window` hands key status back on a click in the app (`platform_macos.rs:297-331`, `window.rs:2033-2037`).
- A `sendEvent:` swizzle re-points the first responder per key event.

The macOS model has more explicit structure than Windows: one key window, a single parked request with a TTL. It has the same problem of several requesters with no owner; #4958 (§4) fixed one race between them. The "redirect page focus" layer doesn't exist there: `callbacks.rs:316-338` and `creation_views.rs:389` return focus to main after create.

### 2.5 Linux

Panes are Views overlays. Focus is `host.set_focus` only: `callbacks.rs:316-338` after create, `pane_geometry.rs:115,367` after bounds. A DOM overlay over a pane hides the whole pane (`clip.rs:518-535`), with a freeze-frame image standing in (`use-freeze-frame.ts`). There is no wheel or key interception and no focus verification.

## 3. The implicit state machine today

### 3.1 States, events and owners (Windows)

Nobody stores these states; they can be read off the code.

| State | Win32 `GetFocus()` | Chromium focus | Frontend thinks | Exit |
|---|---|---|---|---|
| S1 App | main render widget | main | a non-browser pane or the address bar is selected | F1/F2/K2 click → S2 |
| S2 Pane(b) | b's widget | b | b is selected | F6/F7/F8/K7 → S1; window deactivated → S5 |
| S3 Pane(b) collapsed | b's widget, 0×0 | b | another tab or pane tab is selected | **none defined**: only a later K7 from some other path |
| S4 Pane, Chromium unaware | a pane widget | not that pane (`WM_SETFOCUS` swallowed by K3) | anything | **none defined**: only the next explicit focus |
| S5 Window inactive | another app or window | unchanged | unchanged (`document.hasFocus()` false) | `WM_ACTIVATE` → K9 → S1/S2/S3, then Chromium's own handling, then F5 |
| S6 Pending | target not yet focused | `set_focus` posted | already switched | K2 sees `WM_SETFOCUS` → S2, or the flag leaks |
| S7 Closing/destroyed owner | dead HWND or the wrapper | none | — | K11 → S1 |
| S8 Pointer-blocked | any | any | b is visible | **none defined**: the next rect change re-raises (P2) |

Events and their sources:
- Frontend intents: F1, F2, F4, F6, F7, F8, F9.
- Win32 messages: `WM_LBUTTONDOWN`, `WM_SETFOCUS`, `WM_KILLFOCUS`, `WM_ACTIVATE`.
- Chromium's own `SetFocus` calls: page load, `window.focus()`, its activation handling.
- Layout changes: rects, collapse, overlays.
- Window lifecycle: create, close, pool refill, tear-off and redock.

Ownership is split three ways, and two owners often act on the same event:
- On `WM_ACTIVATE`, K9 and Chromium both set focus.
- After window activation, K9 and F5 both act.
- On a click in a pane, K2 and F3 both act, and then F1 calls into K6.

### 3.2 Gaps

| # | Gap | Evidence |
|---|---|---|
| G1 | **Two models of focus, no feedback.** The frontend's model is layout selection plus `document.activeElement`. Win32's is the focused HWND. The host never tells the frontend which surface actually has the keys. The frontend uses `document.hasFocus()` as a proxy, and that is false whenever a pane holds the keys | `focusManager.ts:89,183-185` |
| G2 | **Collapsing a focused pane never moves focus (S3).** A window-tab switch or pane-tab switch sends 0×0. F4's "tab-activated" bails because the main document isn't focused, so the hidden pane keeps the keyboard. Browser panes have no `onDeactivate` | `use-pane-rect-sync.ts:157`, `focusManager.ts:89,353`, `browser.tsx:34-44` |
| G3 | **The `WM_ACTIVATE` restore is undone by Chromium and can target the wrong pane.** It calls `SetFocus` before CEF's own `WM_ACTIVATE` handling, which then takes focus back (log 13:02:11 and 13:02:12, under 1 ms). It doesn't check that the child is a visible, selected pane. It arms the global flag even when the child is the main render widget, which has no subclass to consume it | `wndproc.rs:244-271`, `:315-319` |
| G4 | **One global one-shot flag stands in for "intent".** Every `browser_pane_focus`, every pane click and every activation restore arms it, for any pane. Whichever pane's `WM_SETFOCUS` comes next consumes it. Repeated requests leave it armed: four requests at 13:03:23 produced one `WM_SETFOCUS`, so the flag stayed armed for about 8 s, and any page's `focus()` in that window would have been let through and recorded as intentional | `hwnd.rs:190,445,497`, `clip.rs:431`, log 13:03:23 |
| G5 | **`browser_pane_focus` calls Win32 `SetFocus` from a tokio worker.** Win32 `SetFocus` fails for a window owned by another thread's queue, so focus depends on CEF's own posted `SetFocus`. `main_window_focus` is correctly marshalled. This is inferred from the Win32 contract; the log can't tell the two `SetFocus` calls apart | `ipc.rs:713-717`, `clip.rs:435-437`, vs `ipc.rs:1094-1102` |
| G6 | **A refused redirect swallows `WM_SETFOCUS` (S4).** `WM_SETFOCUS` reports a change that has already happened. Returning 0 without calling Chromium leaves the pane holding Win32 focus while Chromium thinks it is blurred, so keys go nowhere. Redirects and refusals are not logged | `hwnd.rs:515-522` |
| G7 | **Nothing verifies the outcome.** `WM_KILLFOCUS` is only logged. No `GetFocus()` check after a transition, no `on_got_focus`, no `on_take_focus` | `hwnd.rs:429-431`, `handlers.rs:148-174` |
| G8 | **Many uncoordinated requesters; last writer wins.** Logged: 3 `giveFocus` per selection, 2 `main_window_focus` per click. F6 reclaims on every `focusin`, as INCIDENT_2026_10_07 already noted | log 12:50:05, 13:03:23, 13:03:27; `block.tsx:266-289` |
| G9 | **Pointer routing has no owner (S8).** Z-order is set only at create and on rect changes. The frontend never re-sends an unchanged rect, and nothing re-raises after a window hide, show or activation. Nothing checks `WindowFromPoint` over a pane | `callbacks.rs:275-284`, `bounds.rs:426-437`, `use-pane-rect-sync.ts:158-166` |
| G10 | **Overlay clips can apply out of order.** Requests are fire-and-forget and run on any tokio worker. The log shows overlapping applies on threads 3, 17, 32 and 34 at 12:50:09–12:50:11. The per-pane cache lock makes each apply atomic, not ordered. A late stale clip leaves a hole that eats pointer input over part of a pane, and the frontend's dedupe never re-sends the clear. Not the cause here (§1) | `pane-overlay.ts:204`, `clip.rs:58-350`, log 12:50:11.29–.30 |
| G11 | **Logs can't map HWND to pane.** `[pane-subclass]` logs the HWND without the block id. A child HWND that Chromium recreates without a navigation is never subclassed, so its input is neither redirected nor logged | `hwnd.rs:554,583`, `callbacks.rs:496-511` |

### 3.3 The most likely mechanism for each symptom

- **Wheel stuck until a tab switch (this incident).** High confidence it is pointer hit-testing (S8), not keyboard focus: see §1. Medium-low confidence on the trigger. The leading candidate is a child of the main window ending up above the pane's wrapper during the 13:02:12 hide and show. Chromium manages its own child windows and may reposition them on show or resize. The pane stays visible because that window doesn't paint over it. The pane is re-raised only when its rect changes (G9), which is exactly what the tab switch did. The other candidate is a stale wrapper position after the 12:50:12 layout move: the frontend thinks the rect was sent, the host dropped or misapplied it, and dedupe never re-sends it. The size seen at 12:50:23 makes that less likely. Either way, the code has no check that would catch it.
- **Keystrokes don't reach the pane, or go elsewhere.** Nothing in this incident's log, so this is from code reading. The most direct paths, in rough order of likelihood:
  - G2: switch window tabs while a browser pane has the keys, and they stay with the hidden pane.
  - G3: activation hands the keys to whichever pane was last recorded; Chromium then takes them back and the frontend's 100 ms net puts them somewhere else.
  - G6: a refused redirect leaves keys going nowhere.
  - G4: a leaked flag lets a page's `focus()` take the keyboard.
  - Clicking into the right place resets all of these, which matches "until something resets it".

## 4. On main but not in 0.59.26 (`b7d2552b5`)

| PR | Commit | What it does | Effect on this report |
|---|---|---|---|
| #4958 | `8967f8814` | macOS: the window-focus net waits 100 ms and skips if a press happened; parked pane-key requests expire after 1.5 s | macOS only. The 100 ms settle (F5) also runs on Windows |
| #4959 + #4966 | `ab9479d2f`, `18d94fa55` | macOS cursor while a pane overlay is key; #4966 fixes the Windows/Linux build that #4959 broke | None for input. Any build with #4959 needs #4966 |
| #4944 | `74a96afca` | A pane re-attached by a window reload keeps its page | Not focus |
| #4941 | `9284f2f6e` | Window-pool refactor, no behaviour change | None |
| #4937 | `fa7c110c6` | The address-bar suggestion list stays mounted between keystrokes | Not focus |

No open PR touches pane input. Related open issues: #4430 (browser panes as Views overlays on Windows; would change the whole Windows model), #2908 (Ctrl+wheel in floaters), #4136 (main window stops taking input while painting).

## 5. Existing docs and how they relate

| Doc | Covers | Status header | Matches code? |
|---|---|---|---|
| `SPEC_BROWSER_PANE_FOCUS_LOCK.md` | Desync of Chromium vs Win32 focus; proposes per-HWND `is_focused`, `WM_KILLFOCUS` handling, two-layer `defocus_all` | draft | **Never built.** Its description of the code is stale (it says the redirect goes to `GetParent`). Its diagnosis is still right and is G4/G7 here |
| `SPEC_WINDOW_REACTIVATE_FOCUS_RESTORE_2026_05_23.md` | Alt+Tab back drops keys; `LAST_FOCUSED_BY_ROOT` + `WM_ACTIVATE` restore | Draft | Mostly built, but without the `host.set_focus(1)` it prescribes. The restore's ordering problem (G3) isn't covered |
| `SPEC_FOCUS_FOLLOWS_SELECTION_2026_10_08.md` | The caret follows the selection: observers, retry, `<body>` safety net | active | Matches. R5 (Windows browser reclaim) is correctly listed as not built. Doesn't address panes holding Win32 focus (G1/G2) |
| `SPEC_MACOS_BROWSER_PANE_KEYBOARD_FOCUS_2026_09_24.md` | macOS key-window model | implemented (#3737) | Matches |
| `SPEC_PANE_CLICK_THROUGH_INPUT_FOCUS_2026_09_23.md` | One click selects a pane and keeps the caret (`userCaretInBlock`) | implemented (#3621) | Matches; §8's `isMainInput` note is stale |
| `ANALYSIS_BROWSER_PANE_CLOSE_FOCUS_STORM_2026_06_30.md` | Stale subclass after close caused a storm | analysis | Fix shipped (K12, `window.rs:2000`); status is stale |
| `REPORT_INPUT_FOCUS_STOLEN_BY_POOL_REFILL_2026_10_03.md` | Pool windows activate during creation | analysis | Fix shipped (K10); status is stale |
| `REPORT_FOCUS_ON_OPEN_AUDIT_2026_10_08.md` | Inputs that should take the caret when opened | active | Frontend only; its one browser row is still accurate |
| `INCIDENT_2026_10_07_MULTI_WINDOW_FOCUS_STORM.md` | Windows activating each other forever | retro | Fix matches (`window.rs:1932-1957`). Its open follow-up (F6 on every `focusin`) is G8 |
| `SPEC_BROWSER_PANE_Z_ORDER_2026_04_21.md` | Airspace and z-order | Draft | Fix B was never built (replaced by the `SetWindowRgn` clip). Correctly states that wheel routing depends on z-order; nothing maintains it (G9) |
| `ANALYSIS_BROWSER_PANE_REDOCK_BLACK_TYPING_LOCK_2026_06_15.md` | Focus orphaned when a pane HWND dies | analysis | Defect A fixed (K11, `forget_focus_for_child`); path references are stale |
| `reports/CEF_PANE_SCROLL_RESEARCH.md` | Claims the wheel goes only to the focused window; proposes removing the redirect | none | **Contradicted.** The pane hook receives the wheel directly, and none of its recommendations were adopted |
| `SPEC_PANE_FOCUS_STRESS_TEST.md` | 24-step click-then-type workload | draft | The harness exists as `tools/tests/pane-focus-stress.ps1`. The global-flag race it names is G4, still present |
| `SPEC_BROWSER_PANE_LIFECYCLE.md` | State table and race list | analysis | Its focus races #2, #5 and #8 are resolved; flag and file names are stale |

No doc describes the whole input model, and none treats pointer routing and keyboard focus as one ownership problem.

## 6. Proposed design: one input-owner state machine per window

### 6.1 Where it lives

In the host, as one module, for example `crates/cef/src/input_focus/`:
- a pure core `machine.rs` with no Win32, CEF or locks: `fn step(&mut WindowInput, Event) -> Vec<Effect>`;
- a per-platform adapter that turns Win32 messages, IPC and layout changes into events, and executes effects on the UI thread.

It follows the host reducer pattern the code already uses (`crates/cef/src/reducer/`).

Why the host: Win32 focus, activation, z-order and hit-testing are facts only the host's UI thread can observe or change. The frontend can't see them; `document.hasFocus()` is false whenever a pane has the keys. The frontend keeps owning **selection** (what the user means); the host owns **input** (which surface receives it) and reports it back.

### 6.2 State (per top-level window)

```text
WindowInput {
  active:   bool                      // this window is the foreground window
  want:     Owner                     // latest selection from the frontend (App | Pane(b)), with its seq
  have:     Owner | Pending(Owner, deadline, attempt) | Lost(observed_hwnd)
  panes:    block -> { life: Creating | Live | Collapsed | Closing, rect, tree: [hwnd], z_ok: bool }
}
Owner = App | Pane(block)
```

`want` replaces the frontend's scattered `giveFocus` and `reclaimWindowFocus` calls. `Pending` replaces `ALLOW_BROWSER_PANE_FOCUS_ONCE`: it is per window, names one target, and has a deadline.

### 6.3 Events

| Event | Source |
|---|---|
| `Select(owner, seq, reason)` | frontend: one IPC `input_select { window, target, seq, reason }`, replacing `browser_pane_focus` and `main_window_focus` |
| `PanePointerDown(b)` | `WM_LBUTTONDOWN` in b's tree |
| `FocusGained(hwnd)` / `FocusLost(hwnd)` | `WM_SETFOCUS` / `WM_KILLFOCUS` in a pane tree, plus a subclass on main's render widget |
| `Activated` / `Deactivated` | top-level `WM_ACTIVATE`, handled **after** the original window procedure (posted) |
| `PaneShown(b, rect)` / `PaneCollapsed(b)` / `PaneClosing(b)` / `PaneDestroyed(b)` | rect apply, close paths |
| `WindowShown` / `WindowRestored` | `WM_SHOWWINDOW`, `WM_SIZE` restore |
| `Verify` | a timer the machine itself schedules after each transition |

### 6.4 Transitions (the core of the table)

| From | Event | To | Effects |
|---|---|---|---|
| any | `Select(t)`, `t` Live and shown, `active` | `Pending(t)` | CEF `set_focus(1)` on t, `set_focus(0)` on the previous owner, Win32 `SetFocus(t's widget)`, schedule `Verify(50 ms)` |
| any | `Select(t)`, not `active` | unchanged, `want = t` | none (applied on `Activated`) |
| `Pending(t)` | `FocusGained(h)`, `h ∈ tree(t)` | `t` | let the message through to Chromium; log |
| `Pending(t)` or `t` | `FocusGained(h)`, `h ∉ tree(t)` (a page's own `focus()`) | unchanged | pass the message through, then post `SetFocus(t's widget)`. **Never swallow** `WM_SETFOCUS` (G6) |
| any | `PanePointerDown(b)` | `Pending(Pane b)`; tell the frontend to select b | as `Select` |
| `Pane b` | `PaneCollapsed(b)` or `PaneClosing(b)` | `Pending(App)` | as `Select(App)`. Fixes G2 and K11 in one rule |
| any | `Deactivated` | unchanged, `active = false` | none |
| any | `Activated` | `Pending(want)` | re-apply `want` after CEF's own handling (fixes G3); hand-back for invisible pool windows stays as it is |
| any | `WindowShown`, `WindowRestored`, `PaneShown` | unchanged | re-assert z-order for every Live pane, then `Verify` (fixes G9) |
| `Pending(t)` | `Verify`, `GetFocus() ∈ tree(t)` | `t` | log |
| `Pending(t)` | `Verify`, mismatch, attempt < 3 | `Pending(t, attempt+1)` | re-issue |
| `Pending(t)` | `Verify`, mismatch, attempt = 3 | `Lost(h)` → `Pending(App)` | WARN with the observed HWND; tell the frontend that `want` couldn't be applied |
| `t` | `Verify`, `GetFocus()` drifted | `Lost(h)` → `Pending(want)` | WARN, recover |

### 6.5 Invariants (asserted in tests and checked at `Verify`)

1. **I1:** `have = Pane(b)` ⇒ b is Live, its rect is non-empty, and `GetFocus() ∈ tree(b)`.
2. **I2:** `have = App` ⇒ `GetFocus()` is main's render widget, not inside any pane tree.
3. **I3:** at most one pane per window is CEF-focused, and it is `have`'s pane.
4. **I4:** no pending permission outlives its transition. `Pending` always has a deadline, and there is no process-global flag.
5. **I5:** every Win32 focus call runs on the UI thread (fixes G5). The adapter asserts this.
6. **I6:** for every Live, shown pane b with no overlay covering its centre, `WindowFromPoint(centre(b)) ∈ tree(b)` (pointer ownership, G9).
7. **I7:** `WM_SETFOCUS` and `WM_KILLFOCUS` always reach Chromium's own window procedure (G6).

### 6.6 Reconciling with Win32

- **Observe, don't fight.** Pass every focus message through. Correct by posting a `SetFocus` afterwards, from the machine, never from inside another window's handler.
- **Verify by asking Win32.** After each transition, a posted `Verify` reads `GetFocus()`, `GetForegroundWindow()` and, for I6, `WindowFromPoint` at each visible pane's centre. A mismatch is a logged event with a bounded retry, not a silent state.
- **Keep the existing guards** as conditions in the table, not separate code paths: cross-window foreground (`hwnd.rs:291-296`), the per-root storm limit, foreground-only reclaim (`window.rs:2050`) and pool hand-back (`wndproc.rs:39-53`).
- **Z-order is part of the state.** Re-raise Live panes on `WindowShown`, `WindowRestored` and `Activated`, and whenever a check of I6 fails. That costs one `SetWindowPos(HWND_TOP, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)` per pane.
- **Overlay clips get a sequence number per window.** The host drops a clip older than the last one applied (G10).

### 6.7 Logging

- One line per transition: `[input] win=<label> <from> -> <to> by=<event> seq=<n> reason=<r> getfocus=<hwnd> fg=<hwnd>`.
- WARN on a `Verify` mismatch, a `Lost` state or a failed I6 check, with the HWND, the block it maps to and the pane's rect.
- At subclass install, log `hwnd -> block` (G11).
- Keep the last 200 transitions in a ring that one IPC can dump: `muxlog host input` plus a debug command.

### 6.8 Frontend contract

- The frontend sends `input_select` whenever its selection changes, for any reason: pane click, keyboard navigation, tab switch, pane-tab switch, modal open or close. Each carries a sequence number.
- It stops calling `browser_pane_focus` and `reclaimWindowFocus` directly from F1, F2, F6, F7 and F8.
- It listens to `input-owner-changed`, the host's report of who actually has the keys, and stops using `document.hasFocus()` as a proxy for browser panes (G1).
- DOM caret placement inside the app (which input, `userCaretInBlock`) stays where it is today.

## 7. Test plan

**Pure transition tests** (Rust, no CEF). The table in §6.4 as data, with one test per row. Plus a property test: random sequences of events and simulated Win32 replies, asserting I1–I4 and I7 after every step. Regression cases, one per gap:

- four `Select(b)` in 10 ms produce one `Pending` and leave nothing armed (G4);
- collapse while focused ends in `App` (G2);
- `Activated` with `want = App` and a recorded pane child ends in `App` (G3);
- a page `FocusGained` in pane c while b owns the keys passes through and returns to b (G6);
- a `Verify` mismatch three times ends in `Lost` → `App` with a WARN (G7);
- an old clip `seq` is dropped (G10).

**Frontend unit tests** (vitest): the selection publisher sends exactly one `input_select` per selection change, including a window-tab switch while a browser pane is selected, and never depends on `document.hasFocus()`.

**Live runs** in an isolated `task dev` build on Windows, never the operator's instance. Extend `tools/tests/pane-focus-stress.ps1`; pass criteria come from `[input]` lines and an injected `GetFocus` / `WindowFromPoint` dump:

1. Click a page, type, switch window tabs, type: the keys land in the app.
2. Minimize and restore, and hide and show by other means: the wheel over each visible pane reaches its page window (I6). This directly tests §3.3's hypothesis.
3. Alt+Tab away and back with a browser pane selected, then with an agent pane selected.
4. Two panes, one page calling `window.focus()` on an interval: the keys stay where the user put them.
5. A pool refill while typing.
6. Open and close a menu over a pane quickly, 50 times: no hole is left.
7. A cancelled pane drag, then a layout move: the wheel and clicks still reach every pane.

macOS and Linux runs follow the same script once their adapters exist.

## 8. Phased plan

| Phase | Content | Size |
|---|---|---|
| P0 Diagnostics | Log `hwnd -> block` at subclass. Log redirected and refused `WM_SETFOCUS`. Add an `input_focus_dump` debug IPC that logs, per window: `GetFocus`, the foreground window, each pane's rect, life, z-index relative to main's children, `WindowFromPoint(centre)` and clip state. Run it automatically after `WM_ACTIVATE` and window show. Log dropped rect applies (pane not live, wrong window) at info | small, ship first |
| P1 Targeted fixes | (a) Marshal `browser_pane_focus` to the UI thread (G5). (b) Replace the global flag with a per-window `{target tree, deadline}` (G4). (c) Run the `WM_ACTIVATE` restore after the original procedure, by posting it, and skip collapsed or closing panes (G3). (d) Hand focus to the app when a focused pane collapses (G2). (e) Re-raise Live panes on window show, restore and activate (G9). (f) Give overlay clips a sequence number (G10). Each is independently testable | small to medium |
| P2 Machine on Windows | Pure core and Windows adapter. `browser_pane_focus` and `main_window_focus` become `Select` events. Add `Verify`, invariants and `[input]` logging. Delete K2's redirect branch, K4, K9's direct `SetFocus` and K8 as separate paths | medium |
| P3 Frontend | Selection publisher, `input-owner-changed` listener. Remove direct focus IPCs from F1, F2, F6, F7 and F8, and the `document.hasFocus()` gate for browser panes | medium |
| P4 macOS / Linux adapters | Map key window and Views focus onto the same core. Fold in `PENDING_PANE_KEY` as `Pending` | medium |
| P5 Docs | Mark FOCUS_LOCK and CEF_PANE_SCROLL_RESEARCH superseded by the new spec, update the stale status headers in §5, and add one architecture doc for the input model | small |

Decide on #4430 (Views overlays on Windows) before P2: it would replace the HWND tree and most of G6, G9 and G11, but not the need for one owner.

## 9. Open questions

1. **What blocked the pointer at 13:02–13:04?** A z-order dump of the main window's children, with P0's `input_focus_dump` during the next occurrence, would show whether a main child sat above the wrapper. A live test in a dev build would also answer it: minimize and restore, then check that the wheel reaches the pane. Until then §3.3 is a hypothesis.
2. Is "scroll inactive windows" on (the Windows default) on the operator's machine? It doesn't change the conclusion, since no wheel reached the pane either way, but it matters for the design's wheel assumptions.
3. Were the 13:03:23 and 13:03:31 clicks on the page body (as the extra IPC suggests) or on the pane header? The frontend could log the placeholder `onMouseDown` to settle this next time.
4. Does CEF's `host.set_focus(1)` already call Win32 `SetFocus` on the pane's widget? If so, G5's off-thread call is only redundant, not harmful. Check in a dev build with the P0 logging.
5. When the window is activated, should the keyboard go to the frontend's selection or to the last Win32 owner? The design says the selection; today K9 uses the last owner.
6. Should a page's own `focus()` ever take the keyboard from the app, for example an OAuth page focusing its password field when the user just clicked it? The table allows it only under a pending user intent for that pane.
