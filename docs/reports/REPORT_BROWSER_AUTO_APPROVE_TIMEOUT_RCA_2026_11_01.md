# RCA: "Turn on auto-approve" in a browser pane times out

**Date:** 2026-11-01
**Status:** analysis; fixed in the PR that adds this report. The cause is shown by the code and by the logs; the scan was not timed live (§4).
**Author:** Korp (korp), at AgentY's request.
**Related:** `SPEC_BROWSER_PANE_BYPASS_2026_10_16.md` §5 (the person's Auto-approve button), #5045 (bypass), #5895 (the secret scan on `eval`), #6080 (the banner's wording and retries).

## 1. Symptom

The person presses "Turn on auto-approve" on an agent-driven browser pane and, ten seconds later, the pane shows "read from srv: A connection attempt failed … (os error 10060)". Auto-approve is not on. It worked in the past.

## 2. What the logs show

| Fact | Where |
|---|---|
| Every press of the button in the retained logs ended after the full ten seconds the window waits for srv (three presses, 10 002 ms, 10 006 ms and 10 002 ms), none succeeded. | window log, `[perf] ipc browser_bypass_set` |
| All three were on a Google Cloud console page, a large page with many frames, with the agent working in the pane at the same moment (a `find`, `snapshot` or `act` about every two seconds). | srv log, `[ui-automation]` lines |
| srv was not stuck: it served the agent's calls in the same pane, one every one to three seconds, for the whole wait. The window kept serving other calls too. | both logs |
| No lock is held across the wait: `browser_standing::resume` and `activate_at` are plain functions with no `await`, and neither the window's browser API nor srv serialises calls on one pane. | code |

So AgentY's first guess (the window waits for srv while srv waits for the window's thread) does not hold: the two sides never waited on each other. The second (a lock held across an await in the standing-grants code) does not hold either, for the reason in the last row. The time is spent inside one call.

## 3. Cause

The button's request to srv (`handle_host_browser_bypass`) first reads the address the pane shows, to check it against the origins the person was shown. It reads it with `page_url()`, which asks the window to run the agent's `eval` tool on `location.href`.

`eval` is not a cheap call any more. #5895 (2026-10-30, first shipped in 0.60.8) made it scan the whole page for Vault values and one-time secrets before it runs anything: `DOM.getDocument` with the full tree, then the layout and geometry of every frame, then a scan of each. On a large console page that is many calls into the browser, which all go through the browser's one UI thread together with the agent's own calls in the pane, and the script then runs behind whatever the page itself is doing. The code puts no limit on that time, and the logs show it outlasting the ten seconds the button waits.

The same change has a second effect: on a page that holds a Vault value or shows a secret, or one too large or with a frame it can't read, `eval` refuses ("eval is refused on this tab …"). srv then answered the button with "couldn't read the page's address", so Auto-approve could never be turned on there at all.

`page_url()` is also used for every committing click under a grant (`bypassed_click`) and by the browser terminal's checks, so each of those paid the same scan.

It worked until 0.60.7 because `eval` had no scan; 0.60.8 and later have it. Both retained failing logs are from later builds (0.60.9, 0.60.10); the older logs hold no press of the button.

## 4. What is proven and what is not

Proven by the code: srv's address read goes through `eval`; `eval` runs the scan first; the scan's work grows with the page's frames and elements; `eval` refuses on a page with a secret. Proven by the logs: the press waited the full limit, on a large page, with the agent busy in it, on builds that have the scan.

Not timed: the scan's duration on that page. It would take a build of the host and a signed-in console page. The fix does not depend on the figure: it takes `eval` and its scan out of the path.

## 5. Fix

A host route of its own, `POST /agentmux/browser/page_url`, answers the address from the browser's navigation history (`Page.getNavigationHistory`, the entry the page is on): no page script, no scan, one call. It is for srv's own checks only; the agent's `eval` is unchanged and keeps its scan. srv's `page_url()` calls it.

What stays the same: the address still comes from the browser, never from block meta, which a client can write; the grant is still made only if the origins the person was shown are still true, so a stale banner can't grant.

One difference from `location.href`: on a browser error page the history entry holds the address that failed to load, not `chrome-error://…`. A grant there would name the site that failed, which is what the person was shown.

## 6. Tests

- srv: the button's grant succeeds while the fake host's `eval` refuses the way the real one does on a page with a secret (`bypass_is_granted_only_through_the_host_and_lifts_approvals_on_its_site`). Before the fix it answered 502 "couldn't read the page's address".
- window: the address is the history entry the page is on, and an answer without an entry has no address.

## 7. Follow-up, not in this PR

The window waits ten seconds for srv on the other host calls too. If a call is slow for its own reasons, the person sees the timeout wording from #6080 and not a hang, which is the right outcome; no other call is known to be slow today.
