# SPEC — macOS browser pane: typing reaches the page

**Date:** 2026-09-24
**Type:** Bug (macOS), plus one frontend focus bug on all platforms
**Status:** Implemented in the PR that adds this doc.
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
| Focus request before the overlay exists | `PENDING_PANE_KEY`, applied by `SetPaneBoundsViewsTask` right after it registers the overlay | the claim-on-create IPC beats the overlay setup |
| Click on the app UI → the keyboard comes back | main-window branch of the sendEvent swizzle (button-down) | `reclaim_key_for_window(win)`, a no-op unless a pane overlay is key |
| Frontend reclaims focus (`main_window_focus`) | `MainFocusReclaimTask` | `reclaim_key_for_window` + clear the parked request |
| A DOM menu/popover opens over the pane | `SetPaneOverlayClipViewsTask` (hole mask) | reclaim key, so Escape and typing go to the DOM overlay, not the page under it |
| Resizes no longer steal the keyboard | `SetPaneBoundsViewsTask` | skip `makeKeyAndOrderFront:` on the main window while a pane overlay is key |
| Keys reach Chromium, not the window | `swizzled_nsapp_send_event`, every keyDown/keyUp/flagsChanged for a tagged overlay | make the page's `RenderWidgetHostViewCocoa` first responder before dispatch |

About the last row: making the overlay key wasn't enough. Its first responder
was the overlay NSWindow itself, which swallowed every key. A one-off
`makeFirstResponder:` plus Views `request_focus()` in
`make_pane_overlay_key` got reset before the first keypress (seen live), so
the first responder is re-pointed per key event. It's cheap: a subview walk
only when the responder is wrong.

What deliberately doesn't change:
- `can_activate` stays 0. Activation behaviour on creation is untouched.
- The overlay still doesn't become *main*, so the main window's title bar
  stays active.
- App shortcuts pressed while the page has the keyboard still reach the app
  through the existing macOS `on_pre_key_event` → `browser-pane-shortcut`
  path (`client/handlers.rs`).

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
6. With the page holding the keyboard, press an app shortcut (for example a
   pane-navigation chord): the app handles it.
7. Two browser panes: clicking each one moves the keyboard between them.
8. Close the page's pane while it holds the keyboard: typing goes back to
   the app.
9. Regression: sidebar and header clicks with a pane open (the #1819 case),
   and a pane opened in a background tab doesn't take the caret.

**Verified live (2026-09-24, dev build, real CGEvent input):**
- 1: `hello` typed right after a focused split open landed in Google's search
  box.
- 2: after clicking an app pane, `zz` went to the app, and the page was
  unchanged.
- 3: after clicking back into the page, `abc` went to the page.
- 4: four forced bounds updates (resize IPC), then `e` still went to the page.
- 5: a right-click while the page had the keyboard opened the menu over the
  page; key status went back to the app window and Escape closed the menu.

Items 6–9 are not yet checked live. (The test machine locked partway
through.)

## Follow-ups

- Drop the preview thumbnail `BrowserViewModel`'s event subscriptions (the
  duplicate `browser_pane_focus` calls).
- Consider `giveFocus()` returning a "pending" state while
  `!paneCreated()`, instead of `true`.
