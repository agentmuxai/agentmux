# RETRO: a page's `window.close()` in a browser pane quit AgentMux (2026-10-16)

**Status:** retro. Root cause found and reproduced; fixed in the PR that adds this file.
**Author:** Agent5 (agent), with the log reading by Loap (agent), at the operator's request.

## Summary

A web page open in a browser pane called `window.close()`. AgentMux let Chromium close that browser the default way, and Chromium's default close of a pane's browser closes the whole AgentMux window: the window's other browser panes closed, the window disappeared, and AgentMux quit three re-checks later. Three agents running in that window were stopped mid-turn.

The pane was a sign-in popup: claude.ai's "Reconnect GitHub App" button opened GitHub's sign-in with `window.open`, which AgentMux routes into a new browser pane. After the sign-in, the page showed a "close this window" link; clicking it called `window.close()`.

## Impact

- The 0.59.26 instance (one window, three agent panes, three browser panes) quit at 13:42:33 UTC. Every agent's turn ended: QTest and Agent5 exited, Korp was killed after 5.5 s.
- No data was lost: the session restored on relaunch, including the sign-in pane, now on the sign-in's last page.
- Any page that calls `window.close()` in a browser pane does the same. Sign-in and payment flows commonly end that way.

## Timeline (UTC, host log)

| Time | Event |
|---|---|
| 13:41:48.78 | A page in browser pane `02411f69` calls `window.open` for GitHub sign-in; it opens as browser pane `9d4a484e` ("popup opened as a pane"). |
| 13:42:16.89 | The sign-in finishes; pane `9d4a484e` loads the site's callback page. |
| 13:42:21.12 | The operator clicks in pane `9d4a484e` (`emit-clicked`), on the page's "close this window" link. |
| 13:42:21.29 | Pane `9d4a484e`'s browser closes (`on_before_close`), 160 ms after the click. AgentMux never started that close. The quit watchdog arms ("reducer counts 1 live"). |
| 13:42:21.33 | The window's other two browser panes close (`02411f69`, `08e0abae`); the three agent panes turn hidden. |
| 13:42:24.30, 27.31, 30.32 | Quit watchdog: "0 visible for 3000ms but registered=1 draining=false" — re-armed three times. The only full-size app window, at 128,128, is `visible=false`. |
| 13:42:33.32 | "quit watchdog fired … reducer disagrees (registered=1 draining=false) — quitting on OS signal alone". |
| 13:42:33.37 | Host shutdown complete; srv ends the three agent turns. |

## Root cause

`AgentMuxHandler::do_close` (`crates/cef/src/client/lifecycle.rs`) returned `false` for every browser except an OAuth popup's own window. `false` tells CEF to go on with its default close. For a windowed browser on Windows that is the top-level window's close: CEF treats a pane's browser and the main browser as one close unit, because the pane's window is a child of the main window.

That is a known hazard. `browser_panes/close.rs` documents it ("`close_browser(pane)` fires `do_close` on main too … either quit the whole app or orphaned the pane's pixels") and is why AgentMux never closes a pane through CEF: it marks the pane Closing, re-parents the pane's wrapper window out to the top level and destroys it.

That guard covers every close AgentMux starts, and nothing covered a close the page starts. `window.close()` makes Chromium call `CloseBrowser` on the pane's browser itself, which reaches `do_close`, which returned `false`.

### Why the window was invisible while the reducer still counted it

Loap's second question. The default close took the main window down. The main window's browser never reached `on_before_close`, so it stayed registered: the log has `BrowserUnregistered` for the three panes at 13:42:21 and for two pool windows during the shutdown, none for `main`. The OS-side check found no visible app window. The quit watchdog exists for exactly this disagreement: it re-checked three times, then quit on the OS signal. Given a window the user can no longer see or reach, quitting is the right call. The bug is the close that came before it, not the watchdog.

### Why the earlier popups were fine

Two Google sign-in popups (13:39:26 and 13:40:19) opened as real popup windows, not panes, because their URLs are OAuth authorize endpoints (`is_oauth_authorization_url`). Popups are excluded from the last-window quit gate and close themselves cleanly. This one was a claude.ai URL, so it opened as a browser pane, and in a pane `window.close()` hits the bug.

## Fix

- **Host** (`do_close`, Windows): when the closing browser is a browser pane's own browser and the pane is still Live, the page started the close. Return `true`, so CEF does not run its default close, and send `browser-pane-close-requested` to the window that holds the pane.
- **Frontend** (`browser-model.ts`): on `browser-pane-close-requested`, close the pane as its × does: just its tab when the pane holds others, else the pane. The pane is looked up in the window tab on screen, then in the window's other tabs, since a page can close itself while its tab is in the background. That runs AgentMux's normal pane close, which marks the pane Closing and tears it down the safe way.
- AgentMux's own closes are unchanged: they mark the pane Closing first, so `block_id_for_browser` no longer returns it and `do_close` returns `false` as before. OAuth popups keep their own path.

## Reproduction

An isolated `task dev` build (Windows, 0.59.28 from `main`): a browser pane on a local page whose only content is a button with `onclick="window.close()"`, clicked with a real mouse event through the dev instance's DevTools port.

- **Before the fix:** the pane's page closed, the window's title went blank (no visible window), and 9 s later the host log shows the same three watchdog re-checks and "quitting on OS signal alone", then "host shutdown complete". The same sequence as the incident.
- **After the fix:** "browser pane's page closed itself; asking its window to close the pane", `BrowserPaneClosed` 9 ms later, and the pane is gone from the layout. The window stays up and responsive, with no watchdog. Closing another browser pane the normal way (the layout's close, as its × does) still works.

Unit tests (`browser-model.test.ts`) cover the frontend half: the pane closes when it is its pane's only tab, only its tab closes when the pane holds others, and requests for another pane or after the pane was disposed are ignored.

## Why it wasn't caught

- The close-cascade hazard was documented, and the fix in July went into the path AgentMux controls (`BrowserPaneManager::close`). `do_close` was left returning `false` for panes, because nothing AgentMux does reaches it for a live pane, and nobody listed the page as a source of closes.
- Popups have opened as panes only since #4478 (2026-10-09), three months after that work. That made "a page that closes itself, in a pane" a common case: before it, a page that opens a sign-in window and later closes it was almost always in a real popup window, not a pane.

## Open questions

- **macOS and Linux:** browser panes there are CEF Views (`creation_views.rs`), with a different default close. The fix is Windows-only because only Windows could be tested. Whether `window.close()` in a Views pane also closes the window is untested.
- The log's "`[on_before_close] no backend window ID registered … shells may orphan`" warnings for the three panes are expected for browser panes (they have no backend window) and are noise here.
