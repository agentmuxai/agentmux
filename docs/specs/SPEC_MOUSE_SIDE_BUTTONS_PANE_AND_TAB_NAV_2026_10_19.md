# Mouse side buttons walk back and forward through the panes you selected, and the caret follows

**Status:** implemented in #5209 (Windows host and the frontend; all platforms handle the host events) and for Linux and macOS browser panes in the P3 build (§10.1; checked on Linux X11, macOS to be checked). The mouse behavior awaits a check in a live browser pane on Windows. §10 records what was built and where it differs from this draft.
**Date:** 2026-10-19.
**Requested by:** repo owner (asafebgi): use the mouse's side Forward/Back buttons to move through the selected pane,
across tabs, selecting the input if there is one. Clarified: Back/Forward in browser panes move to Ctrl + side button;
the plain side buttons are **selection history**, like a browser's: select pane B on tab 1, switch to tab 4 and select
pane G, type; Back selects B on tab 1, ready for typing; Forward selects G on tab 4 again, ready for typing.
**Author:** QTimer (research and spec only).
**Builds on:** `SPEC_FOCUS_FOLLOWS_SELECTION_2026_10_08.md` (the caret follows the highlighted pane),
`SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md` (selecting a pane focuses its input),
`SPEC_HELP_HIDDEN_TIPS_2026_10_10.md` (the "Mouse and gestures" table this gesture joins).
Line numbers are against `main` at `10bb4f6dd`. Rows marked *verify* need a live check in a `task dev` build (§8, P0).

An earlier draft of this spec stepped through panes in layout order (like F6). That is **replaced**: this is a history of
what you selected, not a walk through the layout.

---

## 1. The rule

> **The window keeps a history of the panes you selected, across tabs. Mouse side button 1 (Back) re-selects the
> previous entry; side button 2 (Forward) re-selects the one after it. Selecting means: switch to that pane's tab if it
> isn't the active one, highlight the pane, and put the caret in its input (a pane with no input gets its focusable
> root). Ctrl + Back / Ctrl + Forward over a browser pane is that page's history, as the plain buttons are in a browser.**

Your example, as the history:

```
[ tab1·B ] → [ tab4·G ]            you are at G (index 1)
Back     → select tab1·B, caret in B's input   (index 0)
Forward  → select tab4·G, caret in G's input   (index 1)
```

## 2. What exists today

| Piece | Where | Note |
|---|---|---|
| Per-tab recency | `focusedNodeIdStack` (`layoutModel.ts:275`, `layoutFocus.ts:34-66`) | An MRU list **per tab**, deduped, with no forward direction. It picks the survivor when a pane closes. It is not a history and can't cross tabs; this feature leaves it alone. |
| Selection changes | `layoutModel.focusedNode()`; `focusManager.blockFocusAtom` (`focusManager.ts`); `activeTabId()` | One reactive source for "which pane in which tab is selected". |
| Moving there | `setActiveTab(tabId)` (`tab-actions.ts:286`), then `layoutModel.focusNode(nodeId)` (`layoutFocus.ts:189`), then `focusManager.refocusNode()` (`focusManager.ts`, also never steals a caret from an open modal) | All exist. `whenTabContentSettled` exists for waiting on a switched-to tab. |
| Side-button handling | none. No `XBUTTON`, `app-command`, `auxclick` or `button === 3/4` in `frontend/` or `crates/` | |
| Browser panes | native child HWNDs (`crates/cef/src/browser_pane/hwnd.rs`). Their mouse input is taken at the Win32 level, so **DOM listeners never see it**; `wndproc_hook` already intercepts `WM_MOUSEWHEEL` (Ctrl+wheel zoom, reading `MK_CONTROL` from `wParam`) and `WM_LBUTTONDOWN` | Page back/forward today: Alt+←/→ (`client/handlers.rs:399-445`, `BrowserPaneShortcut::GoBack/GoForward`) and the nav bar, both reaching `state.browser_panes.go_back/go_forward`. |

## 3. Research: what the platform gives us

1. **DOM.** The side buttons are `MouseEvent.button` 3 (back, "X1") and 4 (forward, "X2"); `buttons` bits 8 and 16.
   Chromium fires `pointerdown`, `mousedown`, `mouseup`, then `auxclick`. Cancelling `pointerdown` suppresses the
   compatibility mouse events but not `auxclick`; the robust pattern is a **capture-phase, non-passive** listener on
   `window` for all four that calls `preventDefault()` and acts once, on `mouseup`. `ctrlKey` is on the event
   ([MDN `button`][mdn-button], [Pointer Events][pe], [FreeshardBase #65][fs65]). Firefox is reported to swallow them
   before page JS ([Mozilla Discourse][moz]); we ship Chromium (CEF), so that is a cross-check only.
2. **Windows host.** The buttons become `WM_XBUTTONDOWN/UP` (the modifier flags, `MK_CONTROL`, are in the low word of
   `wParam`), and `DefWindowProc` turns an unhandled one into `WM_APPCOMMAND` with `APPCOMMAND_BROWSER_BACKWARD/FORWARD`.
   Electron surfaces that as its `app-command` event; keyboard browser keys produce the same command, so the device
   bits (`FAPPCOMMAND_MOUSE`) tell them apart ([Electron `BrowserWindow`][electron-bw],
   [Rocket.Chat.Electron #3550][rc3550]). `WM_APPCOMMAND` carries no modifier state, so a Ctrl check there means
   `GetKeyState(VK_CONTROL)` (inference). On Linux, Electron **double-fires** (its event plus the built-in
   navigation, [Electron #32763][el32763]): this is why §5 allows one acting path per surface.
3. **CEF has no pre-handler for the mouse yet.** `CefKeyboardHandler::OnPreKeyEvent` is keyboard-only.
   [chromiumembedded/cef #4304][cef4304] asks for one; [PR #4305][cef4305] adds `CefMouseHandler::OnPreMouseEvent`
   with X1/X2 but is **still open** and cannot land until CEF moves to Chromium M158. So we can't rely on it. The CEF
   forum's working recipe on Windows is to subclass the browser's child window and swallow `WM_XBUTTONUP`, and deal with
   `WM_APPCOMMAND` at the frame ([CEF forum][cefforum]). This repo already subclasses the top-level window
   (`client/wndproc.rs`) and every browser-pane HWND (`browser_pane/hwnd.rs`).
4. **Terminals.** xterm's mouse protocol encodes extra buttons as 128/129 because 8/9 collide with the Meta bit
   ([ctlseqs][ctlseqs], [Kettle #507][kettle]). A program in mouse mode (vim, tmux) could get a confusing report, so
   the buttons are **consumed** and never forwarded. Whether xterm.js forwards buttons 3/4 at all is *verify*.
5. **Prior art and UX.** Browsers use these buttons for history, and an extension category exists only to *disable*
   accidental activation ([FFMSB][ffmsb]); terminal/WM tools reach tab switching only through external remappers
   ([Firefox mouse shortcuts][ffmouse], [WezTerm][wez]). Nothing standard exists for a selection history, so we follow
   the browser history model users already know (§4). Practical consequences, partly inference: a **setting to turn it
   off**, act on **release**, ignore it **mid-drag**, and give the **same visible feedback** as keyboard selection.

## 4. Behaviour

### 4.1 The history

- **Scope:** one per window (a floating window has its own, as it has its own tabs and JS context). In memory; not
  persisted across a restart.
- **Entry:** `{ tabId, blockId }`. The block id is the stable one (node ids are layout-internal). The tab id comes from
  the tab that held the block when selected.
- **Cursor:** a list plus an index, exactly a browser's model.
- **Recording:** one reactive effect watches the committed `(activeTabId, focused block)` pair. When it changes to a
  pair that isn't the current entry, and the change is **not** a history navigation (§4.2), the entry list is cut
  after the cursor, the pair is appended, and the cursor moves to it. So selecting something new after going Back
  discards the forward entries, as a browser does. A pair equal to the current entry is ignored (clicking inside the
  pane you're already in adds nothing).
- **What counts as a selection:** anything that changes the highlighted pane or active tab: a click, F6, arrow-key
  pane navigation, Cmd/Ctrl+1..9, Ctrl+Tab, clicking a tab, a pane being opened. A tab switch counts because it selects
  that tab's focused pane. Typing doesn't; neither does moving the caret inside a pane.
- **Not recorded:** a transient state while a tab is still being revealed (record the settled pair, after the tab's
  content has settled, so a switch records its destination pane and not a half-restored one); an agent pane picker is
  recorded like any pane.
- **Cap:** 100 entries; oldest dropped.
- **Dead entries:** a pane or tab that no longer exists, or a block now in another tab, is skipped and removed when
  the cursor reaches it, and the press continues to the next live entry in that direction. If none is left, nothing
  happens.

### 4.2 Back and Forward

1. Compute the target entry (`index − 1` or `index + 1`, skipping dead ones). None: do nothing (no wrap).
2. Set a **navigating** flag with the expected pair, move the cursor to the target.
3. If the entry's tab isn't active: `setActiveTab(tabId)` and wait for the tab's content to settle.
4. Focus the pane in that tab's layout (`focusNode`), then the caret (`refocusNode`/`giveBlockFocus`), which keeps its
   existing rules: no steal from an open modal, `giveFocus` declining on a text selection or an `inert` ancestor.
5. The recording effect sees the pair equal the expected one, clears the flag, and records nothing.

**Rapid presses.** Switching tab is asynchronous. Steps are chained on one promise; each computes from the cursor after
the previous one finishes. At most 3 queued, extras dropped. A fresh *real* selection during a pending step cancels the
queue (the user took over).

**Where it fires.** Anywhere in the window, including over a browser pane. It acts on the window that received the
button.

**It does not fire** while: the left button is held (a drag-select or pane drag-and-drop, `buttons & 1`); a modal, the
command palette or a context menu is open; the setting is off.

**Feedback.** The ordinary ones: the focus border moves, the tab strip reveals the tab (`tab-reveal.ts`). No toast.

### 4.3 Ctrl + side button: page history in a browser pane

- **Ctrl + Back / Ctrl + Forward with the pointer over a browser pane:** that pane's page `go_back` / `go_forward`
  (the same calls as Alt+←/→ and the nav bar). It does **not** select the pane or touch the selection history.
  Keyed on the pane under the pointer, not the selected pane, like Ctrl+wheel zoom.
- **Ctrl + side button anywhere else:** nothing, but consumed, so Chromium can't navigate the app's own page.
- **Plain Back / Forward over a browser pane:** the selection history (§4.2). Page history is Ctrl + button, Alt+←/→, or
  the nav bar.
- Alt+←/→ stay as they are.

## 5. Input path: one acting source per surface

| Surface | Mechanism | Acts? |
|---|---|---|
| Main UI webview, all platforms | `window` capture listener, `pointerdown/mousedown/mouseup/auxclick`, `button` 3/4, non-passive, `preventDefault` + `stopPropagation`; acts on `mouseup` | **Yes**, if P0 shows the DOM receives them in CEF |
| Windows, top-level frame | `client/wndproc.rs`: swallow `WM_APPCOMMAND` `BROWSER_BACKWARD/FORWARD` with the mouse device bit, so Chromium never navigates the SPA's history. Keyboard browser keys untouched | Only if P0 shows the DOM *doesn't* get the buttons; otherwise swallow-only |
| Windows, browser panes | `browser_pane/hwnd.rs` `wndproc_hook`: handle `WM_XBUTTONUP` (`GET_XBUTTON_WPARAM`; `MK_CONTROL` from `wParam`): Ctrl → `go_back/go_forward` on that pane, plain → post a `history:back/forward` event to the owning window; return 0 (swallow the down and up) | **Yes**: the DOM never sees these |
| macOS / Linux browser panes | a listener added to each document of the pane through the in-process CDP client (`browser_pane/side_button_cdp.rs`, §10.1): it claims the buttons in the page, and its release reaches the host through a binding | **Yes** (Chromium doesn't navigate; the window gets the press) |
| Terminal / editor / agent panes | the main-webview listener, which consumes the event so xterm and CodeMirror never see it | Yes |

A 150 ms same-direction debounce is the backstop against any remaining double delivery.

## 6. Surface

- Commands in `command-registry.ts`: `pane:history:back`, `pane:history:forward` ("Back to previous pane" / "Forward
  to next pane"), **no default keys**, so they can be bound and agents can run them. The mouse calls the same
  functions.
- Setting `mouse:sidebuttons`: `"history"` (default) or `"off"`. Off: the app doesn't act and doesn't `preventDefault`,
  so the OS/Chromium default applies.
- Help: two rows in the "Mouse and gestures" table (`SPEC_HELP_HIDDEN_TIPS_2026_10_10.md` §3), pointer word
  `sidebutton`: "Mouse Back / Forward: previously / next selected pane, across tabs" and "Ctrl + Mouse Back / Forward
  over a browser pane: page back / forward".

## 7. Edge cases

- **Close the pane you came back to.** Its entry is dead; Back/Forward skip it. The existing survivor logic picks what
  is selected after the close, and that *is* a new selection, so it is recorded.
- **Same pane revisited:** `B → G → B` is three entries, so Back from B goes to G, then to B. Only consecutive
  duplicates collapse.
- **Pinned tabs, hidden/warm tabs:** tab switching uses the real `setActiveTab` path, so reveal gating applies.
- **A tab closed and its panes gone:** all its entries are dead and skipped.
- **Pane moved between tabs:** the entry is dead for its old tab; if the block exists elsewhere, re-point it on lookup
  (cheap: find the block's current tab). *Open Q3.*
- **F6 and Ctrl+Tab bursts** fill the history with every pane passed through. That is the browser model; see Q2.
- **Remapped buttons** (vendor software sending Alt+Left etc.) arrive as keys and are unaffected.

## 8. Plan

- **P0, probe (no behaviour).** In a `task dev` build, log for X1/X2 over: the main UI, a terminal, an agent pane, a
  browser pane, a floating window: which DOM events fire; whether `WM_XBUTTONUP`/`WM_APPCOMMAND` reach the top-level
  and pane HWNDs; whether Chromium navigates; whether `MK_CONTROL` shows; the same on macOS and Linux; and whether
  xterm.js forwards button 3/4. This decides which row of §5 acts.
- **P1.** The history module (window-scoped; record, cursor, dead-entry skipping, chained steps), the DOM listener,
  the two commands, the setting, the Help rows.
- **P2.** Windows host: top-level `WM_APPCOMMAND` swallow, browser-pane `WM_XBUTTONUP` with the Ctrl split.
- **P3.** macOS / Linux browser panes. P0 on Linux (X11, real buttons through XTest on a private display) found: the page's DOM gets `pointerdown/mousedown/pointerup/mouseup/auxclick` with `button` 3 / 4; Chromium navigates the page back on button 8 unless the pointer events are cancelled; cancelling them stops it. Built as §10.1.

**Tests.** The history is a pure reducer (`record`, `back`, `forward`, `prune`) unit-tested for: your tab1·B → tab4·G
example both ways; recording after Back truncates forward entries; consecutive duplicate collapse; a dead entry skipped
in each direction; cap at 100; the navigating flag not re-recording. A Vitest on the listener: button 3/4 acts once per
press, other buttons ignored, ignored with `buttons & 1`, with a modal open, with the setting off, `preventDefault` only
when acting, Ctrl + button not touching the history. A Rust unit test for the `WM_XBUTTON` decode helper
(`GET_XBUTTON_WPARAM`, `MK_CONTROL`). A manual script for the example, with typing checked after each hop.

## 9. Open questions (recommendations first)

1. **Mouse-only history?** Recommend yes for v1; the commands in §6 let keys be bound later. (Alt+←/→ can't be reused:
   they are page history in browser panes.)
2. **Should quick keyboard cycling (F6 bursts) each be an entry?** Recommend yes, the browser model, simplest to reason
   about. Alternative: collapse entries visited for under ~1 s.
3. **A pane moved to another tab:** follow it (recommended) or treat the entry as dead?
4. **Per-window, not persisted.** Recommend; persistence across restart adds a store for little value.
5. **Ctrl + side button elsewhere than a browser pane:** consumed and ignored (recommended), or something else?
6. **Keyboard browser keys** (dedicated Back/Forward keys) also produce `APPCOMMAND`. Recommend leaving them alone.

## 10. As built (2026-10-20)

The §9 recommendations were taken as written (Q1–Q6). Differences from the plan above:

- **Both input paths are in, with one debounce (150 ms).** The page-level listener acts on `mouseup`. The host also sends
  a `side-button` event: from a browser pane's windows (`browser_pane/hwnd.rs`, plain button; Ctrl + button calls the
  pane's `go_back` / `go_forward` itself) and from the top-level window's mouse `WM_APPCOMMAND`
  (`client/wndproc.rs`, which swallows it so Chromium cannot navigate the app's page). Whichever reports first acts, so
  P0's probe is not a separate step: each path logs `[side-button] … via dom|host:pane|host:appcommand`
  (frontend console and host log).
- **Setting:** `mouse:sidebuttons`, a boolean, default on (not the `"history"`/`"off"` string of §6). Off: the page
  listener consumes nothing, and a plain press over a browser pane does the page's own back / forward.
- **Commands:** `pane:history:back` / `pane:history:forward` are in the command palette; no shortcut-table rows (a row
  without keys would show in Help).
- **Code:** `app/store/pane-history-core.ts` (the reducer), `pane-history.ts` (recording and stepping),
  `side-button-rules.ts` (the press rules), `side-buttons.ts` (listener and host event), `crates/cef/src/side_buttons.rs`
  (Win32 decoding).
- **Not built:** the §4.2 "ignore while a context menu is open" rule (only modals are detected).

### 10.1 Linux and macOS browser panes (P3)

There is no window procedure to hook, so each pane gets a listener through CDP (`browser_pane/side_button_cdp.rs`,
`side_button.js`), added from `on_after_created_browser_pane`:

1. Once per DevTools session: `Runtime.enable` and `Runtime.addBinding { name: "agentmuxSideButton", executionContextName:
   "agentmux-side-buttons" }`. The function exists only in that isolated world, so the page can't call it.
2. For every document, when the host sees it commit (`on_load_start`, any frame): `Page.getFrameTree`, then in each frame not yet done
   (`(frameId, loaderId)`), `Page.createIsolatedWorld { worldName: "agentmux-side-buttons" }` and the listener evaluated in it
   (the way `browser_api/act.rs` works in a page). It claims `button` 3 / 4 on `pointerdown/pointerup/mousedown/mouseup/auxclick`
   in the capture phase (`preventDefault`, `stopImmediatePropagation`). The release (`pointerup` or `mouseup`, one 150 ms
   debounce) calls the binding with `{button, ctrl, blocked}`.
3. The observer in `browser_api/cdp.rs` hands `Runtime.bindingCalled` to the route. From there it is the Windows route: a plain
   press sends the window the `side-button` event (source `pane`, with the block id; `blocked` when Shift, Alt, Meta or the left
   button is down, as the frontend's `hostPressActs` expects), and Ctrl calls the pane's `go_back` / `go_forward`. With
   `mouse:sidebuttons` off the frontend gives the press back to the page's own history, as on Windows.

The event shape is one function for every route (`events.rs`, `emit_side_button`).

**Why not `Page.addScriptToEvaluateOnNewDocument`.** It was the first design and it works (the script runs before the page's own),
but only while the Page domain is enabled: `Page.disable` removes the scripts, and an enabled Page domain takes the page's
JavaScript dialogs away from the host's own handler (`client::jsdialog`). `Runtime.enable` has no such effect. The price is the
second limit below.

**P0 and what was tried** (Linux, X11, real buttons through XTest on a private display, a packaged build):
- Over a pane the page's DOM gets `pointerdown/mousedown/pointerup/mouseup/auxclick` with `button` 3 / 4, and Chromium navigates the
  page back on button 8 unless those are cancelled (`preventDefault` on them stops it).
- With the route: a plain press over the page is claimed (the page stays, the window gets `side-button` and moves the selection
  history); Ctrl + 8 / 9 is the page's own back / forward; a press with the middle button held works the same; a cross-site
  navigation of the pane (a new process) gets a new world and works; the host's own error page works; a same-site iframe works.

**Limits** (each one tried):
- **Cross-site iframes** (built after the first P3): they run in their own process and CDP target, which the page target's session
  can't create a world in. The route now also turns on auto-attach (`CdpSession::attach_frames`, as the forms snapshot does) and,
  for each attached frame's session, adds the binding, then the world and the listener in each of its frames. A frame attaching
  (`Target.attachedToTarget`) runs the setup again, because the host's commit callback can come before the attach. Checked on
  Linux X11 and Wayland against the released build (§10.2). Every browser pane now has auto-attach on from its first load, not only when an
  agent reads a form. An iframe's session gets no `Runtime.enable` (the binding is added after the world exists, which Chromium
  accepts and still reports), and a page with more than nine frames gets no injection into the ones in other processes (a press over
  one acts as the page's own).
- **Earlier page listeners:** the listener is added after the document commits, a few milliseconds after it starts, so a page
  listener registered in its first scripts runs before ours. The press is still claimed (Chromium doesn't navigate, the window
  gets the event) but the page sees it as well; a page that acts on side buttons itself would act in addition. The Windows route
  doesn't have this (the page never gets them).
- **DevTools agent detach:** the binding goes with the session; the route is set up again on the detach event. Until then a plain
  press is claimed but not reported.
- **macOS:** built for it (nothing in the route is Linux-specific) and not yet checked on a Mac.

### 10.2 Cross-site iframes and Wayland, checked (2026-10-28)

Linux, a packaged build of `origin/main` plus the iframe change, against the released v0.60.4 on the same pages (a start page
whose link is clicked for real, so the history entries count; a host page with a same-site and a cross-site iframe, the sites
`127.0.0.1` and `localhost`).

- **X11** (private Xvfb, real buttons through XTest): over the cross-site iframe a plain Back moves the selection history and the page
  stays; the released build navigated the page back and the window heard nothing. Ctrl + Back over it is the page's own back (no window
  event), and so are Ctrl + Back over the same-site iframe and the page. A cross-site iframe added after the load, and one under a
  top page that navigated cross-site, are claimed too.
- **Wayland** (a private headless GNOME Shell on its own socket and session bus, real `BTN_SIDE` / `BTN_EXTRA` through its mutter
  RemoteDesktop session; the live session was not used): the same results, plus a press over the app's own UI (the page listener,
  `back via dom`) and Ctrl + Forward. The released build shows the same cross-site-iframe fault there.
- **Cost, 15 cross-site iframes that each log to the console 50 times a second, 45 s windows interleaved with the released build:**
  about 0.4 s more host CPU and 0.5 to 1.1 s more in all (about 1% to 2% of a core), memory the same; with 4 iframes no difference;
  an outside DevTools client attached to all 15 iframes of the released build changed nothing measurable, so auto-attach itself is
  cheap. The first run of this comparison, not interleaved, showed a difference of several seconds that was machine load.
- **Frozen pane** (`browser_pane_set_frozen`, which marks the page frozen in the host; the automatic freeze after 5 minutes hidden
  did not fire in this local build, whose engine isn't the fork build the window asks for): while it is frozen, `ensure` returns
  without a CDP call (each frame commit of a navigation logged the skip, and no "thawing a frozen page for a call" line appeared);
  on thaw (`note_frozen(false)`, or any other call that thaws it) the listener is added, and a press over a cross-site iframe works.
- **Order:** a history entry made without a user gesture is skipped by Chromium on Back, so a test page has to be reached by a real click.

## Sources

[mdn-button]: https://developer.mozilla.org/en-US/docs/Web/API/MouseEvent/button
[pe]: https://www.w3.org/TR/pointerevents/
[fs65]: https://github.com/FreeshardBase/files/issues/65
[moz]: https://discourse.mozilla.org/t/firefox-mouse-back-forward-buttons-do-not-reliably-reach-page-javascript/148988
[electron-bw]: https://www.electronjs.org/docs/latest/api/browser-window
[rc3550]: https://github.com/RocketChat/Rocket.Chat.Electron/pull/3550
[el32763]: https://github.com/electron/electron/issues/32763
[cef4304]: https://github.com/chromiumembedded/cef/issues/4304
[cef4305]: https://github.com/chromiumembedded/cef/pull/4305
[cefforum]: https://magpcss.org/ceforum/viewtopic.php?f=6&t=16543
[ctlseqs]: https://invisible-island.net/xterm/ctlseqs/ctlseqs.html
[kettle]: https://github.com/Reddimus/kettle/pull/507
[ffmsb]: https://chromewebstore.google.com/detail/ffmsb-freedom-for-mouse-s/najmjkgimjhilaleingfkhedncclmmaf?hl=en
[ffmouse]: https://support.mozilla.org/en-US/kb/mouse-shortcuts-perform-common-tasks
[wez]: https://ersantana.com/tools/wezterm_keybindings

Agent: QTimer@narko
