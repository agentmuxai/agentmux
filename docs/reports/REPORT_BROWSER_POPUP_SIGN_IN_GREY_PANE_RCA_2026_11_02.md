# RCA: a sign-in link clicked in an agent's limited pane ends on a grey pane

**Date:** 2026-11-02
**Status:** analysis, fixed by the PR that adds this report.
**Author:** Korp (korp@narko)
**Trigger:** the person opened a Microsoft 365 admin sign-in link from an email. The page it opened in AgentMux stayed grey, with the tab titled just "Browser". The same link works in Chrome.
**Code:** `main` @ `d11578351`; the build that showed it is 0.60.11.
**Related:** `SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md`, `REPORT_BROWSER_PANE_BLANK_AFTER_BLOCKED_SIGN_IN_REDIRECT_RCA_2026_10_16.md`.

## 1. Summary

- **Nothing is wrong with the Microsoft flow or with redirects.** The mail was open in a pane an agent had opened and limited to Google's sites (`allowed_origins`). The link opened as a popup pane, and a popup pane **inherits its opener's site list**. The first redirect hop (the mail provider's safe-link service) is not on that list, so the host cancelled it, which left the popup on an empty page. Every later hop of the chain (the admin site, the sign-in site) would have needed its own Allow.
- **Why the person saw nothing useful:** the pane did ask ("This pane is limited to the sites it was opened for. The page wants to go to …"), but the person had opened no pane for any sites, the page behind the question was empty, and the question names only the next hop of a chain that has several.
- **Fix:** a popup the person clicked is the person's pane: no list, no agent. And a pane whose first navigation was stopped shows a page that says so, with Reload and Open in system browser.

## 2. What the logs show

Five attempts, between 05:53 and 06:00 UTC, all the same:

```
browser-pane new window routed        route=Pane  opener=<the mail pane>
[browser-popup] popup opened as a pane  owner=<the agent>
browser pane: stopped a navigation off its allowed sites   url=<safe-link service>/redirect/?destination=...admin.microsoft.com...
[pane-trace] load-error url=<same> err=ERR_ABORTED
[browser-allowlist] stopped navigation  outcome=asking
... (minutes later) question answered  answer=Cancelled  outcome=dropped: the pane was closed
```

The last attempt was stopped at `admin.microsoft.com` itself: the person had allowed nothing, so each retry started again at the first hop.

## 3. Root cause

`handle_host_browser_popup` gave a popup pane the opener's owner and joined it to the opener's chain, so it shared the list (`browser_allowlist::join`). That is right for a popup the agent caused: the agent's page must not slip its limit through `window.open`. It is wrong for a popup the person caused. The list exists to keep the agent's page where the agent sent it; a link the person chose is the person's to follow.

The page can't say who clicked: the agent's clicks are real input events, so `user_gesture` is true for both.

## 4. Fix

1. **srv knows who acted.** `browser_agent_calls.rs` notes each `Browser` call on a pane (every call resolves its pane through `ui_handlers::target_block_id`). A popup with a click, on a pane the agent has not called for 10 seconds, is the person's (`person_clicked`). When both are possible it counts as the agent's, so the limit stays.
2. **The person's popup is theirs.** `handle_host_browser_popup` gives it no agent, no list, and focus. It is not refused for being off the opener's list. The agent's popups are unchanged.
3. **No grey dead end.** When the host cancels a navigation and the frame has no document of its own (nothing committed, `about:blank`, or only the address that was stopped), `on_load_error`'s `ERR_ABORTED` now loads the stopped page (`client/stopped_page.rs`): what was stopped, the pane's sites, Reload, and Open in system browser. A popup from that page goes to the system browser. The banner still asks the person; the page is what stays behind it.

## 5. Tests

- `a_popup_the_person_clicks_in_a_limited_pane_is_theirs_and_unlimited` (srv): a limited, agent-owned pane; the person's click opens an unowned, unlimited pane, also for an off-list address; the agent's click (after a `Browser` call) is refused off the list and shares it on it.
- The existing popup tests now make their clicks the agent's (a `Browser` call first).
- `browser_agent_calls` unit tests: the window, the person's click, the bounded notes.
- `stopped_page` unit tests: when a frame has no document, the page's controls, and that an address can't break out of it.

## 6. Not covered

- A real redirect chain through CEF (a 302, a `form_post`, a cross-origin hop) can't run in CI. It was not run by hand either: a dev build of the host takes about 1.5 hours on the machine this was written on.
- A person's click on a link in the **agent's own** limited pane (not a popup) still asks, as before: the question is the person's way to widen the list, and Take over lifts it.
