# Mouse side buttons walk back and forward through the panes you selected, and the caret follows

**Status:** built on Windows host and frontend (all platforms handle the host events; mouse behavior awaits browser interaction). macOS/Linux browser panes (P3) are not built. §10 records what was built and where it differs from this draft.
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
| macOS / Linux browser panes | no per-pane hook exists (`hwnd.rs` is Windows-only, the same gap Ctrl+wheel zoom has) | *verify*; until then the buttons over a browser pane keep Chromium's default there |
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
- **P3.** macOS / Linux browser panes from what P0 finds.

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
- **Not built:** the §4.2 "ignore while a context menu is open" rule (only modals are detected), and macOS / Linux
  browser panes.

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
