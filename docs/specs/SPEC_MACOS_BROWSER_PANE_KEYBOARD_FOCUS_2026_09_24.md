# SPEC — macOS browser pane: typing reaches the page

**Date:** 2026-09-24
**Type:** Bug (macOS), plus one frontend focus bug on all platforms
**Status:** implemented — PR #3737 (native key-window handoff + split focus + claim-on-create + unhandled-key loop guards). Test plan verified live 2026-09-25 (all items except the background-tab case, which is covered by unit tests).
**Scope:** `agentmux-cef/src/ui_tasks/{platform_macos,pane_geometry,window}.rs`,
`agentmux-cef/src/browser_panes/clip.rs` (macOS only), and on the frontend
`frontend/layout/lib/layoutPersistence.ts` and
`frontend/app/view/browser/use-pane-rect-sync.ts`.
**Related:** `docs/analysis/RETRO_BROWSER_PANE_MACOS_FIX_2026_06_26.md` (#1819,
the June click fix this builds on), `SPEC_PANE_OPEN_FOCUS_ROUTING_2026_09_16.md`
and `SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md` (claim-on-mount), and
`SPEC_MACOS_BROWSER_PANE_CONTEXT_MENU_2026_09_24.md` (found in the same
investigation).

## Symptom

`muxsh web google.com` in a terminal opens a browser pane, but you can't type
into it. Clicking into the page doesn't help either. The same thing happens for
a browser pane opened any other way. It was reported as "flaky" and suspected
to come from the recent pane-tab work. It doesn't.

## Investigation (dev build of main @ 1d02df120, real OS input)

The tests drove a separate `task dev` instance through its CDP port (9223) plus
real CGEvent clicks and keystrokes. Synthetic CDP input would have skipped
AppKit's routing entirely.

1. **After `muxsh web`, the frontend never focused the page.** No
   `browser_pane_focus` IPC was sent. The terminal kept the DOM caret, and the
   new pane wasn't even selected (the terminal kept `block-focused`).
2. **A real click in Google's search box, then typing:** the page got nothing.
   The click reached the frontend (`browser-pane-clicked` →
   `giveFocus` → `browser_pane_focus` → `host.set_focus(1)`, all logged), but
   the keystrokes landed in whatever DOM element last had focus in the main
   window. In one run that was an Untitled editor buffer, which received
   "zqx". Clicking again didn't change it. A pane opened by insert (not split)
   behaved the same way (a keydown logger in the page recorded nothing).
3. **A temporary native probe** in the `NSApp sendEvent:` swizzle (key window,
   first responder and `canBecomeKeyWindow` on every mouse-down/keyDown)
   showed:
   ```
   ev=1  ev_win=NativeWidgetMacNSWindow canBecomeKey=0 NSApp.keyWindow=CefNSWindow
   ev=10 ev_win=CefNSWindow ... NSApp.keyWindow=CefNSWindow firstResponder=RenderWidgetHostViewCocoa
   ```
   The pane's overlay window **cannot become key**, so AppKit keeps sending
   every keyDown to the main window.

## Root causes

### 1. macOS: the pane can never be the key window (native)

- Pane overlays are created with `add_overlay_view(..., can_activate=0)`
  (`browser_pane/creation_views.rs`). Chromium's `NativeWidgetMacNSWindow`
  therefore answers `canBecomeKeyWindow = NO`.
- The June design (#1819) deliberately keeps the main window key so that
  sidebar clicks keep working:
  - `SetPaneBoundsViewsTask` calls `makeKeyAndOrderFront:` on the main window
    on **every** bounds change (resize, split animation, tab switch).
  - The overlay is swizzled to *report* `isMainWindow`/`isKeyWindow = YES`,
    which is enough for Chromium to accept clicks, but AppKit routes keys by
    the real key window.
- `BrowserPaneManager::focus` only calls `host.set_focus(1)`. That sets
  Chromium's focus state, but it doesn't change which NSWindow AppKit
  delivers keys to.
- The June retro says the overlay "becomes key when the user interacts with
  it". That is no longer true, and nothing in the current code makes it so.

### 2. Split-placed opens never selected the new pane (frontend, all platforms)

- `muxsh` always sends `split_reference_block_id` (the calling terminal), so
  the server queues a `splithorizontal` layout action with `focused: true`.
- `layoutPersistence.ts` copied `focused` for `InsertNode`, but not for
  `SplitHorizontal`/`SplitVertical`, so the new pane was never selected.

### 3. The browser pane never claimed the caret once its page existed (frontend)

- Terminal, editor and agent panes call `focusManager.claimFocusOnMount` once
  their input surface is ready.
- The browser pane didn't. The only `giveFocus()` attempt happens at layout
  insert, before `browser_pane_create` has produced a page, and it has no
  effect.

### Not the cause

- The pane-tab commits (#3706, #3708, #3714) don't touch focus.
- One side issue found in passing: each browser pane also gets a second
  `BrowserViewModel` for its preview thumbnail. Both listen to
  `browser-pane-clicked`, so one click sends `browser_pane_focus` up to three
  times. That is harmless (idempotent) but wasteful, and is left as a
  follow-up.

## Fix

### Native (macOS only)

The overlay becomes the key window only on an explicit user or frontend
request, and key status is handed back when the user goes back to the app.

| Piece | Where | What |
|---|---|---|
| Let the overlay become key | `swizzled_can_become_key_window`, installed next to the isMain/isKey swizzles | YES for the tagged overlay instance only; every other window gets the original answer |
| Click into the page → the page gets the keyboard | `swizzled_nsapp_send_event`, overlay mouse-down branch | `make_pane_overlay_key(block_id)` **before** the click is dispatched |
| Frontend focus → the page gets the keyboard | `BrowserPaneManager::focus` → `post_make_pane_overlay_key` (UI thread) | covers keyboard pane selection and open-with-focus |
| Focus request before the overlay exists or is on screen | `PENDING_PANE_KEY`, applied by `SetPaneBoundsViewsTask` right after it registers the overlay | the claim-on-create IPC beats the overlay setup; `makeKeyWindow` is a silent no-op on a window not yet shown, so an unsuccessful attempt is parked too |
| Click on the app UI → the keyboard comes back | main-window branch of the sendEvent swizzle (button-down) | `reclaim_key_for_window(win)`, a no-op unless a pane overlay is key |
| Frontend reclaims focus (`main_window_focus`) | `MainFocusReclaimTask` | `reclaim_key_for_window` + clear the parked request |
| A DOM menu/popover opens over the pane | `SetPaneOverlayClipViewsTask` (hole mask) | reclaim key, so Escape and typing go to the DOM overlay, not the page under it |
| Resizes no longer steal the keyboard | `SetPaneBoundsViewsTask` | skip `makeKeyAndOrderFront:` on the main window while a pane overlay is key |
| Keys reach Chromium, not the window | `swizzled_nsapp_send_event`, every keyDown/keyUp/flagsChanged for a tagged overlay | make the page's `RenderWidgetHostViewCocoa` first responder before dispatch |
| Unhandled keys don't loop | same place | a key event re-sent for a pane overlay (same NSEvent: pointer + timestamp) is offered to the main menu once and dropped |
| Only the key overlay's page takes keys | `release_inactive_pane_responders`, before every key event and on handback | any non-key overlay whose first responder is its page view is reset to its window |

About the first-responder rows: making the overlay key wasn't enough. Its
first responder was the overlay NSWindow itself, which swallowed every key.
A one-off `makeFirstResponder:` got reset before the first keypress (seen
live), so the first responder is re-pointed per key event. It's cheap: a
subview walk only when the responder is wrong.

### Unhandled-key re-dispatch loops (found in live testing)

The first cut of this fix, before the last two rows existed, spun the host
at 100% CPU. Traced with a native call stack at the re-send:

```
[NSApp sendEvent:]  ← -[CommandDispatcher redispatchKeyEvent:]
  ← NativeWidgetMacNSWindowHost::RedispatchKeyEvent
  ← views::UnhandledKeyboardEventHandler::HandleKeyboardEvent
  ← CefBrowserViewImpl::HandleKeyboardEvent ← WebContentsImpl::HandleKeyboardEvent
```

Chromium re-sends every key the page doesn't consume (the same NSEvent) so
that menu key equivalents get a chance. Two loops came out of that:

1. **Within a pane.** A re-sent event for the pane overlay was routed to the
   page again (its view is now first responder), reported unhandled again,
   and re-sent forever. Any key on a page with nothing focused triggered it.
   Fixed by the re-dispatch guard: the re-send gets `[NSApp.mainMenu
   performKeyEquivalent:]` and is never routed back into the pane.
2. **Across windows.** A key typed into the app (for example after Cmd+L
   moved the caret to the address bar) that the app's page doesn't consume
   is re-sent to the main window. `CommandDispatcher` then offers it to
   child windows via `performKeyEquivalent:`. Another pane overlay whose page
   view was still first responder accepted it, and the key ping-ponged
   between the main window and that page. Fixed by the invariant that only
   the key overlay keeps its page view as first responder.

An earlier attempt also called Views `request_focus()` on the pane's
BrowserView. That's gone: the overlay view belongs to the **main** widget's
views hierarchy, so it made the main window's FocusManager route the app's
own keystrokes into the page.

What deliberately doesn't change:
- `can_activate` stays 0. Activation behaviour on creation is untouched.
- The overlay still doesn't become *main*, so the main window's title bar
  stays active.
- While the page has the keyboard, only the pane shortcuts reach the app,
  through the existing `on_pre_key_event` path (`client/handlers.rs`):
  Cmd+L (address bar), Cmd+R, Alt+Left/Right. Other app chords go to the page,
  the same as on Windows, where the pane window owns the keyboard. Menu-bar
  key equivalents still work through the re-dispatch guard.

### Frontend

- `layoutPersistence.ts`: pass `focused` through for `SplitHorizontal` and
  `SplitVertical`. The new pane is now selected, exactly as with an insert.
  Floating-pane redock splits also send `focused: true`, so a pane you dock
  back is now selected too, which matches the insert path.
- `use-pane-rect-sync.ts`: after `browser_pane_create` resolves, call
  `focusManager.claimFocusOnMount(blockId, giveFocus)`. That's the same
  guarded claim terminals and editors use. It does nothing unless this pane is
  the active tab's selected pane, and never takes the caret from this pane's
  own URL bar.

## Test plan (macOS dev build, real input)

1. `muxsh web google.com` from a terminal: the pane is selected, and typing
   immediately goes into Google's search box (autofocus field). Nothing goes
   into the terminal.
2. Click another pane (terminal/editor), then click into the page and type:
   the text goes into the page.
3. Click back into a terminal and type: the text goes into the terminal, not
   the page.
4. Resize the split or switch tabs and back while the page has the
   keyboard: the page keeps the keyboard.
5. With the page holding the keyboard, right-click and press Escape: the menu
   closes (see the context menu spec).
6. With the page holding the keyboard, press Cmd+L, then type: the text goes
   into that pane's address bar.
7. Two browser panes: clicking each one moves the keyboard between them.
8. Close the page's pane while it holds the keyboard: typing goes back to
   the app.
9. Regression: sidebar and header clicks with a pane open (the #1819 case),
   and a pane opened in a background tab doesn't take the caret.

**Verified live (2026-09-25, dev build of #3736 + #3737 combined, real CGEvent
input, with a CPU/key-log watchdog after every step):**

| # | Step | Result |
|---|---|---|
| 1 | focused split open, type `hi` with no click | page input = `hi` |
| 2 | click an app pane, type `zz` | app input = `zz`; pages unchanged |
| 3 | click into page A, type `a1` | page A = `a1` |
| 4 | forced resizes (resize IPC), type `r` | page A = `a1a3r` |
| 5 | right-click page A, Escape | menu shown over the page, closed by Escape |
| 6 | page A: Cmd+L, type `zz` | address bar = `zz` |
| 7 | click page B `b2`, back to page A `a3` | A = `a1a3`, B = `b2` |
| 8 | close the pane holding the keyboard, type `xy` | app received `xy` |
| – | click page body (no input), type `qj` | keys logged, no loop, idle CPU |

Not checked live: a pane opened in a background tab (covered by the
`claimFocusOnMount` guard and its unit tests).

## Follow-ups

- Drop the preview thumbnail `BrowserViewModel`'s event subscriptions (the
  duplicate `browser_pane_focus` calls).
- Consider `giveFocus()` returning a "pending" state while
  `!paneCreated()`, instead of `true`.
